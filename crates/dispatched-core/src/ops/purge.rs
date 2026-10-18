//! Purge: remove the messages of an entity's sub-queues, never more than were confirmed.
//!
//! A [`PurgePlan`] counts first (management counts when it can read them: exact; otherwise a
//! peek: about, or at least the cap). The plan's total is what the gate confirms, and the run
//! removes at most that many: every receive-and-delete link gets credit only up to the
//! remaining allowance, and whatever a link may have removed (the messages it returned, the
//! ones it took but never returned, and credit the drain left unaccounted) is taken off the
//! allowance. Scheduled messages are never purged.

use super::receive::{self, ReceiveAction, ReceiveRequest};
use super::{failed, gated_logged};
use crate::jobs::{ItemOutcome, ItemResult, JobEvent, JobKind, JobReport, JobRun, Progress};
use crate::messaging::backend::{LockReceiver, MessagingBackend};
use crate::messaging::scan::walk_messages;
use crate::messaging::{
    MessageState, ReceiveMode, ReceiverSpec, SessionChoice, Settlement, count_messages,
    is_session_required,
};
use crate::mgmt::EntityCounts;
use crate::namespace::{EntityPath, SubQueue};
use crate::safety::{CountEstimate, Gate, Operation, Target, count_text};
use crate::{Error, NoProgress, Result};
use horizon_azure_kit::safety::CancellationToken;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::time::Duration;
use time::OffsetDateTime;

/// The most messages a peek examines per sub-queue, to count or to find deferred messages.
pub const PURGE_COUNT_CAP: u32 = 10_000;

/// A purge link stops when this passes without a delivery (an idle timeout), and a
/// next-available session accept waits this long for a session.
const PURGE_IDLE: Duration = Duration::from_secs(2);

/// Which messages a purge removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PurgeScope {
    /// Active messages (not deferred, not scheduled).
    Active,
    /// The dead-letter queue.
    DeadLetter,
    /// The transfer dead-letter queue.
    TransferDeadLetter,
    /// Active, deferred, dead-letter and transfer dead-letter messages.
    All,
}

impl PurgeScope {
    /// The sub-queues the scope receives from, in purge order.
    fn sub_queues(self) -> &'static [SubQueue] {
        match self {
            Self::Active => &[SubQueue::Active],
            Self::DeadLetter => &[SubQueue::DeadLetter],
            Self::TransferDeadLetter => &[SubQueue::TransferDeadLetter],
            Self::All => &[
                SubQueue::Active,
                SubQueue::DeadLetter,
                SubQueue::TransferDeadLetter,
            ],
        }
    }
}

/// What a purge of one entity will remove, counted before the confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgePlan {
    /// Queue or subscription.
    pub entity: EntityPath,
    /// Which messages.
    pub scope: PurgeScope,
    /// Per sub-queue, at most how many it removes (`Active` excludes deferred messages).
    pub per_queue: Vec<(SubQueue, CountEstimate)>,
    /// Deferred messages to remove (scope `All` only), counted by peek.
    pub deferred: u64,
    /// The deferred peek stopped at [`PURGE_COUNT_CAP`]: `deferred` is a lower bound, and more
    /// deferred messages may lie beyond the messages examined.
    pub deferred_truncated: bool,
    /// The whole purge: what the gate confirms and the most it removes.
    pub total: CountEstimate,
    /// Whether the entity is session-enabled; `None` when management cannot say.
    pub requires_session: Option<bool>,
}

impl PurgePlan {
    /// The target the confirmation names: the whole entity for `All`, else the sub-queue.
    pub fn target(&self) -> Target {
        match self.scope {
            PurgeScope::All => Target::all(self.entity.clone()),
            scope => Target::new(self.entity.clone(), scope.sub_queues()[0]),
        }
    }

    /// The operation the gate confirms for this plan.
    pub fn operation(&self) -> Operation {
        let op = Operation::purge(self.target(), self.total);
        if self.deferred_truncated {
            op.deferred_at_least(self.deferred)
        } else {
            op
        }
    }

    /// The deferred count: `AtLeast` when its peek stopped at the cap.
    pub fn deferred_estimate(&self) -> CountEstimate {
        peeked_exact(self.deferred, self.deferred_truncated)
    }

    /// The most messages the plan removes; refused when the total or a line is not bounded
    /// (an unknown count cannot be confirmed as a limit).
    pub(crate) fn bound(&self) -> Result<u64> {
        let total = self.total.bound();
        match total {
            Some(n) if self.per_queue.iter().all(|(_, c)| c.bound().is_some()) => Ok(n),
            _ => Err(Error::Invalid(format!(
                "the number of messages to purge in {} is not known; count them first",
                self.target().address_text()
            ))),
        }
    }

    /// Mark the entity session-enabled (a test convenience; real plans ask management).
    #[cfg(test)]
    pub(crate) fn sessions(mut self) -> Self {
        self.requires_session = Some(true);
        self
    }
}

/// A count from management: exact unless it says otherwise.
fn managed(c: &EntityCounts, n: u64) -> CountEstimate {
    if c.truncated {
        CountEstimate::AtLeast(n)
    } else if c.approximate {
        CountEstimate::About(n)
    } else {
        CountEstimate::Exact(n)
    }
}

/// A count from a peek: about, or at least what was seen when the cap was hit.
fn peeked(n: u64, truncated: bool) -> CountEstimate {
    if truncated {
        CountEstimate::AtLeast(n)
    } else {
        CountEstimate::About(n)
    }
}

/// The sum of `parts`, as sure as the least sure of them.
fn combined(parts: &[CountEstimate]) -> CountEstimate {
    let sum = parts.iter().filter_map(CountEstimate::bound).sum();
    if parts.iter().any(|c| matches!(c, CountEstimate::Unknown)) {
        CountEstimate::Unknown
    } else if parts.iter().any(|c| matches!(c, CountEstimate::AtLeast(_))) {
        CountEstimate::AtLeast(sum)
    } else if parts
        .iter()
        .any(|c| matches!(c, CountEstimate::About(_) | CountEstimate::UpTo(_)))
    {
        CountEstimate::About(sum)
    } else {
        CountEstimate::Exact(sum)
    }
}

