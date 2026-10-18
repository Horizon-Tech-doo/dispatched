//! Lock-and-match: find messages by sequence number with their lock held, so delete, move
//! and resubmit act on exactly those messages.

use super::failed;
use super::send::distinct;
use crate::diagnostics::error_kind;
use crate::jobs::{ItemOutcome, ItemResult, NotReached};
use crate::messaging::backend::{LockReceiver, MessagingBackend};
use crate::messaging::sender::{chunks, copy_error};
use crate::messaging::{
    LockToken, MessageState, PeekedMessage, ReceiveMode, ReceivedMessage, ReceiverSpec,
    SessionChoice, collect_sequences,
};
use crate::namespace::PeekView;
use crate::safety::count_text;
use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::safety::CancellationToken;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::time::Instant;

/// How far a lock-and-match search may go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocateLimits {
    /// An idle timeout: stop when this passes without a delivery.
    pub idle: Duration,
    /// Examine at most this many messages: in the peek, and again in each receive (where
    /// every delivery counts, redeliveries of a message whose lock expired included).
    pub max_examined: u64,
    /// Hold at most this many locks on messages that were not asked for.
    pub max_held: usize,
    /// Renew the held locks (or a session's lock) when the earliest expires within this. Locks
    /// shorter than twice the margin are renewed at half their span instead, and never more
    /// often than once a second.
    pub renew_margin: Duration,
}

impl Default for LocateLimits {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(5),
            max_examined: 10_000,
            max_held: 2_000,
            renew_margin: Duration::from_secs(10),
        }
    }
}

/// A wanted message, found.
#[derive(Debug)]
pub(crate) enum Found {
    /// Received with its lock (active or deferred). An active message is held by the
    /// receiver lent with it, so what `on_found` leaves unsettled is released afterwards. A
    /// deferred one is not held: `on_found` must settle it, or its lock expires (which
    /// raises its delivery count).
    Locked { received: ReceivedMessage },
    /// A scheduled message (it cannot be locked).
    Scheduled(PeekedMessage),
}

/// What to do with each wanted message [`locate`] finds. A trait rather than a closure, so
/// the action can borrow the receiver it is lent and use the plane `locate` holds (to cancel
/// scheduled messages); an action that sends owns its sender, opened before `locate`.
///
/// Nothing renews held locks while `on_found` runs: keep it short (one send and one settle).
#[async_trait]
pub(crate) trait OnFound: Send {
    /// Act on `found`. `rx` is the receiver that holds the lock (settle through it); `None`
    /// for [`Found::Scheduled`].
    async fn on_found(
        &mut self,
        plane: &mut dyn MessagingBackend,
        rx: Option<&mut dyn LockReceiver>,
        found: Found,
    ) -> ItemResult;
}

/// The local clock read as the wall clock: anchored once, then monotonic (tokio time, so
/// tests drive it with a paused clock). Lock expiries come in broker wall time; the margin
/// covers clock skew.
struct Clock {
    wall: OffsetDateTime,
    at: Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            wall: crate::messaging::wall_now(),
            at: Instant::now(),
        }
    }

    fn now(&self) -> OffsetDateTime {
        self.wall + (Instant::now() - self.at)
    }

    /// The local instant at which the wall clock reads `t` (the anchor when `t` is earlier).
    fn instant(&self, t: OffsetDateTime) -> Instant {
        Duration::try_from(t - self.wall).map_or(self.at, |d| self.at + d)
    }
}

