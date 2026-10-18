//! The message plane the operations run on: sending, scheduling and lock receivers.
//!
//! `pub(crate)` on purpose: every mutating call goes through `ops::Operator` and its
//! safety gate, so no surface can reach these traits directly. Two implementations:
//! `explorer::ExplorerPlane` (the cached AMQP connection) and, in tests,
//! `messaging::fake::FakeBroker`.

// The operations (Task 12 on) are the callers; until then only the fake's tests use this.
#![allow(dead_code)]

use super::outgoing::OutgoingMessage;
use super::receiver::{AmqpReceiver, LockToken, ReceivedMessage, ReceiverSpec, Settlement};
use super::scan::PeekSource;
use super::sender::AmqpSender;
use crate::mgmt::EntityCounts;
use crate::namespace::EntityPath;
use crate::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;
use time::OffsetDateTime;

/// A per-index result callback for batch calls (`each(index, result)`).
pub(crate) type Each<'a, R> = &'a mut (dyn FnMut(usize, Result<R>) + Send);

/// Links and management calls on one namespace. Object safe; the jobs take
/// `&mut dyn MessagingBackend`.
#[async_trait]
pub(crate) trait MessagingBackend: PeekSource + Send {
    /// A sender to the queue or topic `address`.
    async fn open_sender(&mut self, address: &str) -> Result<Box<dyn MessageSender>>;

    /// Schedule `batch` on `address` for `at`; `each(index, Ok(sequence_number))` per
    /// message. A failed request reports its messages and all later ones with the
    /// error, which is also returned.
    async fn schedule(
        &mut self,
        address: &str,
        batch: &[OutgoingMessage],
        at: OffsetDateTime,
        each: Each<'_, i64>,
    ) -> Result<()>;

    /// Cancel scheduled messages by sequence number; `each(index, result)` per number.
    async fn cancel_scheduled(
        &mut self,
        address: &str,
        seqs: &[i64],
        each: Each<'_, ()>,
    ) -> Result<()>;

    /// A receiver for `spec`. `Ok(None)` only for `SessionChoice::NextAvailable` when no
    /// session is available within `wait`.
    async fn open_receiver(
        &mut self,
        spec: &ReceiverSpec,
        wait: Duration,
    ) -> Result<Option<Box<dyn LockReceiver>>>;

    /// Management counts of `entity`; `None` when management cannot say.
    async fn entity_counts(&mut self, entity: &EntityPath) -> Result<Option<EntityCounts>>;

    /// Whether `entity` is session-enabled; `None` when management cannot say.
    async fn requires_session(&mut self, entity: &EntityPath) -> Result<Option<bool>>;

    /// Session ids of `address`: the broker's list merged with a peek tally.
    async fn session_ids(&mut self, address: &str) -> Result<Vec<String>>;
}

