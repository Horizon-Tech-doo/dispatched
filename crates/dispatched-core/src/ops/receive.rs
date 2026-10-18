//! Receive (peek-lock and settle, or receive-and-delete) and settling deferred messages.

use super::failed;
use super::send::distinct;
use crate::diagnostics::error_kind;
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, NotReached, Progress};
use crate::messaging::backend::{LockReceiver, MessagingBackend};
use crate::messaging::sender::{chunks, copy_error};
use crate::messaging::{
    PeekedMessage, ReceiveMode, ReceivedMessage, ReceiverSpec, SessionChoice, Settlement,
};
use crate::namespace::{EntityPath, SubQueue};
use crate::safety::{AuditFacts, Audited, count_text};
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

/// The most messages one receive may take.
pub const MAX_RECEIVE: u32 = 5_000;

/// What happens to each received message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveAction {
    /// Peek-lock, then settle each message this way.
    Settle(Settlement),
    /// Receive-and-delete: messages are removed as they are delivered.
    Delete,
}

/// What to receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveRequest {
    /// Queue or subscription.
    pub entity: EntityPath,
    /// Which sub-queue.
    pub sub_queue: SubQueue,
    /// Accept this session (session-enabled entities).
    pub session: Option<String>,
    /// At most this many messages (1 to 5,000).
    pub max: u32,
    /// Stop when this passes without a delivery.
    pub wait: Duration,
    /// Settle or delete.
    pub action: ReceiveAction,
}

/// The received messages and the job report.
#[derive(Debug, Clone, Serialize)]
pub struct ReceiveOutcome {
    /// Every message received, in order.
    pub messages: Vec<PeekedMessage>,
    /// One item per message.
    pub report: JobReport,
}

impl Audited for ReceiveOutcome {
    fn audit_outcome(&self) -> AuditFacts {
        self.report.audit_outcome()
    }

    fn set_op_id(&mut self, op_id: String) {
        self.report.set_op_id(op_id);
    }
}

/// The report note for credit a receive-and-delete link left unaccounted when a receive
/// stopped early (cancelled or failed) instead of at the wait.
pub(crate) fn stopped_drain_note(n: u32) -> String {
    if n == 1 {
        "up to 1 more message may have been removed after the receive stopped and is not in this result".into()
    } else {
        format!(
            "up to {} more messages may have been removed after the receive stopped and are not in this result",
            count_text(u64::from(n))
        )
    }
}

/// The report note for credit a receive-and-delete link left unaccounted after the wait.
pub(crate) fn drain_note(n: u32) -> String {
    if n == 1 {
        "up to 1 more message may have been removed after the wait and is not in this result".into()
    } else {
        format!(
            "up to {} more messages may have been removed after the wait and are not in this result",
            count_text(u64::from(n))
        )
    }
}

/// The report note for receive-and-delete messages a closing receiver had taken but not returned.
pub(crate) fn unreturned_note(n: usize) -> String {
    if n == 1 {
        "1 message was removed but is not in this result".into()
    } else {
        format!(
            "{} messages were removed but are not in this result",
            count_text(n as u64)
        )
    }
}

