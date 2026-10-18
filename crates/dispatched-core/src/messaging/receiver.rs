//! AMQP receiver links: peek-lock and receive-and-delete, with bounded credit,
//! settlement through `com.microsoft:update-disposition`, and session receivers.
//!
//! Protocol notes (fe2o3-amqp 0.18):
//! - Link dispositions are fire-and-forget (the broker's answer is dropped), so
//!   every settlement that must report an outcome goes through
//!   `update-disposition` on the receiver's private management link. Only
//!   `release` stays on the link: its failure is harmless, the lock expires.
//! - `Receiver::drain()` only sends a flow with `drain = true` and nothing
//!   reports the drain finishing, so after it the receiver reads until its
//!   credit is spent or [`DRAIN_GRACE`] passes without a delivery.

use super::lockops::{
    self, DispositionStatus, GET_SESSION_STATE, RECEIVE_BY_SEQUENCE, RENEW_LOCK,
    RENEW_SESSION_LOCK, SET_SESSION_STATE, UPDATE_DISPOSITION, update_disposition_request,
};
use super::management::{
    ManagementLink, SETUP_TIMEOUT, map_receiver_attach_error, next_available_outcome,
};
use super::message::{PeekedMessage, decode_message};
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::definitions::{
    Error as AmqpError, ErrorCondition, Fields, ReceiverSettleMode, SenderSettleMode,
};
use fe2o3_amqp::types::messaging::{Body, FilterSet, Modified, Source};
use fe2o3_amqp::types::primitives::{OrderedMap, Symbol, Value};
use fe2o3_amqp::{Delivery, Receiver};
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::time::Instant;
use uuid::Uuid;

/// How long a drain waits for the broker after `wait` passed with credit outstanding.
pub(crate) const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Credit granted at most per flow.
const CREDIT_CHUNK: u32 = 50;

/// Upper bound for closing a link; an unresponsive peer must not hang callers.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `close` waits for late deliveries before closing.
const CLOSE_SWEEP_IDLE: Duration = Duration::from_millis(200);

/// How a receiver settles what it receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveMode {
    /// Messages are locked until settled.
    PeekLock,
    /// Messages are removed as they are delivered.
    ReceiveAndDelete,
}

/// Which session a receiver accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionChoice {
    /// Not a session receiver.
    None,
    /// That session.
    Id(String),
    /// Whichever session the broker hands out next.
    NextAvailable,
}

/// What to receive from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverSpec {
    /// Entity address (`orders`, `orders/$DeadLetterQueue`, ...).
    pub address: String,
    /// Peek-lock or receive-and-delete.
    pub mode: ReceiveMode,
    /// Session to accept, if any.
    pub session: SessionChoice,
}

impl ReceiverSpec {
    /// A peek-lock receiver without a session.
    pub fn peek_lock(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            mode: ReceiveMode::PeekLock,
            session: SessionChoice::None,
        }
    }

    /// A receive-and-delete receiver without a session.
    pub fn receive_and_delete(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            mode: ReceiveMode::ReceiveAndDelete,
            session: SessionChoice::None,
        }
    }
}

/// The lock token of a peek-lock delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LockToken(pub Uuid);

impl serde::Serialize for LockToken {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

/// The lock token in a delivery tag: 16 bytes read as a .NET GUID.
pub fn lock_token_from_tag(tag: &[u8]) -> Option<LockToken> {
    let bytes: [u8; 16] = tag.try_into().ok()?;
    Some(LockToken(Uuid::from_bytes_le(bytes)))
}

/// A received message and, in peek-lock mode, its lock token.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReceivedMessage {
    /// The message.
    pub message: PeekedMessage,
    /// The lock token; `None` for receive-and-delete.
    pub lock_token: Option<LockToken>,
}

/// How to settle a peek-lock delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    /// Remove the message.
    Complete,
    /// Unlock it; its delivery count goes up.
    Abandon,
    /// Defer it; it is then received by sequence number only.
    Defer,
    /// Move it to the dead-letter queue.
    DeadLetter {
        /// `DeadLetterReason`; not set when `None`.
        reason: Option<String>,
        /// `DeadLetterErrorDescription`; not set when `None`.
        description: Option<String>,
    },
}

/// Credit for one grant: never above what is still wanted.
pub(crate) fn credit_for(remaining: u64, chunk: u32) -> u32 {
    u32::try_from(remaining.min(u64::from(chunk))).unwrap_or(chunk)
}

/// The credit decisions of one `receive` call. Credit is set absolutely on
/// the link, so a grant is the whole outstanding credit; a new grant is made
/// only when the previous one was used up, so outstanding credit never
/// exceeds what is still wanted.
#[derive(Debug)]
struct CreditPlan {
    wanted: u64,
    chunk: u32,
    delivered: u64,
    outstanding: u32,
    outstanding_max: u32,
}

impl CreditPlan {
    fn new(wanted: u64, chunk: u32) -> Self {
        Self {
            wanted,
            chunk,
            delivered: 0,
            outstanding: 0,
            outstanding_max: 0,
        }
    }

