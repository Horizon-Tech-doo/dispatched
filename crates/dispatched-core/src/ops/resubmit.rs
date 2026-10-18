//! Resubmit, move, and clone and resend: copies of messages sent to a queue or topic, with
//! the originals removed (move, resubmit with "remove original") or kept.

use super::locate::{Found, OnFound, locate};
use super::send::distinct;
use super::{Resolved, SelectSpec, failed, record, seq_item};
use crate::diagnostics::error_kind;
use crate::jobs::{
    ItemOutcome, ItemResult, JobKind, JobReport, JobRun, LostProperties, NotReached, Progress,
};
use crate::messaging::backend::{LockReceiver, MessageSender, MessagingBackend};
use crate::messaging::sender::{copy_error, send_target};
use crate::messaging::{
    CloneOptions, Cloned, Edits, MessageState, OutgoingMessage, PeekedMessage, Settlement,
    collect_sequences,
};
use crate::namespace::{EntityPath, PeekView};
use crate::safety::{Operation, Target};
use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::safety::CancellationToken;
use std::collections::HashMap;

/// What to resubmit, where to, and how.
#[derive(Debug, Clone)]
pub struct ResubmitSpec {
    /// The messages to copy.
    pub source: SelectSpec,
    /// The queue or topic the copies go to; empty means [`default_target`] of the source.
    pub target: String,
    /// Changes made to each copy.
    pub edits: Edits,
    /// Give each copy a fresh MessageId.
    pub new_message_id: bool,
    /// Remove each original once its copy is sent (at-least-once: sent first, then removed).
    pub remove_original: bool,
    /// Keep `DeadLetterReason` and `DeadLetterErrorDescription` on copies from a dead-letter
    /// queue.
    pub keep_dead_letter_properties: bool,
    /// When originals are removed, send a copy even if it must lose custom properties whose
    /// type cannot be copied (`other`). Without it such an item fails and its original stays.
    pub drop_uncopyable_properties: bool,
}

impl ResubmitSpec {
    /// The queue or topic the copies go to (the default when none is given), checked.
    pub fn target_name(&self) -> Result<String> {
        let name = self.target.trim();
        let name = if name.is_empty() {
            default_target(&self.source.entity)
        } else {
            name.to_string()
        };
        Ok(send_target(&name)?.base())
    }

    fn clone_options(&self) -> CloneOptions {
        CloneOptions {
            new_message_id: self.new_message_id,
            keep_dead_letter_properties: self.keep_dead_letter_properties,
            from_dead_letter: matches!(
                self.source.view,
                PeekView::DeadLetter | PeekView::TransferDeadLetter
            ),
        }
    }
}

/// Where copies go when no target is given: the queue itself, or a subscription's topic.
pub fn default_target(source: &EntityPath) -> String {
    source.entity.clone()
}

/// Refuse a resubmit or move that cannot run, before any I/O: a target that is not a queue or
/// topic, or the scheduled view (scheduled messages cannot be locked, so none would move).
/// Returns the target.
pub(crate) fn check(spec: &ResubmitSpec) -> Result<String> {
    if spec.source.view == PeekView::Scheduled {
        return Err(Error::Invalid(
            "scheduled messages cannot be moved or resubmitted; wait until they are enqueued"
                .into(),
        ));
    }
    spec.target_name()
}

/// The operation a resubmit (or, with `JobKind::Move`, a move) of the `resolved` selection
/// confirms, listing its sequence numbers. It is searched (the sentence says examined messages
/// are locked) when originals are removed from a view found with a peek-lock link.
pub(crate) fn operation(
    spec: &ResubmitSpec,
    resolved: &Resolved,
    kind: JobKind,
) -> Result<Operation> {
    let to = check(spec)?;
    let source = &spec.source;
    let target = Target::new(source.entity.clone(), source.view.sub_queue());
    let n = resolved.seqs.len() as u64;
    let op = match kind {
        JobKind::Move => Operation::move_to(target, &to, n),
        _ => Operation::resubmit(target, &to, n, spec.remove_original),
    };
    let searched = spec.remove_original
        && matches!(
            source.view,
            PeekView::Active | PeekView::DeadLetter | PeekView::TransferDeadLetter
        );
    let op = resolved.describe(op);
    Ok(if searched { op.searched() } else { op })
}

