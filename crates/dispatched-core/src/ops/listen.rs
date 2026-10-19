//! The listener: a live receive stream on a queue or subscription, until cancelled.
//!
//! - `Watch` tails the entity with peeks: no lock is taken and no delivery count changes. (A
//!   peek-lock receive that releases each message would get the same message straight back,
//!   because a released message is first in line again.)
//! - `Complete` runs N peek-lock receivers at credit 1 (§14 Decision 10) and completes each
//!   message as it comes.
//! - `Delete` runs N receive-and-delete receivers with at most `prefetch` credit each.
//!
//! Every receiver is closed when the listener stops, and what the close returns (buffered and
//! swept receive-and-delete messages) is emitted and counted like any other (§14 Decision 13).
//! The report holds counts, not items: a listener can run for hours. Only failures are items.

use crate::diagnostics::error_kind;
use crate::jobs::{ItemOutcome, ItemResult, JobEvent, JobKind, JobReport, JobRun, Progress};
use crate::messaging::backend::{LockReceiver, MessagingBackend};
use crate::messaging::{
    PeekedMessage, ReceiveMode, ReceiverSpec, SessionChoice, Settlement, wall_now,
};
use crate::namespace::{EntityPath, PeekView, SubQueue};
use crate::ops::receive::{stopped_drain_note, undecodable_note};
use crate::safety::{AuditFacts, Audited, Operation, Target, count_text};
use crate::{Error, Result};
use futures_util::future::join_all;
use horizon_azure_kit::safety::CancellationToken;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// The most receivers one listener runs.
pub const MAX_CONCURRENCY: u8 = 8;
/// The most messages a receive-and-delete listener asks for at once, per receiver.
pub const MAX_PREFETCH: u32 = 100;

/// Idle time of one receive at credit 1: it returns as soon as a message comes, so this only
/// sets how often an idle link drains.
const RECEIVE_WAIT: Duration = Duration::from_secs(30);
/// Idle time of one receive with more credit: it returns only when its credit is spent or
/// the link goes quiet, so a short wait keeps a trickle of messages live.
const PREFETCH_WAIT: Duration = Duration::from_secs(1);
/// `Each`: a session quiet this long is let go for the next one.
const SESSION_IDLE: Duration = Duration::from_secs(5);
/// `Each`: how long one accept waits for a session to become available.
const ACCEPT_WAIT: Duration = Duration::from_secs(10);
/// `Watch`: the pause between peeks that found nothing new.
const WATCH_POLL: Duration = Duration::from_secs(1);
/// `Watch`: messages per peek.
const WATCH_PAGE: u32 = 100;

/// What a listener does with each message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenMode {
    /// Show messages and leave them: no lock, delivery count unchanged. Not gated.
    Watch,
    /// Peek-lock (credit 1 per receiver), then complete each message.
    Complete,
    /// Receive-and-delete, with up to `prefetch` messages of credit per receiver.
    Delete,
}

/// Which sessions a listener takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenSessions {
    /// Not a session entity.
    None,
    /// This session only.
    Id(String),
    /// Each session in turn: the next available one, until it goes quiet.
    Each,
}

/// What to listen to and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenRequest {
    /// Queue or subscription.
    pub entity: EntityPath,
    /// Which sub-queue.
    pub sub_queue: SubQueue,
    /// Sessions.
    pub sessions: ListenSessions,
    /// Watch, complete or delete.
    pub mode: ListenMode,
    /// Receivers run at once (1 to 8). `Watch` peeks with one.
    pub concurrency: u8,
    /// Credit per receiver for `Delete` (0 to 100; 0 means 1). Peek-lock refuses it.
    pub prefetch: u32,
}

/// How a listener ended: the report and its counts.
#[derive(Debug, Clone)]
pub struct ListenOutcome {
    /// Counts in its first note, failures as items.
    pub report: JobReport,
    /// Messages received (watched, completed or deleted), each emitted as it came.
    pub received: u64,
    /// Messages removed from the entity (those that could not be decoded included).
    pub removed: u64,
}

impl Audited for ListenOutcome {
    fn audit_outcome(&self) -> AuditFacts {
        let mut facts = self.report.audit_outcome();
        facts.succeeded = Some(self.removed);
        facts
    }