/// Count what a purge of `scope` on `entity` would remove (a read: nothing is locked or
/// removed). Management counts first; when management cannot say, a peek of up to
/// [`PURGE_COUNT_CAP`] messages per sub-queue. The management active count includes deferred
/// messages, so the `Active` line is active minus deferred (deferred counted by peek); `All`
/// lists deferred messages on their own. An entity without a transfer dead-letter queue has no
/// line for it.
pub(crate) async fn plan<B: MessagingBackend + ?Sized>(
    plane: &mut B,
    entity: &EntityPath,
    scope: PurgeScope,
) -> Result<PurgePlan> {
    let requires_session = plane.requires_session(entity).await?;
    let counts = plane.entity_counts(entity).await?;
    let main = if scope.sub_queues().contains(&SubQueue::Active) {
        Some(count_messages(plane, &entity.address(SubQueue::Active), PURGE_COUNT_CAP).await?)
    } else {
        None
    };
    let mut per_queue = Vec::new();
    for &q in scope.sub_queues() {
        let line = match (&counts, q) {
            (Some(c), SubQueue::Active) => {
                let m = main.as_ref().expect("counted");
                let n = c.active.saturating_sub(m.deferred);
                if m.truncated {
                    // The deferred count is a lower bound: the line may be too high.
                    CountEstimate::About(n)
                } else {
                    managed(c, n)
                }
            }
            (Some(c), SubQueue::DeadLetter) => managed(c, c.dead_letter),
            (Some(c), SubQueue::TransferDeadLetter) => managed(c, c.transfer_dead_letter),
            (None, SubQueue::Active) => {
                let m = main.as_ref().expect("counted");
                peeked(m.active, m.truncated)
            }
            (None, q) => match count_messages(plane, &entity.address(q), PURGE_COUNT_CAP).await {
                Ok(c) => peeked(c.total, c.truncated),
                Err(Error::NotFound(_)) if q == SubQueue::TransferDeadLetter => continue,
                Err(e) => return Err(e),
            },
        };
        per_queue.push((q, line));
    }
    let deferred = match (scope, &main) {
        (PurgeScope::All, Some(m)) => m.deferred,
        _ => 0,
    };
    let deferred_truncated = scope == PurgeScope::All && main.as_ref().is_some_and(|m| m.truncated);
    let mut parts: Vec<CountEstimate> = per_queue.iter().map(|(_, c)| *c).collect();
    if deferred > 0 || deferred_truncated {
        parts.push(peeked_exact(deferred, deferred_truncated));
    }
    Ok(PurgePlan {
        entity: entity.clone(),
        scope,
        per_queue,
        deferred,
        deferred_truncated,
        total: combined(&parts),
        requires_session,
    })
}

/// The deferred count: exact unless the peek hit its cap.
fn peeked_exact(n: u64, truncated: bool) -> CountEstimate {
    if truncated {
        CountEstimate::AtLeast(n)
    } else {
        CountEstimate::Exact(n)
    }
}

/// The item for a whole sub-queue or session (a failure or skip without a message).
fn bare(address: &str, session: Option<String>, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: None,
        message_id: None,
        session_id: session,
        entity: address.to_string(),
        new_sequence_number: None,
        outcome,
    }
}

/// Deferred sequence numbers per session (one group without a session unless `by_session`).
fn group_deferred(
    found: &[(i64, Option<String>)],
    by_session: bool,
) -> VecDeque<(Option<String>, Vec<i64>)> {
    let mut groups: BTreeMap<Option<String>, Vec<i64>> = BTreeMap::new();
    for (seq, session) in found {
        let key = if by_session { session.clone() } else { None };
        groups.entry(key).or_default().push(*seq);
    }
    groups.into_iter().collect()
}

/// The note for a sub-queue whose allowance ran out with messages left.
fn more_remain_note(n: u64, address: &str) -> String {
    format!(
        "{} removed from {address}; more remain, run purge again",
        count_text(n)
    )
}

/// The note for a sub-queue whose allowance ran out and whose check for leftovers hit its cap.
fn may_remain_note(n: u64, address: &str) -> String {
    format!(
        "{} removed from {address}; more may remain, run purge again",
        count_text(n)
    )
}

/// Whether a sub-queue still holds messages a purge would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Remains {
    Yes,
    No,
    /// None found within the peek cap, but there are more messages beyond it.
    Maybe,
}

/// What the deferred purge left behind.
#[derive(Debug, Default)]
struct Leftover {
    /// More deferred messages were found than the allowance covers.
    more: bool,
    /// The search hit its cap: deferred messages may lie beyond it.
    truncated: bool,
}

/// The purge of one sub-queue (or the deferred messages) so far.
#[derive(Debug, Default)]
struct Drain {
    /// Messages that may have been removed: counted against the allowance.
    used: u64,
    /// Messages removed and reported.
    removed: u64,
    cancelled: bool,
}

/// One entity's purge in progress.
struct Purge<'p, 'r, B: ?Sized> {
    plane: &'p mut B,
    plan: &'p PurgePlan,
    job: JobRun<'r>,
    cancel: &'p CancellationToken,
    /// The entity is session-enabled (from the plan, or learned from an attach refusal).
    sessions: bool,
}

impl<B: MessagingBackend + ?Sized> Purge<'_, '_, B> {
    /// Receive-and-delete on `rx` up to `allowance - d.used` messages, then close it.
    async fn take(
        &mut self,
        rx: Box<dyn LockReceiver>,
        q: SubQueue,
        address: &str,
        allowance: u64,
        d: &mut Drain,
    ) {
        let want = allowance - d.used;
        self.take_up_to(rx, q, address, allowance, want, d).await;
    }

    /// Receive-and-delete on `rx` up to `want` messages (never above `allowance - d.used`),
    /// then close it. The allowance is charged with what was removed: the messages received
    /// (those the close returned too), and any credit a close that did not complete left
    /// uncertain, so the run never removes more than confirmed.
    async fn take_up_to(
        &mut self,
        rx: Box<dyn LockReceiver>,
        q: SubQueue,
        address: &str,
        allowance: u64,
        want: u64,
        d: &mut Drain,
    ) {
        let session = rx.session_id().map(str::to_string);
        let req = ReceiveRequest {
            entity: self.plan.entity.clone(),
            sub_queue: q,
            session: session.clone(),
            max: u32::try_from(want.min(allowance - d.used)).unwrap_or(u32::MAX),
            wait: PURGE_IDLE,
            action: ReceiveAction::Delete,
        };
        let ended =
            receive::take_and_close(rx, &req, address, &mut self.job, None, self.cancel).await;
        d.used = d
            .used
            .saturating_add(ended.removed_at_most())
            .min(allowance);
        d.removed += ended.received;
        d.cancelled |= receive::record_end(&mut self.job, &req, address, ended);
    }