/// The copy of `m` to send, edited, the notes from cloning it, and the custom properties it
/// lost (those whose type cannot be copied, unless the edits removed or replaced them). With
/// `keep_lossless`, a copy that would lose any is refused, so the original is kept.
fn prepare(
    m: &PeekedMessage,
    edits: &Edits,
    clone: &CloneOptions,
    keep_lossless: bool,
) -> Result<(OutgoingMessage, Vec<String>, Vec<String>)> {
    let Cloned {
        mut message,
        notes,
        mut lost,
    } = OutgoingMessage::from_peeked(m, clone);
    edits.apply(&mut message);
    lost.retain(|name| !edits.remove.contains(name) && !edits.set.contains_key(name));
    if keep_lossless && !lost.is_empty() {
        return Err(Error::Invalid(format!(
            "the copy would lose {} {} (a type that cannot be copied), so the original was kept; \
             pass --drop-uncopyable-properties to send the copy without {}",
            if lost.len() == 1 {
                "property"
            } else {
                "properties"
            },
            lost.join(", "),
            if lost.len() == 1 { "it" } else { "them" },
        )));
    }
    message.validate()?;
    Ok((message, notes, lost))
}

/// The detail of a sent copy: what cloning changed, if anything.
fn detail(notes: &[String]) -> Option<String> {
    (!notes.is_empty()).then(|| notes.join("; "))
}

fn scheduled_skip(kind: JobKind) -> ItemOutcome {
    let verb = if kind == JobKind::Move {
        "moved"
    } else {
        "resubmitted"
    };
    ItemOutcome::Skipped {
        reason: format!("scheduled messages are not {verb}; wait until they are enqueued"),
    }
}

/// Resubmit the messages `seqs` (what `resolve_selection` returned) as `JobKind::Resubmit`.
/// The brief's entry point (the tests use it); the `Operator` goes through [`run_as`] with its
/// kind.
#[allow(dead_code)]
pub(crate) async fn run(
    plane: &mut dyn MessagingBackend,
    spec: &ResubmitSpec,
    seqs: &[i64],
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    run_as(plane, spec, seqs, JobKind::Resubmit, progress, cancel).await
}

/// Resubmit the messages `seqs`, reported as `kind` (`Move` or `Resubmit`). With
/// `remove_original` the originals are found with lock-and-match and each is completed after
/// its copy is sent; otherwise the originals are peeked and kept.
pub(crate) async fn run_as(
    plane: &mut dyn MessagingBackend,
    spec: &ResubmitSpec,
    seqs: &[i64],
    kind: JobKind,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let to = spec.target_name()?;
    let address = spec.source.address();
    let seqs = distinct(seqs);
    let mut job = JobRun::new(
        kind,
        address.clone(),
        Some(to.clone()),
        progress,
        Some(seqs.len() as u64),
    );
    let mut lost = LostProperties::default();
    let cancelled = if spec.remove_original {
        let sender = plane.open_sender(&to).await?;
        let mut on = ResubmitOnFound {
            sender: Some(sender),
            target: to,
            address: address.clone(),
            edits: spec.edits.clone(),
            clone: spec.clone_options(),
            kind,
            keep_lossless: !spec.drop_uncopyable_properties,
            lost: LostProperties::default(),
        };
        let located = locate(
            plane,
            &address,
            spec.source.view,
            &seqs,
            &spec.source.limits,
            cancel,
            &mut on,
        )
        .await;
        // On every path, the error one included.
        on.close().await;
        let cancelled = record(&mut job, located?);
        lost = on.lost;
        cancelled
    } else {
        clone_and_resend(
            plane, spec, &to, &address, &seqs, kind, &mut job, &mut lost, cancel,
        )
        .await?
    };
    if let Some(note) = lost.note() {
        job.note(note);
    }
    Ok(job.finish(cancelled))
}

/// Sends a copy of each message [`locate`] finds locked, then completes the original
/// (Decision 5). It owns the job's sender; a sender whose link broke is closed and the next
/// message opens a new one.
struct ResubmitOnFound {
    sender: Option<Box<dyn MessageSender>>,
    target: String,
    address: String,
    edits: Edits,
    clone: CloneOptions,
    kind: JobKind,
    /// Refuse copies that would lose properties (`!drop_uncopyable_properties`).
    keep_lossless: bool,
    /// Copies sent without some properties.
    lost: LostProperties,
}

/// A copy that was not sent, as far as the sender knows.
struct SendFailure {
    error: Error,
    /// The link failed without a result for this copy: it may have reached the broker.
    maybe_sent: bool,
}

impl From<Error> for SendFailure {
    fn from(error: Error) -> Self {
        Self {
            error,
            maybe_sent: false,
        }
    }
}

/// Added to the reason of a copy whose outcome is unknown.
const MAYBE_SENT: &str = "; the copy may have been sent";

