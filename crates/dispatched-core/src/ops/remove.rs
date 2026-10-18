//! Delete messages by sequence number: complete the ones found locked, cancel scheduled ones.

use super::locate::{Found, OnFound, locate};
use super::send::distinct;
use super::{Resolved, SelectSpec, failed, lock_lost_outcome, record, seq_item};
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, Progress};
use crate::messaging::Settlement;
use crate::messaging::backend::{LockReceiver, MessagingBackend};
use crate::messaging::sender::{copy_error, send_target};
use crate::namespace::{PeekView, SubQueue};
use crate::safety::{Operation, Target};
use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::safety::CancellationToken;

/// Refuse a delete that cannot run, before any I/O: scheduled messages live on the queue or
/// topic, so a subscription's scheduled view is refused.
pub(crate) fn check(spec: &SelectSpec) -> Result<()> {
    if spec.view == PeekView::Scheduled {
        send_target(&spec.entity.base())?;
    }
    Ok(())
}

/// The operation a delete of the `resolved` selection confirms: cancelling for the scheduled
/// view, a plain delete for the deferred view (received by sequence number), and a searched
/// delete for the views found with a peek-lock link (active, dead-letter, transfer
/// dead-letter). The sentence lists the sequence numbers.
pub(crate) fn operation(spec: &SelectSpec, resolved: &Resolved) -> Result<Operation> {
    check(spec)?;
    let seqs = &resolved.seqs;
    Ok(resolved.describe(match spec.view {
        PeekView::Scheduled => {
            let target = Target::new(send_target(&spec.entity.base())?, SubQueue::Active);
            Operation::cancel_scheduled(target, seqs.len() as u64)
        }
        PeekView::Deferred => {
            Operation::delete(Target::new(spec.entity.clone(), SubQueue::Active), seqs)
        }
        view => {
            Operation::delete(Target::new(spec.entity.clone(), view.sub_queue()), seqs).searched()
        }
    }))
}

/// Completes what [`locate`] finds locked and cancels what it finds scheduled.
struct DeleteOnFound {
    address: String,
}

#[async_trait]
impl OnFound for DeleteOnFound {
    async fn on_found(
        &mut self,
        plane: &mut dyn MessagingBackend,
        rx: Option<&mut dyn LockReceiver>,
        found: Found,
    ) -> ItemResult {
        match found {
            Found::Locked { received } => {
                let outcome = match (rx, received.lock_token) {
                    (Some(rx), Some(token)) => {
                        match rx.settle(token, &Settlement::Complete).await {
                            Ok(()) => ItemOutcome::Succeeded { detail: None },
                            Err(Error::LockLost(_)) => lock_lost_outcome(),
                            Err(e) => failed(&e),
                        }
                    }
                    _ => failed(&Error::Protocol("the message came without its lock".into())),
                };
                seq_item(&self.address, &received.message, outcome)
            }
            Found::Scheduled(m) => {
                let mut one = None;
                let call = plane
                    .cancel_scheduled(&self.address, &[m.sequence_number], &mut |_, r| {
                        one = Some(r)
                    })
                    .await;
                let outcome = match one {
                    Some(Ok(())) => ItemOutcome::Succeeded { detail: None },
                    Some(Err(e)) => failed(&e),
                    None => failed(&call.err().map_or_else(
                        || Error::Protocol("the broker reported no outcome".into()),
                        |e| copy_error(&e),
                    )),
                };
                seq_item(&self.address, &m, outcome)
            }
        }
    }
}

/// Delete the messages `seqs` (what `resolve_selection` returned) from `spec`'s view: one item
/// per distinct number, in the order given, and the search's notes in the report.
pub(crate) async fn run(
    plane: &mut dyn MessagingBackend,
    spec: &SelectSpec,
    seqs: &[i64],
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let address = spec.address();
    let seqs = distinct(seqs);
    let mut job = JobRun::new(
        JobKind::Delete,
        address.clone(),
        None,
        progress,
        Some(seqs.len() as u64),
    );
    let mut on = DeleteOnFound {
        address: address.clone(),
    };
    let located = locate(
        plane,
        &address,
        spec.view,
        &seqs,
        &spec.limits,
        cancel,
        &mut on,
    )
    .await?;
    let cancelled = record(&mut job, located);
    Ok(job.finish(cancelled))
}

#[cfg(test)]
mod tests {
    use super::super::testkit::*;
    use super::*;
    use crate::activity::Outcome;
    use crate::jobs::{NoProgress, NotReached};
    use crate::messaging::fake::{FakeBroker, FakeMsg, FakeOp};
    use crate::namespace::EntityPath;
    use crate::safety::ProfileRef;

