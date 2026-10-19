//! The operation model: what a mutating call will do, in the exact words a human confirms.
//!
//! Every `Operation` knows its mutation kind, its target, how many messages it may touch and one
//! exact sentence naming the effect. The [`Gate`] asks a policy to confirm it, writes the activity
//! entry before the work starts and another when it ends (also when the work panics or the future
//! is dropped, unless the process is killed or exits without unwinding).

use crate::activity::{ActivityEntry, Audit, Outcome, Phase};
use crate::error::{Error, Result};
use crate::jobs::JobReport;
use crate::mgmt::{AccessRight, EntityKind, EntityRef, EntityStatus, KeySlot};
use crate::namespace::{EntityPath, SubQueue};
use horizon_azure_kit::safety::{ConfirmRequest, Origin, Policy, require};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// What kind of change an operation makes to the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mutation {
    /// Messages are added (send, schedule, import, resubmit without removal).
    Send,
    /// Received messages are settled (complete, abandon, defer, dead-letter).
    Settle,
    /// Messages are removed (delete, receive-and-delete, cancel scheduled).
    Delete,
    /// Many messages are removed at once.
    Purge,
    /// A queue, topic, subscription or rule changes.
    EntityChange,
    /// A session's state is replaced or cleared.
    SessionState,
}

/// Whether an operation reads or changes the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpKind {
    /// Read only.
    Read,
    /// Changes the broker.
    Mutating(Mutation),
}

/// How many messages an operation may touch, and how sure we are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CountEstimate {
    /// Exactly this many.
    Exact(u64),
    /// About this many (a broker counter that may lag).
    About(u64),
    /// At least this many exist; the operation touches at most this many in one run.
    AtLeast(u64),
    /// No more than this many.
    UpTo(u64),
    /// Not known.
    Unknown,
}

impl CountEstimate {
    /// The largest number of messages the operation may touch, if bounded.
    pub fn bound(&self) -> Option<u64> {
        match *self {
            CountEstimate::Exact(n)
            | CountEstimate::About(n)
            | CountEstimate::AtLeast(n)
            | CountEstimate::UpTo(n) => Some(n),
            CountEstimate::Unknown => None,
        }
    }

    /// The count as words, e.g. `1,284 messages`, `about 40 messages`, `up to 10 messages`.
    pub fn text(&self, noun: &str) -> String {
        let plural = |n: u64| {
            if n == 1 {
                format!("{} {noun}", count_text(n))
            } else {
                format!("{} {noun}s", count_text(n))
            }
        };
        match *self {
            CountEstimate::Exact(n) => plural(n),
            CountEstimate::About(n) => format!("about {}", plural(n)),
            CountEstimate::AtLeast(n) | CountEstimate::UpTo(n) => format!("up to {}", plural(n)),
            CountEstimate::Unknown => format!("an unknown number of {noun}s"),
        }
    }
}

/// A number with thousands separators: `1284` becomes `1,284`.
pub fn count_text(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The connection profile an operation runs against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRef {
    /// Profile name.
    pub name: String,
    /// Marked production: confirmations get stricter.
    pub production: bool,
}

/// Who runs an operation, and against which profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    /// Which surface asked.
    pub origin: Origin,
    /// The profile it runs against.
    pub profile: ProfileRef,
}

/// The queue an operation acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Queue or subscription.
    pub entity: EntityPath,
    /// Which sub-queue; `None` is the whole entity (purge `all`).
    pub sub_queue: Option<SubQueue>,
}

impl Target {
    /// One sub-queue of an entity.
    pub fn new(entity: EntityPath, sub_queue: SubQueue) -> Self {
        Self {
            entity,
            sub_queue: Some(sub_queue),
        }
    }

    /// The whole entity.
    pub fn all(entity: EntityPath) -> Self {
        Self {
            entity,
            sub_queue: None,
        }
    }

    /// `orders`, `orders/$DeadLetterQueue`, or the entity base for the whole entity.
    pub fn address_text(&self) -> String {
        match self.sub_queue {
            Some(q) => self.entity.address(q),
            None => self.entity.base(),
        }
    }
}

/// How a received message is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettleVerb {
    /// Remove it.
    Complete,
    /// Release the lock.
    Abandon,
    /// Set it aside for receive-by-sequence-number.
    Defer,
    /// Move it to the dead-letter queue.
    DeadLetter,
}

impl SettleVerb {
    fn text(self) -> &'static str {
        match self {
            SettleVerb::Complete => "complete",
            SettleVerb::Abandon => "abandon",
            SettleVerb::Defer => "defer",
            SettleVerb::DeadLetter => "dead-letter",
        }
    }

    fn capitalised(self) -> &'static str {
        match self {
            SettleVerb::Complete => "Complete",
            SettleVerb::Abandon => "Abandon",
            SettleVerb::Defer => "Defer",
            SettleVerb::DeadLetter => "Dead-letter",
        }
    }
}

/// An entity a delete removes, with what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    /// The entity.
    pub entity: EntityRef,
    /// The messages it holds (for a topic, those of all its subscriptions).
    pub messages: CountEstimate,
    /// For a topic, how many subscriptions it has.
    pub subscriptions: Option<u64>,
}

impl DeleteTarget {
    /// `queue orders and its 1,284 messages`, `topic events, its 3 subscriptions and their
    /// 1,204 messages`.
    fn text(&self) -> String {
        let messages = match self.messages {
            CountEstimate::Unknown => "messages (count unknown)".to_string(),
            CountEstimate::AtLeast(n) => format!("at least {}", messages(n)),
            c => c.text("message"),
        };
        match (self.entity.kind(), self.subscriptions) {
            (EntityKind::Topic, Some(0)) => format!("{} (no subscriptions)", self.entity),
            (EntityKind::Topic, Some(n)) => format!(
                "{}, its {} {} and their {messages}",
                self.entity,
                count_text(n),
                if n == 1 {
                    "subscription"
                } else {
                    "subscriptions"
                }
            ),
            _ => format!("{} and its {messages}", self.entity),
        }
    }
}

/// One exact sentence per kind of operation.
#[derive(Debug, Clone)]
enum Effect {
    CreateEntity {
        entity: EntityRef,
        rule: Option<String>,
    },
    UpdateEntity {
        entity: EntityRef,
    },
    DeleteEntities {
        targets: Vec<DeleteTarget>,
    },
    SetStatus {
        entity: EntityRef,
        status: EntityStatus,
    },
    Duplicate {
        from: EntityRef,
        to: EntityRef,
    },
    Rename {
        from: EntityRef,
        to: EntityRef,
    },
    AddRule {
        subscription: EntityRef,
        rule: String,
    },
    RemoveRule {
        subscription: EntityRef,
        rule: String,
    },
    AddPolicy {
        entity: EntityRef,
        name: String,
        rights: Vec<AccessRight>,
    },
    RemovePolicy {
        entity: EntityRef,
        name: String,
    },
    RegenerateKey {
        entity: EntityRef,
        name: String,
        slot: KeySlot,
    },
    ImportEntities {
        entities: Vec<EntityRef>,
        overwrite: bool,
    },
    Send {
        n: u64,
        schedule: Option<OffsetDateTime>,
    },
    CancelScheduled {
        n: u64,
    },
    ReceiveAndDelete {
        n: u64,
    },
    ReceiveAndSettle {
        n: u64,
        verb: SettleVerb,
    },
    SettleDeferred {
        n: u64,
        verb: SettleVerb,
    },
    Delete {
        n: u64,
    },
    Move {
        to: String,
        n: u64,
    },
    Resubmit {
        to: String,
        n: u64,
        remove_original: bool,
    },
    Purge,
    Import {
        n: u64,
    },
    SessionState {
        session: String,
        replace: bool,
    },
}

/// How many sequence numbers a sentence lists before `and N more`.
const SEQUENCES_SHOWN: usize = 20;