/// Why the deferred original of a failed copy is where it is (it was abandoned, or the abandon
/// failed and its lock runs out).
fn deferred_kept(from_dead_letter: bool, abandoned: bool) -> &'static str {
    match (abandoned, from_dead_letter) {
        (false, _) => {
            "the copy failed; the original stays locked until its lock expires, then its delivery count rises by 1"
        }
        (true, true) => "the copy failed; the original stays deferred in the dead-letter queue",
        (true, false) => {
            "the copy failed; the original stays deferred (its delivery count rose by 1, and at the entity's limit it moves to the dead-letter queue)"
        }
    }
}

impl ResubmitOnFound {
    /// Clone, edit and send the copy of `m`; the cloning notes and lost properties on success.
    async fn send(
        &mut self,
        plane: &mut dyn MessagingBackend,
        m: &PeekedMessage,
    ) -> std::result::Result<(Vec<String>, Vec<String>), SendFailure> {
        let (copy, notes, lost) = prepare(m, &self.edits, &self.clone, self.keep_lossless)?;
        let sender = match &mut self.sender {
            Some(s) => s,
            empty => empty.insert(plane.open_sender(&self.target).await?),
        };
        let mut one = None;
        let call = sender
            .send_all(std::slice::from_ref(&copy), &mut |_, r| one = Some(r))
            .await;
        if call.is_err()
            && let Some(broken) = self.sender.take()
        {
            broken.close().await;
        }
        match (one, call) {
            (Some(r), _) => Ok(r.map(|()| (notes, lost))?),
            (None, Err(error)) => Err(SendFailure {
                error,
                maybe_sent: true,
            }),
            (None, Ok(())) => Err(SendFailure {
                error: Error::Protocol("the sender reported no outcome".into()),
                maybe_sent: true,
            }),
        }
    }

    async fn close(&mut self) {
        if let Some(s) = self.sender.take() {
            s.close().await;
        }
    }
}

#[async_trait]
impl OnFound for ResubmitOnFound {
    async fn on_found(
        &mut self,
        plane: &mut dyn MessagingBackend,
        rx: Option<&mut dyn LockReceiver>,
        found: Found,
    ) -> ItemResult {
        let received = match found {
            Found::Locked { received } => received,
            Found::Scheduled(m) => return seq_item(&self.address, &m, scheduled_skip(self.kind)),
        };
        let m = &received.message;
        let (Some(rx), Some(token)) = (rx, received.lock_token) else {
            let e = Error::Protocol("the message came without its lock".into());
            return seq_item(&self.address, m, failed(&e));
        };
        let outcome = match self.send(plane, m).await {
            Ok((notes, lost)) => match rx.settle(token, &Settlement::Complete).await {
                Ok(()) => {
                    self.lost.add(&lost);
                    ItemOutcome::Succeeded {
                        detail: detail(&notes),
                    }
                }
                Err(e) => {
                    // The copy was sent: what it lost is lost either way.
                    self.lost.add(&lost);
                    ItemOutcome::Failed {
                        kind: error_kind(&e),
                        reason: format!(
                            "sent to {}, but the original could not be removed ({e}); it may be delivered twice",
                            self.target
                        ),
                    }
                }
            },
            Err(f) if m.state == MessageState::Deferred => {
                // Received by sequence number, so not held: it cannot be released. Abandoning
                // it unlocks it now (it stays deferred); its delivery count rises either way,
                // and on an entity at its max delivery count the broker dead-letters it.
                let abandoned = rx.settle(token, &Settlement::Abandon).await.is_ok();
                let kept = deferred_kept(self.clone.from_dead_letter, abandoned);
                let maybe = if f.maybe_sent { MAYBE_SENT } else { "" };
                ItemOutcome::Failed {
                    kind: error_kind(&f.error),
                    reason: format!("{kept}: {}{maybe}", f.error),
                }
            }
            Err(f) => {
                // A failed release leaves it held: `locate` releases it again at the end.
                let _ = rx.release(token).await;
                let maybe = if f.maybe_sent { MAYBE_SENT } else { "" };
                ItemOutcome::Failed {
                    kind: error_kind(&f.error),
                    reason: format!("{}{maybe}", f.error),
                }
            }
        };
        seq_item(&self.address, m, outcome)
    }
}

/// Messages a clone and resend passes to one `send_all` call; cancel is checked between them.
const RESEND_SLICE: usize = 100;