    #[tokio::test]
    async fn delete_by_sequence_removes_only_those() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let s: Vec<i64> = (0..4).map(|_| b.add(dl, FakeMsg::active("x"))).collect();
        let r = run(
            &mut b.clone(),
            &select(dl, PeekView::DeadLetter, &[s[1], s[3]]),
            &[s[1], s[3]],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 2);
        assert_eq!(
            b.all(dl)
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [s[0], s[2]]
        );
        assert_eq!(b.held_locks(), 0);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test]
    async fn delete_cancels_scheduled_messages() {
        let b = FakeBroker::default();
        let keep = b.add("q", FakeMsg::active("k").scheduled());
        let gone = b.add("q", FakeMsg::active("g").scheduled());
        let r = run(
            &mut b.clone(),
            &select("q", PeekView::Scheduled, &[gone]),
            &[gone],
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
            [keep]
        );
        assert_eq!(
            b.held_locks(),
            0,
            "scheduled messages are cancelled, never locked"
        );
    }

    #[tokio::test]
    async fn delete_reports_the_job_and_its_items_by_address() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let a = b.add(dl, FakeMsg::active("x"));
        let r = run(
            &mut b.clone(),
            &select(dl, PeekView::DeadLetter, &[a, 999]),
            &[a, 999],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.kind, JobKind::Delete);
        assert_eq!(r.entity, dl);
        assert_eq!(r.items[0].entity, dl);
        assert_eq!(r.items[0].sequence_number, Some(a));
        assert_eq!(r.items[1].outcome, ItemOutcome::Missing);
        assert!(!r.cancelled);
        assert_eq!(r.skipped(), 1);
        assert_eq!(
            r.outcome(),
            Outcome::Succeeded,
            "a message already gone is not a failure"
        );
    }

    #[tokio::test]
    async fn delete_takes_deferred_messages_by_sequence_number() {
        let b = FakeBroker::default();
        let keep = b.add("q", FakeMsg::active("k").deferred());
        let gone = b.add("q", FakeMsg::active("g").deferred());
        let dl = "q/$DeadLetterQueue";
        let dead = b.add(dl, FakeMsg::active("d").deferred());
        let r = run(
            &mut b.clone(),
            &select("q", PeekView::Deferred, &[gone]),
            &[gone],
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
            [keep]
        );
        let r = run(
            &mut b.clone(),
            &select(dl, PeekView::DeadLetter, &[dead]),
            &[dead],
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.succeeded(), 1, "a deferred dead-lettered message too");
        assert!(b.all(dl).is_empty());
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test]
    async fn a_lock_lost_before_the_complete_fails_only_that_item() {
        let b = FakeBroker::default();
        let s: Vec<i64> = (0..2).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.expire_lock_on_next_settle();
        let r = run(
            &mut b.clone(),
            &select("q", PeekView::Active, &s),
            &s,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            r.items[0].outcome,
            ItemOutcome::Failed {
                kind: "lock_lost",
                reason: "the lock expired; run again for this sequence number".into()
            }
        );
        assert!(matches!(r.items[1].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(r.failed_sequences(), [s[0]]);
    }

    #[tokio::test]
    async fn the_search_notes_are_carried_into_the_report() {
        let b = FakeBroker::default();
        let s: Vec<i64> = (0..3).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        b.fail_next(FakeOp::Release, Error::Network("reset".into()));
        let r = run(
            &mut b.clone(),
            &select("q", PeekView::Active, &[s[2]]),
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

    #[tokio::test]
    async fn a_cancelled_delete_touches_nothing_and_says_so() {
        let b = FakeBroker::default();
        let s = b.add("q", FakeMsg::active("x"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let r = run(
            &mut b.clone(),
            &select("q", PeekView::Active, &[s]),
            &[s],
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
    fn the_operation_follows_the_view() {
        let dev = ProfileRef {
            name: "dev".into(),
            production: false,
        };
        let op = operation(
            &select("q", PeekView::Scheduled, &[1]),
            &Resolved::exact(vec![1]),
        )
        .unwrap();
        assert_eq!(op.verb(), "cancel-scheduled");
        let op = operation(
            &select("q", PeekView::Deferred, &[1, 2]),
            &Resolved::exact(vec![1, 2]),
        )
        .unwrap();
        assert_eq!(
            op.summary(&dev),
            "Delete 2 messages (sequence numbers 1, 2) from q on dev"
        );
        for (address, view) in [
            ("q", PeekView::Active),
            ("q/$DeadLetterQueue", PeekView::DeadLetter),
            ("q/$Transfer/$DeadLetterQueue", PeekView::TransferDeadLetter),
        ] {
            let op = operation(&select(address, view, &[7]), &Resolved::exact(vec![7])).unwrap();
            assert_eq!(
                op.summary(&dev),
                format!(
                    "Delete 1 message (sequence number 7) from {address} on dev. Messages examined during the search are locked while it runs"
                )
            );
        }
        let sub = SelectSpec {
            entity: EntityPath::subscription("events", "all"),
            ..select("q", PeekView::Scheduled, &[1])
        };
        assert!(matches!(check(&sub), Err(Error::Invalid(_))));
        assert!(matches!(
            operation(&sub, &Resolved::exact(vec![1])),
            Err(Error::Invalid(_))
        ));
        // A partial match says how far it looked.
        let partial = Resolved {
            seqs: vec![3, 8],
            examined: 10_000,
            complete: false,
        };
        let op = operation(
            &select("q/$DeadLetterQueue", PeekView::DeadLetter, &[]),
            &partial,
        )
        .unwrap();
        assert_eq!(
            op.summary(&dev),
            "Delete 2 messages (sequence numbers 3, 8) from q/$DeadLetterQueue on dev; matched 2 in the first 10,000 examined; run again for the rest. Messages examined during the search are locked while it runs"
        );
    }
}