    fn set_op_id(&mut self, op_id: String) {
        self.report.set_op_id(op_id);
    }
}

/// Refuse a request the listener cannot run, before any I/O.
pub(crate) fn check(req: &ListenRequest) -> Result<()> {
    if req.concurrency == 0 || req.concurrency > MAX_CONCURRENCY {
        return Err(Error::Invalid(format!(
            "run 1 to {MAX_CONCURRENCY} receivers at a time"
        )));
    }
    if req.prefetch > MAX_PREFETCH {
        return Err(Error::Invalid(format!(
            "prefetch 0 to {MAX_PREFETCH} messages"
        )));
    }
    if req.prefetch > 0 && req.mode != ListenMode::Delete {
        return Err(Error::Invalid(
            "prefetch applies to receive-and-delete only; peek-lock receives one message at a time"
                .into(),
        ));
    }
    if matches!(req.sessions, ListenSessions::Id(_)) && req.concurrency > 1 {
        return Err(Error::Invalid(
            "one receiver holds a session; listen to one session with a concurrency of 1".into(),
        ));
    }
    if req.sessions != ListenSessions::None && req.sub_queue != SubQueue::Active {
        return Err(Error::Invalid(format!(
            "{} is not session-enabled; listen without a session",
            req.entity.address(req.sub_queue)
        )));
    }
    Ok(())
}

/// The gated operation of a listener that removes messages; `None` for `Watch`.
pub(crate) fn operation(req: &ListenRequest) -> Option<Operation> {
    let target = Target::new(req.entity.clone(), req.sub_queue);
    match req.mode {
        ListenMode::Watch => None,
        ListenMode::Complete => Some(Operation::listen(target, false)),
        ListenMode::Delete => Some(Operation::listen(target, true)),
    }
}

/// `3 messages received, 3 removed`.
fn counts_note(received: u64, removed: u64) -> String {
    format!(
        "{} {} received, {} removed",
        count_text(received),
        if received == 1 { "message" } else { "messages" },
        count_text(removed)
    )
}

/// What the receivers did, shared between them.
#[derive(Default)]
struct Counts {
    received: u64,
    removed: u64,
    undecodable: u64,
    uncertain: u64,
    failures: Vec<ItemResult>,
}

/// Emits each message as it comes and keeps the counts.
struct Tally<'a> {
    progress: &'a dyn Progress,
    counts: StdMutex<Counts>,
}

