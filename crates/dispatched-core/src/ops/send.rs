//! Send, batch send, schedule and cancel scheduled.

use super::failed;
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, NotReached, Progress};
use crate::messaging::backend::MessagingBackend;
use crate::messaging::outgoing::{BodySource, MessageIdSource, OutgoingMessage};
use crate::messaging::sender::{REQUEST_CHUNK, chunks, copy_error, send_target};
use crate::messaging::{PeekedMessage, collect_sequences};
use crate::namespace::PeekView;
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;
use std::collections::HashSet;
use time::OffsetDateTime;

/// The most messages one send may make.
pub const MAX_SEND_COUNT: u64 = 100_000;

/// What to send.
#[derive(Debug, Clone)]
pub struct SendRequest {
    /// Queue or topic.
    pub target: String,
    /// Every message starts as a copy of this.
    pub template: OutgoingMessage,
    /// Overrides `template.body` per message.
    pub body: Option<BodySource>,
    /// Where each message's MessageId comes from.
    pub ids: MessageIdSource,
    /// How many messages (1 to 100,000).
    pub count: u64,
    /// Schedule them for this time instead of sending them now.
    pub schedule: Option<OffsetDateTime>,
    /// Send all through one call (pipelined) instead of one call per message.
    pub batch: bool,
}

/// The result for message `index` of a send.
fn send_item(entity: &str, m: Option<&OutgoingMessage>, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: None,
        message_id: m.and_then(|m| m.message_id.clone()),
        session_id: m.and_then(|m| m.session_id.clone()),
        entity: entity.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

/// Message number `index` of `req`: the template with its body and MessageId for that index,
/// and the error when its body could not be read.
fn build(req: &SendRequest, index: u64) -> (OutgoingMessage, Option<Error>) {
    let mut m = req.template.clone();
    if let Some(id) = req.ids.id_for(index) {
        m.message_id = Some(id);
    }
    if let Some(body) = &req.body {
        match body.body_for(index) {
            Ok(b) => m.body = b,
            Err(e) => return (m, Some(e)),
        }
    }
    (m, None)
}

/// Run a send job: one item per message, never stopping at the first failure. Scheduled sends
/// go through `schedule` in chunks and record each message's new sequence number. A cancelled
/// job stops before its next message (or chunk) and returns what it did.
pub(crate) async fn run<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &SendRequest,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let entity = send_target(&req.target)?.base();
    if req.count == 0 || req.count > MAX_SEND_COUNT {
        return Err(Error::Invalid(format!(
            "send 1 to {} messages at a time",
            crate::safety::count_text(MAX_SEND_COUNT)
        )));
    }
    let mut run = JobRun::new(
        JobKind::Send,
        entity.clone(),
        None,
        progress,
        Some(req.count),
    );
    let cancelled = match req.schedule {
        Some(at) => schedule_all(b, req, at, &entity, &mut run, cancel).await,
        None if req.batch => send_batch(b, req, &entity, &mut run, cancel).await?,
        None => send_each(b, req, &entity, &mut run, cancel).await?,
    };
    Ok(run.finish(cancelled))
}

/// One `send_all` call per message. A link failure fails that message; the next one opens a
/// new link (opening has no broker side effect). Returns whether it was cancelled.
async fn send_each<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &SendRequest,
    entity: &str,
    run: &mut JobRun<'_>,
    cancel: &CancellationToken,
) -> Result<bool> {
    let mut sender = None;
    let mut attempted = false;
    for index in 0..req.count {
        if cancel.is_cancelled() {
            if let Some(s) = sender.take() {
                close_sender(s).await;
            }
            return Ok(true);
        }
        let (m, unreadable) = build(req, index);
        if let Some(e) = unreadable {
            run.item(send_item(entity, Some(&m), failed(&e)));
            continue;
        }
        let mut link = match sender.take() {
            Some(s) => s,
            None => match b.open_sender(entity).await {
                Ok(s) => s,
                // Nothing reached the broker yet: an error, not a job.
                Err(e) if !attempted => return Err(e),
                Err(e) => {
                    // The link broke and cannot be opened again: the rest fail with it.
                    fail_rest(run, req, entity, index, &e);
                    return Ok(false);
                }
            },
        };
        attempted = true;
        let mut result = None;
        let call = link
            .send_all(std::slice::from_ref(&m), &mut |_, r| result = Some(r))
            .await;
        let outcome = match result {
            Some(Ok(())) => ItemOutcome::Succeeded { detail: None },
            Some(Err(e)) => failed(&e),
            None => failed(
                call.as_ref()
                    .err()
                    .unwrap_or(&Error::Protocol("the sender reported no outcome".into())),
            ),
        };
        run.item(send_item(entity, Some(&m), outcome));
        if call.is_err() {
            // The link broke: drop it; the next message opens a new one.
            close_sender(link).await;
        } else {
            sender = Some(link);
        }
    }
    if let Some(s) = sender {
        close_sender(s).await;
    }
    Ok(false)
}