fn result(address: &str, seq: i64, m: Option<&PeekedMessage>, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: Some(seq),
        message_id: m.and_then(|m| m.message_id.clone()),
        session_id: m.and_then(|m| m.session_id.clone()),
        entity: address.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

/// What [`locate`] found and did: one item per distinct sequence number, in the order given,
/// and notes for the job report (Task 14 appends them to `JobReport.notes`).
#[derive(Debug)]
pub(crate) struct Located {
    pub items: Vec<ItemResult>,
    pub notes: Vec<String>,
}

/// The note for locks a search could not release.
pub(crate) fn unlock_note(n: usize) -> String {
    if n == 1 {
        "1 message could not be unlocked; its delivery count rises by 1 when the lock expires"
            .into()
    } else {
        format!(
            "{} messages could not be unlocked; their delivery count rises by 1 when the lock expires",
            count_text(n as u64)
        )
    }
}

/// Never renew more often than this.
const RENEW_FLOOR: Duration = Duration::from_secs(1);

fn not_reached(why: NotReached) -> ItemOutcome {
    ItemOutcome::NotReached { why }
}

/// Messages of one state, grouped by session (`None`: no session).
type Groups = BTreeMap<Option<String>, Vec<PeekedMessage>>;

/// Find the messages of `view` on `address` with the sequence numbers `seqs` and hand each
/// one to `on`, its lock held. One result per distinct number, in the order given, plus notes.
/// `address` must be the view's own address (`{entity}/$DeadLetterQueue` for `DeadLetter`,
/// and so on): the peek reads `address` as given.
///
/// 1. A peek classifies them (at most `max_examined`): absent ones are `Missing`, except that
///    when the peek ran out of budget those above the last one examined are `NotReached
///    { Budget }` (never looked at).
/// 2. Deferred ones are received by sequence number on one receiver per session.
/// 3. Scheduled ones go to `on` without a receiver.
/// 4. Active ones are received one at a time on a peek-lock receiver per session (dead-letter
///    views use one plain receiver: sub-queues are not session-enabled). A wanted message goes
///    to `on` at once; any other stays locked (so it is not delivered again) and counts
///    towards `max_held`, as does a wanted one `on` left unsettled. Held locks are renewed in
///    chunks of 100 when the earliest is within `renew_margin` of expiring (on a session
///    receiver, the session lock is renewed instead: it covers its messages); if renewing
///    fails, the search stops (`LockTime`).
///    It also stops on `idle` (`Timeout`), at `max_examined` (`Budget`), at `max_held`
///    (`HeldLimit`) and on cancel (`Cancelled`). Then everything held is released
///    (`released` keeps delivery counts; a close or an expiry would raise them).
///
/// Every receiver is closed on every path. A receiver that fails fails the items it had
/// not reached; one that fails after all were found, and locks that could not be released,
/// become notes. No error is dropped.
pub(crate) async fn locate(
    plane: &mut dyn MessagingBackend,
    address: &str,
    view: PeekView,
    seqs: &[i64],
    limits: &LocateLimits,
    cancel: &CancellationToken,
    on: &mut dyn OnFound,
) -> Result<Located> {
    let wanted = distinct(seqs);
    let clock = Clock::new();
    let budget = limits.max_examined;
    let c = match collect_sequences(plane, address, view, &wanted, budget, Some(cancel), None).await
    {
        Ok(c) => c,
        Err(Error::Cancelled) => {
            return Ok(Located {
                items: wanted
                    .iter()
                    .map(|s| result(address, *s, None, not_reached(NotReached::Cancelled)))
                    .collect(),
                notes: Vec::new(),
            });
        }
        Err(e) => return Err(e),
    };
    let mut out: HashMap<i64, ItemResult> = HashMap::new();
    let mut notes = Vec::new();
    if c.scanned >= budget {
        for s in &c.missing {
            if c.last_examined.is_none_or(|l| *s > l) {
                let why = not_reached(NotReached::Budget);
                out.insert(*s, result(address, *s, None, why));
            }
        }
    }
    // Dead-lettered messages keep their session id, but the sub-queue is not
    // session-enabled: read them without a session.
    let by_session = !matches!(view, PeekView::DeadLetter | PeekView::TransferDeadLetter);
    let (mut deferred, mut active, mut scheduled) = (Groups::new(), Groups::new(), Vec::new());
    for m in c.messages {
        let key = if by_session {
            m.session_id.clone()
        } else {
            None
        };
        match m.state {
            MessageState::Deferred => deferred.entry(key).or_default().push(m),
            MessageState::Scheduled => scheduled.push(m),
            MessageState::Active => active.entry(key).or_default().push(m),
        }
    }

    for (session, msgs) in deferred {
        let Some(mut rx) = open(plane, address, session, Duration::ZERO, &msgs, &mut out).await
        else {
            continue;
        };
        take_deferred(plane, rx.as_mut(), address, &msgs, cancel, on, &mut out).await;
        rx.close().await;
    }

    for m in scheduled {
        let seq = m.sequence_number;
        let r = if cancel.is_cancelled() {
            result(address, seq, Some(&m), not_reached(NotReached::Cancelled))
        } else {
            on.on_found(plane, None, Found::Scheduled(m)).await
        };
        out.insert(seq, r);
    }

    for (session, msgs) in active {
        if cancel.is_cancelled() {
            for m in &msgs {
                let why = not_reached(NotReached::Cancelled);
                out.insert(
                    m.sequence_number,
                    result(address, m.sequence_number, Some(m), why),
                );
            }
            continue;
        }
        let Some(mut rx) = open(plane, address, session, limits.idle, &msgs, &mut out).await else {
            continue;
        };
        let mut search = Search {
            plane: &mut *plane,
            rx: rx.as_mut(),
            address,
            limits,
            clock: &clock,
            out: &mut out,
            notes: &mut notes,
        };
        search.run(msgs, cancel, on).await;
        rx.close().await;
    }

    let items = wanted
        .into_iter()
        .map(|s| {
            out.remove(&s)
                .unwrap_or_else(|| result(address, s, None, ItemOutcome::Missing))
        })
        .collect();
    Ok(Located { items, notes })
}

/// A peek-lock receiver on `address` in `session`; a failure fails `msgs`.
async fn open(
    plane: &mut dyn MessagingBackend,
    address: &str,
    session: Option<String>,
    wait: Duration,
    msgs: &[PeekedMessage],
    out: &mut HashMap<i64, ItemResult>,
) -> Option<Box<dyn LockReceiver>> {
    let spec = ReceiverSpec {
        address: address.to_string(),
        mode: ReceiveMode::PeekLock,
        session: session.map_or(SessionChoice::None, SessionChoice::Id),
    };
    let e = match plane.open_receiver(&spec, wait).await {
        Ok(Some(rx)) => return Some(rx),
        Ok(None) => Error::Protocol(format!("no receiver was opened on {address}")),
        Err(e) => e,
    };
    for m in msgs {
        let seq = m.sequence_number;
        out.insert(seq, result(address, seq, Some(m), failed(&e)));
    }
    None
}

/// Receive `msgs` (deferred) by sequence number in chunks and hand each to `on`. Received
/// deferred messages are not held, so every one goes to `on` (cancel is checked between
/// chunks only).
async fn take_deferred(
    plane: &mut dyn MessagingBackend,
    rx: &mut dyn LockReceiver,
    address: &str,
    msgs: &[PeekedMessage],
    cancel: &CancellationToken,
    on: &mut dyn OnFound,
    out: &mut HashMap<i64, ItemResult>,
) {
    for chunk in chunks(msgs) {
        if cancel.is_cancelled() {
            for m in chunk {
                let why = not_reached(NotReached::Cancelled);
                out.insert(
                    m.sequence_number,
                    result(address, m.sequence_number, Some(m), why),
                );
            }
            continue;
        }
        let seqs: Vec<i64> = chunk.iter().map(|m| m.sequence_number).collect();
        let (received, mut errors) = receive_by_sequence(rx, &seqs).await;
        let mut received: HashMap<i64, ReceivedMessage> = received
            .into_iter()
            .map(|m| (m.message.sequence_number, m))
            .collect();
        for m in chunk {
            let seq = m.sequence_number;
            let r = match (received.remove(&seq), errors.remove(&seq)) {
                (Some(got), _) if got.lock_token.is_some() => {
                    on.on_found(plane, Some(&mut *rx), Found::Locked { received: got })
                        .await
                }
                (Some(got), _) => result(
                    address,
                    seq,
                    Some(&got.message),
                    failed(&Error::Protocol(
                        "a deferred message came without a lock token".into(),
                    )),
                ),
                (None, Some(e)) => result(address, seq, Some(m), failed(&e)),
                // Peeked a moment ago, gone now.
                (None, None) => result(address, seq, Some(m), ItemOutcome::Missing),
            };
            out.insert(seq, r);
        }
    }
}

/// Receive deferred messages by sequence number. The broker refuses the whole request when
/// one number is unknown, so then each number is asked for on its own.
async fn receive_by_sequence(
    rx: &mut dyn LockReceiver,
    seqs: &[i64],
) -> (Vec<ReceivedMessage>, HashMap<i64, Error>) {
    let mut errors = HashMap::new();
    match rx.receive_deferred(seqs).await {
        Ok(got) => return (got, errors),
        Err(Error::NotFound(_)) if seqs.len() > 1 => {}
        Err(Error::NotFound(_)) => return (Vec::new(), errors),
        Err(e) => {
            for seq in seqs {
                errors.insert(*seq, copy_error(&e));
            }
            return (Vec::new(), errors);
        }
    }
    let mut received = Vec::new();
    for seq in seqs {
        match rx.receive_deferred(std::slice::from_ref(seq)).await {
            Ok(got) => received.extend(got),
            Err(Error::NotFound(_)) => {}
            Err(e) => {
                errors.insert(*seq, e);
            }
        }
    }
    (received, errors)
}

/// One active search on one receiver.
struct Search<'a> {
    plane: &'a mut dyn MessagingBackend,
    rx: &'a mut dyn LockReceiver,
    address: &'a str,
    limits: &'a LocateLimits,
    clock: &'a Clock,
    out: &'a mut HashMap<i64, ItemResult>,
    notes: &'a mut Vec<String>,
}