/// The item for a received or settled message.
fn item(entity: &str, seq: i64, m: Option<&PeekedMessage>, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: Some(seq),
        message_id: m.and_then(|m| m.message_id.clone()),
        session_id: m.and_then(|m| m.session_id.clone()),
        entity: entity.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

/// How a receiver's take ended ([`take_and_close`]).
pub(crate) struct Ended {
    pub(crate) cancelled: bool,
    /// The last receive's unaccounted credit.
    pub(crate) shortfall: u32,
    pub(crate) error: Option<Error>,
    /// Messages the receiver returned (each recorded as an item).
    pub(crate) received: u64,
    /// Receive-and-delete messages the closed receiver had taken but never returned.
    pub(crate) unreturned: usize,
}

impl Ended {
    /// The most messages a receive-and-delete link may have removed: those it returned, those
    /// it took but never returned, and its unaccounted credit.
    pub(crate) fn removed_at_most(&self) -> u64 {
        self.received
            .saturating_add(self.unreturned as u64)
            .saturating_add(u64::from(self.shortfall))
    }
}

/// Run a receive job. Peek-lock takes one message per call and settles it at once, so no
/// lock waits; receive-and-delete asks for everything still allowed. `wait` is an idle
/// timeout: both stop at `max`, on cancel, on a call that returns nothing within `wait`, or on
/// one that ends with credit unaccounted. The receiver is always closed; receive-and-delete
/// messages it removed without returning are noted. A link failure after some messages were
/// received (or removed) keeps them and adds a failed item, so the job reads as incomplete.
pub(crate) async fn run<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &ReceiveRequest,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ReceiveOutcome> {
    if req.max == 0 || req.max > MAX_RECEIVE {
        return Err(Error::Invalid(format!(
            "receive 1 to {} messages at a time",
            count_text(u64::from(MAX_RECEIVE))
        )));
    }
    let address = req.entity.address(req.sub_queue);
    let delete = req.action == ReceiveAction::Delete;
    let spec = ReceiverSpec {
        address: address.clone(),
        mode: if delete {
            ReceiveMode::ReceiveAndDelete
        } else {
            ReceiveMode::PeekLock
        },
        session: match &req.session {
            Some(id) => SessionChoice::Id(id.clone()),
            None => SessionChoice::None,
        },
    };
    let Some(rx) = b.open_receiver(&spec, req.wait).await? else {
        return Err(Error::Protocol(format!(
            "no receiver was opened on {address}"
        )));
    };
    let mut run = JobRun::new(
        JobKind::Receive,
        address.clone(),
        None,
        progress,
        Some(u64::from(req.max)),
    );
    let mut messages = Vec::new();
    let mut ended = take_and_close(rx, req, &address, &mut run, Some(&mut messages), cancel).await;
    let nothing_lost = !delete || (ended.shortfall == 0 && ended.unreturned == 0);
    if messages.is_empty()
        && nothing_lost
        && let Some(e) = ended.error.take()
    {
        return Err(e);
    }
    let cancelled = record_end(&mut run, req, &address, ended);
    Ok(ReceiveOutcome {
        messages,
        report: run.finish(cancelled),
    })
}

/// Take messages from `rx` as `req` says (each recorded in `run`, and kept in `messages` when
/// given), then close it, on every path. Stops at `req.max`, on cancel, on a call that returns
/// nothing within `req.wait`, or on one that ends with credit unaccounted; a failure the receiver
/// held back is collected, never dropped.
pub(crate) async fn take_and_close(
    mut rx: Box<dyn LockReceiver>,
    req: &ReceiveRequest,
    address: &str,
    run: &mut JobRun<'_>,
    messages: Option<&mut Vec<PeekedMessage>>,
    cancel: &CancellationToken,
) -> Ended {
    let mut ended = take_all(rx.as_mut(), req, address, run, messages, cancel).await;
    ended.unreturned = rx.close().await;
    ended
}

/// Put how a receiver ended into `run`: a link failure as a failed item (and a note), then the
/// drain and unreturned notes of a receive-and-delete link. Returns whether it was cancelled.
pub(crate) fn record_end(
    run: &mut JobRun<'_>,
    req: &ReceiveRequest,
    address: &str,
    ended: Ended,
) -> bool {
    let delete = req.action == ReceiveAction::Delete;
    let stopped = ended.cancelled || ended.error.is_some();
    if let Some(e) = ended.error {
        let reason = format!("the receive stopped early: {e}");
        run.item(ItemResult {
            sequence_number: None,
            message_id: None,
            session_id: req.session.clone(),
            entity: address.to_string(),
            new_sequence_number: None,
            outcome: ItemOutcome::Failed {
                kind: error_kind(&e),
                reason: reason.clone(),
            },
        });
        run.note(reason);
    }
    if delete && ended.shortfall > 0 {
        run.note(if stopped {
            stopped_drain_note(ended.shortfall)
        } else {
            drain_note(ended.shortfall)
        });
    }
    if ended.unreturned > 0 {
        run.note(unreturned_note(ended.unreturned));
    }
    ended.cancelled
}

async fn take_all(
    rx: &mut dyn LockReceiver,
    req: &ReceiveRequest,
    address: &str,
    run: &mut JobRun<'_>,
    mut messages: Option<&mut Vec<PeekedMessage>>,
    cancel: &CancellationToken,
) -> Ended {
    let mut ended = Ended {
        cancelled: false,
        shortfall: 0,
        error: None,
        received: 0,
        unreturned: 0,
    };
    let mut got = 0u32;
    while got < req.max {
        if cancel.is_cancelled() {
            ended.cancelled = true;
            break;
        }
        let ask = match req.action {
            // One lock at a time: a settle that hangs lasts about as long as a lock, so
            // prefetched messages would lose theirs. Do not raise this.
            ReceiveAction::Settle(_) => 1,
            ReceiveAction::Delete => req.max - got,
        };
        // `receive` is cancel safe: what it took stays buffered on the receiver.
        let batch = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            r = rx.receive(ask, req.wait) => Some(r),
        };
        ended.shortfall = rx.drain_shortfall();
        let batch = match batch {
            Some(Ok(batch)) => batch,
            Some(Err(e)) => {
                ended.error = Some(e);
                break;
            }
            None => {
                ended.cancelled = true;
                if req.action == ReceiveAction::Delete && rx.buffered() > 0 {
                    // Already removed from the entity: return them rather than lose them.
                    // Asking for no more than is buffered grants no credit.
                    let n = u32::try_from(rx.buffered()).unwrap_or(u32::MAX);
                    match rx.receive(n, Duration::ZERO).await {
                        Ok(batch) => {
                            ended.received += batch.len() as u64;
                            record(rx, req, address, run, messages.as_deref_mut(), batch).await
                        }
                        Err(e) => ended.error = Some(e),
                    }
                }
                break;
            }
        };
        let n = u32::try_from(batch.len()).unwrap_or(u32::MAX);
        ended.received += batch.len() as u64;
        record(rx, req, address, run, messages.as_deref_mut(), batch).await;
        got = got.saturating_add(n);
        if n == 0 || ended.shortfall > 0 {
            // `wait` passed without a delivery, or the drain left credit unaccounted.
            break;
        }
    }
    if ended.error.is_none() {
        // A failure the receiver held back while returning what it had taken.
        ended.error = rx.take_error();
    }
    ended
}

