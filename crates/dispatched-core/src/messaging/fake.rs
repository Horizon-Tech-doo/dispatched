//! An in-memory broker for unit tests of the operations: the message plane
//! ([`MessagingBackend`], [`MessageSender`], [`LockReceiver`]) and [`PeekSource`] over
//! shared state, with the lock, delivery-count and session semantics the jobs rely on,
//! and fault injection. Time follows the tokio clock, so tests drive lock expiry and
//! receive delays with `tokio::time::pause()` / `advance()`.

// The operations' tests (Task 12 on) use most of the inspection and fault helpers.
#![allow(dead_code)]

use super::backend::{Each, LockReceiver, MessageSender, MessagingBackend};
use super::message::{MessageBody, MessageState, PeekedMessage, PropValue, dead_letter_fields};
use super::message::{decode_bytes, decode_message, test_message};
use super::outgoing::OutgoingMessage;
use super::receiver::{
    DRAIN_GRACE, LockToken, ReceiveMode, ReceivedMessage, ReceiverSpec, SessionChoice, Settlement,
};
use super::scan::PeekSource;
use super::sender::{copy_error, encode_for_schedule, send_target};
use crate::explorer::retry_after;
use crate::mgmt::EntityCounts;
use crate::namespace::{EntityPath, SubQueue};
use crate::{Error, Result};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::time::Instant;
use uuid::Uuid;

/// Deliveries after which an abandoned (or expired) message is dead-lettered.
const MAX_DELIVERY_COUNT: u32 = 10;
/// How often a waiting receive or next-available accept looks again.
const POLL: Duration = Duration::from_millis(5);

/// An operation [`FakeBroker::fail_next`] can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FakeOp {
    /// One message of a `send_all` (a connection-level error fails the rest too).
    Send,
    /// A `settle`.
    Settle,
    /// A `receive`.
    Receive,
    /// An `open_receiver`.
    OpenReceiver,
    /// A `schedule` call.
    Schedule,
    /// A `cancel_scheduled` call.
    Cancel,
    /// A `renew_lock` call.
    Renew,
}

/// A message to put in the fake broker.
#[derive(Debug, Clone)]
pub(crate) struct FakeMsg {
    pub(crate) session: Option<String>,
    pub(crate) state: MessageState,
    pub(crate) body: &'static str,
    pub(crate) props: Vec<(&'static str, PropValue)>,
    pub(crate) message_id: Option<String>,
}

impl FakeMsg {
    /// An active message with `body`.
    pub(crate) fn active(body: &'static str) -> Self {
        Self {
            session: None,
            state: MessageState::Active,
            body,
            props: Vec::new(),
            message_id: None,
        }
    }

    /// In session `id`.
    pub(crate) fn in_session(mut self, id: &str) -> Self {
        self.session = Some(id.to_string());
        self
    }

    /// Deferred.
    pub(crate) fn deferred(mut self) -> Self {
        self.state = MessageState::Deferred;
        self
    }

    /// Scheduled (an hour from now).
    pub(crate) fn scheduled(mut self) -> Self {
        self.state = MessageState::Scheduled;
        self
    }

    /// With the custom property `key`.
    pub(crate) fn prop(mut self, key: &'static str, value: PropValue) -> Self {
        self.props.push((key, value));
        self
    }
}

#[derive(Debug)]
struct Lock {
    token: LockToken,
    until: Instant,
}

#[derive(Debug)]
struct Stored {
    msg: PeekedMessage,
    lock: Option<Lock>,
}

/// Everything the broker knows; one per [`FakeBroker`] and its clones.
struct State {
    t0: Instant,
    start: OffsetDateTime,
    /// Messages per address, in sequence order.
    entities: HashMap<String, Vec<Stored>>,
    last_seq: HashMap<String, i64>,
    sent: HashMap<String, Vec<OutgoingMessage>>,
    receive_delay: Duration,
    lock_duration: Duration,
    sessions_required: HashSet<String>,
    counts_unavailable: bool,
    faults: HashMap<FakeOp, VecDeque<Error>>,
    expire_next_settle: bool,
    locked_elsewhere: HashSet<(String, String)>,
    /// Sessions accepted by a fake receiver.
    accepted: HashSet<(String, String)>,
    session_states: HashMap<(String, String), Vec<u8>>,
    late: u32,
    stall: u32,
    credit_max: u32,
    renewals: u32,
    receivers: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            t0: Instant::now(),
            start: OffsetDateTime::now_utc(),
            entities: HashMap::new(),
            last_seq: HashMap::new(),
            sent: HashMap::new(),
            receive_delay: Duration::ZERO,
            lock_duration: Duration::from_secs(60),
            sessions_required: HashSet::new(),
            counts_unavailable: false,
            faults: HashMap::new(),
            expire_next_settle: false,
            locked_elsewhere: HashSet::new(),
            accepted: HashSet::new(),
            session_states: HashMap::new(),
            late: 0,
            stall: 0,
            credit_max: 0,
            renewals: 0,
            receivers: 0,
        }
    }
}

fn lock_lost(operation: &str) -> Error {
    Error::LockLost(format!(
        "{operation}: the lock expired or was lost (fake broker); the message is available again, run the operation again"
    ))
}

fn session_locked(id: &str) -> Error {
    Error::Conflict(format!(
        "session {id} is locked by another receiver; stop that receiver or wait for its lock to expire"
    ))
}

impl State {
    /// The broker's wall clock: its start time plus the tokio time elapsed since.
    fn at(&self, instant: Instant) -> OffsetDateTime {
        self.start + (instant - self.t0)
    }

    fn now(&self) -> OffsetDateTime {
        self.at(Instant::now())
    }