async fn close_sender(s: Box<dyn crate::messaging::backend::MessageSender>) {
    s.close().await;
}

/// Messages a batch send passes to one `send_all` call; cancel is checked between slices.
const BATCH_SLICE: u64 = 1_000;

/// The rest of the messages, from `from`, fail with `e` (a link that cannot be opened again).
fn fail_rest(run: &mut JobRun<'_>, req: &SendRequest, entity: &str, from: u64, e: &Error) {
    for rest in from..req.count {
        let mut item = send_item(entity, None, failed(e));
        item.message_id = req
            .ids
            .id_for(rest)
            .or_else(|| req.template.message_id.clone());
        item.session_id = req.template.session_id.clone();
        run.item(item);
    }
}

/// Every message over one sender link, pipelined (not atomic), in slices of [`BATCH_SLICE`]
/// so a cancel stops between slices and bodies are read per slice. A link failure fails the
/// rest of its slice; the next slice opens a new link.
async fn send_batch<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &SendRequest,
    entity: &str,
    run: &mut JobRun<'_>,
    cancel: &CancellationToken,
) -> Result<bool> {
    let mut sender = None;
    let mut attempted = false;
    let mut start = 0;
    while start < req.count {
        if cancel.is_cancelled() {
            if let Some(s) = sender.take() {
                close_sender(s).await;
            }
            return Ok(true);
        }
        let end = (start + BATCH_SLICE).min(req.count);
        let (messages, mut outcomes) = build_range(req, start, end);
        let ready: Vec<usize> = (0..messages.len())
            .filter(|i| outcomes[*i].is_none())
            .collect();
        if !ready.is_empty() {
            let mut link = match sender.take() {
                Some(s) => s,
                None => match b.open_sender(entity).await {
                    Ok(s) => s,
                    Err(e) if !attempted => return Err(e),
                    Err(e) => {
                        fail_rest(run, req, entity, start, &e);
                        return Ok(false);
                    }
                },
            };
            attempted = true;
            let batch: Vec<OutgoingMessage> = ready.iter().map(|i| messages[*i].clone()).collect();
            let call = link
                .send_all(&batch, &mut |j, r| {
                    outcomes[ready[j]] = Some(match r {
                        Ok(()) => ItemOutcome::Succeeded { detail: None },
                        Err(e) => failed(&e),
                    });
                })
                .await;
            fill_unreported(&mut outcomes, &ready, call.as_ref().err());
            if call.is_err() {
                close_sender(link).await;
            } else {
                sender = Some(link);
            }
        }
        for (m, outcome) in messages.iter().zip(outcomes) {
            run.item(send_item(entity, Some(m), outcome.expect("filled")));
        }
        start = end;
    }
    if let Some(s) = sender {
        close_sender(s).await;
    }
    Ok(false)
}

/// Messages `start..end` of `req`, with the failure of those that could not be built.
fn build_range(
    req: &SendRequest,
    start: u64,
    end: u64,
) -> (Vec<OutgoingMessage>, Vec<Option<ItemOutcome>>) {
    (start..end)
        .map(|i| {
            let (m, unreadable) = build(req, i);
            (m, unreadable.as_ref().map(failed))
        })
        .unzip()
}

/// Indexes in `wanted` the backend never reported fail with `err` (defensive: it reports all).
fn fill_unreported(outcomes: &mut [Option<ItemOutcome>], wanted: &[usize], err: Option<&Error>) {
    for i in wanted {
        if outcomes[*i].is_none() {
            let e = err.map_or_else(
                || Error::Protocol("the broker reported no outcome for this message".into()),
                copy_error,
            );
            outcomes[*i] = Some(failed(&e));
        }
    }
}

