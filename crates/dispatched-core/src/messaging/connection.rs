//! One AMQP connection per namespace: connection + session + CBS client,
//! plus cached management links per entity address.

use super::lockops::{
    CANCEL_SCHEDULED, SCHEDULE, cancel_scheduled_request, parse_schedule_response,
    parse_status_only, schedule_request,
};
use super::management::map_sender_attach_error;
use super::management::{ManagementLink, SETUP_TIMEOUT, map_put_token_error};
use super::outgoing::OutgoingMessage;
use super::receiver::{AmqpReceiver, ReceiverSpec, SessionChoice};
use super::sender::{AmqpSender, Chunked, encode_for_schedule, send_target};
use super::tokens_ledger::{LinkLru, TokenLedger, must_reattach};
use crate::namespace::Namespace;
use crate::{Error, Result};
use fe2o3_amqp::connection::ConnectionHandle;
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::primitives::Timestamp;
use fe2o3_amqp::{Sender, Session};
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
    /// Connect (TLS or WebSockets for the cloud, plain TCP for the emulator), begin a
    /// session, and attach the CBS node.
    pub async fn open(ns: &Namespace) -> Result<Self> {
        let mut connection = super::transport::open_connection(
            ns,
            format!("dispatched-{}", uuid::Uuid::new_v4().simple()),
        )
        .await?;
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
    /// re-put when missing or near expiry; a cached link whose token lapsed is
    /// closed and re-attached (the broker closes links when their token
    /// expires). At most 64 links are kept (LRU).
    pub async fn management(&mut self, address: &str) -> Result<&mut ManagementLink> {
        let mgmt_address = format!("{address}/$management");
        let audiences = [
            self.ns.amqp_audience(address),
            self.ns.amqp_audience(&mgmt_address),
        ];
        if self.management.contains_key(address)
            && must_reattach(&self.ledger, &audiences, OffsetDateTime::now_utc())
            && let Some(stale) = self.management.remove(address)
        {
            close_link(stale).await;
        }
        self.ensure_authorized(address).await?;
        self.ensure_authorized(&mgmt_address).await?;
        if !self.management.contains_key(address) {
            let link = ManagementLink::attach(&mut self.session, address).await?;
            self.management.insert(address.to_string(), link);
        }
        if let Some(evicted) = self.lru.touch(address)
            && let Some(old) = self.management.remove(&evicted)
        {
            close_link(old).await;
            self.ledger.forget(&self.ns.amqp_audience(&evicted));
            self.ledger
                .forget(&self.ns.amqp_audience(&format!("{evicted}/$management")));
        }
        Ok(self.management.get_mut(address).expect("inserted above"))
    }

    /// Open a receiver for `spec`: put tokens for the address and its
    /// `$management`, then attach the data link and a private management
    /// link on this connection's session.
    pub(crate) async fn open_receiver(&mut self, spec: &ReceiverSpec) -> Result<AmqpReceiver> {
        if spec.session == SessionChoice::NextAvailable {
            return Err(Error::Invalid(
                "a next-available session receiver is opened with try_accept_next, which can wait for a session".into(),
            ));
        }
        self.authorize_receiver(&spec.address).await?;
        AmqpReceiver::attach(&mut self.session, spec, None)
            .await?
            .ok_or_else(|| Error::Protocol(format!("attach receiver to {}", spec.address)))
    }

    /// Accept the next available session of `spec.address`, waiting at most
    /// `wait`; `Ok(None)` when no session is available. `spec.session` must
    /// be `None` or `NextAvailable` (a named session is opened with
    /// [`open_receiver`](Self::open_receiver)).
    pub(crate) async fn try_accept_next(
        &mut self,
        spec: &ReceiverSpec,
        wait: Duration,
    ) -> Result<Option<AmqpReceiver>> {
        if let SessionChoice::Id(_) = spec.session {
            return Err(Error::Invalid(
                "try_accept_next takes the next available session; open a named session with open_receiver".into(),
            ));
        }
        self.authorize_receiver(&spec.address).await?;
        let next = ReceiverSpec {
            session: SessionChoice::NextAvailable,
            ..spec.clone()
        };
        AmqpReceiver::attach(&mut self.session, &next, Some(wait)).await
    }

    /// Open a sender to a queue or topic `address`: put a token for it, then
    /// attach the data link. Subscription paths are refused before any I/O.
    pub(crate) async fn open_sender(&mut self, address: &str) -> Result<AmqpSender> {
        let target = send_target(address)?.base();
        self.ensure_authorized(&target).await?;
        let sender = tokio::time::timeout(
            SETUP_TIMEOUT,
            Sender::builder()
                .name(format!("{target}-tx-{}", uuid::Uuid::new_v4().simple()))
                .target(target.clone())
                .attach(&mut self.session),
        )
        .await
        .map_err(|_| Error::Timeout(format!("attach sender to {target} timed out")))?
        .map_err(|e| map_sender_attach_error(&target, &target, e))?;
        Ok(AmqpSender::new(sender))
    }

    /// Schedule `batch` on `address` for `at` through the entity's `$management`
    /// link, 100 messages per request. `each(index, Ok(sequence_number))` reports
    /// every scheduled message; a message that cannot be encoded is reported
    /// `Err` for its index alone and the rest still go. If a request fails, its
    /// messages and every later one are reported with that error, which is also
    /// returned; earlier chunks stay scheduled and were already reported. A
    /// message without a message id gets a new UUID.
    pub(crate) async fn schedule(
        &mut self,
        address: &str,
        batch: &[OutgoingMessage],
        at: OffsetDateTime,
        mut each: impl FnMut(usize, Result<i64>),
    ) -> Result<()> {
        let target = send_target(address)?.base();
        let now = OffsetDateTime::now_utc();
        let entries = batch
            .iter()
            .map(|m| encode_for_schedule(m, at, now))
            .collect();
        let mut chunks = Chunked::new(entries, &mut each);
        while let Some(chunk) = chunks.next_chunk() {
            let outcome = async {
                let link = self.management(&target).await?;
                let resp = link.call(SCHEDULE, schedule_request(chunk)).await?;
                parse_schedule_response(resp)
            }
            .await;
            chunks.finish(outcome, &mut each)?;
        }
        Ok(())
    }

    /// Cancel scheduled messages by sequence number, 100 per request.
    /// `each(index, result)` reports every sequence number; a failed request
    /// reports its numbers and all later ones with the error, which is returned.
    pub(crate) async fn cancel_scheduled(
        &mut self,
        address: &str,
        seqs: &[i64],
        mut each: impl FnMut(usize, Result<()>),
    ) -> Result<()> {
        let target = send_target(address)?.base();
        let mut chunks = Chunked::new(seqs.iter().map(|s| Ok(*s)).collect(), &mut each);
        while let Some(chunk) = chunks.next_chunk() {
            let outcome = async {
                let link = self.management(&target).await?;
                let resp = link
                    .call(CANCEL_SCHEDULED, cancel_scheduled_request(chunk))
                    .await?;
                parse_status_only(resp, CANCEL_SCHEDULED)?;
                Ok(vec![(); chunk.len()])
            }
            .await;
            chunks.finish(outcome, &mut each)?;
        }
        Ok(())
    }

    async fn authorize_receiver(&mut self, address: &str) -> Result<()> {
        self.ensure_authorized(address).await?;
        self.ensure_authorized(&format!("{address}/$management"))
            .await
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

/// Close a link, bounded so an unresponsive peer cannot stall the caller.
async fn close_link(link: ManagementLink) {
    if tokio::time::timeout(CLOSE_TIMEOUT, link.close())
        .await
        .is_err()
    {
        tracing::warn!("closing a management link timed out; dropped it");
    }
}