    fn fault(&mut self, op: FakeOp) -> Option<Error> {
        self.faults.get_mut(&op)?.pop_front()
    }

    fn next_seq(&mut self, address: &str) -> i64 {
        let highest = self
            .entities
            .get(address)
            .and_then(|v| v.iter().map(|s| s.msg.sequence_number).max())
            .unwrap_or(0);
        let last = self.last_seq.entry(address.to_string()).or_insert(0);
        *last = (*last).max(highest) + 1;
        *last
    }

    fn insert(&mut self, address: &str, msg: PeekedMessage) {
        let list = self.entities.entry(address.to_string()).or_default();
        let at = list.partition_point(|s| s.msg.sequence_number <= msg.sequence_number);
        list.insert(at, Stored { msg, lock: None });
    }

    /// A new message on `address` with the next sequence number; returns it.
    fn enqueue(&mut self, address: &str, mut msg: PeekedMessage) -> i64 {
        let seq = self.next_seq(address);
        msg.sequence_number = seq;
        msg.enqueued_time = Some(self.now());
        msg.locked_until = None;
        self.insert(address, msg);
        seq
    }

    /// Unlock every message whose lock ran out: that counts as a delivery.
    fn expire(&mut self) {
        let now = Instant::now();
        let expired: Vec<(String, i64)> = self
            .entities
            .iter()
            .flat_map(|(address, list)| {
                list.iter()
                    .filter(|s| s.lock.as_ref().is_some_and(|l| l.until <= now))
                    .map(|s| (address.clone(), s.msg.sequence_number))
            })
            .collect();
        for (address, seq) in expired {
            self.unlock(&address, seq, true);
        }
    }

    fn stored_mut(&mut self, address: &str, seq: i64) -> Option<&mut Stored> {
        self.entities
            .get_mut(address)?
            .iter_mut()
            .find(|s| s.msg.sequence_number == seq)
    }

    /// Unlock message `seq`; `counted` raises its delivery count (abandon, lost lock) and
    /// dead-letters it at the limit, otherwise the count stays (release).
    fn unlock(&mut self, address: &str, seq: i64, counted: bool) {
        let Some(m) = self.stored_mut(address, seq) else {
            return;
        };
        m.lock = None;
        m.msg.locked_until = None;
        if counted {
            m.msg.delivery_count += 1;
            if m.msg.delivery_count >= MAX_DELIVERY_COUNT {
                self.dead_letter_max(address, seq);
            }
        }
    }

    /// The sequence number of the message locked under `token`.
    fn locked_seq(&self, address: &str, token: LockToken) -> Option<i64> {
        self.entities
            .get(address)?
            .iter()
            .find(|s| s.lock.as_ref().is_some_and(|l| l.token == token))
            .map(|s| s.msg.sequence_number)
    }

    fn position(&self, address: &str, pred: impl Fn(&Stored) -> bool) -> Option<usize> {
        self.entities.get(address)?.iter().position(pred)
    }

    fn by_token(&mut self, address: &str, token: LockToken) -> Option<&mut Stored> {
        self.entities
            .get_mut(address)?
            .iter_mut()
            .find(|s| s.lock.as_ref().is_some_and(|l| l.token == token))
    }

    fn remove(&mut self, address: &str, seq: i64) -> Option<PeekedMessage> {
        let list = self.entities.get_mut(address)?;
        let i = list.iter().position(|s| s.msg.sequence_number == seq)?;
        Some(list.remove(i).msg)
    }

    /// Move a message to `{address}/$DeadLetterQueue`, keeping its sequence number.
    fn dead_letter(&mut self, address: &str, seq: i64, reason: &str, description: &str) {
        let Some(mut msg) = self.remove(address, seq) else {
            return;
        };
        let props = &mut msg.application_properties;
        props.insert(
            "DeadLetterReason".into(),
            PropValue::String(reason.to_string()),
        );
        props.insert(
            "DeadLetterErrorDescription".into(),
            PropValue::String(description.to_string()),
        );
        (msg.dead_letter_reason, msg.dead_letter_description) = dead_letter_fields(props);
        msg.state = MessageState::Active;
        msg.locked_until = None;
        self.insert(&format!("{address}/$DeadLetterQueue"), msg);
    }

    fn dead_letter_max(&mut self, address: &str, seq: i64) {
        self.dead_letter(
            address,
            seq,
            "MaxDeliveryCountExceeded",
            &format!("Message could not be consumed after {MAX_DELIVERY_COUNT} delivery attempts."),
        );
    }

    /// Lock (peek-lock) or remove (receive-and-delete) the next deliverable message.
    fn deliver(
        &mut self,
        address: &str,
        mode: ReceiveMode,
        session: Option<&str>,
    ) -> Option<ReceivedMessage> {
        self.expire();
        let i = self.position(address, |s| {
            s.lock.is_none()
                && s.msg.state == MessageState::Active
                && session.is_none_or(|id| s.msg.session_id.as_deref() == Some(id))
        })?;
        match mode {
            ReceiveMode::ReceiveAndDelete => {
                let msg = self.entities.get_mut(address)?.remove(i).msg;
                Some(ReceivedMessage {
                    message: msg,
                    lock_token: None,
                })
            }
            ReceiveMode::PeekLock => Some(self.lock_at(address, i)),
        }
    }

    fn lock_at(&mut self, address: &str, i: usize) -> ReceivedMessage {
        let until = Instant::now() + self.lock_duration;
        let locked_until = self.at(until);
        let token = LockToken(Uuid::new_v4());
        let s = &mut self.entities.get_mut(address).expect("found")[i];
        s.lock = Some(Lock { token, until });
        s.msg.locked_until = Some(locked_until);
        ReceivedMessage {
            message: s.msg.clone(),
            lock_token: Some(token),
        }
    }

