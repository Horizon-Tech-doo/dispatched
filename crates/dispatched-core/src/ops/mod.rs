//! Gated operations: every mutating broker call goes through [`Operator`], which builds an
//! [`Operation`], passes it through the safety [`Gate`] (confirmation, audit entry before,
//! audit entry after) and only then touches the message plane.
//!
//! The public methods are thin wrappers. Every loop and per-message decision lives in a
//! `pub(crate)` function over a [`MessagingBackend`], so tests drive it with the fake broker.

pub mod import;
pub mod locate;
pub mod purge;
pub mod receive;
pub mod remove;
pub mod resubmit;
pub mod send;
pub mod session_state;

pub use crate::messaging::sender::send_target;
pub use import::ImportOptions;
pub use locate::LocateLimits;
pub use purge::{PurgePlan, PurgeScope};
pub use receive::{ReceiveAction, ReceiveOutcome, ReceiveRequest};
pub use resubmit::{ResubmitSpec, default_target};
pub use send::SendRequest;

use crate::diagnostics::error_kind;
use crate::explorer::Explorer;
use crate::filter::MessageFilter;
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, NotReached, Progress};
use crate::messaging::backend::MessagingBackend;
use crate::messaging::{DEFAULT_SCAN_LIMIT, PeekOptions, PeekedMessage, Settlement, scan_messages};
use crate::namespace::{EntityPath, PeekView, SubQueue};
use crate::safety::{Audited, Gate, Operation, SettleVerb, Target, count_text};
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;
use std::future::Future;
use std::sync::Arc;

/// The most messages one selection may name.
pub const MAX_SELECTION: u64 = 100_000;

/// Which messages a delete, move or resubmit acts on.
#[derive(Debug, Clone)]
pub enum MessageSelection {
    /// These sequence numbers.
    Sequences(Vec<i64>),
    /// The messages of the view that match `filter`, at most `limit` (1 to 100,000). Resolved
    /// to sequence numbers with a peek before the confirmation, so it shows an exact count.
    Matching {
        /// The conditions.
        filter: MessageFilter,
        /// At most this many.
        limit: u64,
    },
}

/// The messages to act on: where, in which view, which ones, and how far the search may go.
#[derive(Debug, Clone)]
pub struct SelectSpec {
    /// Queue or subscription.
    pub entity: EntityPath,
    /// The view the messages are in.
    pub view: PeekView,
    /// Which messages.
    pub selection: MessageSelection,
    /// Limits of the lock-and-match search.
    pub limits: LocateLimits,
}

impl SelectSpec {
    /// The address of the view (`orders/$DeadLetterQueue` for the dead-letter view).
    pub fn address(&self) -> String {
        self.entity.address(self.view.sub_queue())
    }
}

fn selection_too_large() -> Error {
    Error::Invalid(format!(
        "select 1 to {} messages at a time",
        count_text(MAX_SELECTION)
    ))
}

/// The sequence numbers a selection resolved to, and how far the peek looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// Sorted, without repeats.
    pub seqs: Vec<i64>,
    /// Messages the peek examined (0 for `Sequences`).
    pub examined: u64,
    /// The whole view was looked at: no budget or limit stopped the peek.
    pub complete: bool,
}

impl Resolved {
    /// Exactly these numbers (a `Sequences` selection).
    pub(crate) fn exact(seqs: Vec<i64>) -> Self {
        Self {
            seqs,
            examined: 0,
            complete: true,
        }
    }

    /// `op` with the selection in its sentence: the numbers, and how far a partial match looked.
    pub(crate) fn describe(&self, op: Operation) -> Operation {
        let op = op.with_sequences(&self.seqs);
        if self.complete {
            op
        } else {
            op.matched_within(self.examined)
        }
    }
}