/// A sender link.
#[async_trait]
pub(crate) trait MessageSender: Send {
    /// Send `batch` in order; `each(index, result)` per message. A link failure fails
    /// every message not yet reported and is returned.
    async fn send_all(&mut self, batch: &[OutgoingMessage], each: Each<'_, ()>) -> Result<()>;

    /// Detach the link.
    async fn close(self: Box<Self>);
}

/// A receiver link (peek-lock or receive-and-delete, with or without a session).
#[async_trait]
pub(crate) trait LockReceiver: Send {
    /// The link name.
    fn name(&self) -> &str;
    /// The accepted session, for session receivers.
    fn session_id(&self) -> Option<&str>;
    /// Receive up to `max` messages within `wait` (plus the drain grace time). Cancel
    /// safe: messages taken while a dropped call ran are returned by the next one.
    async fn receive(&mut self, max: u32, wait: Duration) -> Result<Vec<ReceivedMessage>>;
    /// Credit unaccounted for when the last receive ended.
    fn drain_shortfall(&self) -> u32;
    /// Messages taken off the link but not yet returned.
    fn buffered(&self) -> usize;
    /// A failure the last receive held back while it returned what it had taken (the next
    /// receive would report it); taking it clears it.
    fn take_error(&mut self) -> Option<Error>;
    /// Settle a peek-lock message; a lost lock is `LockLost`.
    async fn settle(&mut self, token: LockToken, how: &Settlement) -> Result<()>;
    /// Unlock a held message without raising its delivery count.
    async fn release(&mut self, token: LockToken) -> Result<()>;
    /// Release everything held; returns how many were released.
    async fn release_all(&mut self) -> Result<u64>;
    /// How many messages this receiver holds locked.
    fn held(&self) -> usize;
    /// Renew the locks of `tokens`; the new expirations in order.
    async fn renew_lock(&mut self, tokens: &[LockToken]) -> Result<Vec<OffsetDateTime>>;
    /// Receive deferred messages by sequence number, locked. Deferred messages received
    /// here are not held: settle each one, or its lock expires.
    async fn receive_deferred(&mut self, seqs: &[i64]) -> Result<Vec<ReceivedMessage>>;
    /// The accepted session's state.
    async fn session_state(&mut self) -> Result<Option<Vec<u8>>>;
    /// Set (or with `None` clear) the accepted session's state.
    async fn set_session_state(&mut self, state: Option<&[u8]>) -> Result<()>;
    /// Renew the session lock; its new expiration.
    async fn renew_session_lock(&mut self) -> Result<OffsetDateTime>;
    /// Release what is held, then close. Returns how many receive-and-delete messages
    /// were removed from the entity but never returned (callers report them).
    async fn close(self: Box<Self>) -> usize;
}

#[async_trait]
impl MessageSender for AmqpSender {
    async fn send_all(&mut self, batch: &[OutgoingMessage], each: Each<'_, ()>) -> Result<()> {
        AmqpSender::send_all(self, batch, each).await
    }

    async fn close(self: Box<Self>) {
        AmqpSender::close(*self).await
    }
}

#[async_trait]
impl LockReceiver for AmqpReceiver {
    fn name(&self) -> &str {
        AmqpReceiver::name(self)
    }
    fn session_id(&self) -> Option<&str> {
        AmqpReceiver::session_id(self)
    }
    async fn receive(&mut self, max: u32, wait: Duration) -> Result<Vec<ReceivedMessage>> {
        AmqpReceiver::receive(self, max, wait).await
    }
    fn drain_shortfall(&self) -> u32 {
        AmqpReceiver::drain_shortfall(self)
    }
    fn buffered(&self) -> usize {
        AmqpReceiver::buffered(self)
    }
    fn take_error(&mut self) -> Option<Error> {
        AmqpReceiver::take_error(self)
    }
    async fn settle(&mut self, token: LockToken, how: &Settlement) -> Result<()> {
        AmqpReceiver::settle(self, token, how).await
    }
    async fn release(&mut self, token: LockToken) -> Result<()> {
        AmqpReceiver::release(self, token).await
    }
    async fn release_all(&mut self) -> Result<u64> {
        AmqpReceiver::release_all(self).await
    }
    fn held(&self) -> usize {
        AmqpReceiver::held(self)
    }
    async fn renew_lock(&mut self, tokens: &[LockToken]) -> Result<Vec<OffsetDateTime>> {
        AmqpReceiver::renew_lock(self, tokens).await
    }
    async fn receive_deferred(&mut self, seqs: &[i64]) -> Result<Vec<ReceivedMessage>> {
        AmqpReceiver::receive_deferred(self, seqs).await
    }
    async fn session_state(&mut self) -> Result<Option<Vec<u8>>> {
        AmqpReceiver::session_state(self).await
    }
    async fn set_session_state(&mut self, state: Option<&[u8]>) -> Result<()> {
        AmqpReceiver::set_session_state(self, state).await
    }
    async fn renew_session_lock(&mut self) -> Result<OffsetDateTime> {
        AmqpReceiver::renew_session_lock(self).await
    }
    async fn close(self: Box<Self>) -> usize {
        AmqpReceiver::close(*self).await
    }
}