/// A message held locked (not asked for, left unsettled by `on`, or seen again after its
/// lock expired).
struct Held {
    token: LockToken,
    until: OffsetDateTime,
}

/// The locks a search holds and when to renew them.
#[derive(Default)]
struct Locks {
    held: HashMap<i64, Held>,
    /// On a session receiver: when the session lock runs out (it bounds its messages').
    session_until: Option<OffsetDateTime>,
    /// The shortest lock span seen (expiry minus the local time it was learnt).
    span: Option<Duration>,
    last_renew: Option<Instant>,
}

impl Locks {
    /// Note a lock that runs until `until`, learnt now.
    fn saw(&mut self, clock: &Clock, until: OffsetDateTime) {
        let span = Duration::try_from(until - clock.now()).unwrap_or(Duration::ZERO);
        self.span = Some(self.span.map_or(span, |s| s.min(span)));
    }

    /// When to renew next: the earliest expiry minus the margin (at most half the lock span),
    /// and never sooner than `RENEW_FLOOR` after the last renewal.
    fn renew_at(&self, clock: &Clock, margin: Duration, session: bool) -> Option<Instant> {
        let earliest = if session {
            self.session_until
        } else {
            self.held.values().map(|h| h.until).min()
        }?;
        let margin = self.span.map_or(margin, |s| margin.min(s / 2));
        let at = clock.instant(earliest - margin);
        Some(
            self.last_renew
                .map_or(at, |last| at.max(last + RENEW_FLOOR)),
        )
    }
}