impl Tally<'_> {
    fn with<R>(&self, f: impl FnOnce(&mut Counts) -> R) -> R {
        f(&mut self.counts.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// A message was received (and `removed` or not): emit it, then the progress.
    fn received(&self, m: &PeekedMessage, removed: bool) {
        let done = self.with(|c| {
            c.received += 1;
            c.removed += u64::from(removed);
            c.received
        });
        self.progress
            .event(&JobEvent::Received(Box::new(m.clone())));
        self.progress
            .event(&JobEvent::Progress { done, total: None });
    }

    fn failed(&self, item: ItemResult) {
        self.progress.event(&JobEvent::Item(item.clone()));
        self.with(|c| c.failures.push(item));
    }
}

/// Run a listener until `cancel`. A failure before anything was received or removed is an
/// error; after that it is a failed item, and the other receivers go on.
pub(crate) async fn run<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &ListenRequest,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ListenOutcome> {
    check(req)?;
    let address = req.entity.address(req.sub_queue);
    let run = JobRun::new(JobKind::Listen, address.clone(), None, progress, None);
    let tally = Tally {
        progress,
        counts: StdMutex::new(Counts::default()),
    };
    let errors: Vec<Error> = if req.mode == ListenMode::Watch {
        watch(b, req, &address, &tally, cancel)
            .await
            .into_iter()
            .collect()
    } else {
        let backend = Mutex::new(b);
        let workers = (0..req.concurrency).map(|_| worker(&backend, req, &address, &tally, cancel));
        join_all(workers).await.into_iter().flatten().collect()
    };
    let counts = tally.counts.into_inner().unwrap_or_else(|e| e.into_inner());
    let touched = counts.received > 0
        || counts.undecodable > 0
        || counts.uncertain > 0
        || !counts.failures.is_empty();
    let mut errors = errors.into_iter();
    if !touched && let Some(e) = errors.next() {
        return Err(e);
    }
    let mut run = run;
    let removed = counts.removed + counts.undecodable;
    run.note(counts_note(counts.received, removed));
    if counts.uncertain > 0 {
        run.note(stopped_drain_note(
            u32::try_from(counts.uncertain).unwrap_or(u32::MAX),
        ));
    }
    if counts.undecodable > 0 {
        run.note(undecodable_note(
            u32::try_from(counts.undecodable).unwrap_or(u32::MAX),
        ));
    }
    let mut failures = counts.failures;
    for e in errors {
        let reason = format!("a receiver stopped: {e}");
        let item = ItemResult {
            sequence_number: None,
            message_id: None,
            session_id: None,
            entity: address.clone(),
            new_sequence_number: None,
            outcome: ItemOutcome::Failed {
                kind: error_kind(&e),
                reason: reason.clone(),
            },
        };
        progress.event(&JobEvent::Item(item.clone()));
        failures.push(item);
        run.note(reason);
    }
    // Stopping is how a listener ends: not a cancelled job.
    let mut report = run.finish(false);
    report.items = failures;
    Ok(ListenOutcome {
        report,
        received: counts.received,
        removed,
    })
}

/// The view a watcher peeks.
fn view_of(sub_queue: SubQueue) -> PeekView {
    match sub_queue {
        SubQueue::Active => PeekView::Active,
        SubQueue::DeadLetter => PeekView::DeadLetter,
        SubQueue::TransferDeadLetter => PeekView::TransferDeadLetter,
    }
}

/// Tail the entity with peeks, from its first message on, until `cancel`. Returns what stopped
/// it other than the cancel. A message scheduled before the watch began and enqueued during it
/// keeps its (earlier) sequence number, so the watch does not show it.
async fn watch<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &ListenRequest,
    address: &str,
    tally: &Tally<'_>,
    cancel: &CancellationToken,
) -> Option<Error> {
    let view = view_of(req.sub_queue);
    let session = match &req.sessions {
        ListenSessions::Id(id) => Some(id.as_str()),
        _ => None,
    };
    let mut whole = false;
    let mut next = 0i64;
    loop {
        let peek = async {
            match session {
                Some(id) if !whole => b.peek_session_batch(address, id, next, WATCH_PAGE).await,
                _ => b.peek_batch(address, next, WATCH_PAGE).await,
            }
        };
        let batch = tokio::select! {
            biased;
            _ = cancel.cancelled() => return None,
            r = peek => r,
        };
        let batch = match batch {
            Ok(batch) => batch,
            Err(Error::Unsupported { .. }) if session.is_some() && !whole => {
                whole = true;
                continue;
            }
            Err(e) => return Some(e),
        };
        let mut fresh = false;
        for m in batch {
            if m.sequence_number < next {
                continue;
            }
            next = m.sequence_number + 1;
            fresh = true;
            let in_session = session.is_none_or(|id| m.session_id.as_deref() == Some(id));
            if view.accepts(m.state) && in_session {
                tally.received(&m, false);
            }
        }
        if !fresh {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return None,
                _ = tokio::time::sleep(WATCH_POLL) => {}
            }
        }
    }
}

/// One receiver slot: opens a receiver (for `Each`, one session after another) and drives it
/// until `cancel`. Returns what stopped it other than the cancel.
async fn worker<B: MessagingBackend + ?Sized>(
    backend: &Mutex<&mut B>,
    req: &ListenRequest,
    address: &str,
    tally: &Tally<'_>,
    cancel: &CancellationToken,
) -> Option<Error> {
    let each = req.sessions == ListenSessions::Each;
    let spec = ReceiverSpec {
        address: address.to_string(),
        mode: if req.mode == ListenMode::Delete {
            ReceiveMode::ReceiveAndDelete
        } else {
            ReceiveMode::PeekLock
        },
        session: match &req.sessions {
            ListenSessions::None => SessionChoice::None,
            ListenSessions::Id(id) => SessionChoice::Id(id.clone()),
            ListenSessions::Each => SessionChoice::NextAvailable,
        },
    };
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        let open = async {
            let mut b = backend.lock().await;
            b.open_receiver(&spec, ACCEPT_WAIT).await
        };
        let opened = tokio::select! {
            biased;
            _ = cancel.cancelled() => return None,
            r = open => r,
        };
        let rx = match opened {
            Ok(Some(rx)) => rx,
            // No session became available within the wait: ask again.
            Ok(None) => continue,
            Err(e) => return Some(e),
        };
        if let Some(e) = drive(rx, req, address, each, tally, cancel).await {
            return Some(e);
        }
        if !each {
            return None;
        }
    }
}