/// A mutating call, before it runs: what, where, how many and the words that name the effect.
#[derive(Debug, Clone)]
pub struct Operation {
    /// What kind of change it makes.
    pub mutation: Mutation,
    /// Where.
    pub target: Target,
    /// How many messages it may touch.
    pub count: CountEstimate,
    /// Always `false`: the field exists for the kit, and Dispatched cannot undo a broker operation.
    pub reversible: bool,
    effect: Effect,
    lock_search: bool,
    /// The first sequence numbers of the selection, listed after the count.
    sequences: Vec<i64>,
    /// The selection matched only within this many examined messages.
    matched_within: Option<u64>,
    /// A purge's deferred count stopped at its cap: at least this many are deferred.
    deferred_at_least: Option<u64>,
}

fn messages(n: u64) -> String {
    CountEstimate::Exact(n).text("message")
}

impl Operation {
    fn build(mutation: Mutation, target: Target, count: CountEstimate, effect: Effect) -> Self {
        Self {
            mutation,
            target,
            count,
            reversible: false,
            effect,
            lock_search: false,
            sequences: Vec::new(),
            matched_within: None,
            deferred_at_least: None,
        }
    }

    /// `n messages`, followed by the listed sequence numbers when there are any:
    /// `3 messages (sequence numbers 4, 9, 12)`, `30 messages (sequence numbers 1, ... 20 and
    /// 10 more)`.
    fn counted(&self, n: u64) -> String {
        format!("{}{}", messages(n), self.listed(n))
    }

    /// ` (sequence numbers 4, 9, 12)` for the listed sequence numbers of `n` messages; empty
    /// when none are listed.
    fn listed(&self, n: u64) -> String {
        if self.sequences.is_empty() {
            return String::new();
        }
        let mut list = self
            .sequences
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let more = n.saturating_sub(self.sequences.len() as u64);
        if more > 0 {
            list = format!("{list} and {} more", count_text(more));
        }
        let label = if n == 1 {
            "sequence number"
        } else {
            "sequence numbers"
        };
        format!(" ({label} {list})")
    }

    /// Send `n` messages, or schedule them for a time.
    pub fn send(target: Target, n: u64, schedule: Option<OffsetDateTime>) -> Self {
        Self::build(
            Mutation::Send,
            target,
            CountEstimate::Exact(n),
            Effect::Send { n, schedule },
        )
    }

    /// Cancel `n` scheduled messages.
    pub fn cancel_scheduled(target: Target, n: u64) -> Self {
        Self::build(
            Mutation::Delete,
            target,
            CountEstimate::Exact(n),
            Effect::CancelScheduled { n },
        )
    }

    /// Receive up to `n` messages and delete them.
    pub fn receive_and_delete(target: Target, n: u64) -> Self {
        Self::build(
            Mutation::Delete,
            target,
            CountEstimate::UpTo(n),
            Effect::ReceiveAndDelete { n },
        )
    }

    /// Receive up to `n` messages and settle them with `verb`.
    pub fn receive_and_settle(target: Target, n: u64, verb: SettleVerb) -> Self {
        Self::build(
            Mutation::Settle,
            target,
            CountEstimate::UpTo(n),
            Effect::ReceiveAndSettle { n, verb },
        )
    }

    /// Settle `n` deferred messages with `verb`.
    pub fn settle_deferred(target: Target, n: u64, verb: SettleVerb) -> Self {
        Self::build(
            Mutation::Settle,
            target,
            CountEstimate::Exact(n),
            Effect::SettleDeferred { n, verb },
        )
    }

    /// Delete the messages with these sequence numbers.
    pub fn delete(target: Target, sequence_numbers: &[i64]) -> Self {
        Self::build(
            Mutation::Delete,
            target,
            CountEstimate::Exact(sequence_numbers.len() as u64),
            Effect::Delete {
                n: sequence_numbers.len() as u64,
            },
        )
        .with_sequences(sequence_numbers)
    }

    /// Move `n` messages to the entity `target_name`: sent first, then removed.
    pub fn move_to(target: Target, target_name: &str, n: u64) -> Self {
        Self::build(
            Mutation::Delete,
            target,
            CountEstimate::Exact(n),
            Effect::Move {
                to: target_name.to_string(),
                n,
            },
        )
    }

    /// Resubmit `n` messages to the entity `target_name`, optionally removing the originals.
    pub fn resubmit(target: Target, target_name: &str, n: u64, remove_original: bool) -> Self {
        let mutation = if remove_original {
            Mutation::Delete
        } else {
            Mutation::Send
        };
        Self::build(
            mutation,
            target,
            CountEstimate::Exact(n),
            Effect::Resubmit {
                to: target_name.to_string(),
                n,
                remove_original,
            },
        )
    }

    /// Remove every message of the target.
    pub fn purge(target: Target, count: CountEstimate) -> Self {
        Self::build(Mutation::Purge, target, count, Effect::Purge)
    }

    /// Send `n` messages read from an import file.
    pub fn import(target: Target, n: u64) -> Self {
        Self::build(
            Mutation::Send,
            target,
            CountEstimate::Exact(n),
            Effect::Import { n },
        )
    }

    /// Replace (`true`) or clear (`false`) the state of a session.
    pub fn set_session_state(target: Target, session: &str, replace: bool) -> Self {
        Self::build(
            Mutation::SessionState,
            target,
            CountEstimate::Exact(1),
            Effect::SessionState {
                session: session.to_string(),
                replace,
            },
        )
    }

    /// An entity change on `primary` (the entity its activity entry names). The count is the
    /// number of entities it changes.
    fn entity_change(primary: &EntityRef, effect: Effect) -> Self {
        let mut op = Self::build(
            Mutation::EntityChange,
            Target::all(primary.entity_path()),
            CountEstimate::Unknown,
            effect,
        );
        op.count = CountEstimate::Exact(op.entities().len() as u64);
        op
    }

    /// Create a queue, topic or subscription (with its initial rule `rule`).
    pub fn create_entity(entity: &EntityRef, rule: Option<&str>) -> Self {
        Self::entity_change(
            entity,
            Effect::CreateEntity {
                entity: entity.clone(),
                rule: rule.map(str::to_string),
            },
        )
    }

    /// Replace the settings of an entity.
    pub fn update_entity(entity: &EntityRef) -> Self {
        Self::entity_change(
            entity,
            Effect::UpdateEntity {
                entity: entity.clone(),
            },
        )
    }

    /// Delete entities with what they hold. `targets` must not be empty.
    pub fn delete_entities(targets: Vec<DeleteTarget>) -> Self {
        let primary = targets
            .first()
            .map(|t| t.entity.clone())
            .expect("a delete names an entity");
        Self::entity_change(&primary, Effect::DeleteEntities { targets })
    }

    /// Change an entity's status.
    pub fn set_status(entity: &EntityRef, status: EntityStatus) -> Self {
        Self::entity_change(
            entity,
            Effect::SetStatus {
                entity: entity.clone(),
                status,
            },
        )
    }

    /// Create `to` with the settings of `from` (no messages).
    pub fn duplicate(from: &EntityRef, to: &EntityRef) -> Self {
        Self::entity_change(
            to,
            Effect::Duplicate {
                from: from.clone(),
                to: to.clone(),
            },
        )
    }

    /// Create `to` as a copy of `from`, then delete `from`.
    pub fn rename(from: &EntityRef, to: &EntityRef) -> Self {
        Self::entity_change(
            from,
            Effect::Rename {
                from: from.clone(),
                to: to.clone(),
            },
        )
    }

    /// Add rule `rule` to a subscription.
    pub fn add_rule(subscription: &EntityRef, rule: &str) -> Self {
        Self::entity_change(
            subscription,
            Effect::AddRule {
                subscription: subscription.clone(),
                rule: rule.to_string(),
            },
        )
    }

    /// Remove rule `rule` from a subscription.
    pub fn remove_rule(subscription: &EntityRef, rule: &str) -> Self {
        Self::entity_change(
            subscription,
            Effect::RemoveRule {
                subscription: subscription.clone(),
                rule: rule.to_string(),
            },
        )
    }