/// Peek the originals, send edited copies over one sender, keep the originals. Returns whether
/// it was cancelled.
#[allow(clippy::too_many_arguments)]
async fn clone_and_resend(
    plane: &mut dyn MessagingBackend,
    spec: &ResubmitSpec,
    to: &str,
    address: &str,
    seqs: &[i64],
    kind: JobKind,
    job: &mut JobRun<'_>,
    lost: &mut LostProperties,
    cancel: &CancellationToken,
) -> Result<bool> {
    let not_reached = |seq: i64, m: Option<&PeekedMessage>, why: NotReached| {
        let outcome = ItemOutcome::NotReached { why };
        match m {
            Some(m) => seq_item(address, m, outcome),
            None => bare_item(address, seq, outcome),
        }
    };
    let budget = spec.source.limits.max_examined;
    let view = spec.source.view;
    let found = if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        collect_sequences(plane, address, view, seqs, budget, Some(cancel), None).await
    };
    let found = match found {
        Ok(found) => found,
        Err(Error::Cancelled) => {
            for seq in seqs {
                job.item(not_reached(*seq, None, NotReached::Cancelled));
            }
            return Ok(true);
        }
        Err(e) => return Err(e),
    };
    let mut out: HashMap<i64, ItemResult> = HashMap::new();
    let beyond_budget =
        |s: i64| found.scanned >= budget && found.last_examined.is_none_or(|l| s > l);
    for s in &found.missing {
        let outcome = if beyond_budget(*s) {
            ItemOutcome::NotReached {
                why: NotReached::Budget,
            }
        } else {
            ItemOutcome::Missing
        };
        out.insert(*s, bare_item(address, *s, outcome));
    }
    let clone = spec.clone_options();
    // The originals stay, so a copy that loses properties is sent (and noted).
    let mut ready: Vec<(OutgoingMessage, Vec<String>, Vec<String>, &PeekedMessage)> = Vec::new();
    for m in &found.messages {
        let seq = m.sequence_number;
        if m.state == MessageState::Scheduled {
            out.insert(seq, seq_item(address, m, scheduled_skip(kind)));
            continue;
        }
        match prepare(m, &spec.edits, &clone, false) {
            Ok((copy, notes, gone)) => ready.push((copy, notes, gone, m)),
            Err(e) => {
                out.insert(seq, seq_item(address, m, failed(&e)));
            }
        }
    }
    let mut sender: Option<Box<dyn MessageSender>> = None;
    let mut cancelled = false;
    let mut broken: Option<Error> = None;
    for slice in ready.chunks(RESEND_SLICE) {
        if cancelled || cancel.is_cancelled() {
            cancelled = true;
            for (_, _, _, m) in slice {
                let r = not_reached(m.sequence_number, Some(m), NotReached::Cancelled);
                out.insert(m.sequence_number, r);
            }
            continue;
        }
        if let Some(e) = &broken {
            // The link broke and could not be opened again: the rest fail with it.
            for (_, _, _, m) in slice {
                out.insert(m.sequence_number, seq_item(address, m, failed(e)));
            }
            continue;
        }
        let link = match &mut sender {
            Some(s) => s,
            empty => match plane.open_sender(to).await {
                Ok(s) => empty.insert(s),
                Err(e) => {
                    for (_, _, _, m) in slice {
                        out.insert(m.sequence_number, seq_item(address, m, failed(&e)));
                    }
                    broken = Some(e);
                    continue;
                }
            },
        };
        let batch: Vec<OutgoingMessage> = slice.iter().map(|(c, ..)| c.clone()).collect();
        let mut results: Vec<Option<Result<()>>> = (0..batch.len()).map(|_| None).collect();
        let call = link
            .send_all(&batch, &mut |i, r| results[i] = Some(r))
            .await;
        for ((_, notes, gone, m), r) in slice.iter().zip(results) {
            let outcome = match r {
                Some(Ok(())) => {
                    lost.add(gone);
                    ItemOutcome::Succeeded {
                        detail: detail(notes),
                    }
                }
                Some(Err(e)) => failed(&e),
                None => {
                    let e = call.as_ref().err().map_or_else(
                        || Error::Protocol("the sender reported no outcome".into()),
                        copy_error,
                    );
                    ItemOutcome::Failed {
                        kind: error_kind(&e),
                        reason: format!("{e}{MAYBE_SENT}"),
                    }
                }
            };
            out.insert(m.sequence_number, seq_item(address, m, outcome));
        }
        if call.is_err()
            && let Some(s) = sender.take()
        {
            // The link broke: the next slice opens a new one.
            s.close().await;
        }
    }
    if let Some(s) = sender {
        s.close().await;
    }
    for seq in seqs {
        let r = out
            .remove(seq)
            .unwrap_or_else(|| bare_item(address, *seq, ItemOutcome::Missing));
        job.item(r);
    }
    Ok(cancelled)
}