    /// Purge sub-queue `q` up to `allowance` messages.
    async fn sub_queue(&mut self, q: SubQueue, allowance: u64) -> Drain {
        let address = self.plan.entity.address(q);
        let mut d = Drain::default();
        // Sub-queues (dead-letter, transfer dead-letter) are never session-enabled.
        let by_session = q == SubQueue::Active && self.sessions;
        if !by_session {
            let spec = ReceiverSpec::receive_and_delete(address.clone());
            match self.plane.open_receiver(&spec, PURGE_IDLE).await {
                Ok(Some(rx)) => {
                    self.take(rx, q, &address, allowance, &mut d).await;
                    return d;
                }
                Ok(None) => {
                    let e = Error::Protocol(format!("no receiver was opened on {address}"));
                    self.job.item(bare(&address, None, failed(&e)));
                    return d;
                }
                Err(e) if q == SubQueue::Active && is_session_required(&e) => self.sessions = true,
                Err(e) => {
                    self.job.item(bare(&address, None, failed(&e)));
                    return d;
                }
            }
        }
        self.sessions_of(q, &address, allowance, &mut d).await;
        d
    }

    /// Purge a session-enabled queue: next-available sessions until none is left, then the
    /// listed sessions not yet visited. A session locked by another receiver is skipped.
    async fn sessions_of(&mut self, q: SubQueue, address: &str, allowance: u64, d: &mut Drain) {
        let mut visited: HashSet<String> = HashSet::new();
        let next = ReceiverSpec {
            address: address.to_string(),
            mode: ReceiveMode::ReceiveAndDelete,
            session: SessionChoice::NextAvailable,
        };
        while d.used < allowance && !d.cancelled {
            if self.cancel.is_cancelled() {
                d.cancelled = true;
                return;
            }
            match self.plane.open_receiver(&next, PURGE_IDLE).await {
                Ok(None) => break,
                Ok(Some(rx)) => {
                    let fresh = rx
                        .session_id()
                        .is_none_or(|id| visited.insert(id.to_string()));
                    if !fresh {
                        // Handed a session already emptied (it may keep state): none is left.
                        // Close it without asking for messages (it grants no credit).
                        self.take_up_to(rx, q, address, allowance, 0, d).await;
                        break;
                    }
                    self.take(rx, q, address, allowance, d).await;
                }
                Err(e) => {
                    self.job.item(bare(address, None, failed(&e)));
                    break;
                }
            }
        }
        if d.used >= allowance || d.cancelled {
            return;
        }
        let ids = match self.plane.session_ids(address).await {
            Ok(ids) => ids,
            Err(e) => {
                self.job
                    .note(format!("could not list the sessions of {address}: {e}"));
                return;
            }
        };
        for id in ids {
            if d.used >= allowance || d.cancelled {
                return;
            }
            if self.cancel.is_cancelled() {
                d.cancelled = true;
                return;
            }
            if !visited.insert(id.clone()) {
                continue;
            }
            let spec = ReceiverSpec {
                address: address.to_string(),
                mode: ReceiveMode::ReceiveAndDelete,
                session: SessionChoice::Id(id.clone()),
            };
            match self.plane.open_receiver(&spec, PURGE_IDLE).await {
                Ok(Some(rx)) => self.take(rx, q, address, allowance, d).await,
                Ok(None) => {
                    let e = Error::Protocol(format!("no receiver was opened on {address}"));
                    self.job.item(bare(address, Some(id), failed(&e)));
                }
                Err(Error::Conflict(_)) => {
                    let reason = format!("session {id} is locked by another receiver");
                    self.job
                        .item(bare(address, Some(id), ItemOutcome::Skipped { reason }));
                }
                Err(e) => self.job.item(bare(address, Some(id), failed(&e))),
            }
        }
    }

    /// Complete up to `allowance` deferred messages of the entity, by sequence number, found by
    /// a peek of up to [`PURGE_COUNT_CAP`] messages. On a session entity they are received
    /// inside their session; when that was not known and the plain receiver is refused as
    /// session-required, they are grouped by session and tried again.
    async fn deferred(&mut self, allowance: u64) -> (Drain, Leftover) {
        let address = self.plan.entity.address(SubQueue::Active);
        let mut d = Drain::default();
        let mut found: Vec<(i64, Option<String>)> = Vec::new();
        let mut left = Leftover::default();
        let cap = usize::try_from(allowance).unwrap_or(usize::MAX);
        let walked = walk_messages(self.plane, &address, PURGE_COUNT_CAP, |m| {
            if m.state == MessageState::Deferred {
                if found.len() < cap {
                    found.push((m.sequence_number, m.session_id.clone()));
                } else {
                    left.more = true;
                }
            }
        })
        .await;
        match walked {
            Ok(w) => left.truncated = w.truncated,
            Err(e) => {
                let reason = format!("could not find the deferred messages: {e}");
                let outcome = ItemOutcome::Failed {
                    kind: crate::diagnostics::error_kind(&e),
                    reason,
                };
                self.job.item(bare(&address, None, outcome));
                return (d, left);
            }
        }
        let mut groups = group_deferred(&found, self.sessions);
        let mut regrouped = false;
        while let Some((session, seqs)) = groups.pop_front() {
            if self.cancel.is_cancelled() {
                d.cancelled = true;
                break;
            }
            let settled = receive::settle_deferred(
                self.plane,
                &address,
                &seqs,
                session.as_deref(),
                &Settlement::Complete,
                &NoProgress,
                self.cancel,
            )
            .await;
            match settled {
                // No receiver opened, so nothing was received: try again inside the sessions.
                Err(e) if session.is_none() && !regrouped && is_session_required(&e) => {
                    self.sessions = true;
                    regrouped = true;
                    groups = group_deferred(&found, true);
                }
                Ok(report) => {
                    d.used += seqs.len() as u64;
                    d.removed += report.succeeded();
                    d.cancelled |= report.cancelled;
                    for item in report.items {
                        self.job.item(item);
                    }
                    for note in report.notes {
                        self.job.note(note);
                    }
                }
                Err(e) => {
                    d.used += seqs.len() as u64;
                    let outcome = match (&e, &session) {
                        (Error::Conflict(_), Some(id)) => ItemOutcome::Skipped {
                            reason: format!("session {id} is locked by another receiver"),
                        },
                        _ => failed(&e),
                    };
                    for seq in seqs {
                        self.job.item(ItemResult {
                            sequence_number: Some(seq),
                            ..bare(&address, session.clone(), outcome.clone())
                        });
                    }
                }
            }
        }
        (d, left)
    }