    /// Add SAS policy `name` with `rights`.
    pub fn add_policy(entity: &EntityRef, name: &str, rights: &[AccessRight]) -> Self {
        Self::entity_change(
            entity,
            Effect::AddPolicy {
                entity: entity.clone(),
                name: name.to_string(),
                rights: rights.to_vec(),
            },
        )
    }

    /// Remove SAS policy `name`.
    pub fn remove_policy(entity: &EntityRef, name: &str) -> Self {
        Self::entity_change(
            entity,
            Effect::RemovePolicy {
                entity: entity.clone(),
                name: name.to_string(),
            },
        )
    }

    /// Replace one key of SAS policy `name`.
    pub fn regenerate_key(entity: &EntityRef, name: &str, slot: KeySlot) -> Self {
        Self::entity_change(
            entity,
            Effect::RegenerateKey {
                entity: entity.clone(),
                name: name.to_string(),
                slot,
            },
        )
    }

    /// Create (or with `overwrite` replace) the entities of an import file. `entities` must
    /// not be empty.
    pub fn import_entities(entities: Vec<EntityRef>, overwrite: bool) -> Self {
        let primary = entities
            .first()
            .cloned()
            .expect("an import names an entity");
        Self::entity_change(
            &primary,
            Effect::ImportEntities {
                entities,
                overwrite,
            },
        )
    }

    /// List the selection's sequence numbers in the sentence: the first 20, then `and N more`
    /// (none listed for an empty slice). Delete, cancel scheduled, move and resubmit use it.
    pub fn with_sequences(mut self, sequence_numbers: &[i64]) -> Self {
        self.sequences = sequence_numbers
            .iter()
            .copied()
            .take(SEQUENCES_SHOWN)
            .collect();
        self
    }

    /// Say that the selection was matched only in the first `examined` messages of the view
    /// (a budget or the limit stopped the scan), so more may match.
    pub fn matched_within(mut self, examined: u64) -> Self {
        self.matched_within = Some(examined);
        self
    }

    /// Say that the purge's deferred count is a lower bound (its peek stopped at the cap).
    pub fn deferred_at_least(mut self, n: u64) -> Self {
        self.deferred_at_least = Some(n);
        self
    }

    /// Mark the operation as finding its messages with a peek-lock search: the sentence then says
    /// that examined messages stay locked while it runs.
    pub fn searched(mut self) -> Self {
        self.lock_search = true;
        self
    }