impl Search<'_> {
    async fn run(
        &mut self,
        msgs: Vec<PeekedMessage>,
        cancel: &CancellationToken,
        on: &mut dyn OnFound,
    ) {
        let mut wanted: HashMap<i64, PeekedMessage> =
            msgs.into_iter().map(|m| (m.sequence_number, m)).collect();
        let session = self.rx.session_id().is_some();
        let mut locks = Locks::default();
        let mut examined = 0u64;
        let mut idle_until = Instant::now() + self.limits.idle;
        let mut stop = None;
        let mut error = None;
        while !wanted.is_empty() {
            if cancel.is_cancelled() {
                stop = Some(NotReached::Cancelled);
                break;
            }
            if examined >= self.limits.max_examined {
                stop = Some(NotReached::Budget);
                break;
            }
            if locks.held.len() >= self.limits.max_held {
                stop = Some(NotReached::HeldLimit);
                break;
            }
            let now = Instant::now();
            if now >= idle_until {
                stop = Some(NotReached::Timeout);
                break;
            }
            let renew_at = locks.renew_at(self.clock, self.limits.renew_margin, session);
            if renew_at.is_some_and(|at| now >= at) {
                locks.last_renew = Some(now);
                if let Err(e) = self.renew(&mut locks, session).await {
                    tracing::warn!(error = %e, held = locks.held.len(), "renewing held locks failed; releasing them and stopping the search");
                    stop = Some(NotReached::LockTime);
                    break;
                }
                continue;
            }
            let renew_due = async {
                match renew_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            // `receive` is cancel safe: a delivery taken while it was dropped stays buffered.
            let batch = tokio::select! {
                biased;
                _ = cancel.cancelled() => continue,
                r = self.rx.receive(1, idle_until - now) => r,
                _ = renew_due => continue,
            };
            let batch = match batch {
                Ok(batch) => batch,
                Err(e) => {
                    error = Some(e);
                    break;
                }
            };
            if batch.is_empty() {
                // The idle time passed without a delivery.
                stop = Some(NotReached::Timeout);
                break;
            }
            for m in batch {
                examined += 1;
                idle_until = Instant::now() + self.limits.idle;
                self.take(m, &mut wanted, &mut locks, session, on).await;
            }
        }
        if error.is_none() {
            // A failure the receiver held back while it returned what it had taken.
            error = self.rx.take_error();
        }
        let all_found = wanted.is_empty();
        for (seq, m) in wanted {
            let outcome = match (&error, stop) {
                (Some(e), _) => ItemOutcome::Failed {
                    kind: error_kind(e),
                    reason: format!("the search stopped early: {e}"),
                },
                (None, Some(why)) => not_reached(why),
                (None, None) => not_reached(NotReached::Timeout),
            };
            self.out
                .insert(seq, result(self.address, seq, Some(&m), outcome));
        }
        if let (Some(e), true) = (&error, all_found) {
            self.notes.push(format!(
                "the receiver on {} failed after the search finished: {e}",
                self.address
            ));
        }
        // Explicitly, before the link closes: closing with locks held would raise the counts.
        // A partial failure answers `Ok` with fewer released, so count what is left.
        let before = self.rx.held();
        let left = match self.rx.release_all().await {
            Ok(n) => before.saturating_sub(usize::try_from(n).unwrap_or(usize::MAX)),
            Err(e) => {
                tracing::warn!(error = %e, "releasing held messages failed; their locks expire");
                before
            }
        };
        if left > 0 {
            self.notes.push(unlock_note(left));
        }
    }

    /// Handle one delivery.
    async fn take(
        &mut self,
        m: ReceivedMessage,
        wanted: &mut HashMap<i64, PeekedMessage>,
        locks: &mut Locks,
        session: bool,
        on: &mut dyn OnFound,
    ) {
        let seq = m.message.sequence_number;
        let Some(token) = m.lock_token else {
            if let Some(p) = wanted.remove(&seq) {
                let e = Error::Protocol("a peek-lock delivery came without a lock token".into());
                self.out
                    .insert(seq, result(self.address, seq, Some(&p), failed(&e)));
            }
            return;
        };
        // Unknown expiry: renew at once, which tells.
        let until = m.message.locked_until.unwrap_or_else(|| self.clock.now());
        locks.saw(self.clock, until);
        if session {
            // In a session, a message's lock is the session lock.
            locks.session_until = Some(locks.session_until.map_or(until, |s| s.max(until)));
        }
        // `wanted` is the guard: a number leaves it when it is acted on, so a message that
        // comes back (its lock expired or was lost) is never acted on twice; it is held
        // instead, so it is not delivered once more.
        if wanted.remove(&seq).is_some() {
            let before = self.rx.held();
            let r = on
                .on_found(
                    &mut *self.plane,
                    Some(&mut *self.rx),
                    Found::Locked { received: m },
                )
                .await;
            self.out.insert(seq, r);
            if self.rx.held() < before {
                return;
            }
            // Left unsettled (skipped, or the settle failed): still held, so renew it.
        }
        locks.held.insert(seq, Held { token, until });
    }

    /// Renew the session lock, or every held message lock in chunks of 100, and take the new
    /// expiries. The first failure ends it.
    async fn renew(&mut self, locks: &mut Locks, session: bool) -> Result<()> {
        if session {
            let until = self.rx.renew_session_lock().await?;
            locks.saw(self.clock, until);
            locks.session_until = Some(until);
            for h in locks.held.values_mut() {
                h.until = until;
            }
            return Ok(());
        }
        let order: Vec<i64> = locks.held.keys().copied().collect();
        for part in chunks(&order) {
            let tokens: Vec<LockToken> = part.iter().map(|s| locks.held[s].token).collect();
            let untils = self.rx.renew_lock(&tokens).await?;
            if untils.len() != tokens.len() {
                return Err(Error::Protocol(format!(
                    "renew-lock answered {} expiries for {} locks",
                    untils.len(),
                    tokens.len()
                )));
            }
            for (seq, until) in part.iter().zip(untils) {
                locks.saw(self.clock, until);
                if let Some(h) = locks.held.get_mut(seq) {
                    h.until = until;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::diagnostics::error_kind;
    use crate::jobs::NotReached;
    use crate::messaging::fake::{FakeBroker, FakeMsg, FakeOp};
    use crate::messaging::{ReceiverSpec, Settlement};

    fn item(m: &PeekedMessage, outcome: ItemOutcome) -> ItemResult {
        ItemResult {
            sequence_number: Some(m.sequence_number),
            message_id: m.message_id.clone(),
            session_id: m.session_id.clone(),
            entity: "q".into(),
            new_sequence_number: None,
            outcome,
        }
    }

    /// Completes `Found::Locked` through the receiver that holds it; skips `Found::Scheduled`.
    struct CompleteIt;
    #[async_trait]
    impl OnFound for CompleteIt {
        async fn on_found(
            &mut self,
            _plane: &mut dyn MessagingBackend,
            rx: Option<&mut dyn LockReceiver>,
            found: Found,
        ) -> ItemResult {
            match found {
                Found::Locked { received } => {
                    let rx = rx.expect("a locked message comes with its receiver");
                    match rx
                        .settle(received.lock_token.unwrap(), &Settlement::Complete)
                        .await
                    {
                        Ok(()) => item(&received.message, ItemOutcome::Succeeded { detail: None }),
                        Err(Error::LockLost(_)) => item(
                            &received.message,
                            ItemOutcome::Failed {
                                kind: "lock_lost",
                                reason: "the lock expired; run again for this sequence number"
                                    .into(),
                            },
                        ),
                        Err(e) => item(
                            &received.message,
                            ItemOutcome::Failed {
                                kind: error_kind(&e),
                                reason: e.to_string(),
                            },
                        ),
                    }
                }
                Found::Scheduled(m) => item(
                    &m,
                    ItemOutcome::Skipped {
                        reason: "scheduled".into(),
                    },
                ),
            }
        }
    }

    /// Like `CompleteIt`, and cancels `Found::Scheduled` through the plane it is lent.
    struct CompleteOrCancel(&'static str);
    #[async_trait]
    impl OnFound for CompleteOrCancel {
        async fn on_found(
            &mut self,
            plane: &mut dyn MessagingBackend,
            rx: Option<&mut dyn LockReceiver>,
            found: Found,
        ) -> ItemResult {
            match found {
                Found::Scheduled(m) => {
                    let mut one = Ok(());
                    let call = plane
                        .cancel_scheduled(self.0, &[m.sequence_number], &mut |_, r| one = r)
                        .await;
                    match call.and(one) {
                        Ok(()) => item(&m, ItemOutcome::Succeeded { detail: None }),
                        Err(e) => item(
                            &m,
                            ItemOutcome::Failed {
                                kind: error_kind(&e),
                                reason: e.to_string(),
                            },
                        ),
                    }
                }
                locked => CompleteIt.on_found(plane, rx, locked).await,
            }
        }
    }

    /// Cancels the job token on the first message it sees, then completes it.
    struct CancelAfterFirst(CancellationToken);
    #[async_trait]
    impl OnFound for CancelAfterFirst {
        async fn on_found(
            &mut self,
            plane: &mut dyn MessagingBackend,
            rx: Option<&mut dyn LockReceiver>,
            found: Found,
        ) -> ItemResult {
            self.0.cancel();
            CompleteIt.on_found(plane, rx, found).await
        }
    }

    #[tokio::test]
    async fn finds_wanted_messages_and_releases_the_rest_without_raising_counts() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..6)
            .map(|i| {
                b.add(
                    "q",
                    FakeMsg::active(if i % 2 == 0 { "want" } else { "skip" }),
                )
            })
            .collect();
        let wanted = [seqs[0], seqs[2], seqs[4]];
        let results = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &wanted,
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(
            results
                .iter()
                .all(|r| matches!(r.outcome, ItemOutcome::Succeeded { .. }))
        );
        assert_eq!(
            b.all("q")
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [seqs[1], seqs[3], seqs[5]],
            "exactly the wanted ones were completed"
        );
        for s in [seqs[1], seqs[3], seqs[5]] {
            assert_eq!(b.delivery_count("q", s), 0, "released, not abandoned");
        }
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn each_view_finds_its_own_state_and_reports_missing() {
        let b = FakeBroker::default();
        let d = b.add("q", FakeMsg::active("d").deferred());
        let s = b.add("q", FakeMsg::active("s").scheduled());
        let none = CancellationToken::new();
        let lim = LocateLimits::default();
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Deferred,
            &[d, 404],
            &lim,
            &none,
            &mut CompleteOrCancel("q"),
        )
        .await
        .unwrap()
        .items;
        assert!(
            matches!(r[0].outcome, ItemOutcome::Succeeded { .. }),
            "deferred: received by sequence number"
        );
        assert!(matches!(r[1].outcome, ItemOutcome::Missing));
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Scheduled,
            &[s],
            &lim,
            &none,
            &mut CompleteOrCancel("q"),
        )
        .await
        .unwrap()
        .items;
        assert!(
            matches!(r[0].outcome, ItemOutcome::Succeeded { .. }),
            "scheduled: handed to on_found as Found::Scheduled, cancelled through the plane"
        );
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[d],
            &lim,
            &none,
            &mut CompleteOrCancel("q"),
        )
        .await
        .unwrap()
        .items;
        assert!(
            matches!(r[0].outcome, ItemOutcome::Missing),
            "not in the active view"
        );
        assert!(b.all("q").is_empty());
    }

    #[tokio::test]
    async fn active_match_runs_inside_the_message_session() {
        let b = FakeBroker::default();
        let a = b.add("s", FakeMsg::active("x").in_session("A"));
        b.add("s", FakeMsg::active("y").in_session("B"));
        let r = locate(
            &mut b.clone(),
            "s",
            PeekView::Active,
            &[a],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(r[0].session_id.as_deref(), Some("A"));
    }

    #[tokio::test]
    async fn limits_stop_the_search_with_a_reason() {
        let b = FakeBroker::default();
        let mut last = 0;
        for _ in 0..50 {
            last = b.add("q", FakeMsg::active("x"));
        }
        let limits = LocateLimits {
            max_held: 10,
            ..Default::default()
        };
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[last],
            &limits,
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(
            r[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::HeldLimit
            }
        ));
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn wanted_messages_beyond_the_budget_are_not_reached_not_missing() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..300).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let limits = LocateLimits {
            max_examined: 120,
            ..Default::default()
        };
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &seqs,
            &limits,
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(r.iter().all(|i| matches!(
            i.outcome,
            ItemOutcome::Succeeded { .. }
                | ItemOutcome::NotReached {
                    why: NotReached::Budget
                }
        )));
        assert!(r.iter().any(|i| matches!(
            i.outcome,
            ItemOutcome::NotReached {
                why: NotReached::Budget
            }
        )));
        assert!(
            !r.iter().any(|i| matches!(i.outcome, ItemOutcome::Missing)),
            "never looked at is not missing"
        );
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn held_locks_are_renewed_or_released_before_they_expire() {
        tokio::time::pause();
        // 40 messages, a 30 s lock and 1 s per delivery: the search outlives the first locks.
        let b = FakeBroker::default()
            .with_lock_duration(Duration::from_secs(30))
            .with_receive_delay(Duration::from_secs(1));
        let seqs: Vec<i64> = (0..40).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[39]],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
        assert!(b.renewals() >= 1, "held locks were renewed in time");
        for s in &seqs[..39] {
            assert_eq!(b.delivery_count("q", *s), 0, "no held lock expired");
        }
        assert_eq!(b.held_locks(), 0);
        // When renewing fails, everything held is released (before it expires) and the search stops.
        let b = FakeBroker::default()
            .with_lock_duration(Duration::from_secs(30))
            .with_receive_delay(Duration::from_secs(1));
        let seqs: Vec<i64> = (0..40).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.fail_next(FakeOp::Renew, Error::Network("reset".into()));
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[39]],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(
            r[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::LockTime
            }
        ));
        for s in &seqs[..39] {
            assert_eq!(b.delivery_count("q", *s), 0, "released, not expired");
        }
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn lost_lock_fails_only_that_item() {
        let b = FakeBroker::default();
        let (x, y) = (
            b.add("q", FakeMsg::active("x")),
            b.add("q", FakeMsg::active("y")),
        );
        b.expire_lock_on_next_settle();
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[x, y],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(
            matches!(&r[0].outcome, ItemOutcome::Failed { kind: "lock_lost", reason } if reason.contains("run again"))
        );
        assert!(matches!(r[1].outcome, ItemOutcome::Succeeded { .. }));
    }

    #[tokio::test]
    async fn locks_are_renewed_only_within_the_margin() {
        tokio::time::pause();
        // 30 s locks, 10 s margin, 1 s per delivery: a 15-message search never gets close.
        let b = FakeBroker::default()
            .with_lock_duration(Duration::from_secs(30))
            .with_receive_delay(Duration::from_secs(1));
        let seqs: Vec<i64> = (0..15).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[14]],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(b.renewals(), 0, "no lock came within the margin");
        // 25 messages: the first lock (until 31 s) is within 10 s at 21 s, so one renewal
        // covers the rest of the search.
        let b = FakeBroker::default()
            .with_lock_duration(Duration::from_secs(30))
            .with_receive_delay(Duration::from_secs(1));
        let seqs: Vec<i64> = (0..25).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[24]],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap();
        assert_eq!(b.renewals(), 1);
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn the_receive_budget_and_the_idle_time_stop_the_search() {
        // The peek jumps straight to the wanted number; the receive must take every one before it.
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..50).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let limits = LocateLimits {
            max_examined: 10,
            ..Default::default()
        };
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[49]],
            &limits,
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(
            r[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Budget
            }
        ));
        assert_eq!(b.held_locks(), 0);
        for s in &seqs {
            assert_eq!(b.delivery_count("q", *s), 0);
        }
        // A wanted message another receiver holds is never delivered: the idle time ends it.
        tokio::time::pause();
        let b = FakeBroker::default();
        let x = b.add("q", FakeMsg::active("x"));
        let mut other = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), Duration::ZERO)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            other
                .receive(1, Duration::from_millis(10))
                .await
                .unwrap()
                .len(),
            1
        );
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[x],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(
            r[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Timeout
            }
        ));
        other.close().await;
    }

    #[tokio::test]
    async fn a_failing_receiver_fails_what_it_had_not_reached_and_still_releases() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..6).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        // The third delivery comes with a held-back link error.
        b.fail_receive_after(3, Error::Network("link detached".into()));
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[0], seqs[5]],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
        assert!(
            matches!(&r[1].outcome, ItemOutcome::Failed { kind: "network", reason } if reason.contains("link detached")),
            "{:?}",
            r[1].outcome
        );
        assert_eq!(b.held_locks(), 0);
        for s in &seqs[1..] {
            assert_eq!(b.delivery_count("q", *s), 0, "released, not dropped");
        }
        // A receiver that cannot be opened fails its items; the job goes on.
        let b = FakeBroker::default();
        let x = b.add("q", FakeMsg::active("x"));
        b.fail_next(FakeOp::OpenReceiver, Error::Network("refused".into()));
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[x],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(
            &r[0].outcome,
            ItemOutcome::Failed {
                kind: "network",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn deferred_messages_are_received_inside_their_session() {
        let b = FakeBroker::default();
        let a = b.add("s", FakeMsg::active("a").in_session("A").deferred());
        let c = b.add("s", FakeMsg::active("c").in_session("C").deferred());
        let plain = b.add("s", FakeMsg::active("p").deferred());
        let r = locate(
            &mut b.clone(),
            "s",
            PeekView::Deferred,
            &[c, a, plain],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert!(
            r.iter()
                .all(|i| matches!(i.outcome, ItemOutcome::Succeeded { .. }))
        );
        assert_eq!(
            r.iter()
                .map(|i| i.session_id.as_deref())
                .collect::<Vec<_>>(),
            [Some("C"), Some("A"), None],
            "one result per number, in the order asked"
        );
        assert!(b.all("s").is_empty());
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn cancel_before_the_search_reaches_nothing() {
        let b = FakeBroker::default();
        let x = b.add("q", FakeMsg::active("x"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[x, x],
            &LocateLimits::default(),
            &cancel,
            &mut CompleteIt,
        )
        .await
        .unwrap()
        .items;
        assert_eq!(r.len(), 1, "distinct numbers");
        assert!(matches!(
            r[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Cancelled
            }
        ));
        assert_eq!(b.delivery_count("q", x), 0);
    }

    /// Counts its calls per sequence number, then completes like `CompleteIt`.
    #[derive(Default)]
    struct Counting(HashMap<i64, u32>);
    #[async_trait]
    impl OnFound for Counting {
        async fn on_found(
            &mut self,
            plane: &mut dyn MessagingBackend,
            rx: Option<&mut dyn LockReceiver>,
            found: Found,
        ) -> ItemResult {
            let seq = match &found {
                Found::Locked { received } => received.message.sequence_number,
                Found::Scheduled(m) => m.sequence_number,
            };
            *self.0.entry(seq).or_default() += 1;
            CompleteIt.on_found(plane, rx, found).await
        }
    }

    /// Leaves every message unsettled.
    struct LeaveIt;
    #[async_trait]
    impl OnFound for LeaveIt {
        async fn on_found(
            &mut self,
            _plane: &mut dyn MessagingBackend,
            _rx: Option<&mut dyn LockReceiver>,
            found: Found,
        ) -> ItemResult {
            let m = match found {
                Found::Locked { received } => received.message,
                Found::Scheduled(m) => m,
            };
            item(
                &m,
                ItemOutcome::Skipped {
                    reason: "left".into(),
                },
            )
        }
    }

    fn slow(lock_secs: u64, delivery_ms: u64) -> FakeBroker {
        FakeBroker::default()
            .with_lock_duration(Duration::from_secs(lock_secs))
            .with_receive_delay(Duration::from_millis(delivery_ms))
    }

    async fn find(b: &FakeBroker, address: &str, view: PeekView, seqs: &[i64]) -> Located {
        locate(
            &mut b.clone(),
            address,
            view,
            seqs,
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn locks_left_held_after_the_release_become_a_note() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..6).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.fail_next(FakeOp::Release, Error::Network("reset".into()));
        let l = find(&b, "q", PeekView::Active, &[seqs[0], seqs[2], seqs[4]]).await;
        assert!(
            l.items
                .iter()
                .all(|i| matches!(i.outcome, ItemOutcome::Succeeded { .. }))
        );
        assert_eq!(
            l.notes,
            [
                "1 message could not be unlocked; its delivery count rises by 1 when the lock expires"
            ]
        );
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..6).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        for _ in 0..3 {
            b.fail_next(FakeOp::Release, Error::Network("reset".into()));
        }
        let l = find(&b, "q", PeekView::Active, &[seqs[5]]).await;
        assert_eq!(
            l.notes,
            [
                "3 messages could not be unlocked; their delivery count rises by 1 when the lock expires"
            ],
            "a partial release answers Ok with fewer released: the rest is in the note"
        );
        // Nothing released at all (`release_all` fails): every held lock is in the note.
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..6).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        for _ in 0..5 {
            b.fail_next(FakeOp::Release, Error::Network("reset".into()));
        }
        let l = find(&b, "q", PeekView::Active, &[seqs[5]]).await;
        assert_eq!(
            l.notes,
            [
                "5 messages could not be unlocked; their delivery count rises by 1 when the lock expires"
            ]
        );
        // A clean run has no notes.
        let b = FakeBroker::default();
        let x = b.add("q", FakeMsg::active("x"));
        b.add("q", FakeMsg::active("y"));
        assert!(find(&b, "q", PeekView::Active, &[x]).await.notes.is_empty());
    }

    #[tokio::test]
    async fn held_locks_are_renewed_in_chunks_of_100() {
        tokio::time::pause();
        // 30 s locks, 80 ms per delivery: the one renewal (at 20 s) covers 247 held locks.
        let b = slow(30, 80);
        let seqs: Vec<i64> = (0..300).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let l = find(&b, "q", PeekView::Active, &[seqs[299]]).await;
        assert!(matches!(l.items[0].outcome, ItemOutcome::Succeeded { .. }));
        // Exact: the paused clock drives the deliveries, and the fake broker and `locate`
        // read one wall clock (`messaging::wall_now`), so no real time leaks in.
        assert_eq!(
            b.renew_sizes(),
            [100, 100, 47],
            "one renewal in three calls of at most 100"
        );
        for s in &seqs[..299] {
            assert_eq!(b.delivery_count("q", *s), 0);
        }
        assert_eq!(b.held_locks(), 0);
        // The second chunk fails: the search stops on lock time and releases everything.
        let b = slow(30, 80);
        let seqs: Vec<i64> = (0..300).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.fail_renew_on_call(2, Error::Network("reset".into()));
        let l = find(&b, "q", PeekView::Active, &[seqs[299]]).await;
        assert!(matches!(
            l.items[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::LockTime
            }
        ));
        assert_eq!(b.renew_sizes(), [100]);
        assert_eq!(b.held_locks(), 0);
        for s in &seqs[..299] {
            assert_eq!(b.delivery_count("q", *s), 0, "released, not expired");
        }
    }

    #[tokio::test]
    async fn locks_shorter_than_the_margin_are_renewed_without_spinning() {
        tokio::time::pause();
        // 5 s locks (the Service Bus minimum) against the default 10 s margin.
        let b = slow(5, 1_000);
        let seqs: Vec<i64> = (0..20).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let l = find(&b, "q", PeekView::Active, &[seqs[19]]).await;
        assert!(matches!(l.items[0].outcome, ItemOutcome::Succeeded { .. }));
        assert!(b.renewals() >= 1);
        assert!(b.renewals() <= 15, "renewed {} times", b.renewals());
        for s in &seqs[..19] {
            assert_eq!(b.delivery_count("q", *s), 0, "no held lock expired");
        }
        // A held lock and nothing more to deliver: the idle time (4 s here, so the other
        // receiver's 5 s lock on the wanted one outlasts it) still ends the search while the
        // short lock is being renewed.
        let b = slow(5, 0);
        let wanted = b.add("q", FakeMsg::active("w"));
        let mut other = b
            .clone()
            .open_receiver(&ReceiverSpec::peek_lock("q"), Duration::ZERO)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(other.receive(1, Duration::ZERO).await.unwrap().len(), 1);
        let h = b.add("q", FakeMsg::active("h"));
        let limits = LocateLimits {
            idle: Duration::from_secs(4),
            ..Default::default()
        };
        let l = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[wanted],
            &limits,
            &CancellationToken::new(),
            &mut CompleteIt,
        )
        .await
        .unwrap();
        assert!(matches!(
            l.items[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Timeout
            }
        ));
        assert!(b.renewals() >= 1, "the held lock was renewed while waiting");
        assert_eq!(b.delivery_count("q", h), 0);
        assert_eq!(b.held_locks(), 1, "only the other receiver's lock");
        other.close().await;
    }

    #[tokio::test]
    async fn a_session_entitys_dead_letter_queue_is_searched_without_a_session() {
        let b = FakeBroker::default();
        let dlq = "s/$DeadLetterQueue";
        let a = b.add(dlq, FakeMsg::active("a").in_session("A"));
        let c = b.add(dlq, FakeMsg::active("c").in_session("C"));
        let d = b.add(dlq, FakeMsg::active("d").in_session("A").deferred());
        let l = find(&b, dlq, PeekView::DeadLetter, &[a, c, d]).await;
        assert!(
            l.items
                .iter()
                .all(|i| matches!(i.outcome, ItemOutcome::Succeeded { .. })),
            "{:?}",
            l.items
        );
        assert!(b.all(dlq).is_empty());
    }

    #[tokio::test]
    async fn a_session_search_renews_the_session_lock() {
        tokio::time::pause();
        let b = slow(30, 1_000);
        let seqs: Vec<i64> = (0..40)
            .map(|_| b.add("s", FakeMsg::active("x").in_session("A")))
            .collect();
        let l = find(&b, "s", PeekView::Active, &[seqs[39]]).await;
        assert!(
            matches!(l.items[0].outcome, ItemOutcome::Succeeded { .. }),
            "{:?}",
            l.items[0].outcome
        );
        assert!(b.session_renewals() >= 1);
        assert_eq!(
            b.renewals(),
            0,
            "no message-lock renewal on a session receiver"
        );
        for s in &seqs[..39] {
            assert_eq!(
                b.delivery_count("s", *s),
                0,
                "the session lock never lapsed"
            );
        }
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test]
    async fn a_receiver_failing_after_the_last_match_is_a_note() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..3).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.fail_receive_after(3, Error::Network("link detached".into()));
        let l = find(&b, "q", PeekView::Active, &[seqs[2]]).await;
        assert!(matches!(l.items[0].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(l.notes.len(), 1, "{:?}", l.notes);
        assert!(
            l.notes[0].starts_with("the receiver on q failed after the search finished: ")
                && l.notes[0].contains("link detached"),
            "{}",
            l.notes[0]
        );
    }

    #[tokio::test]
    async fn a_message_that_comes_back_is_acted_on_once() {
        let b = FakeBroker::default();
        let (x, y) = (
            b.add("q", FakeMsg::active("x")),
            b.add("q", FakeMsg::active("y")),
        );
        b.expire_lock_on_next_settle();
        let mut on = Counting::default();
        let l = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[x, y],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut on,
        )
        .await
        .unwrap();
        assert!(matches!(
            &l.items[0].outcome,
            ItemOutcome::Failed {
                kind: "lock_lost",
                ..
            }
        ));
        assert!(matches!(l.items[1].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(
            on.0.get(&x),
            Some(&1),
            "x came back but was not acted on again"
        );
        assert_eq!(b.held_locks(), 0, "released at the end");
        assert_eq!(b.delivery_count("q", x), 1, "only the lost lock counted");
    }

    #[tokio::test]
    async fn a_wanted_message_left_unsettled_is_held_and_renewed() {
        tokio::time::pause();
        let b = slow(30, 1_000);
        let seqs: Vec<i64> = (0..40).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let l = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[0], seqs[39]],
            &LocateLimits::default(),
            &CancellationToken::new(),
            &mut LeaveIt,
        )
        .await
        .unwrap();
        assert!(
            l.items
                .iter()
                .all(|i| matches!(i.outcome, ItemOutcome::Skipped { .. }))
        );
        for s in &seqs {
            assert_eq!(b.delivery_count("q", *s), 0, "{s}: renewed, then released");
        }
        assert_eq!(b.held_locks(), 0);
        assert_eq!(b.all("q").len(), 40);
    }

    #[tokio::test]
    async fn cancel_releases_held_locks_and_reports_not_reached() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..5).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let cancel = CancellationToken::new();
        let r = locate(
            &mut b.clone(),
            "q",
            PeekView::Active,
            &[seqs[0], seqs[4]],
            &LocateLimits::default(),
            &cancel,
            &mut CancelAfterFirst(cancel.clone()),
        )
        .await
        .unwrap()
        .items;
        assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
        assert!(matches!(
            r[1].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Cancelled
            }
        ));
        assert_eq!(b.held_locks(), 0);
        for s in &seqs[1..] {
            assert_eq!(b.delivery_count("q", *s), 0);
        }
    }
}