/// Schedule every message for `at`, one `schedule` call per chunk of [`REQUEST_CHUNK`]
/// (bodies read per chunk); a failed chunk fails only its messages. Returns whether it was
/// cancelled.
async fn schedule_all<B: MessagingBackend + ?Sized>(
    b: &mut B,
    req: &SendRequest,
    at: OffsetDateTime,
    entity: &str,
    run: &mut JobRun<'_>,
    cancel: &CancellationToken,
) -> bool {
    let mut start = 0;
    while start < req.count {
        if cancel.is_cancelled() {
            return true;
        }
        let end = (start + REQUEST_CHUNK as u64).min(req.count);
        let (messages, outcomes) = build_range(req, start, end);
        start = end;
        let ready: Vec<usize> = (0..messages.len())
            .filter(|i| outcomes[*i].is_none())
            .collect();
        let batch: Vec<OutgoingMessage> = ready.iter().map(|i| messages[*i].clone()).collect();
        let mut got: Vec<Option<Result<i64>>> = (0..batch.len()).map(|_| None).collect();
        let call = if batch.is_empty() {
            Ok(())
        } else {
            b.schedule(entity, &batch, at, &mut |j, r| got[j] = Some(r))
                .await
        };
        let mut by_index: Vec<Option<Result<i64>>> = (0..messages.len()).map(|_| None).collect();
        for (j, r) in got.into_iter().enumerate() {
            by_index[ready[j]] = Some(r.unwrap_or_else(|| {
                Err(call.as_ref().err().map_or_else(
                    || Error::Protocol("the broker reported no sequence number".into()),
                    copy_error,
                ))
            }));
        }
        for (k, m) in messages.iter().enumerate() {
            let mut item = send_item(entity, Some(m), ItemOutcome::Succeeded { detail: None });
            match (&outcomes[k], by_index[k].take()) {
                (Some(o), _) => item.outcome = o.clone(),
                (None, Some(Ok(seq))) => item.new_sequence_number = Some(seq),
                (None, Some(Err(e))) => item.outcome = failed(&e),
                (None, None) => {
                    item.outcome = failed(&Error::Protocol(
                        "the broker reported no sequence number".into(),
                    ))
                }
            }
            run.item(item);
        }
    }
    false
}