    /// The credit to set now (0: nothing more is wanted).
    fn next_grant(&mut self) -> u32 {
        let grant = credit_for(self.wanted.saturating_sub(self.delivered), self.chunk);
        self.outstanding = grant;
        self.outstanding_max = self.outstanding_max.max(grant);
        grant
    }

    fn delivered(&mut self, n: u32) {
        self.delivered += u64::from(n);
        self.outstanding = self.outstanding.saturating_sub(n);
    }

    #[cfg(test)]
    fn outstanding_max(&self) -> u32 {
        self.outstanding_max
    }

    /// Whether a drain is over: every granted credit was used (nothing more
    /// can come), or the grace time passed without a delivery.
    fn drain_done(&self, credit_left: u32, since_last_delivery: Duration) -> bool {
        credit_left == 0 || since_last_delivery >= DRAIN_GRACE
    }

    /// Credit still outstanding when a drain ended, never above what this
    /// plan granted and was not delivered.
    fn shortfall(&self, credit_left: u32) -> u32 {
        credit_left.min(self.outstanding)
    }
}

const SESSION_FILTER: &str = "com.microsoft:session-filter";
/// Attach property: how long the broker waits for a next-available session.
const TIMEOUT_PROPERTY: &str = "com.microsoft:timeout";

/// The source filter for a session choice: the session id, or null for the next available
/// session; `None` without sessions. Plain values, as the Azure SDKs send them: the broker's
/// gateway reads a described filter value as a selector and refuses the attach ("System only
/// support Jms or Apache selector filter type").
fn session_filter(choice: &SessionChoice) -> Option<FilterSet> {
    let value = match choice {
        SessionChoice::None => return None,
        SessionChoice::Id(id) => Value::String(id.clone()),
        SessionChoice::NextAvailable => Value::Null,
    };
    let mut filter = FilterSet::new();
    filter.insert(Symbol::from(SESSION_FILTER), value);
    Some(filter)
}

