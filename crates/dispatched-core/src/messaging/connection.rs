//! One AMQP connection per namespace: connection + session + CBS client,
//! plus cached management links per entity address.

use super::management::{ManagementLink, SETUP_TIMEOUT, map_put_token_error};
use super::tokens_ledger::{LinkLru, TokenLedger};
use crate::namespace::Namespace;
use crate::{Error, Result};
use fe2o3_amqp::connection::ConnectionHandle;
use fe2o3_amqp::sasl_profile::SaslProfile;
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::primitives::Timestamp;
use fe2o3_amqp::{Connection, Session};
use fe2o3_amqp_cbs::client::CbsClient;
use fe2o3_amqp_cbs::token::CbsToken;
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::time::Duration;
use time::OffsetDateTime;

/// Upper bound for the whole close sequence; an unresponsive peer must not hang callers.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Management links kept per connection; the least recently used is closed beyond this.
const MAX_MANAGEMENT_LINKS: usize = 64;

/// An open AMQP connection to a Service Bus namespace (or the emulator).
pub struct ServiceBusConnection {
    ns: Namespace,
    connection: ConnectionHandle<()>,
    session: SessionHandle<()>,
    cbs: CbsClient,
    management: HashMap<String, ManagementLink>,
    ledger: TokenLedger,
    lru: LinkLru,
}

impl ServiceBusConnection {
    /// Connect (TLS for the cloud, plain TCP for the emulator), begin a
    /// session, and attach the CBS node.
    pub async fn open(ns: &Namespace) -> Result<Self> {
        let url = ns.amqp_url();
        let builder = Connection::builder()
            .container_id(format!("dispatched-{}", uuid::Uuid::new_v4().simple()))
            .hostname(Some(ns.host()))
            .sasl_profile(SaslProfile::Anonymous)
            .max_frame_size(65_536u32)
            .idle_time_out(60_000u32);
        let builder = if ns.is_emulator() {
            builder
        } else {
            builder.alt_tls_establishment(true)
        };
        let mut connection = tokio::time::timeout(SETUP_TIMEOUT, builder.open(url.as_str()))
            .await
            .map_err(|_| Error::Timeout(format!("connecting to {url} timed out")))?
            .map_err(|e| Error::Network(format!("cannot connect to {url}: {e}")))?;
        let mut session = tokio::time::timeout(SETUP_TIMEOUT, Session::begin(&mut connection))
            .await
            .map_err(|_| Error::Timeout("beginning the AMQP session timed out".into()))?
            .map_err(|e| Error::Protocol(format!("begin AMQP session: {e}")))?;
        let cbs = tokio::time::timeout(SETUP_TIMEOUT, CbsClient::attach(&mut session))
            .await
            .map_err(|_| Error::Timeout("attaching $cbs timed out".into()))?
            .map_err(|e| Error::Protocol(format!("attach $cbs: {e}")))?;
        Ok(Self {
            ns: ns.clone(),
            connection,
            session,
            cbs,
            management: HashMap::new(),
            ledger: TokenLedger::default(),
            lru: LinkLru::new(MAX_MANAGEMENT_LINKS),
        })
    }

    /// Put a token for `address` (e.g. `orders/$DeadLetterQueue`) on the CBS node.
    pub async fn authorize(&mut self, address: &str) -> Result<()> {
        let audience = self.ns.amqp_audience(address);
        let token = self.ns.tokens().token(&audience).await?;
        let expires_on = token.expires_on;
        let cbs_token = CbsToken::new(
            token.token.expose_secret().to_string(),
            token.kind.cbs_type(),
            Some(Timestamp::from(token.expires_on)),
        );
        tokio::time::timeout(
            SETUP_TIMEOUT,
            self.cbs.put_token(audience.clone(), cbs_token),
        )
        .await
        .map_err(|_| Error::Timeout(format!("putting the token for {audience} timed out")))?
        .map_err(|e| map_put_token_error(&audience, address, e))?;
        self.ledger.record(&audience, expires_on);
        Ok(())
    }

    /// Put a token for `address` unless a fresh one is already in place.
    pub async fn ensure_authorized(&mut self, address: &str) -> Result<()> {
        let audience = self.ns.amqp_audience(address);
        if self.ledger.needs_put(&audience, OffsetDateTime::now_utc()) {
            self.authorize(address).await?;
        }
        Ok(())
    }

    /// The management link for `address`, attached on first use. Tokens are
    /// re-put when missing or near expiry; at most 64 links are kept (LRU).
    pub async fn management(&mut self, address: &str) -> Result<&mut ManagementLink> {
        self.ensure_authorized(address).await?;
        self.ensure_authorized(&format!("{address}/$management"))
            .await?;
        if !self.management.contains_key(address) {
            let link = ManagementLink::attach(&mut self.session, address).await?;
            self.management.insert(address.to_string(), link);
        }
        if let Some(evicted) = self.lru.touch(address)
            && let Some(old) = self.management.remove(&evicted)
        {
            old.close().await;
            self.ledger.forget(&self.ns.amqp_audience(&evicted));
            self.ledger
                .forget(&self.ns.amqp_audience(&format!("{evicted}/$management")));
        }
        Ok(self.management.get_mut(address).expect("inserted above"))
    }

    /// The AMQP session (for data links).
    pub fn session_mut(&mut self) -> &mut SessionHandle<()> {
        &mut self.session
    }

    /// Close links, session and connection, bounded by a 5 s overall timeout.
    /// Closing is best effort: if the peer does not answer in time, the
    /// half-closed handles are dropped (which tears the transport down) and
    /// this returns `Ok(())` rather than `Error::Timeout`, since callers only
    /// close to get rid of the connection.
    pub async fn close(mut self) -> Result<()> {
        let sequence = async {
            for (_, link) in self.management.drain() {
                link.close().await;
            }
            let _ = self.cbs.close().await;
            let _ = self.session.end().await;
            self.connection
                .close()
                .await
                .map_err(|e| Error::Network(format!("close AMQP connection: {e}")))
        };
        match tokio::time::timeout(CLOSE_TIMEOUT, sequence).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!("closing the AMQP connection timed out; dropped it");
                Ok(())
            }
        }
    }
}