/// The sequence numbers `spec` selects, sorted and without repeats: `Sequences` as given (1 to
/// 100,000), `Matching` by peeking the view (no locks) for at most `limit` matches within the
/// first `spec.limits.max_examined` messages, which is as far as lock-and-match reaches. A
/// cancel returns `Cancelled` (checked between pages).
pub(crate) async fn resolve_selection<B: MessagingBackend + ?Sized>(
    plane: &mut B,
    spec: &SelectSpec,
    cancel: &CancellationToken,
) -> Result<Resolved> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    match &spec.selection {
        MessageSelection::Sequences(seqs) => {
            let mut seqs = seqs.clone();
            seqs.sort_unstable();
            seqs.dedup();
            if seqs.is_empty() || seqs.len() as u64 > MAX_SELECTION {
                return Err(selection_too_large());
            }
            Ok(Resolved::exact(seqs))
        }
        MessageSelection::Matching { filter, limit } => {
            if *limit == 0 || *limit > MAX_SELECTION {
                return Err(selection_too_large());
            }
            let address = spec.address();
            let budget = spec.limits.max_examined;
            let mut out: Vec<i64> = Vec::new();
            let (mut from, mut examined, mut complete) = (0, 0u64, false);
            while examined < budget {
                if cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let want = u32::try_from(*limit - out.len() as u64).unwrap_or(u32::MAX);
                let scan = u32::try_from(budget - examined)
                    .unwrap_or(u32::MAX)
                    .min(DEFAULT_SCAN_LIMIT);
                let opts = PeekOptions::new(want)
                    .from_sequence(from)
                    .scan_limit(scan)
                    .filter(filter.clone());
                let page = scan_messages(plane, &address, spec.view, &opts).await?;
                examined += u64::from(page.scanned);
                out.extend(page.messages.iter().map(|m| m.sequence_number));
                match page.next_sequence {
                    None => {
                        complete = true;
                        break;
                    }
                    Some(_) if out.len() as u64 >= *limit => break,
                    Some(next) => from = next,
                }
            }
            out.sort_unstable();
            out.dedup();
            Ok(Resolved {
                seqs: out,
                examined,
                complete,
            })
        }
    }
}