    /// Short verb for confirmations and the audit log.
    pub fn verb(&self) -> &'static str {
        match &self.effect {
            Effect::Send { schedule: None, .. } => "send",
            Effect::Send { .. } => "schedule",
            Effect::CancelScheduled { .. } => "cancel-scheduled",
            Effect::ReceiveAndDelete { .. } => "receive-and-delete",
            Effect::ReceiveAndSettle { verb, .. } | Effect::SettleDeferred { verb, .. } => {
                verb.text()
            }
            Effect::Delete { .. } => "delete",
            Effect::Move { .. } => "move",
            Effect::Resubmit { .. } => "resubmit",
            Effect::Purge => "purge",
            Effect::Import { .. } => "import",
            Effect::SessionState { replace: true, .. } => "set-session-state",
            Effect::SessionState { .. } => "clear-session-state",
            Effect::CreateEntity { .. } => "create-entity",
            Effect::UpdateEntity { .. } => "update-entity",
            Effect::DeleteEntities { .. } => "delete-entity",
            Effect::SetStatus { .. } => "set-status",
            Effect::Duplicate { .. } => "duplicate",
            Effect::Rename { .. } => "rename",
            Effect::AddRule { .. } => "add-rule",
            Effect::RemoveRule { .. } => "remove-rule",
            Effect::AddPolicy { .. } => "add-policy",
            Effect::RemovePolicy { .. } => "remove-policy",
            Effect::RegenerateKey { .. } => "regenerate-key",
            Effect::ImportEntities { .. } => "import-entities",
        }
    }

    /// The exact sentence naming the effect.
    pub fn summary(&self, profile: &ProfileRef) -> String {
        let at = self.target.address_text();
        let on = &profile.name;
        let mut text = match &self.effect {
            Effect::Send { n, schedule: None } => {
                format!("Send {} to {at} on {on}", messages(*n))
            }
            Effect::Send {
                n,
                schedule: Some(when),
            } => format!(
                "Schedule {} to {at} for {} on {on}",
                messages(*n),
                rfc3339(*when)
            ),
            Effect::CancelScheduled { n } => {
                format!(
                    "Cancel {} scheduled {}{} in {at} on {on}",
                    count_text(*n),
                    plural_word(*n),
                    self.listed(*n)
                )
            }
            Effect::ReceiveAndDelete { n } => format!(
                "Remove up to {} from {at} on {on} (receive and delete)",
                messages(*n)
            ),
            Effect::ReceiveAndSettle { n, verb } => {
                let tail = match verb {
                    SettleVerb::Complete => "complete them (removes them)",
                    SettleVerb::Abandon => "abandon them",
                    SettleVerb::Defer => "defer them",
                    SettleVerb::DeadLetter => "dead-letter them",
                };
                format!(
                    "Receive up to {} from {at} and {tail} on {on}",
                    messages(*n)
                )
            }
            Effect::SettleDeferred { n, verb } => format!(
                "{} {} deferred {} in {at} on {on}",
                verb.capitalised(),
                count_text(*n),
                plural_word(*n)
            ),
            Effect::Delete { n } => {
                format!("Delete {} from {at} on {on}", self.counted(*n))
            }
            Effect::Move { to, n } => format!(
                "Move {} from {at} to {to} on {on} (sent first, then removed: a message may arrive twice)",
                self.counted(*n)
            ),
            Effect::Resubmit {
                to,
                n,
                remove_original,
            } => format!(
                "Resubmit {} from {at} to {to} on {on}{}",
                self.counted(*n),
                if *remove_original {
                    ", removing the originals"
                } else {
                    ""
                }
            ),
            Effect::Purge => {
                let whole = self.target.sub_queue.is_none();
                let place = if whole {
                    format!("{at} (active, deferred and dead-lettered)")
                } else {
                    at.clone()
                };
                let mut s = format!("Remove {} from {place} on {on}", self.count.text("message"));
                if let CountEstimate::AtLeast(n) = self.count {
                    s.push_str(&format!(
                        " (at least {} are there; run purge again for the rest)",
                        count_text(n)
                    ));
                }
                if let Some(n) = self.deferred_at_least {
                    s.push_str(&format!(
                        "; deferred messages: at least {} (the count stopped at its cap)",
                        count_text(n)
                    ));
                }
                if whole {
                    s.push_str(
                        ". Scheduled messages are kept; cancel them with `scheduled cancel --all`",
                    );
                }
                s
            }
            Effect::Import { n } => {
                format!(
                    "Send {} imported {} to {at} on {on}",
                    count_text(*n),
                    plural_word(*n)
                )
            }
            Effect::SessionState { session, replace } => format!(
                "{} the state of session {session} in {at} on {on}",
                if *replace { "Replace" } else { "Clear" }
            ),
            Effect::CreateEntity { entity, rule } => match rule {
                Some(rule) => format!("Create {entity} with rule {rule} on {on}"),
                None => format!("Create {entity} on {on}"),
            },
            Effect::UpdateEntity { entity } => format!("Update {entity} on {on}"),
            Effect::DeleteEntities { targets } => match targets.as_slice() {
                [one] => format!("Delete {} on {on}", one.text()),
                many => format!(
                    "Delete {} entities on {on}: {}",
                    count_text(many.len() as u64),
                    many.iter()
                        .map(DeleteTarget::text)
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
            Effect::SetStatus { entity, status } => {
                format!("Set {entity} to {status} on {on}")
            }
            Effect::Duplicate { from, to } => {
                let what = match from.kind() {
                    EntityKind::Subscription => "settings and rules",
                    _ => "settings and policies, with new keys",
                };
                format!(
                    "Duplicate {from} as {} on {on} ({what}; messages are not copied)",
                    to.path()
                )
            }
            Effect::Rename { from, to } => {
                let with = if from.kind() == EntityKind::Topic {
                    " with its subscriptions and rules"
                } else {
                    ""
                };
                format!(
                    "Rename {from} to {}{with} (create {}, then delete {}) on {on}",
                    to.path(),
                    to.path(),
                    from.path()
                )
            }
            Effect::AddRule { subscription, rule } => {
                format!("Add rule {rule} to {} on {on}", subscription.path())
            }
            Effect::RemoveRule { subscription, rule } => {
                format!("Remove rule {rule} from {} on {on}", subscription.path())
            }
            Effect::AddPolicy {
                entity,
                name,
                rights,
            } => format!(
                "Add policy {name} ({}) to {} on {on}",
                rights
                    .iter()
                    .map(|r| r.as_str().to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(", "),
                entity.path()
            ),
            Effect::RemovePolicy { entity, name } => {
                format!("Remove policy {name} from {} on {on}", entity.path())
            }
            Effect::RegenerateKey { entity, name, slot } => format!(
                "Regenerate the {} key of policy {name} on {} on {on}",
                slot.as_str(),
                entity.path()
            ),
            Effect::ImportEntities {
                entities,
                overwrite,
            } => format!(
                "Import {} {} on {on}: {}; {}",
                count_text(entities.len() as u64),
                if entities.len() == 1 {
                    "entity"
                } else {
                    "entities"
                },
                entities
                    .iter()
                    .map(EntityRef::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                if *overwrite {
                    "existing ones are replaced"
                } else {
                    "existing ones are skipped"
                }
            ),
        };
        if let Some(examined) = self.matched_within {
            text.push_str(&format!(
                "; matched {} in the first {} examined; run again for the rest",
                count_text(self.count.bound().unwrap_or(0)),
                count_text(examined)
            ));
        }
        if self.lock_search {
            text.push_str(". Messages examined during the search are locked while it runs");
        }
        text
    }

    /// Every entity the operation changes, each named once: the source, and for a move or a
    /// resubmit the entity the copies go to.
    pub fn entities(&self) -> Vec<String> {
        let changed: Option<Vec<&EntityRef>> = match &self.effect {
            Effect::CreateEntity { entity, .. }
            | Effect::UpdateEntity { entity }
            | Effect::SetStatus { entity, .. }
            | Effect::AddPolicy { entity, .. }
            | Effect::RemovePolicy { entity, .. }
            | Effect::RegenerateKey { entity, .. } => Some(vec![entity]),
            Effect::AddRule { subscription, .. } | Effect::RemoveRule { subscription, .. } => {
                Some(vec![subscription])
            }
            Effect::DeleteEntities { targets } => Some(targets.iter().map(|t| &t.entity).collect()),
            Effect::Duplicate { to, .. } => Some(vec![to]),
            Effect::Rename { from, to } => Some(vec![from, to]),
            Effect::ImportEntities { entities, .. } => Some(entities.iter().collect()),
            _ => None,
        };
        if let Some(changed) = changed {
            let mut out: Vec<String> = Vec::new();
            for e in changed {
                let path = e.path();
                if !out.contains(&path) {
                    out.push(path);
                }
            }
            return out;
        }
        let mut out = vec![self.target.entity.base()];
        if let Effect::Move { to, .. } | Effect::Resubmit { to, .. } = &self.effect
            && !out.contains(to)
        {
            out.push(to.clone());
        }
        out
    }

    /// The requests a policy confirms, in order. Off production, one: the source. On
    /// production, one per entity in [`Operation::entities`] (the move or resubmit target
    /// included), each naming that entity as its target with the same sentence, so the typed
    /// confirmation [`require`] enforces covers every production entity the operation changes.
    pub fn confirm_requests(&self, actor: &Actor) -> Vec<ConfirmRequest> {
        let first = self.confirm_request(actor);
        if !actor.profile.production {
            return vec![first];
        }
        let mut out = vec![first];
        for entity in self.entities().into_iter().skip(1) {
            out.push(ConfirmRequest {
                target: entity,
                ..out[0].clone()
            });
        }
        out
    }

    /// The request a policy confirms: the entity base as target, the exact sentence as summary.
    pub fn confirm_request(&self, actor: &Actor) -> ConfirmRequest {
        ConfirmRequest {
            verb: self.verb().to_string(),
            target: self.target.entity.base(),
            summary: self.summary(&actor.profile),
            item_count: self.count.bound(),
            reversible: self.reversible,
            production: actor.profile.production,
            origin: actor.origin,
        }
    }
}

fn plural_word(n: u64) -> &'static str {
    if n == 1 { "message" } else { "messages" }
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_else(|_| t.to_string())
}

/// What an operation's result tells the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditFacts {
    /// How the operation ended.
    pub outcome: Outcome,
    /// Messages that worked.
    pub succeeded: Option<u64>,
    /// Messages that failed.
    pub failed: Option<u64>,
    /// Messages skipped.
    pub skipped: Option<u64>,
    /// A remark for the finished entry (stored in its `note` field; never an error).
    pub note: Option<String>,
}

/// A result the gate can write to the activity log.
pub trait Audited {
    /// The facts for the finished entry.
    fn audit_outcome(&self) -> AuditFacts;

    /// Receives the activity op id before the result is returned, so a report can name it.
    /// Override this when the type carries a report; the default drops the id.
    fn set_op_id(&mut self, _op_id: String) {}
}

impl Audited for () {
    fn audit_outcome(&self) -> AuditFacts {
        AuditFacts {
            outcome: Outcome::Succeeded,
            succeeded: None,
            failed: None,
            skipped: None,
            note: None,
        }
    }
}

impl Audited for JobReport {
    fn audit_outcome(&self) -> AuditFacts {
        AuditFacts {
            outcome: self.outcome(),
            succeeded: Some(self.succeeded()),
            failed: Some(self.failed()),
            skipped: Some(self.skipped()),
            note: if self.notes.is_empty() {
                None
            } else {
                Some(self.notes.join("; "))
            },
        }
    }

    fn set_op_id(&mut self, op_id: String) {
        self.op_id = Some(op_id);
    }
}

/// Writes a Finished entry if the gate's future is dropped or its work panics after Started.
struct FinishGuard {
    audit: Arc<dyn Audit>,
    entry: ActivityEntry,
    armed: bool,
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Never panic in drop: a failing or panicking audit is only logged.
        let audit = &self.audit;
        let entry = &self.entry;
        let written =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audit.record(entry)));
        if !matches!(written, Ok(Ok(_))) {
            tracing::warn!("could not write the interrupted entry to the activity log");
        }
    }
}

/// The only way to a mutating broker call: confirm, audit before, run, audit after.
#[derive(Clone)]
pub struct Gate {
    policy: Arc<dyn Policy>,
    audit: Arc<dyn Audit>,
    actor: Actor,
}

impl Gate {
    /// A gate that asks `policy`, writes to `audit` and acts as `actor`.
    pub fn new(policy: Arc<dyn Policy>, audit: Arc<dyn Audit>, actor: Actor) -> Self {
        Self {
            policy,
            audit,
            actor,
        }
    }

    /// Who this gate acts as.
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    fn entry(
        &self,
        op: &Operation,
        op_id: &str,
        phase: Phase,
        outcome: Outcome,
        summary: &str,
    ) -> ActivityEntry {
        ActivityEntry {
            id: 0,
            op_id: op_id.to_string(),
            at: OffsetDateTime::now_utc(),
            origin: self.actor.origin,
            profile: self.actor.profile.name.clone(),
            entity: Some(op.target.entity.base()),
            action: op.verb().to_string(),
            kind: OpKind::Mutating(op.mutation),
            phase,
            outcome,
            count: op.count.bound(),
            succeeded: None,
            failed: None,
            skipped: None,
            summary: summary.to_string(),
            error: None,
            note: None,
        }
    }

