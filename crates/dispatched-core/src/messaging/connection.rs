//! One AMQP connection per namespace: connection + session + CBS client,
//! plus cached management links per entity address.

use super::management::{ManagementLink, SETUP_TIMEOUT};
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

/// An open AMQP connection to a Service Bus namespace (or the emulator).
pub struct ServiceBusConnection {
    ns: Namespace,
    connection: ConnectionHandle<()>,
    session: SessionHandle<()>,
    cbs: CbsClient,
    management: HashMap<String, ManagementLink>,
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
        })
    }

    /// Put a token for `address` (e.g. `orders/$DeadLetterQueue`) on the CBS node.
    pub async fn authorize(&mut self, address: &str) -> Result<()> {
        let audience = self.ns.amqp_audience(address);
        let token = self.ns.tokens().token(&audience).await?;
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
        .map_err(|e| Error::Auth(format!("token for {audience} was rejected: {e}")))
    }

    /// The management link for `address`, attached and authorized on first use.
    pub async fn management(&mut self, address: &str) -> Result<&mut ManagementLink> {
        if !self.management.contains_key(address) {
            self.authorize(address).await?;
            self.authorize(&format!("{address}/$management")).await?;
            let link = ManagementLink::attach(&mut self.session, address).await?;
            self.management.insert(address.to_string(), link);
        }
        Ok(self.management.get_mut(address).expect("inserted above"))
    }

    /// The AMQP session (for data links).
    pub fn session_mut(&mut self) -> &mut SessionHandle<()> {
        &mut self.session
    }

    /// Close links, session and connection.
    pub async fn close(mut self) -> Result<()> {
        for (_, link) in self.management.drain() {
            link.close().await;
        }
        let _ = self.cbs.close().await;
        let _ = self.session.end().await;
        self.connection
            .close()
            .await
            .map_err(|e| Error::Network(format!("close AMQP connection: {e}")))
    }
}