/// The item for a sequence-number job.
fn seq_item(entity: &str, seq: i64, m: Option<&PeekedMessage>, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: Some(seq),
        message_id: m.and_then(|m| m.message_id.clone()),
        session_id: m.and_then(|m| m.session_id.clone()),
        entity: entity.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

/// `seqs` without repeats, in their first order.
pub(crate) fn distinct(seqs: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    seqs.iter().copied().filter(|s| seen.insert(*s)).collect()
}

/// Cancel scheduled messages of `address` by sequence number. The broker ignores numbers it
/// does not know, so each one is first looked up in the scheduled view: absent ones are
/// `missing`. The rest are cancelled in chunks; a failed chunk fails only its numbers.
pub(crate) async fn cancel_scheduled<B: MessagingBackend + ?Sized>(
    b: &mut B,
    address: &str,
    seqs: &[i64],
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let entity = send_target(address)?.base();
    let seqs = distinct(seqs);
    let mut run = JobRun::new(
        JobKind::CancelScheduled,
        entity.clone(),
        None,
        progress,
        Some(seqs.len() as u64),
    );
    let not_reached = |run: &mut JobRun<'_>, rest: &[(i64, Option<PeekedMessage>)]| {
        for (seq, m) in rest {
            run.item(seq_item(
                &entity,
                *seq,
                m.as_ref(),
                ItemOutcome::NotReached {
                    why: NotReached::Cancelled,
                },
            ));
        }
    };
    if cancel.is_cancelled() {
        let rest: Vec<_> = seqs.iter().map(|s| (*s, None)).collect();
        not_reached(&mut run, &rest);
        return Ok(run.finish(true));
    }
    let found = match collect_sequences(
        b,
        &entity,
        PeekView::Scheduled,
        &seqs,
        u64::MAX,
        Some(cancel),
        None,
    )
    .await
    {
        Ok(found) => found,
        Err(Error::Cancelled) => {
            let rest: Vec<_> = seqs.iter().map(|s| (*s, None)).collect();
            not_reached(&mut run, &rest);
            return Ok(run.finish(true));
        }
        Err(e) => return Err(e),
    };
    let mut present: Vec<(i64, Option<PeekedMessage>)> = Vec::new();
    for seq in &seqs {
        match found.messages.iter().find(|m| m.sequence_number == *seq) {
            Some(m) => present.push((*seq, Some(m.clone()))),
            None => run.item(seq_item(&entity, *seq, None, ItemOutcome::Missing)),
        }
    }
    let mut done = 0;
    for chunk in chunks(&present) {
        if cancel.is_cancelled() {
            not_reached(&mut run, &present[done..]);
            return Ok(run.finish(true));
        }
        done += chunk.len();
        let numbers: Vec<i64> = chunk.iter().map(|(s, _)| *s).collect();
        let mut got: Vec<Option<Result<()>>> = (0..numbers.len()).map(|_| None).collect();
        let call = b
            .cancel_scheduled(&entity, &numbers, &mut |j, r| got[j] = Some(r))
            .await;
        for ((seq, m), r) in chunk.iter().zip(got) {
            let outcome = match r {
                Some(Ok(())) => ItemOutcome::Succeeded { detail: None },
                Some(Err(e)) => failed(&e),
                None => failed(&call.as_ref().err().map_or_else(
                    || Error::Protocol("the broker reported no outcome for this number".into()),
                    copy_error,
                )),
            };
            run.item(seq_item(&entity, *seq, m.as_ref(), outcome));
        }
    }
    Ok(run.finish(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::NoProgress;
    use crate::messaging::MessageState;
    use crate::messaging::fake::{FakeBroker, FakeMsg, FakeOp};
    use crate::ops::send;
    use crate::ops::testkit::three_texts;

    #[tokio::test]
    async fn send_n_copies_with_incrementing_ids() {
        let b = FakeBroker::default();
        let r = send::run(
            &mut b.clone(),
            &SendRequest {
                target: "q".into(),
                template: OutgoingMessage::text("x"),
                body: None,
                ids: MessageIdSource::Increment { base: "m-1".into() },
                count: 3,
                schedule: None,
                batch: false,
            },
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 3);
        let ids: Vec<_> = b
            .sent("q")
            .into_iter()
            .map(|m| m.message_id.unwrap())
            .collect();
        assert_eq!(ids, ["m-1", "m-2", "m-3"]);
    }

    #[tokio::test]
    async fn a_failed_send_is_one_failed_item_and_the_rest_continue() {
        let b = FakeBroker::default();
        b.fail_next(FakeOp::Send, Error::Protocol("refused".into()));
        let r = send::run(
            &mut b.clone(),
            &three_texts(),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!((r.succeeded(), r.failed()), (2, 1));
    }

    #[tokio::test]
    async fn a_batch_send_goes_through_one_call_and_reports_each_message() {
        let b = FakeBroker::default();
        b.fail_next(FakeOp::Send, Error::Invalid("too big".into()));
        let req = SendRequest {
            batch: true,
            ..three_texts()
        };
        let r = send::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!((r.succeeded(), r.failed()), (2, 1));
        assert_eq!(b.sent("q").len(), 2);
    }

    #[tokio::test]
    async fn a_batch_send_goes_in_slices_and_a_broken_link_fails_only_its_slice() {
        let b = FakeBroker::default();
        // A connection-level error breaks the link: the rest of the first slice fails with it,
        // and the next slice goes over a new link.
        b.fail_next(FakeOp::Send, Error::Protocol("link detached".into()));
        let req = SendRequest {
            batch: true,
            count: 1_500,
            ..three_texts()
        };
        let r = send::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!((r.succeeded(), r.failed()), (500, 1_000));
        assert_eq!(r.items.len(), 1_500);
        assert_eq!(b.sent("q").len(), 500);
    }

    #[tokio::test]
    async fn scheduled_sends_record_their_sequence_numbers() {
        let b = FakeBroker::default();
        let at = OffsetDateTime::now_utc() + time::Duration::minutes(30);
        let req = SendRequest {
            schedule: Some(at),
            count: 2,
            ..three_texts()
        };
        let r = send::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.succeeded(), 2);
        let seqs: Vec<i64> = r
            .items
            .iter()
            .map(|i| i.new_sequence_number.expect("the broker's sequence number"))
            .collect();
        let all = b.all("q");
        assert_eq!(
            all.iter().map(|m| m.sequence_number).collect::<Vec<_>>(),
            seqs
        );
        assert!(
            all.iter()
                .all(|m| m.state == MessageState::Scheduled && m.scheduled_enqueue_time == Some(at))
        );
    }

    #[tokio::test]
    async fn a_failed_schedule_chunk_fails_only_its_messages() {
        let b = FakeBroker::default();
        let at = OffsetDateTime::now_utc() + time::Duration::minutes(30);
        let req = SendRequest {
            schedule: Some(at),
            count: 150,
            ..three_texts()
        };
        // The first chunk (100 messages) fails as a whole; the second still goes.
        b.fail_next(FakeOp::Schedule, Error::Timeout("schedule".into()));
        let r = send::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!((r.succeeded(), r.failed()), (50, 100));
        assert!(
            r.items[..100]
                .iter()
                .all(|i| i.new_sequence_number.is_none())
        );
        let seqs: Vec<i64> = r.items[100..]
            .iter()
            .map(|i| i.new_sequence_number.expect("scheduled"))
            .collect();
        let all: Vec<i64> = b.all("q").iter().map(|m| m.sequence_number).collect();
        assert_eq!(all, seqs);
    }

    #[tokio::test]
    async fn a_body_that_cannot_be_read_fails_only_its_message() {
        let b = FakeBroker::default();
        let req = SendRequest {
            body: Some(BodySource::Files(vec![])),
            count: 2,
            ..three_texts()
        };
        let r = send::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!((r.succeeded(), r.failed()), (0, 2));
        assert!(b.sent("q").is_empty());
    }

    #[tokio::test]
    async fn sends_are_bounded_and_never_go_to_a_subscription() {
        let b = FakeBroker::default();
        let none = SendRequest {
            count: 0,
            ..three_texts()
        };
        let too_many = SendRequest {
            count: MAX_SEND_COUNT + 1,
            ..three_texts()
        };
        let sub = SendRequest {
            target: "t/subscriptions/s".into(),
            ..three_texts()
        };
        for req in [none, too_many, sub] {
            let r = send::run(&mut b.clone(), &req, &NoProgress, &CancellationToken::new()).await;
            assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
        }
        assert!(b.sent("q").is_empty());
    }

    #[tokio::test]
    async fn a_cancelled_send_returns_its_partial_report() {
        let b = FakeBroker::default();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let r = send::run(&mut b.clone(), &three_texts(), &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(r.cancelled);
        assert_eq!(r.succeeded(), 0);
        assert!(b.sent("q").is_empty());
    }

    #[tokio::test]
    async fn cancel_scheduled_reports_unknown_numbers_as_missing() {
        let b = FakeBroker::default();
        let s = b.add("q", FakeMsg::active("x").scheduled());
        let active = b.add("q", FakeMsg::active("y"));
        let r = cancel_scheduled(
            &mut b.clone(),
            "q",
            &[s, active, 999],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!((r.succeeded(), r.skipped()), (1, 2));
        let missing: Vec<i64> = r
            .items
            .iter()
            .filter(|i| i.outcome == ItemOutcome::Missing)
            .filter_map(|i| i.sequence_number)
            .collect();
        assert_eq!(missing, [active, 999]);
        let left: Vec<i64> = b.all("q").iter().map(|m| m.sequence_number).collect();
        assert_eq!(left, [active], "only the scheduled one is cancelled");
    }

    #[tokio::test]
    async fn a_failed_cancel_fails_its_numbers() {
        let b = FakeBroker::default();
        let s = b.add("q", FakeMsg::active("x").scheduled());
        b.fail_next(FakeOp::Cancel, Error::Timeout("cancel".into()));
        let r = cancel_scheduled(
            &mut b.clone(),
            "q",
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.failed(), 1);
        assert_eq!(r.failed_sequences(), [s]);
    }

    #[tokio::test]
    async fn a_cancelled_cancel_reports_the_rest_not_reached() {
        let b = FakeBroker::default();
        let s = b.add("q", FakeMsg::active("x").scheduled());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let r = cancel_scheduled(&mut b.clone(), "q", &[s], &NoProgress, &cancel)
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
}