    /// Whether messages purge would take are still in `q` (active ones for the queue itself),
    /// from a peek of up to [`PURGE_COUNT_CAP`] messages.
    async fn remains(&mut self, q: SubQueue) -> Result<Remains> {
        let address = self.plan.entity.address(q);
        let mut any = false;
        let walked = walk_messages(self.plane, &address, PURGE_COUNT_CAP, |m| {
            if q != SubQueue::Active || m.state == MessageState::Active {
                any = true;
            }
        })
        .await?;
        Ok(if any {
            Remains::Yes
        } else if walked.truncated {
            Remains::Maybe
        } else {
            Remains::No
        })
    }
}

/// Purge one entity as `plan` says: each sub-queue with receive-and-delete, credit never above
/// the remaining allowance, then (scope `All`) the deferred messages by sequence number. The
/// whole run removes at most `plan.total`, and each line at most its own count. Items are per
/// message; a sub-queue whose allowance ran out with messages left gets a note.
pub(crate) async fn run<B: MessagingBackend + ?Sized>(
    plane: &mut B,
    plan: &PurgePlan,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    // `run_many` checks this before the confirmation; here it is a backstop.
    let total = plan.bound()?;
    let mut p = Purge {
        plane,
        plan,
        job: JobRun::new(
            JobKind::Purge,
            plan.target().address_text(),
            None,
            progress,
            Some(total),
        ),
        cancel,
        sessions: plan.requires_session == Some(true),
    };
    let mut left = total;
    let mut cancelled = false;
    for (q, count) in &plan.per_queue {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let bound = count.bound().unwrap_or(0);
        let allowance = bound.min(left);
        // A line whose count the lines before it used up removes nothing (but is noted below).
        let d = if allowance == 0 {
            if bound == 0 {
                continue;
            }
            Drain::default()
        } else {
            p.sub_queue(*q, allowance).await
        };
        left -= d.used.min(left);
        if d.cancelled {
            cancelled = true;
            break;
        }
        if d.used >= allowance {
            match p.remains(*q).await {
                Ok(Remains::Yes) => p
                    .job
                    .note(more_remain_note(d.removed, &plan.entity.address(*q))),
                Ok(Remains::Maybe) => p
                    .job
                    .note(may_remain_note(d.removed, &plan.entity.address(*q))),
                Ok(Remains::No) => {}
                Err(e) => p.job.note(format!(
                    "could not check whether messages remain in {}: {e}",
                    plan.entity.address(*q)
                )),
            }
        }
    }
    let allowance = if plan.scope == PurgeScope::All {
        plan.deferred.min(left)
    } else {
        0
    };
    let mut left_over = Leftover {
        more: false,
        // Deferred messages beyond the plan's capped count may remain even with no allowance.
        truncated: plan.scope == PurgeScope::All && plan.deferred_truncated,
    };
    let mut removed = 0;
    if !cancelled && allowance > 0 {
        if cancel.is_cancelled() {
            cancelled = true;
        } else {
            let (d, found) = p.deferred(allowance).await;
            cancelled |= d.cancelled;
            removed = d.removed;
            left_over.more = found.more;
            left_over.truncated |= found.truncated;
        }
    }
    let address = plan.entity.address(SubQueue::Active);
    match (cancelled, left_over.more, left_over.truncated) {
        (true, ..) => {}
        (false, true, _) => p.job.note(format!(
            "{} deferred removed from {address}; more remain, run purge again",
            count_text(removed)
        )),
        (false, false, true) => p.job.note(format!(
            "more deferred messages may remain in {address}; run purge again"
        )),
        (false, false, false) => {}
    }
    Ok(p.job.finish(cancelled))
}

