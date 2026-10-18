//! Gated operations: every mutating broker call goes through [`Operator`], which builds an
//! [`Operation`], passes it through the safety [`Gate`] (confirmation, audit entry before,
//! audit entry after) and only then touches the message plane.
//!
//! The public methods are thin wrappers. Every loop and per-message decision lives in a
//! `pub(crate)` function over a [`MessagingBackend`], so tests drive it with the fake broker.

pub mod receive;
pub mod send;
pub mod session_state;

pub use crate::messaging::sender::send_target;
pub use receive::{ReceiveAction, ReceiveOutcome, ReceiveRequest};
pub use send::SendRequest;

use crate::diagnostics::error_kind;
use crate::explorer::Explorer;
use crate::jobs::{ItemOutcome, JobReport, Progress};
use crate::messaging::Settlement;
use crate::namespace::{EntityPath, SubQueue};
use crate::safety::{Audited, Gate, Operation, SettleVerb, Target};
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;
use std::future::Future;
use std::sync::Arc;

/// Runs operations on one namespace through one safety gate.
#[derive(Clone)]
pub struct Operator {
    explorer: Arc<Explorer>,
    gate: Gate,
}

/// Run `work` behind `gate` for `op`; the result carries the activity op id.
pub(crate) async fn gated<T, F, Fut>(gate: &Gate, op: &Operation, work: F) -> Result<T>
where
    T: Audited,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    gate.run(op, |op_id| async move {
        let mut t = work().await?;
        t.set_op_id(op_id);
        Ok(t)
    })
    .await
}

/// The safety verb of a settlement.
pub(crate) fn settle_verb(how: &Settlement) -> SettleVerb {
    match how {
        Settlement::Complete => SettleVerb::Complete,
        Settlement::Abandon => SettleVerb::Abandon,
        Settlement::Defer => SettleVerb::Defer,
        Settlement::DeadLetter { .. } => SettleVerb::DeadLetter,
    }
}

/// The item outcome for a failure.
pub(crate) fn failed(e: &Error) -> ItemOutcome {
    ItemOutcome::Failed {
        kind: error_kind(e),
        reason: e.to_string(),
    }
}

impl Operator {
    /// An operator over `explorer`'s connection, gated by `gate`.
    pub fn new(explorer: Arc<Explorer>, gate: Gate) -> Self {
        Self { explorer, gate }
    }

    /// The explorer it runs on.
    pub fn explorer(&self) -> &Explorer {
        &self.explorer
    }

    /// The gate every mutation passes.
    pub fn gate(&self) -> &Gate {
        &self.gate
    }