    /// Sessions of `address` a next-available receiver could take, best first.
    fn free_session(&self, address: &str) -> Option<String> {
        let free = |id: &String| {
            let key = (address.to_string(), id.clone());
            !self.locked_elsewhere.contains(&key) && !self.accepted.contains(&key)
        };
        let with_messages: BTreeSet<String> = self
            .entities
            .get(address)
            .into_iter()
            .flatten()
            .filter(|s| s.msg.state == MessageState::Active && s.lock.is_none())
            .filter_map(|s| s.msg.session_id.clone())
            .collect();
        let with_state: BTreeSet<String> = self
            .session_states
            .keys()
            .filter(|(a, _)| a == address)
            .map(|(_, id)| id.clone())
            .collect();
        with_messages
            .into_iter()
            .find(free)
            .or_else(|| with_state.into_iter().find(free))
    }

    fn count(&self, address: &str, pred: impl Fn(&Stored) -> bool) -> u64 {
        self.entities
            .get(address)
            .map_or(0, |v| v.iter().filter(|s| pred(s)).count() as u64)
    }
}

/// An in-memory Service Bus for tests. Clones share one broker.
#[derive(Clone, Default)]
pub(crate) struct FakeBroker {
    state: Arc<Mutex<State>>,
}

impl FakeBroker {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().expect("fake broker state"))
    }

    /// Put `m` on `address`; returns its sequence number (increasing per address).
    pub(crate) fn add(&self, address: &str, m: FakeMsg) -> i64 {
        self.with(|s| {
            let now = s.now();
            let mut msg = test_message(0);
            msg.state = m.state;
            msg.session_id = m.session;
            msg.message_id = m.message_id;
            msg.body = MessageBody::Binary(m.body.as_bytes().to_vec());
            if m.state == MessageState::Scheduled {
                msg.scheduled_enqueue_time = Some(now + Duration::from_secs(3600));
            }
            msg.application_properties = m
                .props
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<BTreeMap<_, _>>();
            (msg.dead_letter_reason, msg.dead_letter_description) =
                dead_letter_fields(&msg.application_properties);
            s.enqueue(address, msg)
        })
    }

    // Inspection.

    /// Every message on `address`, in sequence order (locked ones too).
    pub(crate) fn all(&self, address: &str) -> Vec<PeekedMessage> {
        self.with(|s| {
            s.expire();
            s.entities
                .get(address)
                .map(|v| v.iter().map(|m| m.msg.clone()).collect())
                .unwrap_or_default()
        })
    }

    /// The delivery count of message `seq` on `address`; panics when it is not there.
    pub(crate) fn delivery_count(&self, address: &str, seq: i64) -> u32 {
        self.all(address)
            .into_iter()
            .find(|m| m.sequence_number == seq)
            .unwrap_or_else(|| panic!("no message {seq} on {address}"))
            .delivery_count
    }

    /// The largest credit any receive had outstanding.
    pub(crate) fn credit_granted_max(&self) -> u32 {
        self.with(|s| s.credit_max)
    }

    /// Messages locked right now, on every address.
    pub(crate) fn held_locks(&self) -> usize {
        self.with(|s| {
            s.expire();
            s.entities
                .values()
                .flatten()
                .filter(|m| m.lock.is_some())
                .count()
        })
    }

    /// How many `renew_lock` calls were made.
    pub(crate) fn renewals(&self) -> u32 {
        self.with(|s| s.renewals)
    }

    /// What senders sent to `address` (the queue or topic), in order.
    pub(crate) fn sent(&self, address: &str) -> Vec<OutgoingMessage> {
        self.with(|s| s.sent.get(address).cloned().unwrap_or_default())
    }

    /// The state of session `id` on `address`.
    pub(crate) fn session_state(&self, address: &str, id: &str) -> Option<Vec<u8>> {
        self.with(|s| {
            s.session_states
                .get(&(address.to_string(), id.to_string()))
                .cloned()
        })
    }

    // Configuration.

    /// Each delivery takes `d` (tokio clock).
    pub(crate) fn with_receive_delay(self, d: Duration) -> Self {
        self.with(|s| s.receive_delay = d);
        self
    }

    /// Peek-lock locks last `d` (default 60 s).
    pub(crate) fn with_lock_duration(self, d: Duration) -> Self {
        self.with(|s| s.lock_duration = d);
        self
    }

    /// `address` is session-enabled: `requires_session` answers `Some(true)` and a
    /// receiver without a session is refused.
    pub(crate) fn require_sessions(&self, address: &str) {
        self.with(|s| s.sessions_required.insert(address.to_string()));
    }

    /// Management cannot say: `entity_counts` and `requires_session` answer `None`.
    pub(crate) fn counts_unavailable(&self) {
        self.with(|s| s.counts_unavailable = true);
    }

    // Fault injection.

    /// The next `op` fails with `e` (queued: several calls fail several times).
    pub(crate) fn fail_next(&self, op: FakeOp, e: Error) {
        self.with(|s| s.faults.entry(op).or_default().push_back(e));
    }

    /// The next `settle` answers `LockLost` and the lock counts as expired.
    pub(crate) fn expire_lock_on_next_settle(&self) {
        self.with(|s| s.expire_next_settle = true);
    }

    /// Session `id` of `address` is held by a receiver elsewhere: opening it is `Conflict`
    /// and next-available skips it.
    pub(crate) fn lock_session_elsewhere(&self, address: &str, id: &str) {
        self.with(|s| {
            s.locked_elsewhere
                .insert((address.to_string(), id.to_string()))
        });
    }

    /// The next `n` deliveries arrive only during a drain (after the wait passed).
    pub(crate) fn deliver_late(&self, n: u32) {
        self.with(|s| s.late = n);
    }

    /// The next drain ends by the grace time with `n` credit unaccounted.
    pub(crate) fn stall_drain(&self, n: u32) {
        self.with(|s| s.stall = n);
    }

    fn fault(&self, op: FakeOp) -> Result<()> {
        self.with(|s| s.fault(op)).map_or(Ok(()), Err)
    }
}