/// Settle (peek-lock) and record each received message.
async fn record(
    rx: &mut dyn LockReceiver,
    req: &ReceiveRequest,
    address: &str,
    run: &mut JobRun<'_>,
    mut messages: Option<&mut Vec<PeekedMessage>>,
    batch: Vec<ReceivedMessage>,
) {
    for m in batch {
        let outcome = match (&req.action, m.lock_token) {
            (ReceiveAction::Delete, _) => ItemOutcome::Succeeded { detail: None },
            (ReceiveAction::Settle(how), Some(token)) => match rx.settle(token, how).await {
                Ok(()) => ItemOutcome::Succeeded { detail: None },
                Err(e) => failed(&e),
            },
            (ReceiveAction::Settle(_), None) => failed(&Error::Protocol(
                "a peek-lock delivery came without a lock token".into(),
            )),
        };
        run.item(item(
            address,
            m.message.sequence_number,
            Some(&m.message),
            outcome,
        ));
        if let Some(kept) = messages.as_deref_mut() {
            kept.push(m.message);
        }
    }
}

/// Receive deferred messages of `address` by sequence number (chunks of 100) and settle each
/// one with `how`. Numbers the broker does not return are `missing`. Every message received is
/// settled (received deferred messages are not held, so nothing else would unlock them).
pub(crate) async fn settle_deferred<B: MessagingBackend + ?Sized>(
    b: &mut B,
    address: &str,
    seqs: &[i64],
    session: Option<&str>,
    how: &Settlement,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let seqs = distinct(seqs);
    let mut run = JobRun::new(
        JobKind::SettleDeferred,
        address.to_string(),
        None,
        progress,
        Some(seqs.len() as u64),
    );
    if cancel.is_cancelled() {
        not_reached(&mut run, address, &seqs);
        return Ok(run.finish(true));
    }
    let spec = ReceiverSpec {
        address: address.to_string(),
        mode: ReceiveMode::PeekLock,
        session: match session {
            Some(id) => SessionChoice::Id(id.to_string()),
            None => SessionChoice::None,
        },
    };
    let Some(mut rx) = b.open_receiver(&spec, Duration::ZERO).await? else {
        return Err(Error::Protocol(format!(
            "no receiver was opened on {address}"
        )));
    };
    let mut cancelled = false;
    let mut done = 0;
    for chunk in chunks(&seqs) {
        if cancel.is_cancelled() {
            not_reached(&mut run, address, &seqs[done..]);
            cancelled = true;
            break;
        }
        done += chunk.len();
        settle_chunk(rx.as_mut(), address, chunk, how, &mut run).await;
    }
    rx.close().await;
    Ok(run.finish(cancelled))
}