fn bare_item(address: &str, seq: i64, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: Some(seq),
        message_id: None,
        session_id: None,
        entity: address.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::*;
    use super::*;
    use crate::jobs::NoProgress;
    use crate::messaging::fake::{FakeBroker, FakeMsg, FakeOp};
    use crate::messaging::{PropValue, TypedValue};
    use crate::safety::ProfileRef;

    #[tokio::test]
    async fn move_sends_then_completes_and_keeps_the_message_id() {
        let b = FakeBroker::default();
        let s = b.add(
            "q/$DeadLetterQueue",
            FakeMsg {
                message_id: Some("m-7".into()),
                ..FakeMsg::active("x")
            },
        );
        let r = run(
            &mut b.clone(),
            &move_spec("q/$DeadLetterQueue", &[s], "target"),
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1);
        assert!(b.all("q/$DeadLetterQueue").is_empty());
        assert_eq!(b.sent("target")[0].message_id.as_deref(), Some("m-7"));
        assert!(
            !b.sent("target")[0]
                .properties
                .contains_key("DeadLetterReason")
        );
        assert_eq!(b.open_senders(), 0);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test]
    async fn move_keeps_the_original_when_the_send_fails() {
        let b = FakeBroker::default();
        let s = b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        b.fail_next(FakeOp::Send, Error::Protocol("refused".into()));
        let r = run(
            &mut b.clone(),
            &move_spec("q/$DeadLetterQueue", &[s], "target"),
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.failed(), 1);
        assert_eq!(b.all("q/$DeadLetterQueue").len(), 1, "still there");
        assert_eq!(b.held_locks(), 0);
        assert_eq!(
            b.delivery_count("q/$DeadLetterQueue", s),
            0,
            "released, not abandoned"
        );
        assert_eq!(b.open_senders(), 0);
    }

    #[tokio::test]
    async fn move_reports_possible_duplicate_when_complete_fails_after_send() {
        let b = FakeBroker::default();
        let s = b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        b.fail_next(FakeOp::Settle, Error::LockLost("expired".into()));
        let r = run(
            &mut b.clone(),
            &move_spec("q/$DeadLetterQueue", &[s], "target"),
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let ItemOutcome::Failed { reason, .. } = &r.items[0].outcome else {
            panic!()
        };
        assert!(
            reason.starts_with("sent to target, but the original could not be removed")
                && reason.ends_with("it may be delivered twice"),
            "{reason}"
        );
        assert_eq!(b.sent("target").len(), 1);
    }

    #[tokio::test]
    async fn resubmit_without_removal_is_clone_and_resend_with_edits_and_new_ids() {
        let b = FakeBroker::default();
        let s = b.add(
            "q/$DeadLetterQueue",
            FakeMsg {
                message_id: Some("orig".into()),
                ..FakeMsg::active("x").prop("kind", PropValue::String("a".into()))
            },
        );
        let mut spec = resubmit_spec("q/$DeadLetterQueue", &[s], "q");
        spec.new_message_id = true;
        spec.edits
            .set
            .insert("kind".into(), TypedValue::String("b".into()));
        let r = run(
            &mut b.clone(),
            &spec,
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1);
        assert_eq!(b.all("q/$DeadLetterQueue").len(), 1, "original kept");
        let sent = &b.sent("q")[0];
        assert_ne!(sent.message_id.as_deref(), Some("orig"));
        assert_eq!(sent.properties["kind"], TypedValue::String("b".into()));
    }

    #[tokio::test]
    async fn a_copy_without_an_outcome_may_have_been_sent() {
        // The link dropped before the broker answered: the reason says the copy may be there.
        for remove_original in [true, false] {
            let b = FakeBroker::default();
            let dl = "q/$DeadLetterQueue";
            let s = b.add(dl, FakeMsg::active("x"));
            b.lose_next_send_outcome(Error::Network("link detached".into()));
            let spec = ResubmitSpec {
                remove_original,
                ..resubmit_spec(dl, &[s], "t")
            };
            let r = run(
                &mut b.clone(),
                &spec,
                &[s],
                &NoProgress,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
            let ItemOutcome::Failed { kind, reason } = &r.items[0].outcome else {
                panic!("{:?}", r.items[0].outcome)
            };
            assert_eq!(*kind, "network");
            assert!(
                reason.ends_with("link detached; the copy may have been sent"),
                "remove_original = {remove_original}: {reason}"
            );
            assert_eq!(b.all(dl).len(), 1, "the original stays");
            assert_eq!((b.open_senders(), b.held_locks()), (0, 0));
        }
    }

    #[tokio::test]
    async fn resubmit_from_an_active_queue_can_remove_the_original() {
        // #771: fix a message in an active queue and put the fixed copy back.
        let b = FakeBroker::default();
        let other = b.add("q", FakeMsg::active("other"));
        let s = b.add(
            "q",
            FakeMsg {
                message_id: Some("m-1".into()),
                ..FakeMsg::active("x")
            },
        );
        let mut spec = resubmit_spec("q", &[s], "fixed");
        spec.source.view = PeekView::Active;
        spec.remove_original = true;
        spec.edits
            .set
            .insert("fixed".into(), TypedValue::Bool(true));
        let r = run(
            &mut b.clone(),
            &spec,
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1);
        assert_eq!(
            b.all("q")
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [other],
            "only the original is removed"
        );
        assert_eq!(
            b.delivery_count("q", other),
            0,
            "the message examined on the way was released"
        );
        let sent = &b.sent("fixed")[0];
        assert_eq!(sent.message_id.as_deref(), Some("m-1"));
        assert_eq!(sent.properties["fixed"], TypedValue::Bool(true));
        assert_eq!(b.held_locks(), 0);
    }

    #[test]
    fn a_subscription_source_defaults_to_its_topic() {
        assert_eq!(
            default_target(&EntityPath::subscription("events", "all")),
            "events"
        );
        assert_eq!(default_target(&EntityPath::queue("orders")), "orders");
    }

    #[test]
    fn an_empty_target_is_the_default_one() {
        let mut spec = resubmit_spec("q/$DeadLetterQueue", &[1], "");
        spec.source.entity = EntityPath::subscription("events", "all");
        assert_eq!(spec.target_name().unwrap(), "events");
        spec.target = "events/subscriptions/all".into();
        assert!(matches!(spec.target_name(), Err(Error::Invalid(_))));
    }

    #[tokio::test]
    async fn dead_letter_properties_are_dropped_unless_kept() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let msg = || {
            FakeMsg::active("x")
                .prop("DeadLetterReason", PropValue::String("bad".into()))
                .prop(
                    "DeadLetterErrorDescription",
                    PropValue::String("why".into()),
                )
                .prop("keep", PropValue::String("me".into()))
        };
        let (a, c) = (b.add(dl, msg()), b.add(dl, msg()));
        let none = CancellationToken::new();
        run(
            &mut b.clone(),
            &move_spec(dl, &[a], "t"),
            &[a],
            &NoProgress,
            &none,
        )
        .await
        .unwrap();
        let mut keep = move_spec(dl, &[c], "t");
        keep.keep_dead_letter_properties = true;
        run(&mut b.clone(), &keep, &[c], &NoProgress, &none)
            .await
            .unwrap();
        let sent = b.sent("t");
        assert!(!sent[0].properties.contains_key("DeadLetterReason"));
        assert!(
            !sent[0]
                .properties
                .contains_key("DeadLetterErrorDescription")
        );
        assert!(sent[0].properties.contains_key("keep"));
        assert_eq!(
            sent[1].properties["DeadLetterReason"],
            TypedValue::String("bad".into())
        );
    }

    #[tokio::test]
    async fn a_broken_link_fails_its_message_and_the_next_one_reopens() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let s: Vec<i64> = (0..2).map(|_| b.add(dl, FakeMsg::active("x"))).collect();
        b.fail_next(FakeOp::Send, Error::Network("link detached".into()));
        let r = run(
            &mut b.clone(),
            &move_spec(dl, &s, "t"),
            &s,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(matches!(
            r.items[0].outcome,
            ItemOutcome::Failed {
                kind: "network",
                ..
            }
        ));
        assert!(matches!(r.items[1].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(
            b.all(dl)
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [s[0]]
        );
        assert_eq!(b.open_senders(), 0, "the broken sender was closed too");
        assert_eq!(b.held_locks(), 0);
    }

    /// Move the deferred message of `address` (dead-letter view for a dead-letter address,
    /// else the deferred view) with its copy refused and, with `abandon_fails`, the abandon too.
    async fn move_deferred_failing(address: &str, abandon_fails: bool) -> (FakeBroker, String) {
        let b = FakeBroker::default();
        let s = b.add(address, FakeMsg::active("x").deferred());
        b.fail_next(FakeOp::Send, Error::Invalid("too big".into()));
        if abandon_fails {
            b.fail_next(FakeOp::Settle, Error::Network("reset".into()));
        }
        let mut spec = move_spec(address, &[s], "t");
        if !address.contains("/$") {
            spec.source.view = PeekView::Deferred;
        }
        let r = run(
            &mut b.clone(),
            &spec,
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let ItemOutcome::Failed { kind, reason } = &r.items[0].outcome else {
            panic!("{:?}", r.items[0].outcome)
        };
        assert_eq!(*kind, "invalid", "the copy's error kind");
        assert!(reason.ends_with(": too big"), "{reason}");
        assert_eq!(b.open_receivers(), 0);
        assert_eq!(b.open_senders(), 0);
        (b, reason.clone())
    }

    #[tokio::test]
    async fn a_deferred_original_whose_copy_fails_stays_deferred() {
        // On the entity: abandoned, so its delivery count rose and the limit may dead-letter it.
        let (b, reason) = move_deferred_failing("q", false).await;
        assert!(reason.starts_with(
            "the copy failed; the original stays deferred (its delivery count rose by 1, and at the entity's limit it moves to the dead-letter queue)"
        ));
        let left = b.all("q");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].state, MessageState::Deferred);
        assert_eq!(left[0].delivery_count, 1);
        assert!(
            left[0].locked_until.is_none(),
            "unlocked now, not on expiry"
        );
        // In a dead-letter queue.
        let dl = "q/$DeadLetterQueue";
        let (b, reason) = move_deferred_failing(dl, false).await;
        assert!(
            reason.starts_with(
                "the copy failed; the original stays deferred in the dead-letter queue"
            )
        );
        assert_eq!(b.all(dl)[0].state, MessageState::Deferred);
        // The abandon failed too: locked until its lock runs out.
        let (b, reason) = move_deferred_failing("q", true).await;
        assert!(reason.starts_with(
            "the copy failed; the original stays locked until its lock expires, then its delivery count rises by 1"
        ));
        assert!(b.all("q")[0].locked_until.is_some(), "still locked");
    }

    #[tokio::test]
    async fn scheduled_messages_are_skipped_and_the_rest_moved() {
        let b = FakeBroker::default();
        let s = b.add("q", FakeMsg::active("s").scheduled());
        let mut spec = move_spec("q", &[s], "t");
        spec.source.view = PeekView::Scheduled;
        let r = run_as(
            &mut b.clone(),
            &spec,
            &[s],
            JobKind::Move,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.kind, JobKind::Move);
        assert_eq!(
            r.items[0].outcome,
            ItemOutcome::Skipped {
                reason: "scheduled messages are not moved; wait until they are enqueued".into()
            }
        );
        assert!(b.sent("t").is_empty());
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test]
    async fn a_cancelled_move_closes_its_sender_and_sends_nothing() {
        let b = FakeBroker::default();
        let s = b.add("q", FakeMsg::active("x"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut spec = move_spec("q", &[s], "t");
        spec.source.view = PeekView::Active;
        let r = run(&mut b.clone(), &spec, &[s], &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(r.cancelled);
        assert!(b.sent("t").is_empty());
        assert_eq!(b.open_senders(), 0);
        let mut keep = resubmit_spec("q", &[s], "t");
        keep.source.view = PeekView::Active;
        let r = run(&mut b.clone(), &keep, &[s], &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(r.cancelled);
        assert_eq!(
            r.items[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Cancelled
            }
        );
        assert!(b.sent("t").is_empty());
    }

    #[tokio::test]
    async fn a_move_copies_null_binary_and_unsigned_properties_as_they_are() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let s = b.add(
            dl,
            FakeMsg::active("x")
                .prop("nothing", PropValue::Null)
                .prop("blob", PropValue::Binary(vec![1, 2, 3]))
                .prop("tiny", PropValue::UByte(7))
                .prop("count", PropValue::UInt(u32::MAX)),
        );
        let r = run(
            &mut b.clone(),
            &move_spec(dl, &[s], "t"),
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1, "{:?}", r.items);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        let copy = &b.all("t")[0].application_properties;
        assert_eq!(copy["nothing"], PropValue::Null);
        assert_eq!(copy["blob"], PropValue::Binary(vec![1, 2, 3]));
        assert_eq!(copy["tiny"], PropValue::UByte(7));
        assert_eq!(copy["count"], PropValue::UInt(u32::MAX));
    }

    #[tokio::test]
    async fn a_move_that_would_lose_a_property_fails_and_keeps_the_original() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let odd = || FakeMsg::active("x").prop("weird", PropValue::Other("described".into()));
        let s = b.add(dl, odd());
        let r = run(
            &mut b.clone(),
            &move_spec(dl, &[s], "t"),
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.failed(), 1, "{:?}", r.items);
        assert!(
            matches!(&r.items[0].outcome, ItemOutcome::Failed { reason, .. }
                if reason.contains("weird") && reason.contains("--drop-uncopyable-properties")),
            "{:?}",
            r.items[0].outcome
        );
        assert!(b.all("t").is_empty(), "no copy was sent");
        assert_eq!(b.all(dl).len(), 1, "the original stays");
        assert_eq!(b.held_locks(), 0);
        // With the flag the copy goes without it, and the job says so.
        let mut spec = move_spec(dl, &[s], "t");
        spec.drop_uncopyable_properties = true;
        let r = run(
            &mut b.clone(),
            &spec,
            &[s],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1, "{:?}", r.items);
        assert!(b.all(dl).is_empty());
        assert_eq!(r.notes, ["1 copy lost properties: weird"]);
        // A resubmit that keeps the originals sends the copy and notes the loss.
        let k = b.add(dl, odd());
        let r = run(
            &mut b.clone(),
            &resubmit_spec(dl, &[k], "t"),
            &[k],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1);
        assert_eq!(r.notes, ["1 copy lost properties: weird"]);
    }

    #[tokio::test]
    async fn clone_and_resend_reports_missing_and_cloning_notes() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let s = b.add(
            dl,
            FakeMsg::active("x").prop("weird", PropValue::Other("described".into())),
        );
        let r = run(
            &mut b.clone(),
            &resubmit_spec(dl, &[s, 999], "t"),
            &[s, 999],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.target.as_deref(), Some("t"));
        assert_eq!(
            r.items[0].outcome,
            ItemOutcome::Succeeded {
                detail: Some("property weird (unsupported type) was not copied".into())
            }
        );
        assert_eq!(r.items[1].outcome, ItemOutcome::Missing);
        assert_eq!(b.open_senders(), 0);
        assert_eq!(b.held_locks(), 0, "clone and resend only peeks");
    }

    #[tokio::test]
    async fn the_move_carries_the_search_notes() {
        let b = FakeBroker::default();
        let s: Vec<i64> = (0..3).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.fail_next(FakeOp::Release, Error::Network("reset".into()));
        let mut spec = move_spec("q", &[s[2]], "t");
        spec.source.view = PeekView::Active;
        let r = run(
            &mut b.clone(),
            &spec,
            &[s[2]],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1);
        assert_eq!(
            r.notes,
            [
                "1 message could not be unlocked; its delivery count rises by 1 when the lock expires"
            ]
        );
    }

    #[test]
    fn the_operation_names_the_target_and_the_search() {
        let dev = ProfileRef {
            name: "dev".into(),
            production: false,
        };
        let dl = "q/$DeadLetterQueue";
        let one = Resolved::exact(vec![1]);
        let op = operation(&move_spec(dl, &[1], "t"), &one, JobKind::Move).unwrap();
        assert_eq!(
            op.summary(&dev),
            "Move 1 message (sequence number 1) from q/$DeadLetterQueue to t on dev (sent first, then removed: a message may arrive twice). Messages examined during the search are locked while it runs"
        );
        let two = Resolved::exact(vec![1, 2]);
        let op = operation(&resubmit_spec(dl, &[1, 2], ""), &two, JobKind::Resubmit).unwrap();
        assert_eq!(
            op.summary(&dev),
            "Resubmit 2 messages (sequence numbers 1, 2) from q/$DeadLetterQueue to q on dev"
        );
        let mut deferred = move_spec("q", &[1], "t");
        deferred.source.view = PeekView::Deferred;
        let op = operation(&deferred, &one, JobKind::Resubmit).unwrap();
        assert_eq!(
            op.summary(&dev),
            "Resubmit 1 message (sequence number 1) from q to t on dev, removing the originals",
            "deferred messages are taken by sequence number, not searched"
        );
        let partial = Resolved {
            seqs: vec![5],
            examined: 2_500,
            complete: false,
        };
        let op = operation(&resubmit_spec(dl, &[], "t"), &partial, JobKind::Resubmit).unwrap();
        assert_eq!(
            op.summary(&dev),
            "Resubmit 1 message (sequence number 5) from q/$DeadLetterQueue to t on dev; matched 1 in the first 2,500 examined; run again for the rest"
        );
    }

    #[test]
    fn the_scheduled_view_is_refused_before_anything_runs() {
        let mut spec = move_spec("q", &[1], "t");
        spec.source.view = PeekView::Scheduled;
        let Err(Error::Invalid(text)) = check(&spec) else {
            panic!("refused")
        };
        assert_eq!(
            text,
            "scheduled messages cannot be moved or resubmitted; wait until they are enqueued"
        );
        assert!(operation(&spec, &Resolved::exact(vec![1]), JobKind::Move).is_err());
    }
}