    /// Confirm `op`, write the started entry, run `work` (given the op id), write the finished
    /// entry. A refused confirmation or a failed started write means `work` never runs.
    pub async fn run<T, F, Fut>(&self, op: &Operation, work: F) -> Result<T>
    where
        T: Audited,
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.run_logged(op, work).await.1
    }

    /// [`Gate::run`], also returning the op id when an activity entry was written under it (a
    /// denial included), so a caller can name the entry even when the operation was refused.
    pub(crate) async fn run_logged<T, F, Fut>(
        &self,
        op: &Operation,
        work: F,
    ) -> (Option<String>, Result<T>)
    where
        T: Audited,
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let op_id = format!("op-{}", uuid::Uuid::new_v4().simple());
        let requests = op.confirm_requests(&self.actor);
        let summary = requests[0].summary.clone();

        let mut confirmed = Ok(());
        for request in &requests {
            confirmed = require(self.policy.as_ref(), request).await;
            if confirmed.is_err() {
                break;
            }
        }
        if let Err(denied) = confirmed {
            let error: Error = denied.into();
            let mut entry = self.entry(op, &op_id, Phase::Finished, Outcome::Denied, &summary);
            entry.error = Some(error.to_string());
            let logged = match self.audit.record(&entry) {
                Ok(_) => Some(op_id),
                Err(e) => {
                    tracing::warn!("could not write the denial to the activity log: {e}");
                    None
                }
            };
            return (logged, Err(error));
        }

        let started = self.entry(op, &op_id, Phase::Started, Outcome::Pending, &summary);
        if let Err(e) = self.audit.record(&started) {
            return (
                None,
                Err(Error::Io(format!(
                    "cannot write the activity log ({e}); nothing was changed"
                ))),
            );
        }

        let mut guard = FinishGuard {
            audit: self.audit.clone(),
            entry: {
                let mut e = self.entry(op, &op_id, Phase::Finished, Outcome::Cancelled, &summary);
                e.error = Some("interrupted before it finished".into());
                e
            },
            armed: true,
        };
        let result = work(op_id.clone()).await;
        guard.armed = false;

        let finished = match &result {
            Ok(value) => {
                let facts = value.audit_outcome();
                let mut entry = self.entry(op, &op_id, Phase::Finished, facts.outcome, &summary);
                entry.succeeded = facts.succeeded;
                entry.failed = facts.failed;
                entry.skipped = facts.skipped;
                entry.note = facts.note;
                entry
            }
            Err(e) => {
                let outcome = if matches!(e, Error::Cancelled) {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                };
                let mut entry = self.entry(op, &op_id, Phase::Finished, outcome, &summary);
                entry.error = Some(e.to_string());
                entry
            }
        };
        if let Err(e) = self.audit.record(&finished) {
            tracing::warn!("could not write the finished entry to the activity log: {e}");
        }
        (Some(op_id), result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn prod() -> ProfileRef {
        ProfileRef {
            name: "prod-weu".into(),
            production: true,
        }
    }
    fn dlq(entity: &str) -> Target {
        Target::new(EntityPath::queue(entity), SubQueue::DeadLetter)
    }

    #[test]
    fn purge_text_matches_the_spec_example() {
        let op = Operation::purge(dlq("orders"), CountEstimate::Exact(1284));
        assert_eq!(
            op.summary(&prod()),
            "Remove 1,284 messages from orders/$DeadLetterQueue on prod-weu"
        );
        assert_eq!(op.verb(), "purge");
        assert_eq!(op.mutation, Mutation::Purge);
    }

    #[test]
    fn every_effect_names_count_entity_and_profile() {
        let p = prod();
        let q = || Target::new(EntityPath::queue("orders"), SubQueue::Active);
        let cases = [
            (
                Operation::send(q(), 3, None),
                "Send 3 messages to orders on prod-weu",
            ),
            (
                Operation::send(q(), 1, Some(datetime!(2026-10-18 09:00 UTC))),
                "Schedule 1 message to orders for 2026-10-18T09:00:00Z on prod-weu",
            ),
            (
                Operation::cancel_scheduled(q(), 4),
                "Cancel 4 scheduled messages in orders on prod-weu",
            ),
            (
                Operation::receive_and_delete(q(), 10),
                "Remove up to 10 messages from orders on prod-weu (receive and delete)",
            ),
            (
                Operation::receive_and_settle(q(), 10, SettleVerb::Complete),
                "Receive up to 10 messages from orders and complete them (removes them) on prod-weu",
            ),
            (
                Operation::receive_and_settle(q(), 2, SettleVerb::DeadLetter),
                "Receive up to 2 messages from orders and dead-letter them on prod-weu",
            ),
            (
                Operation::delete(dlq("orders"), &[4, 9, 12]),
                "Delete 3 messages (sequence numbers 4, 9, 12) from orders/$DeadLetterQueue on prod-weu",
            ),
            (
                Operation::move_to(dlq("orders"), "orders", 3),
                "Move 3 messages from orders/$DeadLetterQueue to orders on prod-weu (sent first, then removed: a message may arrive twice)",
            ),
            (
                Operation::resubmit(dlq("orders"), "orders", 3, false),
                "Resubmit 3 messages from orders/$DeadLetterQueue to orders on prod-weu",
            ),
            (
                Operation::resubmit(dlq("orders"), "orders", 3, true),
                "Resubmit 3 messages from orders/$DeadLetterQueue to orders on prod-weu, removing the originals",
            ),
            (
                Operation::set_session_state(q(), "RMA-1", true),
                "Replace the state of session RMA-1 in orders on prod-weu",
            ),
            (
                Operation::set_session_state(q(), "RMA-1", false),
                "Clear the state of session RMA-1 in orders on prod-weu",
            ),
            (
                Operation::settle_deferred(q(), 2, SettleVerb::DeadLetter),
                "Dead-letter 2 deferred messages in orders on prod-weu",
            ),
            (
                Operation::import(q(), 12),
                "Send 12 imported messages to orders on prod-weu",
            ),
        ];
        for (op, text) in cases {
            assert_eq!(op.summary(&p), text);
        }
    }

    #[test]
    fn large_counts_use_thousands_separators() {
        let p = prod();
        let q = || Target::new(EntityPath::queue("orders"), SubQueue::Active);
        assert_eq!(
            Operation::cancel_scheduled(q(), 1000).summary(&p),
            "Cancel 1,000 scheduled messages in orders on prod-weu"
        );
        assert_eq!(
            Operation::settle_deferred(q(), 1234, SettleVerb::Abandon).summary(&p),
            "Abandon 1,234 deferred messages in orders on prod-weu"
        );
        assert_eq!(
            Operation::import(q(), 2500).summary(&p),
            "Send 2,500 imported messages to orders on prod-weu"
        );
        assert_eq!(
            Operation::purge(dlq("orders"), CountEstimate::Unknown).summary(&p),
            "Remove an unknown number of messages from orders/$DeadLetterQueue on prod-weu"
        );
    }

    #[test]
    fn estimated_counts_say_so() {
        let t = || dlq("orders");
        assert_eq!(
            Operation::purge(t(), CountEstimate::About(40)).summary(&prod()),
            "Remove about 40 messages from orders/$DeadLetterQueue on prod-weu"
        );
        assert_eq!(
            Operation::purge(t(), CountEstimate::AtLeast(10_000)).summary(&prod()),
            "Remove up to 10,000 messages from orders/$DeadLetterQueue on prod-weu (at least 10,000 are there; run purge again for the rest)"
        );
        assert_eq!(
            Operation::delete(t(), &(1..=30).collect::<Vec<_>>()).summary(&prod()),
            "Delete 30 messages (sequence numbers 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20 and 10 more) from orders/$DeadLetterQueue on prod-weu"
        );
    }

    #[test]
    fn move_and_resubmit_list_up_to_20_sequence_numbers() {
        let t = || dlq("orders");
        let mv = |seqs: &[i64]| {
            Operation::move_to(t(), "orders", seqs.len() as u64)
                .with_sequences(seqs)
                .summary(&prod())
        };
        assert_eq!(
            mv(&[]),
            "Move 0 messages from orders/$DeadLetterQueue to orders on prod-weu (sent first, then removed: a message may arrive twice)"
        );
        assert_eq!(
            mv(&[7]),
            "Move 1 message (sequence number 7) from orders/$DeadLetterQueue to orders on prod-weu (sent first, then removed: a message may arrive twice)"
        );
        let twenty: Vec<i64> = (1..=20).collect();
        let listed = (1..=20)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(
            mv(&twenty),
            format!(
                "Move 20 messages (sequence numbers {listed}) from orders/$DeadLetterQueue to orders on prod-weu (sent first, then removed: a message may arrive twice)"
            )
        );
        let many: Vec<i64> = (1..=1_021).collect();
        assert_eq!(
            Operation::resubmit(t(), "orders", 1_021, true)
                .with_sequences(&many)
                .summary(&prod()),
            format!(
                "Resubmit 1,021 messages (sequence numbers {listed} and 1,001 more) from orders/$DeadLetterQueue to orders on prod-weu, removing the originals"
            )
        );
        assert_eq!(
            Operation::delete(t(), &(1..=21).collect::<Vec<_>>()).summary(&prod()),
            format!(
                "Delete 21 messages (sequence numbers {listed} and 1 more) from orders/$DeadLetterQueue on prod-weu"
            )
        );
    }

    #[test]
    fn a_capped_deferred_count_says_at_least() {
        let op = Operation::purge(
            Target::all(EntityPath::queue("orders")),
            CountEstimate::AtLeast(3),
        )
        .deferred_at_least(0);
        assert_eq!(
            op.summary(&prod()),
            "Remove up to 3 messages from orders (active, deferred and dead-lettered) on prod-weu (at least 3 are there; run purge again for the rest); deferred messages: at least 0 (the count stopped at its cap). Scheduled messages are kept; cancel them with `scheduled cancel --all`"
        );
    }

    #[test]
    fn cancel_scheduled_lists_its_sequence_numbers() {
        let q = Target::new(EntityPath::queue("orders"), SubQueue::Active);
        assert_eq!(
            Operation::cancel_scheduled(q.clone(), 2)
                .with_sequences(&[4, 9])
                .summary(&prod()),
            "Cancel 2 scheduled messages (sequence numbers 4, 9) in orders on prod-weu"
        );
        let many: Vec<i64> = (1..=25).collect();
        assert!(
            Operation::cancel_scheduled(q, 25)
                .with_sequences(&many)
                .summary(&prod())
                .starts_with("Cancel 25 scheduled messages (sequence numbers 1, 2, ")
        );
    }

    #[test]
    fn a_partial_match_says_how_far_it_looked() {
        assert_eq!(
            Operation::delete(dlq("orders"), &[4, 9])
                .matched_within(10_000)
                .searched()
                .summary(&prod()),
            "Delete 2 messages (sequence numbers 4, 9) from orders/$DeadLetterQueue on prod-weu; matched 2 in the first 10,000 examined; run again for the rest. Messages examined during the search are locked while it runs"
        );
    }

    #[test]
    fn purge_all_mentions_scheduled_messages_are_kept() {
        let op = Operation::purge(
            Target::all(EntityPath::queue("orders")),
            CountEstimate::Exact(7),
        );
        assert_eq!(
            op.summary(&prod()),
            "Remove 7 messages from orders (active, deferred and dead-lettered) on prod-weu. Scheduled messages are kept; cancel them with `scheduled cancel --all`"
        );
    }

    #[test]
    fn confirm_request_targets_the_entity_base_and_carries_production() {
        let actor = Actor {
            origin: Origin::Cli,
            profile: prod(),
        };
        let op = Operation::purge(
            Target::new(
                EntityPath::subscription("events", "all"),
                SubQueue::DeadLetter,
            ),
            CountEstimate::Exact(2),
        );
        let r = op.confirm_request(&actor);
        assert_eq!(r.target, "events/Subscriptions/all");
        assert_eq!(r.verb, "purge");
        assert_eq!(r.item_count, Some(2));
        assert!(r.production && !r.reversible && r.origin == Origin::Cli);
        assert!(
            r.summary
                .contains("events/Subscriptions/all/$DeadLetterQueue")
        );
    }

    #[test]
    fn lock_searches_say_that_examined_messages_are_locked() {
        let op = Operation::delete(dlq("orders"), &[4, 9]).searched();
        assert_eq!(
            op.summary(&prod()),
            "Delete 2 messages (sequence numbers 4, 9) from orders/$DeadLetterQueue on prod-weu. Messages examined during the search are locked while it runs"
        );
        assert_eq!(
            Operation::move_to(dlq("orders"), "orders", 3)
                .searched()
                .summary(&prod()),
            "Move 3 messages from orders/$DeadLetterQueue to orders on prod-weu (sent first, then removed: a message may arrive twice). Messages examined during the search are locked while it runs"
        );
    }

    #[test]
    fn no_broker_operation_is_reversible() {
        // The field exists for the kit; nothing a broker op does can be undone by Dispatched.
        let q = Target::new(EntityPath::queue("q"), SubQueue::Active);
        assert!(!Operation::send(q.clone(), 1, None).reversible);
        assert!(!Operation::delete(q.clone(), &[1]).reversible);
        assert!(
            !Operation::purge(Target::all(EntityPath::queue("q")), CountEstimate::Exact(1))
                .reversible
        );
        assert!(!Operation::set_session_state(q, "s", false).reversible);
    }

    #[test]
    fn mutations_follow_the_spec() {
        let q = || Target::new(EntityPath::queue("q"), SubQueue::Active);
        assert_eq!(Operation::send(q(), 1, None).mutation, Mutation::Send);
        assert_eq!(Operation::import(q(), 1).mutation, Mutation::Send);
        assert_eq!(
            Operation::cancel_scheduled(q(), 1).mutation,
            Mutation::Delete
        );
        assert_eq!(
            Operation::receive_and_delete(q(), 1).mutation,
            Mutation::Delete
        );
        assert_eq!(
            Operation::receive_and_settle(q(), 1, SettleVerb::Abandon).mutation,
            Mutation::Settle
        );
        assert_eq!(
            Operation::resubmit(q(), "x", 1, false).mutation,
            Mutation::Send
        );
        assert_eq!(
            Operation::resubmit(q(), "x", 1, true).mutation,
            Mutation::Delete
        );
        assert_eq!(Operation::move_to(q(), "x", 1).mutation, Mutation::Delete);
        assert_eq!(
            serde_json::to_string(&Mutation::SessionState).unwrap(),
            "\"session-state\""
        );
    }

    fn dev() -> ProfileRef {
        ProfileRef {
            name: "dev".into(),
            production: false,
        }
    }

    #[test]
    fn effects_name_every_entity_they_touch() {
        use crate::mgmt::{AccessRight, EntityRef, EntityStatus, KeySlot};
        let q = |n: &str| EntityRef::queue(n);
        let sub = EntityRef::subscription("events", "billing");
        let rename = Operation::rename(&q("a"), &q("b"));
        assert_eq!(rename.entities(), ["a", "b"]);
        assert_eq!(
            rename.summary(&dev()),
            "Rename queue a to b (create b, then delete a) on dev"
        );
        let bulk = Operation::delete_entities(vec![
            DeleteTarget {
                entity: q("orders"),
                messages: CountEstimate::Exact(1284),
                subscriptions: None,
            },
            DeleteTarget {
                entity: EntityRef::topic("events"),
                messages: CountEstimate::Exact(1204),
                subscriptions: Some(3),
            },
            DeleteTarget {
                entity: sub.clone(),
                messages: CountEstimate::Unknown,
                subscriptions: None,
            },
        ]);
        assert_eq!(
            bulk.entities(),
            ["orders", "events", "events/Subscriptions/billing"]
        );
        assert_eq!(
            bulk.summary(&prod()),
            "Delete 3 entities on prod-weu: queue orders and its 1,284 messages; topic events, \
             its 3 subscriptions and their 1,204 messages; subscription \
             events/Subscriptions/billing and its messages (count unknown)"
        );
        assert_eq!(bulk.mutation, Mutation::EntityChange);
        assert_eq!(bulk.verb(), "delete-entity");
        let one = Operation::delete_entities(vec![DeleteTarget {
            entity: q("orders"),
            messages: CountEstimate::Exact(1284),
            subscriptions: None,
        }]);
        assert_eq!(
            one.summary(&prod()),
            "Delete queue orders and its 1,284 messages on prod-weu"
        );
        let cases = [
            (
                Operation::create_entity(&q("orders"), None),
                "Create queue orders on dev",
                vec!["orders"],
            ),
            (
                Operation::create_entity(&sub, Some("eu")),
                "Create subscription events/Subscriptions/billing with rule eu on dev",
                vec!["events/Subscriptions/billing"],
            ),
            (
                Operation::update_entity(&q("orders")),
                "Update queue orders on dev",
                vec!["orders"],
            ),
            (
                Operation::set_status(&q("orders"), EntityStatus::SendDisabled),
                "Set queue orders to SendDisabled on dev",
                vec!["orders"],
            ),
            (
                Operation::duplicate(&q("a"), &q("b")),
                "Duplicate queue a as b on dev (settings and policies, with new keys; messages are not copied)",
                vec!["b"],
            ),
            (
                Operation::rename(&EntityRef::topic("a"), &EntityRef::topic("b")),
                "Rename topic a to b with its subscriptions and rules (create b, then delete a) on dev",
                vec!["a", "b"],
            ),
            (
                Operation::add_rule(&sub, "eu"),
                "Add rule eu to events/Subscriptions/billing on dev",
                vec!["events/Subscriptions/billing"],
            ),
            (
                Operation::remove_rule(&sub, "eu"),
                "Remove rule eu from events/Subscriptions/billing on dev",
                vec!["events/Subscriptions/billing"],
            ),
            (
                Operation::add_policy(
                    &q("orders"),
                    "app",
                    &[AccessRight::Listen, AccessRight::Send],
                ),
                "Add policy app (listen, send) to orders on dev",
                vec!["orders"],
            ),
            (
                Operation::remove_policy(&q("orders"), "app"),
                "Remove policy app from orders on dev",
                vec!["orders"],
            ),
            (
                Operation::regenerate_key(&q("orders"), "app", KeySlot::Primary),
                "Regenerate the primary key of policy app on orders on dev",
                vec!["orders"],
            ),
        ];
        for (op, text, entities) in cases {
            assert_eq!(op.summary(&dev()), text);
            assert_eq!(op.entities(), entities, "{text}");
            assert_eq!(op.mutation, Mutation::EntityChange, "{text}");
            assert!(!op.reversible);
        }
    }

    #[tokio::test]
    async fn on_production_a_rename_confirms_source_and_target() {
        use crate::mgmt::EntityRef;
        let policy = Arc::new(TypedFor {
            allow: vec!["a".into()],
            seen: Default::default(),
        });
        let g = Gate::new(
            policy.clone(),
            Arc::new(MemoryAudit::new()),
            Actor {
                origin: Origin::Cli,
                profile: prod(),
            },
        );
        let op = Operation::rename(&EntityRef::queue("a"), &EntityRef::queue("b"));
        assert!(g.run(&op, |_| async { Ok(()) }).await.is_err());
        assert_eq!(policy.seen.lock().unwrap().clone(), ["a", "b"]);
    }

    #[test]
    fn counts_are_grouped_and_bounded() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(999), "999");
        assert_eq!(count_text(1284), "1,284");
        assert_eq!(count_text(1_000_000), "1,000,000");
        assert_eq!(CountEstimate::Unknown.bound(), None);
        assert_eq!(CountEstimate::AtLeast(5).bound(), Some(5));
        assert_eq!(CountEstimate::Exact(1).text("message"), "1 message");
    }

    use crate::activity::MemoryAudit;
    use crate::jobs::{ItemOutcome, ItemResult, JobKind};
    use async_trait::async_trait;
    use horizon_azure_kit::safety::Decision;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    struct Scripted(Decision);
    #[async_trait]
    impl Policy for Scripted {
        async fn confirm(&self, _: &ConfirmRequest) -> Decision {
            self.0.clone()
        }
    }

    fn gate(decision: Decision, production: bool) -> (Gate, Arc<MemoryAudit>) {
        let audit = Arc::new(MemoryAudit::new());
        let g = Gate::new(
            Arc::new(Scripted(decision)),
            audit.clone(),
            Actor {
                origin: Origin::Cli,
                profile: ProfileRef {
                    name: "prod-weu".into(),
                    production,
                },
            },
        );
        (g, audit)
    }

    /// Records each request and confirms only the typed `allow` targets.
    struct TypedFor {
        allow: Vec<String>,
        seen: std::sync::Mutex<Vec<String>>,
    }
    #[async_trait]
    impl Policy for TypedFor {
        async fn confirm(&self, req: &ConfirmRequest) -> Decision {
            self.seen.lock().unwrap().push(req.target.clone());
            if self.allow.contains(&req.target) {
                Decision::ConfirmedTyped(req.target.clone())
            } else {
                Decision::Deny(format!("{} not confirmed", req.target))
            }
        }
    }

    #[tokio::test]
    async fn on_production_the_gate_confirms_every_entity_the_operation_touches() {
        let mv = Operation::move_to(dlq("orders"), "archive", 2);
        assert_eq!(mv.entities(), ["orders", "archive"]);
        assert_eq!(
            Operation::resubmit(dlq("orders"), "orders", 1, false).entities(),
            ["orders"],
            "an entity is named once"
        );
        assert_eq!(purge_op().entities(), ["orders"]);
        let run = |allow: &[&str], production: bool| {
            let policy = Arc::new(TypedFor {
                allow: allow.iter().map(|s| s.to_string()).collect(),
                seen: Default::default(),
            });
            let g = Gate::new(
                policy.clone(),
                Arc::new(MemoryAudit::new()),
                Actor {
                    origin: Origin::Cli,
                    profile: ProfileRef {
                        name: "prod-weu".into(),
                        production,
                    },
                },
            );
            let op = mv.clone();
            async move {
                let ran = g.run(&op, |_| async { Ok(()) }).await.is_ok();
                let seen = policy.seen.lock().unwrap().clone();
                (ran, seen)
            }
        };
        // Only the source named: the move target is refused, nothing runs.
        let (ran, seen) = run(&["orders"], true).await;
        assert!(!ran);
        assert_eq!(seen, ["orders", "archive"]);
        let (ran, _) = run(&["orders", "archive"], true).await;
        assert!(ran, "both named");
        // Not production: one request (one prompt), for the source.
        let (_, seen) = run(&[], false).await;
        assert_eq!(seen, ["orders"]);
    }

    fn purge_op() -> Operation {
        Operation::purge(dlq("orders"), CountEstimate::Exact(3))
    }

    fn report_with(ok: usize, failed: usize, skipped: usize) -> JobReport {
        let item = |seq: i64, outcome| ItemResult {
            sequence_number: Some(seq),
            message_id: None,
            session_id: None,
            entity: "orders/$DeadLetterQueue".into(),
            new_sequence_number: None,
            outcome,
        };
        let mut items = vec![];
        for i in 0..ok {
            items.push(item(i as i64, ItemOutcome::Succeeded { detail: None }));
        }
        for i in 0..failed {
            items.push(item(
                100 + i as i64,
                ItemOutcome::Failed {
                    kind: "failed",
                    reason: "no".into(),
                },
            ));
        }
        for i in 0..skipped {
            items.push(item(
                200 + i as i64,
                ItemOutcome::Skipped {
                    reason: "skip".into(),
                    expected: false,
                },
            ));
        }
        JobReport {
            op_id: None,
            kind: JobKind::Purge,
            entity: "orders/$DeadLetterQueue".into(),
            target: None,
            started_at: OffsetDateTime::now_utc(),
            finished_at: OffsetDateTime::now_utc(),
            cancelled: false,
            items,
            notes: vec![],
            not_attempted: 0,
        }
    }

    #[tokio::test]
    async fn allowed_work_is_audited_before_and_after() {
        let (g, audit) = gate(Decision::Allow, false);
        let calls = AtomicUsize::new(0);
        g.run(&purge_op(), |_| async {
            calls.fetch_add(1, SeqCst);
            assert_eq!(audit.entries().len(), 1, "started is written first");
            Ok(())
        })
        .await
        .unwrap();
        let e = audit.entries();
        assert_eq!(calls.load(SeqCst), 1);
        assert_eq!(
            (e[0].phase, e[0].outcome),
            (Phase::Started, Outcome::Pending)
        );
        assert_eq!(
            (e[1].phase, e[1].outcome),
            (Phase::Finished, Outcome::Succeeded)
        );
        assert_eq!(e[0].op_id, e[1].op_id);
        assert_eq!(
            e[0].summary,
            "Remove 3 messages from orders/$DeadLetterQueue on prod-weu"
        );
        assert_eq!(e[0].count, Some(3));
    }

    #[tokio::test]
    async fn denied_operation_touches_nothing_and_is_logged() {
        let (g, audit) = gate(Decision::Deny("not now".into()), false);
        let e = g
            .run(&purge_op(), |_| async {
                panic!("work must not run");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Denied(_)));
        let log = audit.entries();
        assert_eq!(log.len(), 1);
        assert_eq!(
            (log[0].phase, log[0].outcome),
            (Phase::Finished, Outcome::Denied)
        );
        assert!(log[0].error.as_deref().unwrap().contains("not now"));
    }

    #[tokio::test]
    async fn production_needs_the_typed_entity() {
        let (g, audit) = gate(Decision::Allow, true);
        assert!(matches!(
            g.run(&purge_op(), |_| async { Ok(()) }).await,
            Err(Error::Denied(_))
        ));
        let log = audit.entries();
        assert_eq!(log.len(), 1, "no started entry for a refused operation");
        assert_eq!(
            (log[0].phase, log[0].outcome),
            (Phase::Finished, Outcome::Denied)
        );
        let (g, _) = gate(Decision::ConfirmedTyped("orders".into()), true);
        g.run(&purge_op(), |_| async { Ok(()) }).await.unwrap();
    }

    #[tokio::test]
    async fn failing_audit_start_stops_the_operation() {
        let (g, audit) = gate(Decision::Allow, false);
        audit.fail_next();
        let e = g
            .run(&purge_op(), |_| async {
                panic!("must not run");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(e.to_string().contains("nothing was changed"));
    }

    #[tokio::test]
    async fn cancelled_work_is_audited_as_cancelled() {
        let (g, audit) = gate(Decision::Allow, false);
        let r = g
            .run::<(), _, _>(&purge_op(), |_| async { Err(Error::Cancelled) })
            .await;
        assert!(matches!(r, Err(Error::Cancelled)));
        let fin = &audit.entries()[1];
        assert_eq!(fin.outcome, Outcome::Cancelled);
        assert_eq!(fin.error.as_deref(), Some("cancelled"));
    }

    #[tokio::test]
    async fn job_report_facts_reach_the_finished_entry() {
        let (g, audit) = gate(Decision::Allow, false);
        let r = g
            .run(&purge_op(), |op_id| async move {
                let mut r = report_with(2, 1, 0);
                r.set_op_id(op_id);
                Ok(r)
            })
            .await
            .unwrap();
        let fin = &audit.entries()[1];
        assert_eq!(
            (fin.outcome, fin.succeeded, fin.failed, fin.skipped),
            (Outcome::PartlyFailed, Some(2), Some(1), Some(0))
        );
        assert_eq!(
            r.op_id.as_deref(),
            Some(fin.op_id.as_str()),
            "the report names the activity op it belongs to"
        );
    }

    #[tokio::test]
    async fn failed_work_is_audited_as_failed_and_a_bad_finish_write_changes_nothing() {
        let (g, audit) = gate(Decision::Allow, false);
        let e = g
            .run::<(), _, _>(&purge_op(), |_| async {
                Err(Error::Network("down".into()))
            })
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Network(_)));
        let log = audit.entries();
        assert_eq!(log[1].outcome, Outcome::Failed);
        assert!(log[1].error.as_deref().unwrap().contains("down"));

        let (g, audit) = gate(Decision::Allow, false);
        let a2 = audit.clone();
        let r = g
            .run(&purge_op(), |_| async move {
                a2.fail_next();
                Ok(())
            })
            .await;
        assert!(r.is_ok(), "the work result survives a failed finish write");
        assert_eq!(audit.entries().len(), 1);
    }

    #[tokio::test]
    async fn a_cancelled_job_report_is_audited_with_its_counts_and_notes() {
        let (g, audit) = gate(Decision::Allow, false);
        g.run(&purge_op(), |_| async {
            let mut r = report_with(2, 0, 0);
            r.cancelled = true;
            r.notes
                .push("up to 3 more messages may have been removed".into());
            Ok(r)
        })
        .await
        .unwrap();
        let fin = &audit.entries()[1];
        assert_eq!((fin.outcome, fin.succeeded), (Outcome::Cancelled, Some(2)));
        assert_eq!(fin.error, None, "a note is not an error");
        assert_eq!(
            fin.note.as_deref(),
            Some("up to 3 more messages may have been removed")
        );
    }

    #[tokio::test]
    async fn a_panicking_work_still_gets_a_finished_entry() {
        let (g, audit) = gate(Decision::Allow, false);
        let a = audit.clone();
        let joined = tokio::spawn(async move {
            g.run::<(), _, _>(&purge_op(), |_| async {
                panic!("boom");
            })
            .await
        })
        .await;
        assert!(joined.is_err());
        let e = a.entries();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[1].phase, e[1].outcome),
            (Phase::Finished, Outcome::Cancelled)
        );
        assert_eq!(
            e[1].error.as_deref(),
            Some("interrupted before it finished")
        );
        assert_eq!(e[0].op_id, e[1].op_id);
    }

    #[tokio::test]
    async fn a_dropped_run_future_still_gets_a_finished_entry() {
        let (g, audit) = gate(Decision::Allow, false);
        let r = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            g.run::<(), _, _>(&purge_op(), |_| std::future::pending()),
        )
        .await;
        assert!(r.is_err());
        let e = audit.entries();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[1].phase, e[1].outcome),
            (Phase::Finished, Outcome::Cancelled)
        );
        assert_eq!(
            e[1].error.as_deref(),
            Some("interrupted before it finished")
        );
    }

    #[tokio::test]
    async fn a_success_with_a_note_has_no_error() {
        struct Noted;
        impl Audited for Noted {
            fn audit_outcome(&self) -> AuditFacts {
                AuditFacts {
                    outcome: Outcome::Succeeded,
                    succeeded: Some(1),
                    failed: Some(0),
                    skipped: Some(0),
                    note: Some("more may remain".into()),
                }
            }
        }
        let (g, audit) = gate(Decision::Allow, false);
        g.run(&purge_op(), |_| async { Ok(Noted) }).await.unwrap();
        let fin = &audit.entries()[1];
        assert_eq!(fin.error, None);
        assert_eq!(fin.note.as_deref(), Some("more may remain"));
    }
}