#[async_trait]
impl PeekSource for FakeBroker {
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        Ok(self
            .all(address)
            .into_iter()
            .filter(|m| m.sequence_number >= from)
            .take(max as usize)
            .collect())
    }

    async fn peek_session_batch(
        &mut self,
        address: &str,
        session_id: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        Ok(self
            .all(address)
            .into_iter()
            .filter(|m| m.sequence_number >= from && m.session_id.as_deref() == Some(session_id))
            .take(max as usize)
            .collect())
    }
}

#[async_trait]
impl MessagingBackend for FakeBroker {
    async fn open_sender(&mut self, address: &str) -> Result<Box<dyn MessageSender>> {
        let target = send_target(address)?.base();
        Ok(Box::new(FakeSender {
            broker: self.clone(),
            target,
        }))
    }

    async fn schedule(
        &mut self,
        address: &str,
        batch: &[OutgoingMessage],
        at: OffsetDateTime,
        each: Each<'_, i64>,
    ) -> Result<()> {
        let target = send_target(address)?.base();
        if let Err(e) = self.fault(FakeOp::Schedule) {
            for i in 0..batch.len() {
                each(i, Err(copy_error(&e)));
            }
            return Err(e);
        }
        let now = OffsetDateTime::now_utc();
        for (i, m) in batch.iter().enumerate() {
            // The real encoding, so ids and properties come out as the broker would store them.
            let encoded = encode_for_schedule(m, at, now).and_then(|(b, ..)| decode_bytes(&b));
            each(
                i,
                encoded.map(|mut msg| {
                    msg.state = MessageState::Scheduled;
                    msg.scheduled_enqueue_time = Some(at);
                    self.with(|s| s.enqueue(&target, msg))
                }),
            );
        }
        Ok(())
    }

    async fn cancel_scheduled(
        &mut self,
        address: &str,
        seqs: &[i64],
        each: Each<'_, ()>,
    ) -> Result<()> {
        let target = send_target(address)?.base();
        if let Err(e) = self.fault(FakeOp::Cancel) {
            for i in 0..seqs.len() {
                each(i, Err(copy_error(&e)));
            }
            return Err(e);
        }
        for (i, seq) in seqs.iter().enumerate() {
            let removed = self.with(|s| {
                let found = s.position(&target, |m| {
                    m.msg.sequence_number == *seq && m.msg.state == MessageState::Scheduled
                });
                found.and_then(|_| s.remove(&target, *seq))
            });
            each(
                i,
                removed
                    .map(|_| ())
                    .ok_or_else(|| Error::NotFound(format!("scheduled message {seq} in {target}"))),
            );
        }
        Ok(())
    }

    async fn open_receiver(
        &mut self,
        spec: &ReceiverSpec,
        wait: Duration,
    ) -> Result<Option<Box<dyn LockReceiver>>> {
        self.fault(FakeOp::OpenReceiver)?;
        let address = spec.address.clone();
        let session = match &spec.session {
            SessionChoice::None => {
                if self.with(|s| s.sessions_required.contains(&address)) {
                    return Err(Error::Invalid(format!(
                        "{address} requires sessions; open a session receiver"
                    )));
                }
                None
            }
            SessionChoice::Id(id) => {
                let key = (address.clone(), id.clone());
                self.with(|s| {
                    if s.locked_elsewhere.contains(&key) || s.accepted.contains(&key) {
                        Err(session_locked(id))
                    } else {
                        s.accepted.insert(key);
                        Ok(())
                    }
                })?;
                Some(id.clone())
            }
            SessionChoice::NextAvailable => {
                let deadline = Instant::now() + wait;
                loop {
                    let taken = self.with(|s| {
                        let id = s.free_session(&address)?;
                        s.accepted.insert((address.clone(), id.clone()));
                        Some(id)
                    });
                    if let Some(id) = taken {
                        break Some(id);
                    }
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    tokio::time::sleep(POLL.min(deadline - Instant::now())).await;
                }
            }
        };
        let n = self.with(|s| {
            s.receivers += 1;
            s.receivers
        });
        Ok(Some(Box::new(FakeReceiver {
            broker: self.clone(),
            name: format!("fake-rx-{n}"),
            address,
            mode: spec.mode,
            session,
            tokens: HashSet::new(),
            buffer: VecDeque::new(),
            shortfall: 0,
        })))
    }

    async fn entity_counts(&mut self, entity: &EntityPath) -> Result<Option<EntityCounts>> {
        Ok(self.with(|s| {
            if s.counts_unavailable {
                return None;
            }
            s.expire();
            let main = entity.address(SubQueue::Active);
            Some(EntityCounts {
                // As management counts it: deferred messages are active.
                active: s.count(&main, |m| m.msg.state != MessageState::Scheduled),
                dead_letter: s.count(&entity.address(SubQueue::DeadLetter), |_| true),
                scheduled: s.count(&main, |m| m.msg.state == MessageState::Scheduled),
                transfer: 0,
                transfer_dead_letter: s
                    .count(&entity.address(SubQueue::TransferDeadLetter), |_| true),
                approximate: false,
                truncated: false,
            })
        }))
    }