/// The session the broker attached the link to, from the remote source filter.
fn accepted_session(source: Option<&Source>) -> Option<String> {
    let value = source?
        .filter
        .as_ref()?
        .get(&Symbol::from(SESSION_FILTER))?;
    let inner = match value {
        Value::Described(d) => &d.value,
        other => other,
    };
    match inner {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// A link-level settlement outcome.
#[cfg_attr(not(feature = "test-support"), allow(dead_code))] // only the emulator probe settles on the link
#[derive(Debug)]
enum LinkOutcome {
    Accepted,
    Modified {
        delivery_failed: Option<bool>,
        undeliverable_here: Option<bool>,
    },
    Rejected(AmqpError),
}

#[cfg_attr(not(feature = "test-support"), allow(dead_code))] // only the emulator probe settles on the link
fn outcome_for(how: &Settlement) -> LinkOutcome {
    match how {
        Settlement::Complete => LinkOutcome::Accepted,
        Settlement::Abandon => LinkOutcome::Modified {
            delivery_failed: Some(true),
            undeliverable_here: Some(false),
        },
        Settlement::Defer => LinkOutcome::Modified {
            delivery_failed: None,
            undeliverable_here: Some(true),
        },
        Settlement::DeadLetter {
            reason,
            description,
        } => {
            // Only the values given: an absent reason is not sent as an empty one.
            let mut info = Fields::default();
            if let Some(reason) = reason {
                info.insert(
                    Symbol::from("DeadLetterReason"),
                    Value::from(reason.as_str()),
                );
            }
            if let Some(description) = description {
                info.insert(
                    Symbol::from("DeadLetterErrorDescription"),
                    Value::from(description.as_str()),
                );
            }
            LinkOutcome::Rejected(AmqpError::new(
                ErrorCondition::Custom(Symbol::from("com.microsoft:dead-letter")),
                None,
                (!info.is_empty()).then_some(info),
            ))
        }
    }
}

/// The `update-disposition` request `settle` sends.
fn disposition_call(
    token: LockToken,
    how: &Settlement,
    session: Option<&str>,
) -> (&'static str, OrderedMap<String, Value>) {
    let (status, dead_letter) = match how {
        Settlement::Complete => (DispositionStatus::Completed, None),
        Settlement::Abandon => (DispositionStatus::Abandoned, None),
        Settlement::Defer => (DispositionStatus::Defered, None),
        Settlement::DeadLetter {
            reason,
            description,
        } => (
            DispositionStatus::Suspended,
            Some((reason.as_deref(), description.as_deref())),
        ),
    };
    (
        UPDATE_DISPOSITION,
        update_disposition_request(&[token.0], status, dead_letter, session),
    )
}

/// Run one network step bounded by `limit`; failures read `{what}: {error}`.
async fn bounded<T, E: Display>(
    what: &str,
    limit: Duration,
    fut: impl Future<Output = std::result::Result<T, E>>,
) -> Result<T> {
    match tokio::time::timeout(limit, fut).await {
        Err(_) => Err(Error::Timeout(format!(
            "{what} took longer than {}s",
            limit.as_secs()
        ))),
        Ok(r) => r.map_err(|e| Error::Protocol(format!("{what}: {e}"))),
    }
}

fn bad_tag() -> Error {
    Error::Protocol("a peek-lock delivery carried no 16-byte lock token; it was released".into())
}

fn not_held() -> Error {
    Error::Invalid(
        "this receiver holds no message with that lock token; it was settled, released or never received here".into(),
    )
}

/// What a receiver has taken off the link but not yet handed to a caller,
/// plus the shortfall and failure of the last `receive`. All of a `receive`'s
/// bookkeeping lives here, so a `receive` future dropped half-way (a `select!`
/// with a cancel token) loses nothing: its messages are returned by the next
/// call, or counted by `close`.
#[derive(Debug, Default)]
struct Intake {
    ready: VecDeque<ReceivedMessage>,
    shortfall: u32,
    /// A failure after some messages were taken; those are returned first and
    /// the failure is reported by the next call (whatever kind it is).
    pending_error: Option<Error>,
}

impl Intake {
    /// What a `receive` of `max` returns without touching the link: buffered
    /// messages behind a pending failure, or the failure itself.
    fn settled_before(&mut self, max: u32) -> Option<Result<Vec<ReceivedMessage>>> {
        let e = self.pending_error.take()?;
        if self.ready.is_empty() {
            Some(Err(e))
        } else {
            // The failure stays pending until everything buffered was returned.
            self.pending_error = Some(e);
            Some(Ok(self.take(max)))
        }
    }

    /// Start a `receive` of up to `max`: buffered messages count toward it.
    fn begin(&mut self, max: u32) -> CreditPlan {
        self.shortfall = 0;
        let buffered = u64::try_from(self.ready.len()).unwrap_or(u64::MAX);
        CreditPlan::new(u64::from(max).saturating_sub(buffered), CREDIT_CHUNK)
    }

    /// Credit was granted: until the receive ends, all of it counts as shortfall.
    fn granted(&mut self, plan: &CreditPlan) {
        self.shortfall = plan.outstanding;
    }

    /// A delivery came off the link (`None`: one that cannot be returned).
    fn arrived(&mut self, plan: &mut CreditPlan, message: Option<ReceivedMessage>) {
        plan.delivered(1);
        self.ready.extend(message);
        self.shortfall = plan.outstanding;
    }

    /// The receive failed: credit still out is the shortfall; buffered
    /// messages are returned and the failure waits for the next call.
    fn stop(&mut self, plan: &CreditPlan, e: Error, max: u32) -> Result<Vec<ReceivedMessage>> {
        self.shortfall = plan.outstanding;
        if self.ready.is_empty() {
            Err(e)
        } else {
            tracing::warn!(error = %e, "receive stopped early; returning what arrived");
            self.pending_error = Some(e);
            Ok(self.take(max))
        }
    }

    /// The receive ended normally with `credit_left` (0 when it was spent).
    fn finish(&mut self, plan: &CreditPlan, credit_left: u32, max: u32) -> Vec<ReceivedMessage> {
        self.shortfall = plan.shortfall(credit_left);
        self.take(max)
    }

    fn take(&mut self, max: u32) -> Vec<ReceivedMessage> {
        let n = self.ready.len().min(max as usize);
        self.ready.drain(..n).collect()
    }

    /// Drop a buffered peek-lock message (it was settled or released).
    fn forget(&mut self, token: LockToken) {
        self.ready.retain(|m| m.lock_token != Some(token));
    }
}

/// A receiver link with its own management link on the same session.
pub(crate) struct AmqpReceiver {
    #[allow(dead_code)] // read by `address()`, which no operation needs yet
    address: String,
    mode: ReceiveMode,
    receiver: Receiver,
    management: ManagementLink,
    session_id: Option<String>,
    held: HashMap<LockToken, Delivery<Body<Value>>>,
    /// Peek-lock deliveries without a usable lock token, released at the next chance.
    unkeyed: Vec<Delivery<Body<Value>>>,
    intake: Intake,
}

impl AmqpReceiver {
    /// Attach the management link, then the data link. With `next_wait`
    /// (next-available session) a timeout means no session: `Ok(None)`.
    pub(crate) async fn attach(
        session: &mut SessionHandle<()>,
        spec: &ReceiverSpec,
        next_wait: Option<Duration>,
    ) -> Result<Option<Self>> {
        let management = ManagementLink::attach(session, &spec.address).await?;
        let mut source = Source::builder().address(spec.address.clone());
        if let Some(filter) = session_filter(&spec.session) {
            source = source.filter(filter);
        }
        let mut builder = Receiver::builder()
            .name(format!("dispatched-rx-{}", Uuid::new_v4().simple()))
            .source(source.build())
            .credit_mode(CreditMode::Manual);
        builder = match spec.mode {
            ReceiveMode::PeekLock => builder.receiver_settle_mode(ReceiverSettleMode::Second),
            ReceiveMode::ReceiveAndDelete => builder.sender_settle_mode(SenderSettleMode::Settled),
        };
        if spec.session != SessionChoice::None {
            // The broker answers with the session it chose; accept it.
            builder = builder.verify_incoming_source(false);
        }
        if let Some(wait) = next_wait {
            let mut props = Fields::default();
            props.insert(
                Symbol::from(TIMEOUT_PROPERTY),
                Value::Uint(u32::try_from(wait.as_millis()).unwrap_or(u32::MAX)),
            );
            builder = builder.properties(props);
        }
        let limit = next_wait.map_or(SETUP_TIMEOUT, |w| w + SETUP_TIMEOUT);
        let node = &spec.address;
        let attached = match tokio::time::timeout(limit, builder.attach(session)).await {
            Err(_) => Err(Error::Timeout(format!(
                "attach receiver to {node} took longer than {}s",
                limit.as_secs()
            ))),
            Ok(r) => r.map_err(|e| map_receiver_attach_error(node, node, e)),
        };
        let attached = attached.map_err(|e| match (e, &spec.session) {
            (Error::Conflict(_), SessionChoice::Id(id)) => Error::Conflict(format!(
                "session {id} is locked by another receiver; stop that receiver or wait for its lock to expire"
            )),
            (e, _) => e,
        });
        let attached = match next_wait {
            Some(_) => next_available_outcome(attached),
            None => attached.map(Some),
        };
        let receiver = match attached {
            Ok(Some(r)) => r,
            Ok(None) => {
                close_management(management).await;
                return Ok(None);
            }
            Err(e) => {
                close_management(management).await;
                return Err(e);
            }
        };
        let session_id = match &spec.session {
            SessionChoice::None => None,
            SessionChoice::Id(id) => {
                Some(accepted_session(receiver.source().as_ref()).unwrap_or_else(|| id.clone()))
            }
            SessionChoice::NextAvailable => match accepted_session(receiver.source().as_ref()) {
                Some(id) => Some(id),
                None => {
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, receiver.close()).await;
                    close_management(management).await;
                    return Err(Error::Protocol(format!(
                        "attach receiver to {node}: the broker accepted a session receiver without naming the session"
                    )));
                }
            },
        };
        Ok(Some(Self {
            address: spec.address.clone(),
            mode: spec.mode,
            receiver,
            management,
            session_id,
            held: HashMap::new(),
            unkeyed: Vec::new(),
            intake: Intake::default(),
        }))
    }

    /// The data link's name (sent as `associated-link-name`).
    pub(crate) fn name(&self) -> &str {
        self.receiver.name()
    }

    /// The entity address.
    #[allow(dead_code)] // no operation needs it yet
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// The accepted session, for session receivers.
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Receive up to `max` messages, waiting at most `wait` (plus the drain
    /// grace time when credit is still outstanding then). Messages buffered
    /// by an earlier call (one that failed or was cancelled) come first and
    /// count toward `max`.
    ///
    /// Cancel-safe: every delivery goes into the receiver's buffer as it
    /// arrives and leaves it only on return, so dropping this future loses
    /// nothing; [`drain_shortfall`](Self::drain_shortfall) then still counts
    /// the credit that was out.
    pub(crate) async fn receive(
        &mut self,
        max: u32,
        wait: Duration,
    ) -> Result<Vec<ReceivedMessage>> {
        if let Some(out) = self.intake.settled_before(max) {
            return out;
        }
        let mut plan = self.intake.begin(max);
        let deadline = Instant::now() + wait;
        loop {
            if plan.outstanding == 0 {
                let grant = plan.next_grant();
                if grant == 0 {
                    return Ok(self.intake.finish(&plan, 0, max));
                }
                self.intake.granted(&plan);
                if let Err(e) = bounded(
                    "grant credit",
                    SETUP_TIMEOUT,
                    self.receiver.set_credit(grant),
                )
                .await
                {
                    return self.stop(&plan, e, max).await;
                }
            }
            match tokio::time::timeout_at(deadline, self.receiver.recv::<Body<Value>>()).await {
                Err(_) => break,
                Ok(Err(e)) => {
                    return self
                        .stop(&plan, Error::Protocol(format!("receive: {e}")), max)
                        .await;
                }
                Ok(Ok(delivery)) => {
                    let taken = self.take(delivery);
                    let failed = taken.is_err();
                    self.intake.arrived(&mut plan, taken.ok());
                    if failed {
                        return self.stop(&plan, bad_tag(), max).await;
                    }
                }
            }
        }
        // `wait` passed with credit outstanding: drain, then read until the
        // credit is spent or the grace time passes without a delivery.
        if let Err(e) = bounded("drain", SETUP_TIMEOUT, self.receiver.drain()).await {
            return self.stop(&plan, e, max).await;
        }
        let mut last = Instant::now();
        loop {
            let left = self.receiver.credit().min(plan.outstanding);
            if plan.drain_done(left, last.elapsed()) {
                return Ok(self.intake.finish(&plan, left, max));
            }
            match tokio::time::timeout_at(last + DRAIN_GRACE, self.receiver.recv::<Body<Value>>())
                .await
            {
                Err(_) => {}
                Ok(Err(e)) => {
                    return self
                        .stop(&plan, Error::Protocol(format!("receive: {e}")), max)
                        .await;
                }
                Ok(Ok(delivery)) => {
                    last = Instant::now();
                    let taken = self.take(delivery);
                    let failed = taken.is_err();
                    self.intake.arrived(&mut plan, taken.ok());
                    if failed {
                        return self.stop(&plan, bad_tag(), max).await;
                    }
                }
            }
        }
    }

    /// End a `receive` early (see [`Intake::stop`]), releasing deliveries
    /// that carried no lock token.
    async fn stop(
        &mut self,
        plan: &CreditPlan,
        e: Error,
        max: u32,
    ) -> Result<Vec<ReceivedMessage>> {
        let out = self.intake.stop(plan, e, max);
        self.release_unkeyed().await;
        out
    }

    async fn release_unkeyed(&mut self) -> u64 {
        let mut released = 0;
        for delivery in std::mem::take(&mut self.unkeyed) {
            if bounded("release", SETUP_TIMEOUT, self.receiver.release(&delivery))
                .await
                .is_ok()
            {
                released += 1;
            }
        }
        released
    }

    /// Decode a delivery; a peek-lock one is held under its lock token. Not
    /// async, so nothing can be lost between the link and the buffer.
    fn take(
        &mut self,
        delivery: Delivery<Body<Value>>,
    ) -> std::result::Result<ReceivedMessage, ()> {
        match self.mode {
            ReceiveMode::ReceiveAndDelete => Ok(ReceivedMessage {
                message: decode_message(delivery.into_message()),
                lock_token: None,
            }),
            ReceiveMode::PeekLock => {
                let Some(token) = lock_token_from_tag(delivery.delivery_tag()) else {
                    self.unkeyed.push(delivery);
                    return Err(());
                };
                let message = decode_message(delivery.message().clone());
                self.held.insert(token, delivery);
                Ok(ReceivedMessage {
                    message,
                    lock_token: Some(token),
                })
            }
        }
    }

    /// Credit still outstanding when the last `receive` ended: by the drain
    /// grace time instead of spent credit, by a failure, or by being
    /// cancelled; 0 when all its credit was used. On a receive-and-delete
    /// link up to this many more messages may have been removed after the
    /// wait.
    pub(crate) fn drain_shortfall(&self) -> u32 {
        self.intake.shortfall
    }

    /// Messages taken off the link but not yet returned by `receive` (left
    /// by a failed or cancelled call). On a receive-and-delete link they are
    /// already removed from the entity.
    pub(crate) fn buffered(&self) -> usize {
        self.intake.ready.len()
    }

    /// A failure the last `receive` held back while returning what it had
    /// taken (see [`Intake::stop`]); taking it clears it, so the next
    /// `receive` does not report it again.
    pub(crate) fn take_error(&mut self) -> Option<Error> {
        self.intake.pending_error.take()
    }

    /// Settle a peek-lock message through `update-disposition` (the only
    /// settlement answer fe2o3 surfaces). A lost lock is `LockLost`. Any
    /// other failure (timeout, network) leaves the outcome unknown: the
    /// message may already be settled, so callers report it and never retry.
    pub(crate) async fn settle(&mut self, token: LockToken, how: &Settlement) -> Result<()> {
        if self.mode == ReceiveMode::ReceiveAndDelete {
            return Err(Error::Invalid(
                "a receive-and-delete receiver has nothing to settle".into(),
            ));
        }
        let (op, body) = disposition_call(token, how, self.session_id.as_deref());
        let name = self.receiver.name().to_string();
        let result = match self.management.call_for_link(op, body, Some(&name)).await {
            Ok(resp) => lockops::parse_status_only(resp, op),
            Err(e) => Err(e),
        };
        if matches!(result, Ok(()) | Err(Error::LockLost(_))) {
            // Settled (or gone) at the broker: drop it without a link disposition.
            self.held.remove(&token);
            self.intake.forget(token);
        }
        result
    }

    /// Settle over the link (fire-and-forget: no outcome is reported). Only
    /// for callers that ignore the outcome.
    #[cfg_attr(not(feature = "test-support"), allow(dead_code))] // only the emulator probe settles on the link
    pub(crate) async fn settle_on_link(
        &mut self,
        token: LockToken,
        how: &Settlement,
    ) -> Result<()> {
        let delivery = self.held.remove(&token).ok_or_else(not_held)?;
        self.intake.forget(token);
        let what = "settle on the link";
        match outcome_for(how) {
            LinkOutcome::Accepted => {
                bounded(what, SETUP_TIMEOUT, self.receiver.accept(&delivery)).await
            }
            LinkOutcome::Modified {
                delivery_failed,
                undeliverable_here,
            } => {
                bounded(
                    what,
                    SETUP_TIMEOUT,
                    self.receiver.modify(
                        &delivery,
                        Modified {
                            delivery_failed,
                            undeliverable_here,
                            message_annotations: None,
                        },
                    ),
                )
                .await
            }
            LinkOutcome::Rejected(e) => {
                bounded(what, SETUP_TIMEOUT, self.receiver.reject(&delivery, e)).await
            }
        }
    }

    /// Unlock a held message (AMQP `released`).
    pub(crate) async fn release(&mut self, token: LockToken) -> Result<()> {
        let delivery = self.held.remove(&token).ok_or_else(not_held)?;
        self.intake.forget(token);
        bounded("release", SETUP_TIMEOUT, self.receiver.release(&delivery)).await
    }

    /// Release every held message, buffered ones included; returns how many
    /// were released.
    pub(crate) async fn release_all(&mut self) -> Result<u64> {
        // Buffered peek-lock messages are held too; once released they must not be returned.
        self.intake.ready.retain(|m| m.lock_token.is_none());
        let mut released = self.release_unkeyed().await;
        let mut failed = 0u64;
        let mut first_error = None;
        for (_, delivery) in self.held.drain() {
            match bounded("release", SETUP_TIMEOUT, self.receiver.release(&delivery)).await {
                Ok(()) => released += 1,
                Err(e) => {
                    failed += 1;
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) if released == 0 => Err(e),
            Some(e) => {
                tracing::warn!(released, failed, error = %e, "some held messages could not be released; their locks expire");
                Ok(released)
            }
            None => Ok(released),
        }
    }

    /// How many messages this receiver holds locked.
    pub(crate) fn held(&self) -> usize {
        self.held.len()
    }

    /// Renew the locks of `tokens`; returns the new expirations in order.
    pub(crate) async fn renew_lock(&mut self, tokens: &[LockToken]) -> Result<Vec<OffsetDateTime>> {
        let uuids: Vec<Uuid> = tokens.iter().map(|t| t.0).collect();
        let mut body = lockops::renew_lock_request(&uuids);
        if let Some(s) = &self.session_id {
            body.insert("session-id".to_string(), Value::String(s.clone()));
        }
        let resp = self.call(RENEW_LOCK, body).await?;
        lockops::parse_renew_lock_response(resp)
    }

    /// Receive deferred messages by sequence number (peek-lock). Their locks
    /// are not held on the link; settle them with [`settle`](Self::settle).
    pub(crate) async fn receive_deferred(&mut self, seqs: &[i64]) -> Result<Vec<ReceivedMessage>> {
        let body = lockops::receive_by_sequence_request(seqs, true, self.session_id.as_deref());
        let resp = self.call(RECEIVE_BY_SEQUENCE, body).await?;
        Ok(lockops::parse_receive_by_sequence_response(resp, true)?
            .into_iter()
            .map(|(message, token)| ReceivedMessage {
                message,
                lock_token: token.map(LockToken),
            })
            .collect())
    }

    fn require_session(&self) -> Result<String> {
        self.session_id
            .clone()
            .ok_or_else(|| Error::Invalid("this receiver has not accepted a session".into()))
    }

    /// The accepted session's state.
    pub(crate) async fn session_state(&mut self) -> Result<Option<Vec<u8>>> {
        let session = self.require_session()?;
        let resp = self
            .call(
                GET_SESSION_STATE,
                lockops::get_session_state_request(&session),
            )
            .await?;
        lockops::parse_session_state_response(resp)
    }

    /// Set (or with `None` clear) the accepted session's state.
    pub(crate) async fn set_session_state(&mut self, state: Option<&[u8]>) -> Result<()> {
        let session = self.require_session()?;
        let resp = self
            .call(
                SET_SESSION_STATE,
                lockops::set_session_state_request(&session, state),
            )
            .await?;
        lockops::parse_status_only(resp, SET_SESSION_STATE)
    }

    /// Renew the session lock; returns its new expiration.
    pub(crate) async fn renew_session_lock(&mut self) -> Result<OffsetDateTime> {
        let session = self.require_session()?;
        let resp = self
            .call(
                RENEW_SESSION_LOCK,
                lockops::renew_session_lock_request(&session),
            )
            .await?;
        lockops::parse_renew_session_lock_response(resp)
    }

    async fn call(
        &mut self,
        op: &str,
        body: OrderedMap<String, Value>,
    ) -> Result<super::management::MgmtResponse> {
        let name = self.receiver.name().to_string();
        self.management.call_for_link(op, body, Some(&name)).await
    }

    /// Stop the link, release what is held (closing a link with locks held
    /// counts as a lost lock and raises the delivery count), then close both
    /// links. Deliveries the broker sent after the last `receive` are swept
    /// first: peek-lock ones are released.
    ///
    /// Returns how many receive-and-delete messages were removed from the
    /// entity but never returned to a caller (those [`buffered`](Self::buffered)
    /// plus any swept here); callers report them. Always 0 for peek-lock.
    pub(crate) async fn close(mut self) -> usize {
        if bounded("drain", CLOSE_TIMEOUT, self.receiver.drain())
            .await
            .is_ok()
        {
            while let Ok(Ok(delivery)) =
                tokio::time::timeout(CLOSE_SWEEP_IDLE, self.receiver.recv::<Body<Value>>()).await
            {
                if let Ok(m) = self.take(delivery) {
                    self.intake.ready.push_back(m);
                }
            }
        }
        if let Err(e) = self.release_all().await {
            tracing::warn!(error = %e, "releasing held messages before closing failed");
        }
        let unreturned = self.intake.ready.len();
        if unreturned > 0 {
            tracing::warn!(
                unreturned,
                "receive-and-delete messages were removed but not returned"
            );
        }
        if tokio::time::timeout(CLOSE_TIMEOUT, self.receiver.close())
            .await
            .is_err()
        {
            tracing::warn!("closing a receiver link timed out; dropped it");
        }
        close_management(self.management).await;
        unreturned
    }
}

async fn close_management(link: ManagementLink) {
    if tokio::time::timeout(CLOSE_TIMEOUT, link.close())
        .await
        .is_err()
    {
        tracing::warn!("closing a management link timed out; dropped it");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_never_exceeds_what_is_still_wanted() {
        assert_eq!(credit_for(3, 50), 3);
        assert_eq!(credit_for(500, 50), 50);
        assert_eq!(credit_for(0, 50), 0);
    }

    #[test]
    fn receive_and_delete_never_grants_more_credit_than_remaining() {
        // The credit plan for 120 wanted with 50 per grant and deliveries of 50, 50, 20.
        let mut plan = CreditPlan::new(120, 50);
        assert_eq!(plan.next_grant(), 50);
        plan.delivered(50);
        assert_eq!(plan.next_grant(), 50);
        plan.delivered(50);
        assert_eq!(plan.next_grant(), 20);
        plan.delivered(20);
        assert_eq!(plan.next_grant(), 0);
        assert!(plan.outstanding_max() <= 120);
    }

    #[test]
    fn drain_waits_for_spent_credit_or_the_grace_time() {
        let mut plan = CreditPlan::new(3, 50);
        assert_eq!(plan.next_grant(), 3);
        plan.delivered(2);
        assert!(
            !plan.drain_done(1, Duration::from_millis(500)),
            "credit left and still inside the grace time: keep reading"
        );
        assert!(
            plan.drain_done(1, DRAIN_GRACE),
            "the grace time passed without a delivery"
        );
        assert!(
            plan.drain_done(0, Duration::ZERO),
            "all credit used: nothing more can come"
        );
        assert_eq!(plan.shortfall(1), 1);
        assert_eq!(plan.shortfall(0), 0);
        assert_eq!(
            plan.shortfall(7),
            1,
            "never above the credit still outstanding (3 granted, 2 delivered)"
        );
    }

    #[test]
    fn lock_tokens_read_the_delivery_tag_as_a_dotnet_guid() {
        let tag = [
            0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        assert_eq!(
            lock_token_from_tag(&tag).unwrap().0.to_string(),
            "00112233-4455-6677-8899-aabbccddeeff"
        );
        assert!(lock_token_from_tag(&[1, 2, 3]).is_none());
    }

    #[test]
    fn session_filter_is_the_plain_session_id_or_null() {
        // Plain values, as the Azure SDKs send them: the broker's gateway reads a described
        // filter value as a selector and refuses the attach (seen on the emulator, Task 22).
        let f = session_filter(&SessionChoice::Id("RMA-1".into())).unwrap();
        let v = f
            .get(&Symbol::from("com.microsoft:session-filter"))
            .unwrap();
        assert_eq!(v, &Value::String("RMA-1".into()));
        assert_eq!(
            session_filter(&SessionChoice::NextAvailable)
                .unwrap()
                .values()
                .next(),
            Some(&Value::Null)
        );
        assert!(session_filter(&SessionChoice::None).is_none());
    }

    #[test]
    fn settlements_map_to_the_service_bus_outcomes() {
        // Link level only: `release` and `settle_on_link` (callers that ignore the outcome).
        assert!(matches!(
            outcome_for(&Settlement::Complete),
            LinkOutcome::Accepted
        ));
        assert!(matches!(
            outcome_for(&Settlement::Abandon),
            LinkOutcome::Modified {
                delivery_failed: Some(true),
                undeliverable_here: Some(false)
            }
        ));
        assert!(matches!(
            outcome_for(&Settlement::Defer),
            LinkOutcome::Modified {
                delivery_failed: None,
                undeliverable_here: Some(true)
            }
        ));
        let LinkOutcome::Rejected(e) = outcome_for(&Settlement::DeadLetter {
            reason: Some("R".into()),
            description: Some("D".into()),
        }) else {
            panic!()
        };
        assert_eq!(
            e.condition,
            ErrorCondition::Custom(Symbol::from("com.microsoft:dead-letter"))
        );
        let info = e.info.expect("reason and description");
        assert!(info.contains_key(&Symbol::from("DeadLetterReason")));
        assert!(info.contains_key(&Symbol::from("DeadLetterErrorDescription")));
        // Absent values are not sent (not even as empty strings).
        let LinkOutcome::Rejected(e) = outcome_for(&Settlement::DeadLetter {
            reason: Some("R".into()),
            description: None,
        }) else {
            panic!()
        };
        let info = e.info.expect("the reason");
        assert!(!info.contains_key(&Symbol::from("DeadLetterErrorDescription")));
        let LinkOutcome::Rejected(e) = outcome_for(&Settlement::DeadLetter {
            reason: None,
            description: None,
        }) else {
            panic!()
        };
        assert!(e.info.is_none());
    }

    #[test]
    fn settle_builds_an_update_disposition_request() {
        // `settle` sends exactly this with call_for_link(op, body, Some(self.name())).
        let t = LockToken(Uuid::from_u128(7));
        let (op, body) = disposition_call(t, &Settlement::Complete, None);
        assert_eq!(op, UPDATE_DISPOSITION);
        assert_eq!(
            body.get("disposition-status"),
            Some(&Value::String("completed".into()))
        );
        assert!(
            matches!(body.get("lock-tokens"), Some(Value::Array(a)) if a.0 == vec![Value::Uuid(t.0.into())])
        );
        assert!(body.get("session-id").is_none());
        let (_, dl) = disposition_call(
            t,
            &Settlement::DeadLetter {
                reason: Some("R".into()),
                description: Some("D".into()),
            },
            Some("S"),
        );
        assert_eq!(
            dl.get("disposition-status"),
            Some(&Value::String("suspended".into()))
        );
        assert_eq!(
            dl.get("deadletter-reason"),
            Some(&Value::String("R".into()))
        );
        assert_eq!(
            dl.get("deadletter-description"),
            Some(&Value::String("D".into()))
        );
        assert_eq!(dl.get("session-id"), Some(&Value::String("S".into())));
        assert_eq!(
            disposition_call(t, &Settlement::Abandon, None)
                .1
                .get("disposition-status"),
            Some(&Value::String("abandoned".into()))
        );
        assert_eq!(
            disposition_call(t, &Settlement::Defer, None)
                .1
                .get("disposition-status"),
            Some(&Value::String("defered".into()))
        );
    }
    fn received(seq: i64) -> ReceivedMessage {
        ReceivedMessage {
            message: crate::messaging::test_message(seq),
            lock_token: None,
        }
    }

    fn seqs(v: &[ReceivedMessage]) -> Vec<i64> {
        v.iter().map(|m| m.message.sequence_number).collect()
    }

    #[test]
    fn buffered_messages_come_first_and_count_toward_max() {
        let mut intake = Intake::default();
        intake.ready.extend([received(1), received(2)]);
        // receive(3): only 1 more is wanted from the link.
        let mut plan = intake.begin(3);
        assert_eq!(plan.next_grant(), 1);
        intake.granted(&plan);
        intake.arrived(&mut plan, Some(received(3)));
        assert_eq!(plan.next_grant(), 0);
        assert_eq!(seqs(&intake.finish(&plan, 0, 3)), [1, 2, 3]);
        assert_eq!(intake.shortfall, 0);
        // More buffered than wanted: no credit, the rest stays buffered.
        intake.ready.extend([received(4), received(5)]);
        let mut plan = intake.begin(1);
        assert_eq!(plan.next_grant(), 0);
        assert_eq!(seqs(&intake.finish(&plan, 0, 1)), [4]);
        assert_eq!(seqs(&intake.ready.iter().cloned().collect::<Vec<_>>()), [5]);
    }

    #[test]
    fn an_early_failure_keeps_the_outstanding_credit_as_shortfall() {
        let mut intake = Intake::default();
        let mut plan = intake.begin(5);
        assert_eq!(plan.next_grant(), 5);
        intake.granted(&plan);
        intake.arrived(&mut plan, Some(received(1)));
        let out = intake
            .stop(&plan, Error::Network("down".into()), 5)
            .unwrap();
        assert_eq!(seqs(&out), [1], "what arrived is returned");
        assert_eq!(
            intake.shortfall, 4,
            "credit still out is never reported as 0"
        );
        // The failure is reported by the next call.
        assert!(matches!(
            intake.settled_before(5),
            Some(Err(Error::Network(_)))
        ));
        // A failure before anything arrived is returned at once, with the shortfall.
        let mut plan = intake.begin(2);
        plan.next_grant();
        intake.granted(&plan);
        assert!(
            intake
                .stop(&plan, Error::Network("down".into()), 2)
                .is_err()
        );
        assert_eq!(intake.shortfall, 2);
    }

    #[test]
    fn a_cancelled_receive_leaves_its_messages_for_the_next_call() {
        let mut intake = Intake::default();
        // A receive(2) that took one delivery and was then dropped (no finish).
        {
            let mut plan = intake.begin(2);
            plan.next_grant();
            intake.granted(&plan);
            intake.arrived(&mut plan, Some(received(7)));
        }
        assert_eq!(
            intake.shortfall, 1,
            "the cancelled call's credit still counts"
        );
        // The next receive(1) returns it without asking the link for more.
        assert!(intake.settled_before(1).is_none());
        let mut plan = intake.begin(1);
        assert_eq!(plan.next_grant(), 0);
        assert_eq!(seqs(&intake.finish(&plan, 0, 1)), [7]);
    }

    #[test]
    fn released_or_settled_messages_leave_the_buffer() {
        let mut intake = Intake::default();
        let t = LockToken(Uuid::from_u128(9));
        intake.ready.push_back(ReceivedMessage {
            message: crate::messaging::test_message(1),
            lock_token: Some(t),
        });
        intake.ready.push_back(received(2));
        intake.forget(t);
        assert_eq!(seqs(&intake.take(5)), [2]);
    }
}