/// Purge several entities in order, each behind its own confirmation (one gate run per
/// entity, confirming that entity's total). A denied entity is one skipped item and the rest go
/// on; another failure before an entity's purge ran is one failed item. A plan without a bounded
/// count is one failed item and is never put to the confirmation. The combined report has no op
/// id of its own: a note per entity the gate logged names its activity op id (`(denied)` for a
/// denial).
pub(crate) async fn run_many<B: MessagingBackend + ?Sized>(
    plane: &mut B,
    gate: &Gate,
    plans: Vec<PurgePlan>,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let entity = plans
        .iter()
        .map(|p| p.target().address_text())
        .collect::<Vec<_>>()
        .join(", ");
    let started_at = OffsetDateTime::now_utc();
    let (mut items, mut notes, mut cancelled) = (Vec::new(), Vec::new(), false);
    let push = |items: &mut Vec<ItemResult>, item: ItemResult| {
        progress.event(&JobEvent::Item(item.clone()));
        items.push(item);
    };
    for plan in &plans {
        let name = plan.target().address_text();
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        // Refused before the confirmation: never ask to confirm what would then be refused.
        if let Err(e) = plan.bound() {
            push(&mut items, bare(&plan.entity.base(), None, failed(&e)));
            continue;
        }
        let op = plan.operation();
        let (logged, ran) =
            gated_logged(gate, &op, || run(&mut *plane, plan, progress, cancel)).await;
        if let Some(op_id) = &logged {
            let denied = if matches!(ran, Err(Error::Denied(_))) {
                " (denied)"
            } else {
                ""
            };
            notes.push(format!("{name}: activity op {op_id}{denied}"));
        }
        match ran {
            Ok(report) => {
                notes.extend(report.notes.iter().map(|n| format!("{name}: {n}")));
                items.extend(report.items);
                if report.cancelled {
                    cancelled = true;
                    break;
                }
            }
            Err(Error::Cancelled) => {
                cancelled = true;
                break;
            }
            Err(e) => {
                let outcome = match &e {
                    Error::Denied(_) => ItemOutcome::Skipped {
                        reason: e.to_string(),
                    },
                    _ => failed(&e),
                };
                push(&mut items, bare(&plan.entity.base(), None, outcome));
            }
        }
    }
    Ok(JobReport {
        op_id: None,
        kind: JobKind::Purge,
        entity,
        target: None,
        started_at,
        finished_at: OffsetDateTime::now_utc(),
        cancelled,
        items,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::super::testkit::*;
    use super::*;
    use crate::activity::{Outcome, Phase};
    use crate::messaging::fake::{FakeBroker, FakeMsg, FakeOp};

    /// One line of `scope` on queue `entity`, with `n` as its count and the total.
    fn plan_with(entity: &str, scope: PurgeScope, n: CountEstimate) -> PurgePlan {
        PurgePlan {
            entity: EntityPath::queue(entity),
            scope,
            per_queue: vec![(scope.sub_queues()[0], n)],
            deferred: 0,
            deferred_truncated: false,
            total: n,
            requires_session: None,
        }
    }

    /// Exact lines and deferred count; the total is their sum.
    fn plan_with_counts(
        entity: &str,
        scope: PurgeScope,
        lines: &[(SubQueue, u64)],
        deferred: u64,
    ) -> PurgePlan {
        let sum: u64 = lines.iter().map(|(_, n)| n).sum::<u64>() + deferred;
        PurgePlan {
            entity: EntityPath::queue(entity),
            scope,
            per_queue: lines
                .iter()
                .map(|(q, n)| (*q, CountEstimate::Exact(*n)))
                .collect(),
            deferred,
            deferred_truncated: false,
            total: CountEstimate::Exact(sum),
            requires_session: None,
        }
    }

    async fn purge(b: &FakeBroker, plan: &PurgePlan) -> JobReport {
        run(&mut b.clone(), plan, &NoProgress, &CancellationToken::new())
            .await
            .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn purge_never_removes_more_than_confirmed() {
        let b = FakeBroker::default();
        for _ in 0..10 {
            b.add("q", FakeMsg::active("x"));
        }
        let plan = plan_with("q", PurgeScope::Active, CountEstimate::AtLeast(6));
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 6);
        assert_eq!(b.all("q").len(), 4);
        assert!(b.credit_granted_max() <= 6);
        assert!(
            r.notes
                .iter()
                .any(|n| n == "6 removed from q; more remain, run purge again"),
            "{:?}",
            r.notes
        );
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn purge_all_covers_deferred_and_dead_letters_but_keeps_scheduled() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        b.add("q", FakeMsg::active("d").deferred());
        let s = b.add("q", FakeMsg::active("s").scheduled());
        b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        let plan = plan_with_counts(
            "q",
            PurgeScope::All,
            &[(SubQueue::Active, 1), (SubQueue::DeadLetter, 1)],
            1,
        );
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 3, "{:?}", r.items);
        assert_eq!(
            b.all("q")
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [s]
        );
        assert!(b.all("q/$DeadLetterQueue").is_empty());
        assert_eq!((b.held_locks(), b.open_receivers()), (0, 0));
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }

    #[tokio::test(start_paused = true)]
    async fn purge_drains_every_session_and_skips_locked_ones() {
        let b = FakeBroker::default();
        b.add("s", FakeMsg::active("1").in_session("A"));
        b.add("s", FakeMsg::active("2").in_session("B"));
        b.add("s", FakeMsg::active("3").in_session("C"));
        b.lock_session_elsewhere("s", "B");
        let plan = plan_with("s", PurgeScope::Active, CountEstimate::Exact(3)).sessions();
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 2);
        assert!(r.items.iter().any(|i| matches!(&i.outcome, ItemOutcome::Skipped { reason } if reason == "session B is locked by another receiver")));
        assert_eq!(
            r.items
                .iter()
                .filter(|i| matches!(i.outcome, ItemOutcome::Succeeded { .. }))
                .map(|i| i.session_id.as_deref())
                .collect::<Vec<_>>(),
            [Some("A"), Some("C")],
            "items carry their session"
        );
        assert_eq!(b.all("s").len(), 1);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_queue_purge_removes_every_session_in_one_run() {
        // Each session's receiver gets more credit than its session holds; the unspent credit
        // is unconfirmed until the close settles it, so it must not use up the allowance.
        let b = FakeBroker::default();
        for (id, n) in [("A", 4), ("B", 3), ("C", 3)] {
            for _ in 0..n {
                b.add("s", FakeMsg::active("x").in_session(id));
            }
        }
        let plan = plan_with("s", PurgeScope::Active, CountEstimate::Exact(10)).sessions();
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 10, "{:?}", r.items);
        assert!(b.all("s").is_empty());
        assert!(r.notes.is_empty(), "no false note: {:?}", r.notes);
        assert!(b.credit_granted_max() <= 10);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_close_that_does_not_complete_charges_the_allowance_and_keeps_the_note() {
        let b = FakeBroker::default();
        for _ in 0..2 {
            b.add("q", FakeMsg::active("x"));
        }
        b.fail_next_close();
        let plan = plan_with("q", PurgeScope::Active, CountEstimate::Exact(5));
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 2);
        assert!(
            r.notes
                .iter()
                .any(|n| n.starts_with("up to 3 more messages may have been removed")),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_entity_is_learned_from_the_first_attach() {
        // Management cannot say, and the plain receiver is refused: purge goes by session.
        let b = FakeBroker::default();
        b.require_sessions("s");
        b.counts_unavailable();
        b.add("s", FakeMsg::active("1").in_session("A"));
        b.add("s", FakeMsg::active("2").in_session("B"));
        let plan = purge::plan(&mut b.clone(), &EntityPath::queue("s"), PurgeScope::Active)
            .await
            .unwrap();
        assert_eq!(plan.requires_session, None);
        assert_eq!(plan.total, CountEstimate::About(2));
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 2, "{:?}", r.items);
        assert_eq!(r.failed(), 0);
        assert!(b.all("s").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn active_purge_bound_leaves_deferred_and_new_arrivals() {
        let b = FakeBroker::default();
        for _ in 0..5 {
            b.add("q", FakeMsg::active("a"));
        }
        for _ in 0..5 {
            b.add("q", FakeMsg::active("d").deferred());
        }
        let plan = purge::plan(&mut b.clone(), &EntityPath::queue("q"), PurgeScope::Active)
            .await
            .unwrap();
        assert_eq!(
            plan.total.bound(),
            Some(5),
            "the management active count (10) includes the 5 deferred"
        );
        for _ in 0..3 {
            b.add("q", FakeMsg::active("new"));
        }
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 5, "at most the confirmed 5");
        assert_eq!(
            b.all("q").len(),
            8,
            "the 5 deferred and 3 new arrivals stay"
        );
        assert!(b.credit_granted_max() <= 5);
    }

    #[tokio::test(start_paused = true)]
    async fn plans_count_by_management_or_by_peek() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        b.add("q", FakeMsg::active("d").deferred());
        b.add("q", FakeMsg::active("s").scheduled());
        b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        b.add("q/$DeadLetterQueue", FakeMsg::active("y"));
        let q = EntityPath::queue("q");
        let all = purge::plan(&mut b.clone(), &q, PurgeScope::All)
            .await
            .unwrap();
        assert_eq!(
            all.per_queue,
            [
                (SubQueue::Active, CountEstimate::Exact(1)),
                (SubQueue::DeadLetter, CountEstimate::Exact(2)),
                (SubQueue::TransferDeadLetter, CountEstimate::Exact(0)),
            ]
        );
        assert_eq!((all.deferred, all.total), (1, CountEstimate::Exact(4)));
        assert_eq!(all.requires_session, Some(false));
        assert_eq!(all.target(), Target::all(q.clone()));
        let dl = purge::plan(&mut b.clone(), &q, PurgeScope::DeadLetter)
            .await
            .unwrap();
        assert_eq!(
            dl.per_queue,
            [(SubQueue::DeadLetter, CountEstimate::Exact(2))]
        );
        assert_eq!((dl.deferred, dl.total), (0, CountEstimate::Exact(2)));
        assert_eq!(dl.target(), Target::new(q.clone(), SubQueue::DeadLetter));

        b.counts_unavailable();
        let peek = purge::plan(&mut b.clone(), &q, PurgeScope::All)
            .await
            .unwrap();
        assert_eq!(
            peek.per_queue[..2],
            [
                (SubQueue::Active, CountEstimate::About(1)),
                (SubQueue::DeadLetter, CountEstimate::About(2)),
            ]
        );
        assert_eq!((peek.deferred, peek.total), (1, CountEstimate::About(4)));
        assert_eq!(peek.requires_session, None);
        assert_eq!(b.all("q").len(), 3, "planning removes nothing");
        assert_eq!(b.held_locks(), 0);
        let json = serde_json::to_value(&peek).unwrap();
        assert_eq!(json["scope"], "all");
    }

    #[tokio::test(start_paused = true)]
    async fn a_peek_that_hits_the_cap_counts_at_least() {
        let b = FakeBroker::default();
        b.counts_unavailable();
        for _ in 0..=PURGE_COUNT_CAP {
            b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        }
        let plan = purge::plan(
            &mut b.clone(),
            &EntityPath::queue("q"),
            PurgeScope::DeadLetter,
        )
        .await
        .unwrap();
        assert_eq!(plan.total, CountEstimate::AtLeast(10_000));
    }

    #[tokio::test(start_paused = true)]
    async fn drain_shortfall_counts_against_the_allowance_and_is_noted() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("x"));
        b.add("q/$DeadLetterQueue", FakeMsg::active("y"));
        b.stall_drain(2);
        b.fail_next_close();
        // The active line may take 3; the whole purge 3: the drain's 2 unaccounted credits,
        // which the close did not settle, use up the rest, so the dead-letter queue gets
        // nothing.
        let mut plan = plan_with_counts(
            "q",
            PurgeScope::All,
            &[(SubQueue::Active, 3), (SubQueue::DeadLetter, 1)],
            0,
        );
        plan.total = CountEstimate::Exact(3);
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 1);
        assert!(b.credit_granted_max() <= 3);
        assert_eq!(
            b.all("q/$DeadLetterQueue").len(),
            1,
            "no allowance was left"
        );
        assert!(
            r.notes
                .iter()
                .any(|n| n == "0 removed from q/$DeadLetterQueue; more remain, run purge again"),
            "{:?}",
            r.notes
        );
        assert!(
            r.notes.iter().any(|n| n
                == "up to 2 more messages may have been removed after the wait and are not in this result"),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lines_never_take_more_than_the_confirmed_total() {
        let b = FakeBroker::default();
        for _ in 0..5 {
            b.add("q", FakeMsg::active("a"));
            b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        }
        let mut plan = plan_with_counts(
            "q",
            PurgeScope::All,
            &[(SubQueue::Active, 5), (SubQueue::DeadLetter, 5)],
            0,
        );
        plan.total = CountEstimate::Exact(6);
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 6);
        assert_eq!(b.all("q").len() + b.all("q/$DeadLetterQueue").len(), 4);
        assert!(
            r.notes
                .iter()
                .any(|n| n == "1 removed from q/$DeadLetterQueue; more remain, run purge again"),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_sub_queue_fails_one_item_and_the_rest_go_on() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
        b.fail_next(FakeOp::OpenReceiver, Error::Network("reset".into()));
        let plan = plan_with_counts(
            "q",
            PurgeScope::All,
            &[(SubQueue::Active, 1), (SubQueue::DeadLetter, 1)],
            0,
        );
        let r = purge(&b, &plan).await;
        assert_eq!((r.succeeded(), r.failed()), (1, 1));
        assert!(matches!(
            &r.items[0],
            ItemResult { sequence_number: None, entity, outcome: ItemOutcome::Failed { kind: "network", .. }, .. } if entity == "q"
        ));
        assert!(b.all("q/$DeadLetterQueue").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_back_link_error_is_reported() {
        let b = FakeBroker::default();
        for _ in 0..3 {
            b.add("q", FakeMsg::active("a"));
        }
        b.fail_receive_after(1, Error::Network("link detached".into()));
        let r = purge(
            &b,
            &plan_with("q", PurgeScope::Active, CountEstimate::Exact(3)),
        )
        .await;
        assert_eq!(r.succeeded(), 1);
        assert!(r.items.iter().any(|i| matches!(&i.outcome, ItemOutcome::Failed { reason, .. } if reason.contains("link detached"))), "{:?}", r.items);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_purge_removes_nothing_more() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let plan = plan_with("q", PurgeScope::Active, CountEstimate::Exact(1));
        let r = run(&mut b.clone(), &plan, &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(r.cancelled);
        assert_eq!(b.all("q").len(), 1);
        assert_eq!(b.open_receivers(), 0);
    }

    #[tokio::test]
    async fn an_unknown_count_is_refused_before_any_io() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        b.fail_next(FakeOp::OpenReceiver, Error::Network("must not open".into()));
        let plan = plan_with("q", PurgeScope::Active, CountEstimate::Unknown);
        let e = run(
            &mut b.clone(),
            &plan,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{e:?}");
        assert_eq!(b.all("q").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn bulk_purge_confirms_each_entity_and_continues_after_a_denial() {
        let b = FakeBroker::default();
        for q in ["a", "b", "c"] {
            b.add(q, FakeMsg::active("x"));
        }
        let (gate, audit) = gate_denying_target("b");
        let plans = ["a", "b", "c"]
            .map(|q| plan_with(q, PurgeScope::Active, CountEstimate::Exact(1)))
            .to_vec();
        let r = run_many(
            &mut b.clone(),
            &gate,
            plans,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(b.all("a").is_empty() && b.all("c").is_empty());
        assert_eq!(b.all("b").len(), 1, "denied: untouched");
        assert_eq!(r.succeeded(), 2);
        assert!(r.items.iter().any(|i| i.entity == "b"
            && matches!(&i.outcome, ItemOutcome::Skipped { reason } if reason.starts_with("denied: "))));
        let e = audit.entries();
        assert_eq!(
            e.iter()
                .filter(|e| e.outcome == Outcome::Denied)
                .map(|e| e.entity.as_deref())
                .collect::<Vec<_>>(),
            [Some("b")]
        );
        assert_eq!(
            e.iter().filter(|e| e.phase == Phase::Started).count(),
            2,
            "one gate run per allowed entity"
        );
        assert_eq!(
            e.iter()
                .filter(|e| e.phase == Phase::Finished && e.outcome == Outcome::Succeeded)
                .count(),
            2
        );
        assert_eq!(r.op_id, None);
        let started: Vec<&str> = e
            .iter()
            .filter(|e| e.phase == Phase::Started)
            .map(|e| e.op_id.as_str())
            .collect();
        let denied = e.iter().find(|e| e.outcome == Outcome::Denied).unwrap();
        assert_eq!(
            r.notes,
            [
                format!("a: activity op {}", started[0]),
                format!("b: activity op {} (denied)", denied.op_id),
                format!("c: activity op {}", started[1]),
            ],
            "every entity the gate logged is traceable"
        );
        assert_eq!(
            e[0].summary, "Remove 1 message from a on dev",
            "each confirmation names its own entity and count"
        );
    }

    #[tokio::test]
    async fn an_unknown_count_is_never_put_to_the_confirmation() {
        let b = FakeBroker::default();
        b.add("a", FakeMsg::active("x"));
        b.add("c", FakeMsg::active("x"));
        let (gate, audit) = fake_gate();
        let plans = vec![
            plan_with("a", PurgeScope::Active, CountEstimate::Unknown),
            plan_with("c", PurgeScope::Active, CountEstimate::Exact(1)),
        ];
        let r = run_many(
            &mut b.clone(),
            &gate,
            plans,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(matches!(
            &r.items[0],
            ItemResult { entity, outcome: ItemOutcome::Failed { kind: "invalid", .. }, .. } if entity == "a"
        ));
        assert_eq!(b.all("a").len(), 1);
        assert!(b.all("c").is_empty(), "the next entity still runs");
        let e = audit.entries();
        assert!(
            e.iter().all(|e| e.entity.as_deref() == Some("c")),
            "no confirmation or activity entry for the refused plan: {e:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deferred_messages_beyond_the_peek_cap_are_noted() {
        // The first 10,000 messages are scheduled (never purged); the deferred one lies beyond.
        let b = FakeBroker::default();
        for _ in 0..PURGE_COUNT_CAP {
            b.add("q", FakeMsg::active("s").scheduled());
        }
        b.add("q", FakeMsg::active("d").deferred());
        let plan = plan_with_counts("q", PurgeScope::All, &[(SubQueue::Active, 0)], 1);
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 0);
        assert_eq!(
            b.all("q").len(),
            PURGE_COUNT_CAP as usize + 1,
            "nothing removed"
        );
        assert!(
            r.notes
                .iter()
                .any(|n| n == "more deferred messages may remain in q; run purge again"),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_plan_whose_deferred_count_hit_the_cap_says_so_and_the_run_notes_it() {
        // Through the real plan: 10,000 scheduled messages (never purged), then deferred ones
        // the capped peek never sees, so the plan counts 0 deferred, at least.
        let b = FakeBroker::default();
        for _ in 0..PURGE_COUNT_CAP {
            b.add("q", FakeMsg::active("s").scheduled());
        }
        for _ in 0..2 {
            b.add("q", FakeMsg::active("d").deferred());
        }
        let plan = purge::plan(&mut b.clone(), &EntityPath::queue("q"), PurgeScope::All)
            .await
            .unwrap();
        assert_eq!((plan.deferred, plan.deferred_truncated), (0, true));
        assert_eq!(plan.deferred_estimate(), CountEstimate::AtLeast(0));
        let dev = crate::safety::ProfileRef {
            name: "dev".into(),
            production: false,
        };
        let text = plan.operation().summary(&dev);
        assert!(
            text.contains("; deferred messages: at least 0 (the count stopped at its cap)"),
            "{text}"
        );
        let confirmed = plan.total.bound().unwrap();
        let before = b.all("q").len();
        let r = purge(&b, &plan).await;
        let removed = (before - b.all("q").len()) as u64;
        assert!(removed <= confirmed, "{removed} > {confirmed}");
        assert_eq!(removed, r.succeeded());
        assert!(
            r.notes
                .iter()
                .any(|n| n == "more deferred messages may remain in q; run purge again"),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deferred_messages_left_beyond_the_allowance_are_noted() {
        let b = FakeBroker::default();
        for _ in 0..3 {
            b.add("q", FakeMsg::active("d").deferred());
        }
        let plan = plan_with_counts("q", PurgeScope::All, &[(SubQueue::Active, 0)], 2);
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 2);
        assert_eq!(b.all("q").len(), 1);
        assert!(
            r.notes
                .iter()
                .any(|n| n == "2 deferred removed from q; more remain, run purge again"),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_leftover_check_that_hits_the_cap_says_more_may_remain() {
        let b = FakeBroker::default();
        b.add("q", FakeMsg::active("a"));
        for _ in 0..PURGE_COUNT_CAP {
            b.add("q", FakeMsg::active("d").deferred());
        }
        b.add("q", FakeMsg::active("late"));
        let r = purge(
            &b,
            &plan_with("q", PurgeScope::Active, CountEstimate::Exact(1)),
        )
        .await;
        assert_eq!(r.succeeded(), 1);
        assert!(
            r.notes
                .iter()
                .any(|n| n == "1 removed from q; more may remain, run purge again"),
            "{:?}",
            r.notes
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deferred_messages_of_a_session_entity_go_by_session() {
        let b = FakeBroker::default();
        b.add("s", FakeMsg::active("1").in_session("A").deferred());
        b.add("s", FakeMsg::active("2").in_session("B").deferred());
        b.lock_session_elsewhere("s", "B");
        let plan = plan_with_counts("s", PurgeScope::All, &[(SubQueue::Active, 0)], 2).sessions();
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 1);
        assert!(r.items.iter().any(|i| i.session_id.as_deref() == Some("B")
            && i.sequence_number.is_some()
            && matches!(&i.outcome, ItemOutcome::Skipped { reason } if reason == "session B is locked by another receiver")));
        assert_eq!(b.all("s").len(), 1);
        assert_eq!((b.held_locks(), b.open_receivers()), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn deferred_messages_learn_that_the_entity_needs_sessions() {
        // Management cannot say and no active line ran: the plain receiver is refused, so the
        // deferred messages are tried again inside their sessions.
        let b = FakeBroker::default();
        b.require_sessions("s");
        b.add("s", FakeMsg::active("1").in_session("A").deferred());
        b.add("s", FakeMsg::active("2").in_session("B").deferred());
        let plan = plan_with_counts("s", PurgeScope::All, &[(SubQueue::Active, 0)], 2);
        let r = purge(&b, &plan).await;
        assert_eq!((r.succeeded(), r.failed()), (2, 0), "{:?}", r.items);
        assert!(b.all("s").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_handed_out_again_ends_the_next_available_loop() {
        // Session Z holds only state: next-available keeps handing it out.
        let b = FakeBroker::default();
        b.add("s", FakeMsg::active("1").in_session("A"));
        let spec = ReceiverSpec {
            address: "s".into(),
            mode: ReceiveMode::PeekLock,
            session: SessionChoice::Id("Z".into()),
        };
        let mut rx = b
            .clone()
            .open_receiver(&spec, Duration::ZERO)
            .await
            .unwrap()
            .unwrap();
        rx.set_session_state(Some(b"state")).await.unwrap();
        let _ = rx.close().await;
        let plan = plan_with("s", PurgeScope::Active, CountEstimate::Exact(5)).sessions();
        let r = purge(&b, &plan).await;
        assert_eq!(r.succeeded(), 1);
        assert_eq!(b.open_receivers(), 0);
        assert_eq!(b.session_state("s", "Z").as_deref(), Some(&b"state"[..]));
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancel_during_a_bulk_purge_stops_before_the_next_entity() {
        use crate::activity::MemoryAudit;
        use crate::safety::{Actor, ProfileRef};
        use horizon_azure_kit::safety::{ConfirmRequest, Decision, Origin, Policy};
        use std::sync::Arc;
        /// Allows everything, and cancels the job when asked about `b`.
        struct CancelOnB(CancellationToken);
        #[async_trait::async_trait]
        impl Policy for CancelOnB {
            async fn confirm(&self, req: &ConfirmRequest) -> Decision {
                if req.target == "b" {
                    self.0.cancel();
                }
                Decision::Allow
            }
        }
        let b = FakeBroker::default();
        for q in ["a", "b", "c"] {
            b.add(q, FakeMsg::active("x"));
        }
        let cancel = CancellationToken::new();
        let audit = Arc::new(MemoryAudit::new());
        let gate = Gate::new(
            Arc::new(CancelOnB(cancel.clone())),
            audit.clone(),
            Actor {
                origin: Origin::Cli,
                profile: ProfileRef {
                    name: "dev".into(),
                    production: false,
                },
            },
        );
        let plans = ["a", "b", "c"]
            .map(|q| plan_with(q, PurgeScope::Active, CountEstimate::Exact(1)))
            .to_vec();
        let r = run_many(&mut b.clone(), &gate, plans, &NoProgress, &cancel)
            .await
            .unwrap();
        assert!(r.cancelled);
        assert!(b.all("a").is_empty());
        assert_eq!((b.all("b").len(), b.all("c").len()), (1, 1));
        assert!(
            audit
                .entries()
                .iter()
                .all(|e| e.entity.as_deref() != Some("c")),
            "c was never confirmed"
        );
        assert_eq!(b.open_receivers(), 0);
    }

    #[test]
    fn counts_say_how_sure_they_are() {
        let c = |approximate, truncated| EntityCounts {
            active: 0,
            dead_letter: 0,
            scheduled: 0,
            transfer: 0,
            transfer_dead_letter: 0,
            approximate,
            truncated,
        };
        assert_eq!(managed(&c(false, false), 3), CountEstimate::Exact(3));
        assert_eq!(managed(&c(true, false), 3), CountEstimate::About(3));
        assert_eq!(managed(&c(true, true), 3), CountEstimate::AtLeast(3));
        use CountEstimate::*;
        assert_eq!(combined(&[Exact(1), Exact(2)]), Exact(3));
        assert_eq!(combined(&[Exact(1), About(2)]), About(3));
        assert_eq!(combined(&[About(1), AtLeast(2)]), AtLeast(3));
        assert_eq!(combined(&[Exact(1), Unknown]), Unknown);
    }

    use super::super::purge;
}