fn not_reached(run: &mut JobRun<'_>, address: &str, seqs: &[i64]) {
    for seq in seqs {
        run.item(item(
            address,
            *seq,
            None,
            ItemOutcome::NotReached {
                why: NotReached::Cancelled,
            },
        ));
    }
}

/// Receive one chunk by sequence number and settle what came back. The broker refuses the
/// whole request when one number is unknown, so then each number is received on its own.
async fn settle_chunk(
    rx: &mut dyn LockReceiver,
    address: &str,
    chunk: &[i64],
    how: &Settlement,
    run: &mut JobRun<'_>,
) {
    let mut received = Vec::new();
    let mut errors: HashMap<i64, Error> = HashMap::new();
    match rx.receive_deferred(chunk).await {
        Ok(got) => received = got,
        Err(Error::NotFound(_)) if chunk.len() > 1 => {
            for seq in chunk {
                match rx.receive_deferred(std::slice::from_ref(seq)).await {
                    Ok(got) => received.extend(got),
                    Err(Error::NotFound(_)) => {}
                    Err(e) => {
                        errors.insert(*seq, e);
                    }
                }
            }
        }
        Err(Error::NotFound(_)) => {}
        Err(e) => {
            for seq in chunk {
                errors.insert(*seq, copy_error(&e));
            }
        }
    }
    let mut settled: HashMap<i64, (PeekedMessage, ItemOutcome)> = HashMap::new();
    for m in received {
        let outcome = match m.lock_token {
            Some(token) => match rx.settle(token, how).await {
                Ok(()) => ItemOutcome::Succeeded { detail: None },
                Err(e) => failed(&e),
            },
            None => failed(&Error::Protocol(
                "a deferred message came without a lock token".into(),
            )),
        };
        settled.insert(m.message.sequence_number, (m.message, outcome));
    }
    for seq in chunk {
        let result = match (settled.remove(seq), errors.remove(seq)) {
            (Some((m, outcome)), _) => item(address, *seq, Some(&m), outcome),
            (None, Some(e)) => item(address, *seq, None, failed(&e)),
            (None, None) => item(address, *seq, None, ItemOutcome::Missing),
        };
        run.item(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::jobs::NoProgress;
    use crate::messaging::MessageState;
    use crate::messaging::fake::{FakeBroker, FakeMsg, FakeOp};
    use crate::ops::receive;
    use crate::ops::testkit::recv;

    #[tokio::test(start_paused = true)]
    async fn peek_lock_receive_settles_each_message_and_stops_at_max() {
        let b = FakeBroker::default();
        for _ in 0..5 {
            b.add("q", FakeMsg::active("x"));
        }
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 3, ReceiveAction::Settle(Settlement::Complete)),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.messages.len(), 3);
        assert_eq!(b.all("q").len(), 2);
        assert_eq!(b.held_locks(), 0);
        assert!(b.credit_granted_max() <= 3);
    }

    #[tokio::test(start_paused = true)]
    async fn receive_and_delete_reports_messages_drained_after_the_wait() {
        let b = FakeBroker::default();
        for _ in 0..4 {
            b.add("q", FakeMsg::active("x"));
        }
        b.deliver_late(2);
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 4, ReceiveAction::Delete),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            out.messages.len(),
            4,
            "late deliveries are already removed, so they are returned"
        );
        assert!(b.credit_granted_max() <= 4);
        assert!(
            out.report.notes.is_empty(),
            "the drain was confirmed: no shortfall note"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unconfirmed_drain_is_noted_in_the_report() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        b.stall_drain(2);
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 3, ReceiveAction::Delete),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.messages.len(), 1);
        assert!(
            out.report.notes.iter().any(|n| n
                == "up to 2 more messages may have been removed after the wait and are not in this result"),
            "{:?}",
            out.report.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn dead_letter_receive_carries_the_reason() {
        let b = FakeBroker::default();
        let seq = b.add("q", FakeMsg::active("x"));
        let how = Settlement::DeadLetter {
            reason: Some("Manual".into()),
            description: Some("bad".into()),
        };
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 1, ReceiveAction::Settle(how)),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.report.succeeded(), 1);
        assert!(b.all("q").is_empty());
        let dl = b.all("q/$DeadLetterQueue");
        assert_eq!(dl[0].sequence_number, seq);
        assert_eq!(
            (
                dl[0].dead_letter_reason.as_deref(),
                dl[0].dead_letter_description.as_deref()
            ),
            (Some("Manual"), Some("bad"))
        );
        // No reason given: none is set (not an empty one).
        b.add("q", FakeMsg::active("y"));
        let how = Settlement::DeadLetter {
            reason: None,
            description: None,
        };
        receive::run(
            &mut b.clone(),
            &recv("q", 1, ReceiveAction::Settle(how)),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let dl = b.all("q/$DeadLetterQueue");
        assert_eq!(dl.len(), 2);
        assert!(
            !dl[1]
                .application_properties
                .contains_key("DeadLetterReason")
        );
        assert_eq!(dl[1].dead_letter_reason, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_settle_fails_its_item_and_the_receive_goes_on() {
        let b = FakeBroker::default();
        for _ in 0..3 {
            b.add("q", FakeMsg::active("x"));
        }
        b.expire_lock_on_next_settle();
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 2, ReceiveAction::Settle(Settlement::Complete)),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!((out.report.succeeded(), out.report.failed()), (1, 1));
        assert!(matches!(
            &out.report.items[0].outcome,
            ItemOutcome::Failed {
                kind: "lock_lost",
                ..
            }
        ));
        assert_eq!(b.held_locks(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn receive_stops_when_the_wait_passes_without_a_delivery() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 10, ReceiveAction::Settle(Settlement::Complete)),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.messages.len(), 1);
        assert!(!out.report.cancelled);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_peek_lock_receive_closes_and_says_cancelled() {
        let b = FakeBroker::default().with_receive_delay(Duration::from_secs(10));
        b.add("q", FakeMsg::active("x"));
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            stop.cancel();
        });
        let mut req = recv("q", 5, ReceiveAction::Settle(Settlement::Complete));
        req.wait = Duration::from_secs(60);
        let out = receive::run(&mut b.clone(), &req, &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(out.report.cancelled);
        assert!(out.messages.is_empty());
        assert_eq!(b.held_locks(), 0);
        assert_eq!(b.all("q")[0].delivery_count, 0, "closed, not dropped");
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_receive_and_delete_returns_what_the_link_already_delivered() {
        // One message is taken off the link, then the receive is cancelled while it waits
        // for the second: the first is already removed, so it is returned, not lost.
        let b = FakeBroker::default().with_receive_delay(Duration::from_secs(1));
        for _ in 0..2 {
            b.add("q", FakeMsg::active("x"));
        }
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            stop.cancel();
        });
        let mut req = recv("q", 2, ReceiveAction::Delete);
        req.wait = Duration::from_secs(60);
        let out = receive::run(&mut b.clone(), &req, &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(out.report.cancelled);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.report.succeeded(), 1);
        assert_eq!(b.all("q").len(), 1);
        assert!(b.credit_granted_max() <= 2);
        assert!(
            out.report.notes.contains(&stopped_drain_note(1)),
            "credit was out when it stopped: {:?}",
            out.report.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn wait_is_an_idle_timeout_so_a_steady_trickle_is_all_received() {
        // A message arrives every second; the wait is 1.5 s. The whole run takes longer than
        // the wait, but no gap is as long as it, so every message is received.
        for action in [
            ReceiveAction::Delete,
            ReceiveAction::Settle(Settlement::Complete),
        ] {
            let b = FakeBroker::default().with_receive_delay(Duration::from_millis(1));
            let feed = b.clone();
            tokio::spawn(async move {
                for _ in 0..3 {
                    feed.add("q", FakeMsg::active("x"));
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
            let mut req = recv("q", 5, action.clone());
            req.wait = Duration::from_millis(1_500);
            let out = receive::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new())
                .await
                .unwrap();
            assert_eq!(out.messages.len(), 3, "{action:?}");
            assert!(b.all("q").is_empty());
            assert!(out.report.notes.is_empty(), "{:?}", out.report.notes);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn messages_removed_but_never_returned_are_noted_and_not_an_error() {
        // One message is taken, then the receive is cancelled and getting the buffered
        // message back fails: it stays on the receiver, and `close` counts it.
        let b = FakeBroker::default().with_receive_delay(Duration::from_secs(1));
        for _ in 0..2 {
            b.add("q", FakeMsg::active("x"));
        }
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let fail = b.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            fail.fail_next(FakeOp::Receive, Error::Timeout("receive".into()));
            stop.cancel();
        });
        let mut req = recv("q", 2, ReceiveAction::Delete);
        req.wait = Duration::from_secs(60);
        let out = receive::run(&mut b.clone(), &req, &NoProgress, &cancel)
            .await
            .expect("something was removed: a report, not an error");
        assert!(out.report.cancelled);
        assert!(out.messages.is_empty());
        assert_eq!(b.all("q").len(), 1);
        let notes = &out.report.notes;
        assert!(notes.contains(&unreturned_note(1)), "{notes:?}");
        assert!(notes.contains(&stopped_drain_note(1)), "{notes:?}");
        assert!(
            notes
                .iter()
                .any(|n| n == "the receive stopped early: timed out: receive"),
            "{notes:?}"
        );
        assert_eq!(out.report.failed(), 1, "the job reads as incomplete");
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_failure_held_back_by_the_receiver_is_reported_as_failed() {
        // The link fails after two deliveries: the receiver returns them and holds the error
        // back. The job keeps the two and records the failure.
        let b = FakeBroker::default();
        for _ in 0..3 {
            b.add("q", FakeMsg::active("x"));
        }
        b.fail_receive_after(2, Error::Protocol("link detached".into()));
        let out = receive::run(
            &mut b.clone(),
            &recv("q", 5, ReceiveAction::Delete),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.messages.len(), 2);
        assert_eq!((out.report.succeeded(), out.report.failed()), (2, 1));
        assert_ne!(out.report.outcome(), crate::activity::Outcome::Succeeded);
        assert!(matches!(
            &out.report.items[2].outcome,
            ItemOutcome::Failed { reason, .. } if reason == "the receive stopped early: protocol error: link detached"
        ));
        assert!(
            out.report.notes.contains(&stopped_drain_note(3)),
            "{:?}",
            out.report.notes
        );
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_receive_that_cannot_open_is_an_error() {
        let b = FakeBroker::default();
        b.fail_next(FakeOp::OpenReceiver, Error::Conflict("locked".into()));
        let r = receive::run(
            &mut b.clone(),
            &recv("q", 1, ReceiveAction::Delete),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(r, Err(Error::Conflict(_))));
    }

    #[tokio::test]
    async fn receive_limits_are_checked_first() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        for max in [0, MAX_RECEIVE + 1] {
            let r = receive::run(
                &mut b.clone(),
                &recv("q", max, ReceiveAction::Delete),
                &NoProgress,
                &CancellationToken::new(),
            )
            .await;
            assert!(matches!(r, Err(Error::Invalid(_))));
        }
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test]
    async fn deferred_messages_settle_by_sequence_number_and_missing_ones_are_reported() {
        let b = FakeBroker::default();
        let d = b.add("q", FakeMsg::active("x").deferred());
        let r = receive::settle_deferred(
            &mut b.clone(),
            "q",
            &[d, 999],
            None,
            &Settlement::Complete,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!((r.succeeded(), r.skipped()), (1, 1));
        assert!(matches!(r.items[1].outcome, ItemOutcome::Missing));
        assert!(b.all("q").is_empty());
    }

    #[tokio::test]
    async fn every_deferred_message_received_is_settled_and_a_failed_settle_is_its_item() {
        let b = FakeBroker::default();
        let seqs: Vec<i64> = (0..3)
            .map(|_| b.add("q", FakeMsg::active("x").deferred()))
            .collect();
        b.fail_next(FakeOp::Settle, Error::Timeout("settle".into()));
        let r = receive::settle_deferred(
            &mut b.clone(),
            "q",
            &seqs,
            None,
            &Settlement::Complete,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!((r.succeeded(), r.failed()), (2, 1));
        assert_eq!(r.failed_sequences(), [seqs[0]]);
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test]
    async fn deferred_settle_defers_to_the_dead_letter_queue_with_a_reason() {
        let b = FakeBroker::default();
        let d = b.add("q", FakeMsg::active("x").deferred());
        let how = Settlement::DeadLetter {
            reason: Some("Manual".into()),
            description: Some("stale".into()),
        };
        let r = receive::settle_deferred(
            &mut b.clone(),
            "q",
            &[d],
            None,
            &how,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1);
        let dl = b.all("q/$DeadLetterQueue");
        assert_eq!(dl[0].sequence_number, d);
        assert_eq!(dl[0].state, MessageState::Active);
    }

    #[tokio::test]
    async fn a_cancelled_deferred_settle_reports_the_rest_not_reached() {
        let b = FakeBroker::default();
        let d = b.add("q", FakeMsg::active("x").deferred());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let r = receive::settle_deferred(
            &mut b.clone(),
            "q",
            &[d],
            None,
            &Settlement::Complete,
            &NoProgress,
            &cancel,
        )
        .await
        .unwrap();
        assert!(r.cancelled);
        assert_eq!(
            r.items[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Cancelled
            }
        );
        assert_eq!(b.all("q").len(), 1);
    }

    #[test]
    fn notes_count_with_separators_and_singulars() {
        assert_eq!(
            stopped_drain_note(2),
            "up to 2 more messages may have been removed after the receive stopped and are not in this result"
        );
        assert_eq!(
            drain_note(2),
            "up to 2 more messages may have been removed after the wait and are not in this result"
        );
        assert_eq!(
            drain_note(1),
            "up to 1 more message may have been removed after the wait and is not in this result"
        );
        assert_eq!(
            unreturned_note(1_200),
            "1,200 messages were removed but are not in this result"
        );
        assert_eq!(
            unreturned_note(1),
            "1 message was removed but is not in this result"
        );
    }
}