/// When a session receiver renews its session lock next: at half its remaining span.
fn renew_at(until: time::OffsetDateTime) -> (Instant, Duration) {
    let left = until - wall_now();
    let half = Duration::try_from(left / 2)
        .unwrap_or(Duration::ZERO)
        .max(Duration::from_secs(1));
    (Instant::now() + half, half)
}

/// Receive from `rx` until `cancel`, a failure, or (`each`) a quiet session; then close it and
/// count what the close returned. Returns the failure.
async fn drive(
    mut rx: Box<dyn LockReceiver>,
    req: &ListenRequest,
    address: &str,
    each: bool,
    tally: &Tally<'_>,
    cancel: &CancellationToken,
) -> Option<Error> {
    let delete = req.mode == ListenMode::Delete;
    // Peek-lock keeps credit 1 (§14 Decision 10).
    let ask = if delete { req.prefetch.max(1) } else { 1 };
    let mut wait = if ask > 1 {
        PREFETCH_WAIT
    } else if each {
        SESSION_IDLE
    } else {
        RECEIVE_WAIT
    };
    let mut error = None;
    // A session receiver keeps its session lock: renewed at half its span, and no receive
    // waits longer than that.
    let mut renew = None;
    if rx.session_id().is_some() {
        match rx.renew_session_lock().await {
            Ok(until) => {
                let (at, half) = renew_at(until);
                wait = wait.min(half);
                renew = Some(at);
            }
            Err(e) => error = Some(e),
        }
    }
    while error.is_none() && !cancel.is_cancelled() {
        if let Some(at) = renew
            && Instant::now() >= at
        {
            match rx.renew_session_lock().await {
                Ok(until) => renew = Some(renew_at(until).0),
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        // `receive` is cancel safe: what it took stays buffered, and the close returns it.
        let batch = tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            r = rx.receive(ask, wait) => r,
        };
        let batch = match batch {
            Ok(batch) => batch,
            Err(e) => {
                error = Some(e);
                break;
            }
        };
        let quiet = batch.is_empty();
        for m in batch {
            match (delete, m.lock_token) {
                (true, _) => tally.received(&m.message, true),
                (false, Some(token)) => match rx.settle(token, &Settlement::Complete).await {
                    Ok(()) => tally.received(&m.message, true),
                    Err(e) => {
                        tally.received(&m.message, false);
                        tally.failed(failed_item(address, &m.message, &e));
                    }
                },
                (false, None) => {
                    tally.received(&m.message, false);
                    tally.failed(failed_item(
                        address,
                        &m.message,
                        &Error::Protocol("a peek-lock delivery came without a lock token".into()),
                    ));
                }
            }
        }
        if quiet && each {
            // The session went quiet: let it go for the next one.
            break;
        }
    }
    if error.is_none() {
        // A failure the receiver held back while returning what it had taken.
        error = rx.take_error();
    }
    let closed = rx.close().await;
    for m in &closed.removed {
        tally.received(&m.message, true);
    }
    tally.with(|c| {
        c.undecodable += u64::from(closed.undecodable);
        c.uncertain += u64::from(closed.uncertain);
    });
    error
}

/// The failed item of a message that could not be completed.
fn failed_item(address: &str, m: &PeekedMessage, e: &Error) -> ItemResult {
    ItemResult {
        sequence_number: Some(m.sequence_number),
        message_id: m.message_id.clone(),
        session_id: m.session_id.clone(),
        entity: address.to_string(),
        new_sequence_number: None,
        outcome: ItemOutcome::Failed {
            kind: error_kind(e),
            reason: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobEvent, JobKind, RecordingProgress};
    use crate::messaging::fake::{FakeBroker, FakeMsg};
    use crate::safety::{Operation, ProfileRef, Target};
    use std::time::Duration;
    use tokio::time::{Instant, sleep};

    fn req(mode: ListenMode) -> ListenRequest {
        ListenRequest {
            entity: EntityPath::queue("q"),
            sub_queue: SubQueue::Active,
            sessions: ListenSessions::None,
            mode,
            concurrency: 1,
            prefetch: 0,
        }
    }

    /// Cancel `token` after `after`.
    fn cancel_after(after: Duration) -> CancellationToken {
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            sleep(after).await;
            stop.cancel();
        });
        cancel
    }

    fn bodies(events: &RecordingProgress) -> Vec<String> {
        events
            .events()
            .into_iter()
            .filter_map(|e| match e {
                JobEvent::Received(m) => m.body.as_text(),
                _ => None,
            })
            .collect()
    }

    fn sessions_received(events: &RecordingProgress) -> Vec<String> {
        events
            .events()
            .into_iter()
            .filter_map(|e| match e {
                JobEvent::Received(m) => m.session_id.clone(),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn watch_releases_and_leaves_delivery_count() {
        // Watch shows every message, the ones already there and those that arrive, and leaves
        // them: no lock, no delivery count, nothing removed.
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("one"));
        b.add("q", FakeMsg::active("two"));
        let feed = b.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(3)).await;
            feed.add("q", FakeMsg::active("three"));
        });
        let events = RecordingProgress::default();
        let out = run(
            &mut b.clone(),
            &req(ListenMode::Watch),
            &events,
            &cancel_after(Duration::from_secs(10)),
        )
        .await
        .unwrap();
        assert_eq!(bodies(&events), ["one", "two", "three"]);
        assert_eq!((out.received, out.removed), (3, 0));
        let left = b.all("q");
        assert_eq!(left.len(), 3);
        assert!(left.iter().all(|m| m.delivery_count == 0));
        assert_eq!(b.held_locks(), 0);
        assert_eq!(b.open_receivers(), 0);
        assert_eq!(out.report.kind, JobKind::Listen);
        assert!(!out.report.cancelled, "stopping is how a listener ends");
        assert_eq!(out.report.notes[0], "3 messages received, 0 removed");
        assert_eq!(
            out.report.summary_line(),
            "Listened to q: 3 messages received, 0 removed"
        );
        assert!(
            operation(&req(ListenMode::Watch)).is_none(),
            "watch is not gated"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn complete_removes_each_and_streams_it() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("one"));
        b.add("q", FakeMsg::active("two"));
        let feed = b.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(2)).await;
            feed.add("q", FakeMsg::active("three"));
        });
        let events = RecordingProgress::default();
        let out = run(
            &mut b.clone(),
            &req(ListenMode::Complete),
            &events,
            &cancel_after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(bodies(&events), ["one", "two", "three"]);
        assert!(b.all("q").is_empty());
        assert_eq!((out.received, out.removed), (3, 3));
        assert_eq!(b.credit_granted_max(), 1, "peek-lock keeps credit 1");
        assert_eq!(b.held_locks(), 0);
        let done: Vec<u64> = events
            .events()
            .into_iter()
            .filter_map(|e| match e {
                JobEvent::Progress { done, .. } => Some(done),
                _ => None,
            })
            .collect();
        assert_eq!(done, [1, 2, 3]);
        assert!(out.report.items.is_empty(), "counts, not items");
        assert_eq!(out.report.notes, ["3 messages received, 3 removed"]);
        let op = operation(&req(ListenMode::Complete)).expect("gated");
        assert_eq!(
            op.summary(&ProfileRef {
                name: "dev".into(),
                production: false
            }),
            "Remove every message that arrives on q until stopped, on dev (receive and complete)"
        );
        assert_eq!(op.count, crate::safety::CountEstimate::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn delete_with_prefetch_never_exceeds_prefetch_credit() {
        let b = FakeBroker::default();
        for _ in 0..30 {
            b.add("q", FakeMsg::active("x"));
        }
        let mut r = req(ListenMode::Delete);
        r.prefetch = 10;
        let out = run(
            &mut b.clone(),
            &r,
            &RecordingProgress::default(),
            &cancel_after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!((out.received, out.removed), (30, 30));
        assert!(b.all("q").is_empty());
        assert_eq!(b.credit_granted_max(), 10);
        // Without prefetch, one message at a time.
        let b = FakeBroker::default();
        for _ in 0..3 {
            b.add("q", FakeMsg::active("x"));
        }
        let out = run(
            &mut b.clone(),
            &req(ListenMode::Delete),
            &RecordingProgress::default(),
            &cancel_after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(out.removed, 3);
        assert_eq!(b.credit_granted_max(), 1);
        let op = operation(&r).expect("gated");
        assert_eq!(
            op.summary(&ProfileRef {
                name: "prod".into(),
                production: true
            }),
            "Remove every message that arrives on q until stopped, on prod (receive and delete)"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn listener_close_reports_swept_messages() {
        // Ctrl-C while a receive-and-delete receiver has credit out: the close sweeps a late
        // delivery, which was removed, so it is emitted and counted, never lost silently.
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("first"));
        b.add_at_close("q", FakeMsg::active("late"));
        let mut r = req(ListenMode::Delete);
        r.prefetch = 5;
        let events = RecordingProgress::default();
        let out = run(
            &mut b.clone(),
            &r,
            &events,
            &cancel_after(Duration::from_secs(3)),
        )
        .await
        .unwrap();
        assert_eq!(bodies(&events), ["first", "late"]);
        assert_eq!((out.received, out.removed), (2, 2));
        assert!(b.all("q").is_empty());
        assert_eq!(out.report.notes, ["2 messages received, 2 removed"]);
        // A close that does not complete leaves its credit uncertain, and says so.
        let b = FakeBroker::default();
        b.fail_next_close();
        let out = run(
            &mut b.clone(),
            &r,
            &RecordingProgress::default(),
            &cancel_after(Duration::from_secs(3)),
        )
        .await
        .unwrap();
        assert_eq!(out.removed, 0);
        assert_eq!(
            out.report.notes[1],
            crate::ops::receive::stopped_drain_note(5)
        );
        // Removed but undecodable: counted as removed and noted.
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        b.add("q", FakeMsg::active("y"));
        b.undecodable_next(1);
        let out = run(
            &mut b.clone(),
            &r,
            &RecordingProgress::default(),
            &cancel_after(Duration::from_secs(3)),
        )
        .await
        .unwrap();
        assert_eq!((out.received, out.removed), (1, 2));
        assert_eq!(
            out.report.notes,
            [
                "1 message received, 2 removed".to_string(),
                crate::ops::receive::undecodable_note(1)
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn concurrency_runs_n_receivers() {
        let b = FakeBroker::default().with_receive_delay(Duration::from_millis(100));
        for _ in 0..12 {
            b.add("q", FakeMsg::active("x"));
        }
        let probe = b.clone();
        let seen = tokio::spawn(async move {
            sleep(Duration::from_secs(1)).await;
            probe.open_receivers()
        });
        let mut r = req(ListenMode::Complete);
        r.concurrency = 3;
        let out = run(
            &mut b.clone(),
            &r,
            &RecordingProgress::default(),
            &cancel_after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(seen.await.unwrap(), 3);
        assert_eq!(out.removed, 12);
        assert!(b.all("q").is_empty());
        assert_eq!(b.open_receivers(), 0, "every receiver closed");
        assert_eq!(b.credit_granted_max(), 1, "credit 1 on each");
    }

    #[tokio::test]
    async fn peek_lock_with_prefetch_is_refused() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        let cancel = CancellationToken::new();
        let mut bad = Vec::new();
        for mode in [ListenMode::Complete, ListenMode::Watch] {
            let mut r = req(mode);
            r.prefetch = 5;
            bad.push(r);
        }
        let mut r = req(ListenMode::Delete);
        r.prefetch = MAX_PREFETCH + 1;
        bad.push(r);
        for concurrency in [0, MAX_CONCURRENCY + 1] {
            let mut r = req(ListenMode::Delete);
            r.concurrency = concurrency;
            bad.push(r);
        }
        let mut r = req(ListenMode::Complete);
        r.sessions = ListenSessions::Id("s".into());
        r.concurrency = 2;
        bad.push(r);
        for r in bad {
            let e = run(&mut b.clone(), &r, &RecordingProgress::default(), &cancel).await;
            assert!(matches!(e, Err(Error::Invalid(_))), "{r:?}");
        }
        let mut r = req(ListenMode::Complete);
        r.prefetch = 5;
        let Err(Error::Invalid(text)) = check(&r) else {
            panic!("refused")
        };
        assert_eq!(
            text,
            "prefetch applies to receive-and-delete only; peek-lock receives one message at a time"
        );
        assert_eq!(b.all("q").len(), 1);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn each_session_moves_to_the_next_session() {
        let b = FakeBroker::default();
        b.require_sessions("q");
        for id in ["a", "a", "b", "b"] {
            b.add("q", FakeMsg::active("x").in_session(id));
        }
        let feed = b.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(30)).await;
            feed.add("q", FakeMsg::active("x").in_session("c"));
        });
        let mut r = req(ListenMode::Complete);
        r.sessions = ListenSessions::Each;
        let events = RecordingProgress::default();
        let out = run(
            &mut b.clone(),
            &r,
            &events,
            &cancel_after(Duration::from_secs(60)),
        )
        .await
        .unwrap();
        assert_eq!(sessions_received(&events), ["a", "a", "b", "b", "c"]);
        assert_eq!(out.removed, 5);
        assert!(b.all("q").is_empty());
        assert_eq!(b.open_receivers(), 0);
        // One session by id: only its messages.
        for id in ["a", "b"] {
            b.add("q", FakeMsg::active("x").in_session(id));
        }
        r.sessions = ListenSessions::Id("b".into());
        let events = RecordingProgress::default();
        run(
            &mut b.clone(),
            &r,
            &events,
            &cancel_after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(sessions_received(&events), ["b"]);
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_ends_within_two_seconds() {
        // An idle receiver, an idle session listener waiting for a session, and a watcher: each
        // ends within two seconds of the cancel.
        let b = FakeBroker::default();
        b.require_sessions("s");
        let mut each = req(ListenMode::Delete);
        each.entity = EntityPath::queue("s");
        each.sessions = ListenSessions::Each;
        let mut many = req(ListenMode::Complete);
        many.concurrency = 4;
        for r in [
            req(ListenMode::Complete),
            each,
            req(ListenMode::Watch),
            many,
        ] {
            let cancel = cancel_after(Duration::from_secs(7));
            let started = Instant::now();
            run(&mut b.clone(), &r, &RecordingProgress::default(), &cancel)
                .await
                .unwrap();
            let after_cancel = started.elapsed() - Duration::from_secs(7);
            assert!(
                after_cancel <= Duration::from_secs(2),
                "{r:?}: {after_cancel:?}"
            );
            assert_eq!(b.open_receivers(), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_denied_listener_removes_nothing_and_an_allowed_one_is_audited() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        let r = req(ListenMode::Delete);
        let op = operation(&r).unwrap();
        let (gate, audit) = crate::ops::testkit::deny_gate();
        let cancel = cancel_after(Duration::from_secs(3));
        let denied = crate::ops::gated(&gate, &op, || async {
            run(&mut b.clone(), &r, &RecordingProgress::default(), &cancel).await
        })
        .await;
        assert!(matches!(denied, Err(Error::Denied(_))));
        assert_eq!(b.all("q").len(), 1);
        assert_eq!(audit.entries()[0].outcome, crate::activity::Outcome::Denied);
        let (gate, audit) = crate::ops::testkit::fake_gate();
        let out = crate::ops::gated(&gate, &op, || async {
            run(&mut b.clone(), &r, &RecordingProgress::default(), &cancel).await
        })
        .await
        .unwrap();
        assert_eq!(out.removed, 1);
        let finished = &audit.entries()[1];
        assert_eq!(finished.outcome, crate::activity::Outcome::Succeeded);
        assert_eq!(finished.succeeded, Some(1));
        assert_eq!(out.report.op_id.as_deref(), Some(finished.op_id.as_str()));
    }

    #[test]
    fn listeners_name_their_target() {
        let mut r = req(ListenMode::Delete);
        r.sub_queue = SubQueue::DeadLetter;
        let op = operation(&r).unwrap();
        assert_eq!(
            op.target,
            Target::new(EntityPath::queue("q"), SubQueue::DeadLetter)
        );
        let _: Operation = op;
    }
}