/// The item for a message found by sequence number.
pub(crate) fn seq_item(address: &str, m: &PeekedMessage, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: Some(m.sequence_number),
        message_id: m.message_id.clone(),
        session_id: m.session_id.clone(),
        entity: address.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

/// The outcome for a message whose lock was lost before it was settled.
pub(crate) fn lock_lost_outcome() -> ItemOutcome {
    ItemOutcome::Failed {
        kind: "lock_lost",
        reason: "the lock expired; run again for this sequence number".into(),
    }
}

/// Put what [`locate::locate`] found and did into `job`: its items, then its notes. Returns
/// whether the job was cancelled (an item was not reached because of it).
pub(crate) fn record(job: &mut JobRun<'_>, located: locate::Located) -> bool {
    let cancelled = located.items.iter().any(|i| {
        i.outcome
            == ItemOutcome::NotReached {
                why: NotReached::Cancelled,
            }
    });
    for item in located.items {
        job.item(item);
    }
    for note in located.notes {
        job.note(note);
    }
    cancelled
}

/// The report of a job whose selection matched nothing: nothing ran, so nothing is gated.
fn nothing_selected(
    kind: JobKind,
    address: String,
    target: Option<String>,
    progress: &dyn Progress,
) -> JobReport {
    let mut job = JobRun::new(kind, address, target, progress, Some(0));
    job.note("no messages matched the selection; nothing was changed".into());
    job.finish(false)
}

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

/// [`gated`], also returning the activity op id when the gate wrote an entry under it (a
/// denial included).
pub(crate) async fn gated_logged<T, F, Fut>(
    gate: &Gate,
    op: &Operation,
    work: F,
) -> (Option<String>, Result<T>)
where
    T: Audited,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    gate.run_logged(op, |op_id| async move {
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

    /// Send messages read from an export file to a queue or topic, one sender for the job.
    /// The confirmation lists the source sequence numbers.
    pub async fn import(
        &self,
        target: String,
        messages: Vec<crate::ImportedMessage>,
        opts: ImportOptions,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        let address = Target::new(send_target(&target)?, SubQueue::Active);
        let sources: Vec<i64> = messages.iter().filter_map(|m| m.source_sequence).collect();
        let op = Operation::import(address, messages.len() as u64).with_sequences(&sources);
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            async move {
                let mut p = explorer.plane();
                import::run(&mut p, &target, messages, &opts, progress, cancel).await
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
        // Refused before the confirmation: scheduled messages live on the queue or topic.
        let target = Target::new(send_target(&entity.base())?, SubQueue::Active);
        let distinct = send::distinct(seqs);
        let op =
            Operation::cancel_scheduled(target, distinct.len() as u64).with_sequences(&distinct);
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
        let n = send::distinct(seqs).len() as u64;
        let op = Operation::settle_deferred(target, n, settle_verb(&how));
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

    /// Delete the selected messages: active and deferred ones are completed, scheduled ones
    /// cancelled; in the dead-letter views, messages in any state.
    pub async fn delete(
        &self,
        spec: SelectSpec,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        // Refused before any I/O.
        remove::check(&spec)?;
        let resolved = resolve_selection(&mut self.explorer.plane(), &spec, cancel).await?;
        if resolved.seqs.is_empty() {
            return Ok(nothing_selected(
                JobKind::Delete,
                spec.address(),
                None,
                progress,
            ));
        }
        let op = remove::operation(&spec, &resolved)?;
        let seqs = resolved.seqs;
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            async move {
                let mut p = explorer.plane();
                remove::run(&mut p, &spec, &seqs, progress, cancel).await
            }
        })
        .await
    }

    /// Send copies of the selected messages to `spec.target`, edited, optionally removing the
    /// originals (sent first, then removed).
    pub async fn resubmit(
        &self,
        spec: ResubmitSpec,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        self.resubmit_as(spec, JobKind::Resubmit, progress, cancel)
            .await
    }

    /// Move the selected messages to the queue or topic `target`, unchanged and with their
    /// MessageIds: each is sent first, then removed (at-least-once).
    pub async fn move_messages(
        &self,
        source: SelectSpec,
        target: String,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        let spec = ResubmitSpec {
            source,
            target,
            edits: Default::default(),
            new_message_id: false,
            remove_original: true,
            keep_dead_letter_properties: false,
        };
        self.resubmit_as(spec, JobKind::Move, progress, cancel)
            .await
    }

    async fn resubmit_as(
        &self,
        spec: ResubmitSpec,
        kind: JobKind,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        // Refused before any I/O: a bad target or view needs no peek.
        let to = resubmit::check(&spec)?;
        let resolved = resolve_selection(&mut self.explorer.plane(), &spec.source, cancel).await?;
        if resolved.seqs.is_empty() {
            return Ok(nothing_selected(
                kind,
                spec.source.address(),
                Some(to),
                progress,
            ));
        }
        let op = resubmit::operation(&spec, &resolved, kind)?;
        let seqs = resolved.seqs;
        gated(&self.gate, &op, || {
            let explorer = self.explorer.clone();
            async move {
                let mut p = explorer.plane();
                resubmit::run_as(&mut p, &spec, &seqs, kind, progress, cancel).await
            }
        })
        .await
    }

    /// Count what a purge of `scope` on `entity` would remove (a read: not gated). The plan's
    /// total is what [`Operator::purge`] confirms and the most it removes.
    pub async fn plan_purge(&self, entity: &EntityPath, scope: PurgeScope) -> Result<PurgePlan> {
        purge::plan(&mut self.explorer.plane(), entity, scope).await
    }

    /// Purge each planned entity in order, each behind its own confirmation; never more
    /// messages than its plan's total. A denied entity is skipped and the rest go on.
    pub async fn purge(
        &self,
        plans: Vec<PurgePlan>,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        purge::run_many(
            &mut self.explorer.plane(),
            &self.gate,
            plans,
            progress,
            cancel,
        )
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

    /// A gate that refuses operations on the entity `target` and allows the rest.
    pub(crate) fn gate_denying_target(target: &str) -> (Gate, Arc<MemoryAudit>) {
        struct DenyTarget(String);
        #[async_trait::async_trait]
        impl Policy for DenyTarget {
            async fn confirm(
                &self,
                req: &horizon_azure_kit::safety::ConfirmRequest,
            ) -> horizon_azure_kit::safety::Decision {
                use horizon_azure_kit::safety::Decision;
                if req.target == self.0 {
                    Decision::Deny(format!("{} is not to be purged", self.0))
                } else {
                    Decision::Allow
                }
            }
        }
        gate_with(Arc::new(DenyTarget(target.to_string())))
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

    /// A property filter.
    pub(crate) fn filter(src: &str) -> MessageFilter {
        MessageFilter {
            property: Some(crate::filter::PropertyFilter::parse(src).unwrap()),
            ..Default::default()
        }
    }

    /// Select `seqs` in `view` of the entity of `address` (`q`, `q/$DeadLetterQueue`, ...).
    pub(crate) fn select(address: &str, view: PeekView, seqs: &[i64]) -> SelectSpec {
        let base = address.split("/$").next().unwrap();
        SelectSpec {
            entity: EntityPath::parse(base).unwrap(),
            view,
            selection: MessageSelection::Sequences(seqs.to_vec()),
            limits: LocateLimits {
                idle: Duration::from_millis(500),
                ..Default::default()
            },
        }
    }

    /// Resubmit `seqs` of `address` (dead-letter view for a dead-letter address, else active)
    /// to `target`, keeping the originals.
    pub(crate) fn resubmit_spec(address: &str, seqs: &[i64], target: &str) -> ResubmitSpec {
        let view = if address.ends_with("/$Transfer/$DeadLetterQueue") {
            PeekView::TransferDeadLetter
        } else if address.ends_with("/$DeadLetterQueue") {
            PeekView::DeadLetter
        } else {
            PeekView::Active
        };
        ResubmitSpec {
            source: select(address, view, seqs),
            target: target.to_string(),
            edits: Default::default(),
            new_message_id: false,
            remove_original: false,
            keep_dead_letter_properties: false,
        }
    }

    /// Move `seqs` of `address` to `target`.
    pub(crate) fn move_spec(address: &str, seqs: &[i64], target: &str) -> ResubmitSpec {
        ResubmitSpec {
            remove_original: true,
            ..resubmit_spec(address, seqs, target)
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
    use crate::messaging::PropValue;
    use crate::messaging::fake::{FakeBroker, FakeMsg};

    /// Resolve `spec` on `b` with a fresh token.
    async fn resolve(b: &FakeBroker, spec: &SelectSpec) -> Result<Resolved> {
        resolve_selection(&mut b.clone(), spec, &CancellationToken::new()).await
    }

    fn matching(filter: MessageFilter, limit: u64, address: &str, view: PeekView) -> SelectSpec {
        SelectSpec {
            selection: MessageSelection::Matching { filter, limit },
            ..select(address, view, &[])
        }
    }

    #[tokio::test]
    async fn matching_selections_resolve_to_sequence_numbers_before_the_gate() {
        let b = FakeBroker::default();
        let a = b.add(
            "q/$DeadLetterQueue",
            FakeMsg::active("x").prop("kind", PropValue::String("a".into())),
        );
        b.add(
            "q/$DeadLetterQueue",
            FakeMsg::active("y").prop("kind", PropValue::String("b".into())),
        );
        let c = b.add(
            "q/$DeadLetterQueue",
            FakeMsg::active("z").prop("kind", PropValue::String("a".into())),
        );
        let spec = SelectSpec {
            selection: MessageSelection::Matching {
                filter: filter("user.kind = 'a'"),
                limit: 100_000,
            },
            ..select("q/$DeadLetterQueue", PeekView::DeadLetter, &[])
        };
        let all = resolve(&b, &spec).await.unwrap();
        assert_eq!(all.seqs, [a, c]);
        assert!(all.complete, "the whole view was read");
        let one = SelectSpec {
            selection: MessageSelection::Matching {
                filter: filter("user.kind = 'a'"),
                limit: 1,
            },
            ..spec
        };
        let first = resolve(&b, &one).await.unwrap();
        assert_eq!(first.seqs, [a]);
        assert!(!first.complete, "the limit stopped it: more may match");
        assert_eq!(b.held_locks(), 0, "resolving only peeks");
        assert_eq!(b.all("q/$DeadLetterQueue").len(), 3);
    }

    #[tokio::test]
    async fn selections_are_sorted_deduplicated_and_bounded() {
        let b = FakeBroker::default();
        let spec = select("q", PeekView::Active, &[5, 3, 5, 1]);
        assert_eq!(
            resolve(&b, &spec).await.unwrap(),
            Resolved::exact(vec![1, 3, 5])
        );
        let too_many: Vec<i64> = (0..=MAX_SELECTION as i64).collect();
        for seqs in [too_many, Vec::new()] {
            let spec = select("q", PeekView::Active, &seqs);
            let Err(Error::Invalid(text)) = resolve(&b, &spec).await else {
                panic!("refused")
            };
            assert_eq!(text, "select 1 to 100,000 messages at a time");
        }
        for limit in [0, MAX_SELECTION + 1] {
            let spec = matching(MessageFilter::default(), limit, "q", PeekView::Active);
            assert!(matches!(resolve(&b, &spec).await, Err(Error::Invalid(_))));
        }
    }

    #[tokio::test]
    async fn a_matching_selection_reads_past_one_scan_page() {
        // More than one page of `DEFAULT_SCAN_LIMIT` examined messages, within the budget.
        let b = FakeBroker::default();
        let n = crate::messaging::DEFAULT_SCAN_LIMIT as usize + 5;
        let seqs: Vec<i64> = (0..n).map(|_| b.add("q", FakeMsg::active("x"))).collect();
        let mut spec = matching(
            MessageFilter::default(),
            MAX_SELECTION,
            "q",
            PeekView::Active,
        );
        spec.limits.max_examined = 20_000;
        let r = resolve(&b, &spec).await.unwrap();
        assert_eq!(r.seqs, seqs);
        assert!(r.complete);
        assert_eq!(r.examined, n as u64);
    }

    #[tokio::test]
    async fn a_matching_selection_stops_at_the_examined_budget_and_says_so() {
        // Lock-and-match examines at most `max_examined`: the selection stays within it.
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let seqs: Vec<i64> = (0..30).map(|_| b.add(dl, FakeMsg::active("x"))).collect();
        let mut spec = matching(
            MessageFilter::default(),
            MAX_SELECTION,
            dl,
            PeekView::DeadLetter,
        );
        spec.limits.max_examined = 10;
        let r = resolve(&b, &spec).await.unwrap();
        assert_eq!(r.seqs, seqs[..10]);
        assert_eq!(r.examined, 10);
        assert!(!r.complete);
        let op = remove::operation(&spec, &r).unwrap();
        assert_eq!(
            op.summary(&crate::safety::ProfileRef {
                name: "dev".into(),
                production: false
            }),
            format!(
                "Delete 10 messages (sequence numbers {}) from q/$DeadLetterQueue on dev; matched 10 in the first 10 examined; run again for the rest. Messages examined during the search are locked while it runs",
                seqs[..10]
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        );
    }

    #[tokio::test]
    async fn a_cancelled_selection_returns_cancelled() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        for spec in [
            matching(MessageFilter::default(), 10, "q", PeekView::Active),
            select("q", PeekView::Active, &[1]),
        ] {
            assert!(matches!(
                resolve_selection(&mut b.clone(), &spec, &cancel).await,
                Err(Error::Cancelled)
            ));
        }
        assert_eq!(b.held_locks(), 0);
    }

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
        assert_send(&o.delete(select("q", PeekView::Active, &[1]), &NoProgress, c));
        assert_send(&o.resubmit(resubmit_spec("q", &[1], "t"), &NoProgress, c));
        assert_send(&o.move_messages(
            select("q", PeekView::Active, &[1]),
            "t".into(),
            &NoProgress,
            c,
        ));
        assert_send(&o.plan_purge(e, PurgeScope::All));
        assert_send(&o.purge(Vec::new(), &NoProgress, c));
        assert_send(&o.session_state(e, "s"));
        assert_send(&o.set_session_state(e, "s", None));
    }
}