    async fn requires_session(&mut self, entity: &EntityPath) -> Result<Option<bool>> {
        Ok(self.with(|s| {
            (!s.counts_unavailable).then(|| s.sessions_required.contains(&entity.base()))
        }))
    }

    async fn session_ids(&mut self, address: &str) -> Result<Vec<String>> {
        Ok(self.with(|s| {
            let mut ids: BTreeSet<String> = s
                .entities
                .get(address)
                .into_iter()
                .flatten()
                .filter_map(|m| m.msg.session_id.clone())
                .collect();
            ids.extend(
                s.session_states
                    .keys()
                    .filter(|(a, _)| a == address)
                    .map(|(_, id)| id.clone()),
            );
            ids.into_iter().collect()
        }))
    }
}

/// A sender on the fake broker.
struct FakeSender {
    broker: FakeBroker,
    target: String,
}

#[async_trait]
impl MessageSender for FakeSender {
    async fn send_all(&mut self, batch: &[OutgoingMessage], each: Each<'_, ()>) -> Result<()> {
        for (i, m) in batch.iter().enumerate() {
            if let Err(e) = self.broker.fault(FakeOp::Send) {
                if retry_after(&e, 0) {
                    // The link broke: this message and every later one fail.
                    for j in i..batch.len() {
                        each(j, Err(copy_error(&e)));
                    }
                    return Err(e);
                }
                each(i, Err(e));
                continue;
            }
            let stored = m.to_amqp(OffsetDateTime::now_utc()).map(|amqp| {
                let mut msg = decode_message(amqp);
                msg.state = if msg.scheduled_enqueue_time.is_some() {
                    MessageState::Scheduled
                } else {
                    MessageState::Active
                };
                self.broker.with(|s| {
                    s.sent
                        .entry(self.target.clone())
                        .or_default()
                        .push(m.clone());
                    s.enqueue(&self.target, msg);
                })
            });
            each(i, stored);
        }
        Ok(())
    }

    async fn close(self: Box<Self>) {}
}

/// A receiver on the fake broker. Deliveries go into `buffer` the moment they happen and
/// leave it only on return, so a dropped `receive` loses nothing (as `AmqpReceiver`).
struct FakeReceiver {
    broker: FakeBroker,
    name: String,
    address: String,
    mode: ReceiveMode,
    session: Option<String>,
    /// Lock tokens this receiver holds.
    tokens: HashSet<LockToken>,
    buffer: VecDeque<ReceivedMessage>,
    shortfall: u32,
}

impl FakeReceiver {
    /// Take the next deliverable message into the buffer; whether there was one.
    fn take_one(&mut self) -> bool {
        let got = self
            .broker
            .with(|s| s.deliver(&self.address, self.mode, self.session.as_deref()));
        match got {
            Some(m) => {
                self.tokens.extend(m.lock_token);
                self.buffer.push_back(m);
                true
            }
            None => false,
        }
    }

    fn forget(&mut self, token: LockToken) {
        self.tokens.remove(&token);
        self.buffer.retain(|m| m.lock_token != Some(token));
    }

    fn require_session(&self) -> Result<String> {
        self.session
            .clone()
            .ok_or_else(|| Error::Invalid("this receiver has not accepted a session".into()))
    }

    fn take(&mut self, max: u32) -> Vec<ReceivedMessage> {
        let n = self.buffer.len().min(max as usize);
        self.buffer.drain(..n).collect()
    }
}

#[async_trait]
impl LockReceiver for FakeReceiver {
    fn name(&self) -> &str {
        &self.name
    }

    fn session_id(&self) -> Option<&str> {
        self.session.as_deref()
    }