    /// Send (or schedule) `req.count` messages to a queue or topic.
    pub async fn send(
        &self,
        req: SendRequest,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        let target = Target::new(send_target(&req.target)?, SubQueue::Active);
        let op = Operation::send(target, req.count, req.schedule);
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            async move {
                let mut p = explorer.plane();
                send::run(&mut p, &req, progress, cancel).await
            }
        })
        .await
    }

    /// Cancel scheduled messages of `entity` by sequence number.
    pub async fn cancel_scheduled(
        &self,
        entity: &EntityPath,
        seqs: &[i64],
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        let target = Target::new(entity.clone(), SubQueue::Active);
        let op = Operation::cancel_scheduled(target, seqs.len() as u64);
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            let address = entity.base();
            async move {
                let mut p = explorer.plane();
                send::cancel_scheduled(&mut p, &address, seqs, progress, cancel).await
            }
        })
        .await
    }

    /// Receive up to `req.max` messages, settling each (peek-lock) or deleting them.
    pub async fn receive(
        &self,
        req: ReceiveRequest,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<ReceiveOutcome> {
        let target = Target::new(req.entity.clone(), req.sub_queue);
        let max = u64::from(req.max);
        let op = match &req.action {
            ReceiveAction::Delete => Operation::receive_and_delete(target, max),
            ReceiveAction::Settle(how) => {
                Operation::receive_and_settle(target, max, settle_verb(how))
            }
        };
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            async move {
                let mut p = explorer.plane();
                receive::run(&mut p, &req, progress, cancel).await
            }
        })
        .await
    }

    /// Settle deferred messages by sequence number.
    #[allow(clippy::too_many_arguments)]
    pub async fn settle_deferred(
        &self,
        entity: &EntityPath,
        sub_queue: SubQueue,
        seqs: &[i64],
        session: Option<&str>,
        how: Settlement,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        let target = Target::new(entity.clone(), sub_queue);
        let op = Operation::settle_deferred(target, seqs.len() as u64, settle_verb(&how));
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            let address = entity.address(sub_queue);
            async move {
                let mut p = explorer.plane();
                receive::settle_deferred(&mut p, &address, seqs, session, &how, progress, cancel)
                    .await
            }
        })
        .await
    }

    /// The state of `session` (a read: not gated, not audited).
    pub async fn session_state(
        &self,
        entity: &EntityPath,
        session: &str,
    ) -> Result<Option<Vec<u8>>> {
        let address = entity.address(SubQueue::Active);
        session_state::get(&mut self.explorer.plane(), &address, session).await
    }

    /// Replace the state of `session`, or with `None` clear it.
    pub async fn set_session_state(
        &self,
        entity: &EntityPath,
        session: &str,
        state: Option<Vec<u8>>,
    ) -> Result<()> {
        let target = Target::new(entity.clone(), SubQueue::Active);
        let op = Operation::set_session_state(target, session, state.is_some());
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            let address = entity.address(SubQueue::Active);
            async move {
                let mut p = explorer.plane();
                session_state::set(&mut p, &address, session, state).await
            }
        })
        .await
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Shared helpers for the operations' tests.

    use super::*;
    use crate::activity::MemoryAudit;
    use crate::messaging::OutgoingMessage;
    use crate::messaging::outgoing::MessageIdSource;
    use crate::safety::{Actor, ProfileRef};
    use horizon_azure_kit::safety::{AllowAll, DenyAll, Origin, Policy};
    use std::time::Duration;

    fn gate_with(policy: Arc<dyn Policy>) -> (Gate, Arc<MemoryAudit>) {
        let audit = Arc::new(MemoryAudit::new());
        let gate = Gate::new(
            policy,
            audit.clone(),
            Actor {
                origin: Origin::Cli,
                profile: ProfileRef {
                    name: "dev".into(),
                    production: false,
                },
            },
        );
        (gate, audit)
    }

    /// A gate that allows everything, over an in-memory audit.
    pub(crate) fn fake_gate() -> (Gate, Arc<MemoryAudit>) {
        gate_with(Arc::new(AllowAll))
    }

    /// A gate that refuses everything, over an in-memory audit.
    pub(crate) fn deny_gate() -> (Gate, Arc<MemoryAudit>) {
        gate_with(Arc::new(DenyAll))
    }

    /// Three text messages to `q`.
    pub(crate) fn three_texts() -> SendRequest {
        SendRequest {
            target: "q".into(),
            template: OutgoingMessage::text("x"),
            body: None,
            ids: MessageIdSource::Keep,
            count: 3,
            schedule: None,
            batch: false,
        }
    }

    /// Receive up to `max` from the active queue `entity`, waiting 100 ms per call.
    pub(crate) fn recv(entity: &str, max: u32, action: ReceiveAction) -> ReceiveRequest {
        ReceiveRequest {
            entity: EntityPath::queue(entity),
            sub_queue: SubQueue::Active,
            session: None,
            max,
            wait: Duration::from_millis(100),
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::activity::Outcome;
    use crate::jobs::NoProgress;
    use crate::messaging::fake::FakeBroker;

    #[tokio::test]
    async fn gated_send_with_a_denying_policy_sends_nothing() {
        // `ops::gated(&gate, &op, work)` is the one helper every Operator method uses.
        let b = FakeBroker::default();
        let (gate, audit) = deny_gate();
        let op = Operation::send(
            Target::new(EntityPath::queue("q"), SubQueue::Active),
            3,
            None,
        );
        let e = gated(&gate, &op, || async {
            send::run(
                &mut b.clone(),
                &three_texts(),
                &NoProgress,
                &CancellationToken::new(),
            )
            .await
        })
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Denied(_)));
        assert!(b.sent("q").is_empty());
        assert_eq!(audit.entries()[0].outcome, Outcome::Denied);
    }

    #[tokio::test]
    async fn gated_reports_carry_their_activity_op_id() {
        let b = FakeBroker::default();
        let (gate, audit) = fake_gate();
        let op = Operation::send(
            Target::new(EntityPath::queue("q"), SubQueue::Active),
            3,
            None,
        );
        let r = gated(&gate, &op, || async {
            send::run(
                &mut b.clone(),
                &three_texts(),
                &NoProgress,
                &CancellationToken::new(),
            )
            .await
        })
        .await
        .unwrap();
        assert_eq!(r.op_id.as_deref(), Some(audit.entries()[1].op_id.as_str()));
    }

    #[tokio::test]
    async fn a_cancelled_job_is_audited_as_cancelled_with_counts() {
        let b = FakeBroker::default();
        let (gate, audit) = fake_gate();
        let op = Operation::send(
            Target::new(EntityPath::queue("q"), SubQueue::Active),
            3,
            None,
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        let r = gated(&gate, &op, || async {
            send::run(&mut b.clone(), &three_texts(), &NoProgress, &cancel).await
        })
        .await
        .unwrap();
        assert!(r.cancelled);
        let finished = &audit.entries()[1];
        assert_eq!(finished.outcome, Outcome::Cancelled);
        assert_eq!(finished.succeeded, Some(0));
    }

    fn assert_send<T: Send>(_: &T) {}

    /// Compile-time check: the operator's futures can run on a multi-threaded runtime.
    /// Never called; nothing touches the network.
    #[allow(dead_code)]
    fn operator_futures_are_send(o: &Operator, e: &EntityPath, c: &CancellationToken) {
        assert_send(&o.send(three_texts(), &NoProgress, c));
        assert_send(&o.cancel_scheduled(e, &[1], &NoProgress, c));
        assert_send(&o.receive(recv("q", 1, ReceiveAction::Delete), &NoProgress, c));
        assert_send(&o.settle_deferred(
            e,
            SubQueue::Active,
            &[1],
            None,
            Settlement::Complete,
            &NoProgress,
            c,
        ));
        assert_send(&o.session_state(e, "s"));
        assert_send(&o.set_session_state(e, "s", None));
    }
}