    async fn receive(&mut self, max: u32, wait: Duration) -> Result<Vec<ReceivedMessage>> {
        self.broker.fault(FakeOp::Receive)?;
        let buffered = u32::try_from(self.buffer.len()).unwrap_or(u32::MAX);
        let mut outstanding = max.saturating_sub(buffered);
        self.broker
            .with(|s| s.credit_max = s.credit_max.max(outstanding));
        self.shortfall = outstanding;
        let deadline = Instant::now() + wait;
        let delay = self.broker.with(|s| s.receive_delay);
        while outstanding > 0 {
            if self.broker.with(|s| s.late == 0) {
                if !delay.is_zero() {
                    if Instant::now() + delay > deadline {
                        tokio::time::sleep_until(deadline).await;
                        break;
                    }
                    tokio::time::sleep(delay).await;
                }
                if self.take_one() {
                    outstanding -= 1;
                    self.shortfall = outstanding;
                    continue;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            tokio::time::sleep(POLL.min(deadline - now)).await;
        }
        if outstanding > 0 {
            // The drain: whatever is available comes now, late deliveries included.
            while outstanding > 0 && self.take_one() {
                outstanding -= 1;
                self.broker.with(|s| s.late = s.late.saturating_sub(1));
            }
            let stall = self.broker.with(|s| std::mem::take(&mut s.stall));
            self.shortfall = if stall > 0 {
                tokio::time::sleep(DRAIN_GRACE).await;
                stall.min(outstanding)
            } else {
                0
            };
        }
        Ok(self.take(max))
    }

    fn drain_shortfall(&self) -> u32 {
        self.shortfall
    }

    fn buffered(&self) -> usize {
        self.buffer.len()
    }

    async fn settle(&mut self, token: LockToken, how: &Settlement) -> Result<()> {
        if self.mode == ReceiveMode::ReceiveAndDelete {
            return Err(Error::Invalid(
                "a receive-and-delete receiver has nothing to settle".into(),
            ));
        }
        // An injected failure leaves the outcome unknown: nothing changes here.
        self.broker.fault(FakeOp::Settle)?;
        let address = self.address.clone();
        let result = self.broker.with(|s| {
            s.expire();
            let lost_now = std::mem::take(&mut s.expire_next_settle);
            let Some(seq) = s.locked_seq(&address, token) else {
                return Err(lock_lost("update-disposition"));
            };
            if lost_now {
                // Its lock ran out at the broker just before the settlement.
                s.unlock(&address, seq, true);
                return Err(lock_lost("update-disposition"));
            }
            match how {
                Settlement::Complete => {
                    s.remove(&address, seq);
                }
                Settlement::Abandon => s.unlock(&address, seq, true),
                Settlement::Defer => {
                    s.unlock(&address, seq, false);
                    if let Some(m) = s.stored_mut(&address, seq) {
                        m.msg.state = MessageState::Deferred;
                    }
                }
                Settlement::DeadLetter {
                    reason,
                    description,
                } => s.dead_letter(&address, seq, reason, description),
            }
            Ok(())
        });
        // Settled or lost at the broker: either way this receiver no longer holds it.
        self.forget(token);
        result
    }

    async fn release(&mut self, token: LockToken) -> Result<()> {
        if !self.tokens.contains(&token) {
            return Err(Error::Invalid(
                "this receiver holds no message with that lock token; it was settled, released or never received here".into(),
            ));
        }
        let address = self.address.clone();
        self.broker.with(|s| {
            if let Some(seq) = s.locked_seq(&address, token) {
                s.unlock(&address, seq, false);
            }
        });
        self.forget(token);
        Ok(())
    }

    async fn release_all(&mut self) -> Result<u64> {
        let tokens: Vec<LockToken> = self.tokens.iter().copied().collect();
        for token in &tokens {
            self.release(*token).await?;
        }
        Ok(tokens.len() as u64)
    }

    fn held(&self) -> usize {
        self.tokens.len()
    }

    async fn renew_lock(&mut self, tokens: &[LockToken]) -> Result<Vec<OffsetDateTime>> {
        self.broker.fault(FakeOp::Renew)?;
        let address = self.address.clone();
        self.broker.with(|s| {
            s.renewals += 1;
            s.expire();
            let until = Instant::now() + s.lock_duration;
            let at = s.at(until);
            tokens
                .iter()
                .map(|t| {
                    let m = s
                        .by_token(&address, *t)
                        .ok_or_else(|| lock_lost("renew-lock"))?;
                    m.lock.as_mut().expect("locked").until = until;
                    m.msg.locked_until = Some(at);
                    Ok(at)
                })
                .collect()
        })
    }

    async fn receive_deferred(&mut self, seqs: &[i64]) -> Result<Vec<ReceivedMessage>> {
        let address = self.address.clone();
        let session = self.session.clone();
        let got = self.broker.with(|s| {
            s.expire();
            let found: Vec<usize> = seqs
                .iter()
                .map(|seq| {
                    s.position(&address, |m| {
                        m.msg.sequence_number == *seq
                            && m.msg.state == MessageState::Deferred
                            && m.lock.is_none()
                            && session
                                .as_deref()
                                .is_none_or(|id| m.msg.session_id.as_deref() == Some(id))
                    })
                    .ok_or_else(|| Error::NotFound(format!("deferred message {seq} in {address}")))
                })
                .collect::<Result<_>>()?;
            Ok::<_, Error>(
                found
                    .into_iter()
                    .map(|i| s.lock_at(&address, i))
                    .collect::<Vec<_>>(),
            )
        })?;
        self.tokens.extend(got.iter().filter_map(|m| m.lock_token));
        Ok(got)
    }

    async fn session_state(&mut self) -> Result<Option<Vec<u8>>> {
        let id = self.require_session()?;
        Ok(self.broker.session_state(&self.address, &id))
    }

    async fn set_session_state(&mut self, state: Option<&[u8]>) -> Result<()> {
        let key = (self.address.clone(), self.require_session()?);
        self.broker.with(|s| match state {
            Some(bytes) => {
                s.session_states.insert(key, bytes.to_vec());
            }
            None => {
                s.session_states.remove(&key);
            }
        });
        Ok(())
    }

    async fn renew_session_lock(&mut self) -> Result<OffsetDateTime> {
        self.require_session()?;
        Ok(self.broker.with(|s| s.at(Instant::now() + s.lock_duration)))
    }

    async fn close(mut self: Box<Self>) -> usize {
        // Buffered peek-lock messages are released with the rest; what stays buffered
        // was removed by receive-and-delete and never returned.
        let _ = self.release_all().await;
        self.buffer.retain(|m| m.lock_token.is_none());
        if let Some(id) = self.session.take() {
            let key = (self.address.clone(), id);
            self.broker.with(|s| s.accepted.remove(&key));
        }
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_millis(100);

    #[tokio::test]
    async fn peek_lock_holds_until_released_and_release_keeps_the_delivery_count() {
        let b = FakeBroker::default();
        let seq = b.add("q", FakeMsg::active("a"));
        let mut plane = b.clone();
        let mut r = plane
            .open_receiver(&ReceiverSpec::peek_lock("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        let got = r.receive(1, WAIT).await.unwrap();
        assert_eq!(got[0].message.sequence_number, seq);
        assert!(
            r.receive(1, Duration::from_millis(10))
                .await
                .unwrap()
                .is_empty(),
            "locked: not delivered twice"
        );
        r.release(got[0].lock_token.unwrap()).await.unwrap();
        assert_eq!(b.delivery_count("q", seq), 0);
    }

    #[tokio::test]
    async fn abandon_raises_the_count_and_dead_letters_at_the_limit() {
        let b = FakeBroker::default();
        let seq = b.add("q", FakeMsg::active("a"));
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        for i in 1..=9 {
            let got = r.receive(1, WAIT).await.unwrap();
            r.settle(got[0].lock_token.unwrap(), &Settlement::Abandon)
                .await
                .unwrap();
            assert_eq!(b.delivery_count("q", seq), i);
        }
        let got = r.receive(1, WAIT).await.unwrap();
        r.settle(got[0].lock_token.unwrap(), &Settlement::Abandon)
            .await
            .unwrap();
        assert!(b.all("q").is_empty(), "the 10th abandon dead-letters it");
        let dl = b.all("q/$DeadLetterQueue");
        assert_eq!(dl.len(), 1);
        assert_eq!(dl[0].sequence_number, seq);
        assert_eq!(
            dl[0].dead_letter_reason.as_deref(),
            Some("MaxDeliveryCountExceeded")
        );
    }

    #[tokio::test]
    async fn expired_locks_raise_the_count_and_settling_then_is_lock_lost() {
        tokio::time::pause();
        let b = FakeBroker::default().with_lock_duration(Duration::from_secs(30));
        let seq = b.add("q", FakeMsg::active("a"));
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        let got = r.receive(1, WAIT).await.unwrap();
        let token = got[0].lock_token.unwrap();
        tokio::time::advance(Duration::from_secs(20)).await;
        let renewed = r.renew_lock(&[token]).await.unwrap();
        assert!(renewed[0] > got[0].message.locked_until.unwrap());
        tokio::time::advance(Duration::from_secs(31)).await;
        assert!(matches!(
            r.settle(token, &Settlement::Complete).await,
            Err(Error::LockLost(_))
        ));
        assert_eq!(
            b.delivery_count("q", seq),
            1,
            "an expired lock counts as a delivery"
        );
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn receive_and_delete_removes_on_delivery_and_records_credit() {
        let b = FakeBroker::default();
        for _ in 0..5 {
            b.add("q", FakeMsg::active("x"));
        }
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::receive_and_delete("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.receive(3, WAIT).await.unwrap().len(), 3);
        assert_eq!(b.all("q").len(), 2);
        assert!(b.credit_granted_max() <= 3);
    }

    #[tokio::test]
    async fn next_available_session_skips_sessions_locked_elsewhere() {
        let b = FakeBroker::default();
        b.add("s", FakeMsg::active("1").in_session("a"));
        b.add("s", FakeMsg::active("2").in_session("b"));
        b.lock_session_elsewhere("s", "a");
        let spec = ReceiverSpec {
            session: SessionChoice::NextAvailable,
            ..ReceiverSpec::receive_and_delete("s")
        };
        let mut plane = b.clone();
        let mut r = plane
            .open_receiver(&spec, WAIT)
            .await
            .unwrap()
            .expect("session b is free");
        assert_eq!(r.session_id(), Some("b"));
        assert_eq!(r.receive(5, WAIT).await.unwrap().len(), 1);
        r.close().await;
        assert!(
            plane.open_receiver(&spec, WAIT).await.unwrap().is_none(),
            "only a locked session is left: none available"
        );
        let by_id = ReceiverSpec {
            session: SessionChoice::Id("a".into()),
            ..ReceiverSpec::peek_lock("s")
        };
        assert!(matches!(
            plane.open_receiver(&by_id, WAIT).await,
            Err(Error::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn a_stalled_drain_reports_its_shortfall() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        b.stall_drain(2);
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::receive_and_delete("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            r.receive(3, Duration::from_millis(10)).await.unwrap().len(),
            1
        );
        assert_eq!(r.drain_shortfall(), 2);
    }

    #[tokio::test]
    async fn a_dropped_receive_loses_no_receive_and_delete_message() {
        tokio::time::pause();
        let b = FakeBroker::default().with_receive_delay(Duration::from_secs(1));
        for _ in 0..3 {
            b.add("q", FakeMsg::active("x"));
        }
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::receive_and_delete("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        // Cancelled after two deliveries (at 1 s and 2 s), before the third (3 s).
        let dropped = tokio::time::timeout(
            Duration::from_millis(2500),
            r.receive(3, Duration::from_secs(10)),
        )
        .await;
        assert!(dropped.is_err(), "the receive was cancelled");
        assert_eq!(b.all("q").len(), 1, "two were removed from the broker");
        assert_eq!(r.buffered(), 2, "and kept on the receiver");
        let got = r.receive(2, Duration::from_secs(10)).await.unwrap();
        assert_eq!(got.len(), 2, "the next receive returns them first");
        assert_eq!(b.all("q").len(), 1, "without taking the third");
        assert_eq!(r.buffered(), 0);
    }

    #[tokio::test]
    async fn close_reports_removed_but_unreturned_messages() {
        tokio::time::pause();
        let b = FakeBroker::default().with_receive_delay(Duration::from_secs(1));
        for _ in 0..3 {
            b.add("q", FakeMsg::active("x"));
        }
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::receive_and_delete("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        let _ = tokio::time::timeout(
            Duration::from_millis(2500),
            r.receive(3, Duration::from_secs(10)),
        )
        .await;
        assert_eq!(r.close().await, 2, "removed by the broker, never returned");
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test]
    async fn deferred_and_scheduled_are_received_only_by_sequence_number() {
        let b = FakeBroker::default();
        let deferred = b.add("q", FakeMsg::active("d").deferred());
        b.add("q", FakeMsg::active("s").scheduled());
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        assert!(
            r.receive(5, Duration::from_millis(10))
                .await
                .unwrap()
                .is_empty()
        );
        let got = r.receive_deferred(&[deferred]).await.unwrap();
        assert_eq!(got[0].message.sequence_number, deferred);
        assert_eq!(r.held(), 1);
        r.settle(got[0].lock_token.unwrap(), &Settlement::Complete)
            .await
            .unwrap();
        assert!(matches!(
            r.receive_deferred(&[deferred]).await,
            Err(Error::NotFound(_))
        ));
        let counts = b
            .clone()
            .entity_counts(&EntityPath::queue("q"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((counts.active, counts.scheduled), (0, 1));
    }

    #[tokio::test]
    async fn defer_and_dead_letter_settle_as_the_broker_does() {
        let b = FakeBroker::default();
        let a = b.add("q", FakeMsg::active("a"));
        let c = b.add("q", FakeMsg::active("c"));
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        let got = r.receive(2, WAIT).await.unwrap();
        r.settle(got[0].lock_token.unwrap(), &Settlement::Defer)
            .await
            .unwrap();
        let how = Settlement::DeadLetter {
            reason: "bad".into(),
            description: "no customer".into(),
        };
        r.settle(got[1].lock_token.unwrap(), &how).await.unwrap();
        let q = b.all("q");
        assert_eq!(
            (q.len(), q[0].sequence_number, q[0].state),
            (1, a, MessageState::Deferred)
        );
        let dl = b.all("q/$DeadLetterQueue");
        assert_eq!(dl[0].sequence_number, c);
        assert_eq!(dl[0].dead_letter_reason.as_deref(), Some("bad"));
        assert_eq!(
            dl[0].dead_letter_description.as_deref(),
            Some("no customer")
        );
        let counts = b
            .clone()
            .entity_counts(&EntityPath::queue("q"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (counts.active, counts.dead_letter),
            (1, 1),
            "deferred counts as active"
        );
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn injected_faults_and_a_lock_lost_on_settle() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        let mut r = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), WAIT)
            .await
            .unwrap()
            .unwrap();
        b.fail_next(FakeOp::Receive, Error::Network("reset".into()));
        assert!(matches!(r.receive(1, WAIT).await, Err(Error::Network(_))));
        let got = r.receive(1, WAIT).await.unwrap();
        b.expire_lock_on_next_settle();
        assert!(matches!(
            r.settle(got[0].lock_token.unwrap(), &Settlement::Complete)
                .await,
            Err(Error::LockLost(_))
        ));
        assert_eq!(
            b.all("q")[0].delivery_count,
            1,
            "the message is available again"
        );
        assert_eq!(r.held(), 0);
    }

    #[tokio::test]
    async fn sends_are_recorded_and_a_broken_link_fails_the_rest() {
        let mut b = FakeBroker::default();
        let mut tx = b.open_sender("q").await.unwrap();
        let batch = [
            OutgoingMessage::text("1"),
            OutgoingMessage::text("2"),
            OutgoingMessage::text("3"),
        ];
        b.fail_next(FakeOp::Send, Error::Invalid("refused".into()));
        let mut seen = Vec::new();
        tx.send_all(&batch, &mut |i, r| seen.push((i, r.is_ok())))
            .await
            .unwrap();
        assert_eq!(seen, [(0, false), (1, true), (2, true)]);
        assert_eq!(b.sent("q").len(), 2);
        assert_eq!(b.all("q").len(), 2);
        b.fail_next(FakeOp::Send, Error::Network("reset".into()));
        let mut seen = Vec::new();
        assert!(
            tx.send_all(&batch, &mut |i, r| seen.push((i, r.is_ok())))
                .await
                .is_err()
        );
        assert_eq!(seen, [(0, false), (1, false), (2, false)]);
        assert!(b.open_sender("t/Subscriptions/s").await.is_err());
    }

    #[tokio::test]
    async fn scheduled_messages_get_sequence_numbers_and_can_be_cancelled() {
        let mut b = FakeBroker::default();
        let at = OffsetDateTime::now_utc() + Duration::from_secs(3600);
        let mut seqs = Vec::new();
        b.schedule("q", &[OutgoingMessage::text("x")], at, &mut |_, r| {
            seqs.push(r.unwrap())
        })
        .await
        .unwrap();
        assert_eq!(b.all("q")[0].state, MessageState::Scheduled);
        let mut out = Vec::new();
        b.cancel_scheduled("q", &[seqs[0], 999], &mut |i, r| out.push((i, r.is_ok())))
            .await
            .unwrap();
        assert_eq!(out, [(0, true), (1, false)]);
        assert!(b.all("q").is_empty());
    }

    #[tokio::test]
    async fn session_state_needs_the_session_and_management_can_be_unavailable() {
        let mut b = FakeBroker::default();
        b.require_sessions("s");
        assert!(
            b.open_receiver(&ReceiverSpec::peek_lock("s"), WAIT)
                .await
                .is_err()
        );
        let spec = ReceiverSpec {
            session: SessionChoice::Id("a".into()),
            ..ReceiverSpec::peek_lock("s")
        };
        let mut r = b.open_receiver(&spec, WAIT).await.unwrap().unwrap();
        assert!(
            matches!(b.open_receiver(&spec, WAIT).await, Err(Error::Conflict(_))),
            "one receiver per session"
        );
        r.set_session_state(Some(b"step-2")).await.unwrap();
        assert_eq!(b.session_state("s", "a").as_deref(), Some(&b"step-2"[..]));
        assert_eq!(b.session_ids("s").await.unwrap(), ["a"]);
        r.close().await;
        let s = EntityPath::queue("s");
        assert_eq!(b.requires_session(&s).await.unwrap(), Some(true));
        b.counts_unavailable();
        assert_eq!(b.requires_session(&s).await.unwrap(), None);
        assert!(b.entity_counts(&s).await.unwrap().is_none());
    }
}
