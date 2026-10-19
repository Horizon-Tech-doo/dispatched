//! Gated entity management: create, update, delete, status, duplicate, rename, rules and SAS
//! policies. Each change is one `Mutation::EntityChange` operation through the safety gate;
//! everything that can be refused without changing anything is refused before it.

use super::{Operator, gated};
use crate::diagnostics::error_kind;
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, NoProgress, Progress};
use crate::mgmt::incumbent::{ImportEntity, ImportPolicy};
use crate::mgmt::policies::{self, KeySlot};
use crate::mgmt::{
    AccessRight, Description, EntityCounts, EntityKind, EntityRef, EntityStatus, ManagementClient,
    QueueSettings, RuleSpec, SubscriptionSettings, TopicSettings, validate_filter,
};
use crate::safety::{CountEstimate, DeleteTarget, Operation, count_text};
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;

/// An entity with its full description, to create or update.
// Built once per command and moved into the operation; boxing would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum EntitySpec {
    /// A queue.
    Queue(String, Description<QueueSettings>),
    /// A topic.
    Topic(String, Description<TopicSettings>),
    /// A subscription; `rule` is the initial rule on create (`None`: the service's
    /// `$Default`). An update never changes rules.
    Subscription {
        /// Its topic.
        topic: String,
        /// Subscription name.
        name: String,
        /// Its settings.
        d: Description<SubscriptionSettings>,
        /// Initial rule (create only).
        rule: Option<RuleSpec>,
    },
}

impl EntitySpec {
    /// The entity it describes.
    pub fn entity(&self) -> EntityRef {
        match self {
            EntitySpec::Queue(name, _) => EntityRef::queue(name),
            EntitySpec::Topic(name, _) => EntityRef::topic(name),
            EntitySpec::Subscription { topic, name, .. } => EntityRef::subscription(topic, name),
        }
    }
}

/// The item for one entity.
pub(crate) fn entity_item(entity: &EntityRef, outcome: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: None,
        message_id: None,
        session_id: None,
        entity: entity.path(),
        new_sequence_number: None,
        outcome,
    }
}

pub(crate) fn succeeded(detail: Option<String>) -> ItemOutcome {
    ItemOutcome::Succeeded { detail }
}

pub(crate) fn failed_with(e: &Error, reason: String) -> ItemOutcome {
    ItemOutcome::Failed {
        kind: error_kind(e),
        reason,
    }
}

fn outcome(r: Result<Option<String>>) -> ItemOutcome {
    match r {
        Ok(detail) => succeeded(detail),
        Err(e) => super::failed(&e),
    }
}

/// Every message an entity holds, by its counts.
fn total(c: &EntityCounts) -> u64 {
    c.active + c.dead_letter + c.scheduled + c.transfer + c.transfer_dead_letter
}

fn estimate(c: Option<&EntityCounts>) -> CountEstimate {
    match c {
        None => CountEstimate::Unknown,
        Some(c) if c.truncated => CountEstimate::AtLeast(total(c)),
        Some(c) if c.approximate => CountEstimate::About(total(c)),
        Some(c) => CountEstimate::Exact(total(c)),
    }
}

fn add(a: CountEstimate, b: CountEstimate) -> CountEstimate {
    use CountEstimate::*;
    match (a, b) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (AtLeast(x), y) | (y, AtLeast(x)) => AtLeast(x + y.bound().unwrap_or(0)),
        (About(x), y) | (y, About(x)) => About(x + y.bound().unwrap_or(0)),
        (Exact(x), Exact(y)) => Exact(x + y),
        (x, y) => UpTo(x.bound().unwrap_or(0) + y.bound().unwrap_or(0)),
    }
}

/// Create `spec` on the service.
pub(crate) async fn create(m: &ManagementClient, spec: &EntitySpec) -> Result<()> {
    match spec {
        EntitySpec::Queue(name, d) => m.create_queue(name, d).await,
        EntitySpec::Topic(name, d) => m.create_topic(name, d).await,
        EntitySpec::Subscription {
            topic,
            name,
            d,
            rule,
        } => {
            let rule = rule.clone().map(Description::new);
            m.create_subscription_with_rule(topic, name, d, rule.as_ref())
                .await
        }
    }
}

/// Replace the settings of `spec` on the service.
pub(crate) async fn update(m: &ManagementClient, spec: &EntitySpec) -> Result<()> {
    match spec {
        EntitySpec::Queue(name, d) => m.update_queue(name, d).await,
        EntitySpec::Topic(name, d) => m.update_topic(name, d).await,
        EntitySpec::Subscription { topic, name, d, .. } => {
            m.update_subscription(topic, name, d).await
        }
    }
}

async fn delete_one(m: &ManagementClient, e: &EntityRef) -> Result<()> {
    match e {
        EntityRef::Queue { name } => m.delete_queue(name).await,
        EntityRef::Topic { name } => m.delete_topic(name).await,
        EntityRef::Subscription { topic, name } => m.delete_subscription(topic, name).await,
    }
}

/// The rules of a subscription, each read in full (so typed correlation values survive).
async fn read_rules(
    m: &ManagementClient,
    topic: &str,
    subscription: &str,
) -> Result<Vec<Description<RuleSpec>>> {
    let mut out = Vec::new();
    for r in m.list_rules(topic, subscription).await? {
        out.push(m.get_rule(topic, subscription, &r.name).await?);
    }
    Ok(out)
}

/// Create subscription `to_topic/to_name` with the settings and rules of
/// `from_topic/from_name`. Returns how many rules were copied.
async fn copy_subscription(
    m: &ManagementClient,
    (from_topic, from_name): (&str, &str),
    (to_topic, to_name): (&str, &str),
) -> Result<usize> {
    let d = m.get_subscription(from_topic, from_name).await?;
    let rules = read_rules(m, from_topic, from_name).await?;
    // Every rule goes through its raw element, the first one too, so typed correlation
    // values (int, long, …) stay typed in the copy.
    m.create_subscription_with_rule(to_topic, to_name, &d, rules.first())
        .await?;
    if rules.is_empty() {
        // The service adds `$Default`; the source has no rule at all.
        m.delete_rule(to_topic, to_name, "$Default").await?;
    }
    for r in rules.iter().skip(1) {
        let copy = Description::new(r.settings.clone());
        // Keep the value types the source's correlation properties have.
        let copy = Description {
            raw: r.raw.clone(),
            ..copy
        };
        m.create_rule(to_topic, to_name, &copy).await?;
    }
    Ok(rules.len())
}

fn single_job(kind: JobKind, entity: &EntityRef, outcome: ItemOutcome) -> JobReport {
    let mut job = JobRun::new(kind, entity.path(), None, &NoProgress, Some(1));
    job.item(entity_item(entity, outcome));
    job.finish(false)
}

impl Operator {
    fn mgmt(&self) -> &ManagementClient {
        self.explorer.management_client()
    }

    /// The messages `entity` holds and, for a topic, how many subscriptions it has. Counts
    /// the service does not report are counted by peeking (approximate). A missing entity is
    /// `NotFound`.
    pub async fn entity_contents(&self, entity: &EntityRef) -> Result<DeleteTarget> {
        let (messages, subscriptions) = match entity {
            EntityRef::Queue { name } => {
                let d = self
                    .explorer
                    .entity_details(EntityKind::Queue, name)
                    .await?;
                (estimate(d.counts.as_ref()), None)
            }
            EntityRef::Subscription { .. } => {
                let d = self
                    .explorer
                    .entity_details(EntityKind::Subscription, &entity.path())
                    .await?;
                (estimate(d.counts.as_ref()), None)
            }
            EntityRef::Topic { name } => {
                // The listing of a missing topic is empty, so ask for the topic first.
                self.mgmt().get_topic_description(name).await?;
                let subs = self.explorer.list_subscriptions(name).await?;
                let messages = subs.iter().fold(CountEstimate::Exact(0), |acc, s| {
                    add(acc, estimate(s.counts.as_ref()))
                });
                (messages, Some(subs.len() as u64))
            }
        };
        Ok(DeleteTarget {
            entity: entity.clone(),
            messages,
            subscriptions,
        })
    }

    /// Create a queue, topic or subscription (with its initial rule). The rule's filter is
    /// checked before the confirmation.
    pub async fn create_entity(&self, spec: EntitySpec) -> Result<JobReport> {
        let rule = match &spec {
            EntitySpec::Subscription { rule: Some(r), .. } => {
                validate_filter(&r.filter)?;
                Some(r.name.clone())
            }
            _ => None,
        };
        let entity = spec.entity();
        let op = Operation::create_entity(&entity, rule.as_deref());
        gated(&self.gate, &op, || async {
            let r = create(self.mgmt(), &spec).await.map(|()| None);
            Ok(single_job(JobKind::CreateEntities, &entity, outcome(r)))
        })
        .await
    }

    /// Replace the settings of an entity with `spec` (read it first with the client's
    /// `get_*` and change what should change: unknown elements are kept).
    pub async fn update_entity(&self, spec: EntitySpec) -> Result<JobReport> {
        if let EntitySpec::Subscription { rule: Some(_), .. } = &spec {
            return Err(Error::Invalid(
                "an update does not change rules; use `rule add` and `rule rm`".into(),
            ));
        }
        let entity = spec.entity();
        let op = Operation::update_entity(&entity);
        gated(&self.gate, &op, || async {
            let r = update(self.mgmt(), &spec).await.map(|()| None);
            Ok(single_job(JobKind::UpdateEntities, &entity, outcome(r)))
        })
        .await
    }

    /// Delete entities with everything they hold. Each is counted first (a missing one
    /// stops the command before the confirmation), and the confirmation names each with its
    /// messages; on production every one must be confirmed.
    pub async fn delete_entities(&self, targets: Vec<EntityRef>) -> Result<JobReport> {
        let mut unique: Vec<EntityRef> = Vec::new();
        for t in targets {
            if !unique.contains(&t) {
                unique.push(t);
            }
        }
        if unique.is_empty() {
            return Err(Error::Invalid("name at least one entity to delete".into()));
        }
        let mut counted = Vec::with_capacity(unique.len());
        for e in &unique {
            counted.push(self.entity_contents(e).await?);
        }
        let op = Operation::delete_entities(counted.clone());
        let label = unique
            .iter()
            .map(EntityRef::path)
            .collect::<Vec<_>>()
            .join(", ");
        gated(&self.gate, &op, || async {
            let mut job = JobRun::new(
                JobKind::DeleteEntities,
                label,
                None,
                &NoProgress,
                Some(counted.len() as u64),
            );
            for t in &counted {
                let r = delete_one(self.mgmt(), &t.entity).await.map(|()| {
                    Some(match t.messages {
                        CountEstimate::Unknown => "deleted".to_string(),
                        c => format!("deleted with {}", c.text("message")),
                    })
                });
                job.item(entity_item(&t.entity, outcome(r)));
            }
            Ok(job.finish(false))
        })
        .await
    }

    /// Change an entity's status (read-modify-write).
    pub async fn set_status(&self, target: EntityRef, status: EntityStatus) -> Result<JobReport> {
        let op = Operation::set_status(&target, status);
        gated(&self.gate, &op, || async {
            let m = self.mgmt();
            let r = async {
                match &target {
                    EntityRef::Queue { name } => {
                        let mut d = m.get_queue_description(name).await?;
                        d.settings.status = Some(status);
                        m.update_queue(name, &d).await
                    }
                    EntityRef::Topic { name } => {
                        let mut d = m.get_topic_description(name).await?;
                        d.settings.status = Some(status);
                        m.update_topic(name, &d).await
                    }
                    EntityRef::Subscription { topic, name } => {
                        let mut d = m.get_subscription(topic, name).await?;
                        d.settings.status = Some(status);
                        m.update_subscription(topic, name, &d).await
                    }
                }
            }
            .await
            .map(|()| Some(format!("status {status}")));
            Ok(single_job(JobKind::UpdateEntities, &target, outcome(r)))
        })
        .await
    }

    /// Create `new_name` with the settings of `source`: a queue with its policies (with new
    /// keys), or a subscription with its rules. Messages are not copied.
    pub async fn duplicate(&self, source: EntityRef, new_name: String) -> Result<JobReport> {
        if source.kind() == EntityKind::Topic {
            return Err(Error::Invalid(
                "duplicate works on queues and subscriptions; create the topic and its \
                 subscriptions instead"
                    .into(),
            ));
        }
        let to = source.renamed(&new_name);
        if to == source {
            return Err(Error::Invalid("the copy needs a different name".into()));
        }
        let op = Operation::duplicate(&source, &to);
        gated(&self.gate, &op, || async {
            let m = self.mgmt();
            let r = async {
                match &source {
                    EntityRef::Queue { name } => {
                        let mut d = m.get_queue_description(name).await?;
                        for p in &mut d.policies {
                            p.primary_key = policies::new_key();
                            p.secondary_key = policies::new_key();
                        }
                        let n = d.policies.len();
                        m.create_queue(&new_name, &d).await?;
                        Ok(Some(if n == 0 {
                            "settings copied; messages are not copied".to_string()
                        } else {
                            format!(
                                "settings and {n} {} copied with new keys; messages are not copied",
                                if n == 1 { "policy" } else { "policies" }
                            )
                        }))
                    }
                    EntityRef::Subscription { topic, name } => {
                        let n = copy_subscription(m, (topic, name), (topic, &new_name)).await?;
                        Ok(Some(format!(
                            "settings and {n} {} copied; messages are not copied",
                            if n == 1 { "rule" } else { "rules" }
                        )))
                    }
                    EntityRef::Topic { .. } => unreachable!("refused above"),
                }
            }
            .await;
            Ok(single_job(JobKind::Duplicate, &to, outcome(r)))
        })
        .await
    }

    /// Rename a queue or topic: create `new_name` as a copy (a topic with its subscriptions
    /// and rules), then delete the original. Refused before the confirmation when the entity
    /// (any subscription of a topic) holds messages, or when they cannot be counted: the
    /// delete would lose them. Counted again right before the delete: when messages arrived
    /// meanwhile, the copy stays, the original is kept and its item fails.
    pub async fn rename(&self, source: EntityRef, new_name: String) -> Result<JobReport> {
        if source.kind() == EntityKind::Subscription {
            return Err(Error::Invalid(
                "rename works on queues and topics; duplicate the subscription, then delete it"
                    .into(),
            ));
        }
        let to = source.renamed(&new_name);
        if to == source {
            return Err(Error::Invalid("the new name is the current name".into()));
        }
        let contents = self.entity_contents(&source).await?;
        match contents.messages {
            CountEstimate::Exact(0) | CountEstimate::About(0) => {}
            CountEstimate::Unknown => {
                return Err(Error::Conflict(format!(
                    "the messages of {} cannot be counted, and a rename deletes the original; \
                     make sure it is empty and try again",
                    source.path()
                )));
            }
            c => {
                return Err(Error::Conflict(format!(
                    "{} holds {}; {} would be lost; move them first",
                    source.path(),
                    c.text("message"),
                    c.text("message")
                )));
            }
        }
        let op = Operation::rename(&source, &to);
        gated(&self.gate, &op, || async {
            let m = self.mgmt();
            let mut job = JobRun::new(
                JobKind::Rename,
                source.path(),
                Some(to.path()),
                &NoProgress,
                Some(2),
            );
            let copied: Result<Option<String>> = async {
                match &source {
                    EntityRef::Queue { name } => {
                        let d = m.get_queue_description(name).await?;
                        m.create_queue(&new_name, &d).await?;
                        Ok(Some(format!("created as a copy of {name}")))
                    }
                    EntityRef::Topic { name } => {
                        let d = m.get_topic_description(name).await?;
                        let subs = m.list_subscriptions(name).await?;
                        m.create_topic(&new_name, &d).await?;
                        for s in &subs {
                            copy_subscription(m, (name, &s.name), (&new_name, &s.name))
                                .await
                                .map_err(|e| {
                                    Error::Conflict(format!(
                                        "{new_name} was created, but subscription {} could \
                                         not be copied ({e}); {name} was not deleted",
                                        s.name
                                    ))
                                })?;
                        }
                        Ok(Some(format!(
                            "created as a copy of {name} with {} {}",
                            count_text(subs.len() as u64),
                            if subs.len() == 1 {
                                "subscription"
                            } else {
                                "subscriptions"
                            }
                        )))
                    }
                    EntityRef::Subscription { .. } => unreachable!("refused above"),
                }
            }
            .await;
            let copied_ok = copied.is_ok();
            job.item(entity_item(&to, outcome(copied)));
            if copied_ok {
                // Count again right before the delete: messages may have arrived while the
                // confirmation was open or the copy was made, and a rename never deletes any.
                let item = match self.entity_contents(&source).await.map(|t| t.messages) {
                    Ok(CountEstimate::Exact(0) | CountEstimate::About(0)) => {
                        match delete_one(m, &source).await {
                            Ok(()) => {
                                succeeded(Some(format!("deleted after the copy to {new_name}")))
                            }
                            Err(e) => {
                                let reason = format!(
                                    "{} was created; {} could not be deleted ({e}); delete one of them",
                                    to.path(),
                                    source.path()
                                );
                                failed_with(&e, reason)
                            }
                        }
                    }
                    Ok(held) => {
                        let reason = format!(
                            "{} was created; {} now holds {} and was kept; move them to {}, then delete {}",
                            to.path(),
                            source.path(),
                            held.text("message"),
                            to.path(),
                            source.path()
                        );
                        failed_with(&Error::Conflict(reason.clone()), reason)
                    }
                    Err(e) => {
                        let reason = format!(
                            "{} was created; {} could not be counted again ({e}) and was kept; \
                             make sure it is empty, then delete it",
                            to.path(),
                            source.path()
                        );
                        failed_with(&e, reason)
                    }
                };
                job.item(entity_item(&source, item));
            } else {
                job.item(entity_item(
                    &source,
                    ItemOutcome::Skipped {
                        reason: "kept: the copy was not made".into(),
                        expected: false,
                    },
                ));
            }
            Ok(job.finish(false))
        })
        .await
    }

    /// Add `rule` to a subscription. The filter is checked before the confirmation.
    pub async fn add_rule(&self, sub: EntityRef, rule: RuleSpec) -> Result<JobReport> {
        let EntityRef::Subscription { topic, name } = &sub else {
            return Err(Error::Invalid(format!(
                "{} is not a subscription; rules belong to subscriptions",
                sub.path()
            )));
        };
        validate_filter(&rule.filter)?;
        let op = Operation::add_rule(&sub, &rule.name);
        gated(&self.gate, &op, || async {
            let detail = Some(format!("rule {} added", rule.name));
            let r = self
                .mgmt()
                .create_rule(topic, name, &Description::new(rule.clone()))
                .await
                .map(|()| detail);
            Ok(single_job(JobKind::UpdateEntities, &sub, outcome(r)))
        })
        .await
    }

    /// Remove rule `rule` from a subscription.
    pub async fn remove_rule(&self, sub: EntityRef, rule: String) -> Result<JobReport> {
        let EntityRef::Subscription { topic, name } = &sub else {
            return Err(Error::Invalid(format!(
                "{} is not a subscription; rules belong to subscriptions",
                sub.path()
            )));
        };
        let op = Operation::remove_rule(&sub, &rule);
        gated(&self.gate, &op, || async {
            let r = self
                .mgmt()
                .delete_rule(topic, name, &rule)
                .await
                .map(|()| Some(format!("rule {rule} removed")));
            Ok(single_job(JobKind::UpdateEntities, &sub, outcome(r)))
        })
        .await
    }

    /// Read `entity`, change its policies with `change`, write it back.
    async fn change_policies(
        &self,
        entity: &EntityRef,
        change: impl FnOnce(&mut Vec<crate::mgmt::AccessPolicy>) -> Result<String>,
    ) -> Result<Option<String>> {
        let m = self.mgmt();
        match entity {
            EntityRef::Queue { name } => {
                let mut d = m.get_queue_description(name).await?;
                let detail = change(&mut d.policies)?;
                m.update_queue(name, &d).await?;
                Ok(Some(detail))
            }
            EntityRef::Topic { name } => {
                let mut d = m.get_topic_description(name).await?;
                let detail = change(&mut d.policies)?;
                m.update_topic(name, &d).await?;
                Ok(Some(detail))
            }
            EntityRef::Subscription { .. } => Err(no_policies()),
        }
    }

    /// Add SAS policy `name` with `rights` and two new keys.
    pub async fn add_policy(
        &self,
        entity: EntityRef,
        name: String,
        rights: Vec<AccessRight>,
    ) -> Result<JobReport> {
        if entity.kind() == EntityKind::Subscription {
            return Err(no_policies());
        }
        // The same checks as on the description, on a scratch one: refused before the gate.
        policies::add_policy(
            &mut Description::new(QueueSettings::default()),
            &name,
            &rights,
        )?;
        let op = Operation::add_policy(&entity, &name, &rights);
        gated(&self.gate, &op, || async {
            let r = self
                .change_policies(&entity, |p| {
                    let mut d = Description::new(());
                    d.policies = std::mem::take(p);
                    let added = policies::add_policy(&mut d, &name, &rights);
                    *p = d.policies;
                    added.map(|()| format!("policy {name} added"))
                })
                .await;
            Ok(single_job(JobKind::UpdateEntities, &entity, outcome(r)))
        })
        .await
    }

    /// Remove SAS policy `name`.
    pub async fn remove_policy(&self, entity: EntityRef, name: String) -> Result<JobReport> {
        if entity.kind() == EntityKind::Subscription {
            return Err(no_policies());
        }
        let op = Operation::remove_policy(&entity, &name);
        gated(&self.gate, &op, || async {
            let r = self
                .change_policies(&entity, |p| {
                    let mut d = Description::new(());
                    d.policies = std::mem::take(p);
                    let removed = policies::remove_policy(&mut d, &name);
                    *p = d.policies;
                    removed.map(|()| format!("policy {name} removed"))
                })
                .await;
            Ok(single_job(JobKind::UpdateEntities, &entity, outcome(r)))
        })
        .await
    }

    /// Replace one key of SAS policy `name` with a new one. The key is not returned: read it
    /// with the client's `get_*` when it is asked for.
    pub async fn regenerate_key(
        &self,
        entity: EntityRef,
        name: String,
        slot: KeySlot,
    ) -> Result<JobReport> {
        if entity.kind() == EntityKind::Subscription {
            return Err(no_policies());
        }
        let op = Operation::regenerate_key(&entity, &name, slot);
        gated(&self.gate, &op, || async {
            let r = self
                .change_policies(&entity, |p| {
                    let mut d = Description::new(());
                    d.policies = std::mem::take(p);
                    let done = policies::regenerate(&mut d, &name, slot);
                    *p = d.policies;
                    done.map(|()| format!("{} key of policy {name} regenerated", slot.as_str()))
                })
                .await;
            Ok(single_job(JobKind::UpdateEntities, &entity, outcome(r)))
        })
        .await
    }
}

/// Set every field of `$base` that `$file` sets.
macro_rules! overlay {
    ($base:expr, $file:expr; $($field:ident),* $(,)?) => {
        $( if $file.$field.is_some() { $base.$field = $file.$field.clone(); } )*
    };
}

/// The settings of an existing entity with what the file sets on top. A file policy of the
/// same name as an existing one replaces its rights, and its keys only where the file holds
/// keys (a file without keys keeps them, so restoring a default export rotates nothing); a
/// policy new to the entity gets the file's keys or fresh ones; others are kept. Returns the
/// existing policies whose keys the file replaced.
fn merge<T>(
    existing: &mut Description<T>,
    file: &Description<T>,
    policies: &[ImportPolicy],
    apply: impl Fn(&mut T, &T),
) -> Vec<String> {
    apply(&mut existing.settings, &file.settings);
    let mut replaced = Vec::new();
    for p in policies {
        match existing
            .policies
            .iter_mut()
            .find(|e| e.key_name.eq_ignore_ascii_case(&p.key_name))
        {
            Some(e) => {
                if p.has_keys() {
                    replaced.push(e.key_name.clone());
                }
                *e = p.applied(Some(e));
            }
            None => existing.policies.push(p.applied(None)),
        }
    }
    replaced
}

/// `d` with the file's policies, for a new entity: keys the file lacks are fresh.
fn with_policies<T: Clone>(d: &Description<T>, policies: &[ImportPolicy]) -> Description<T> {
    let mut d = d.clone();
    d.policies = policies.iter().map(|p| p.applied(None)).collect();
    d
}

/// The detail of an entity updated by `--overwrite`, naming policies whose keys changed.
fn overwritten(replaced: &[String]) -> String {
    match replaced {
        [] => "updated (--overwrite)".into(),
        [one] => {
            format!("updated (--overwrite); the keys of policy {one} were replaced from the file")
        }
        many => format!(
            "updated (--overwrite); the keys of policies {} were replaced from the file",
            many.join(", ")
        ),
    }
}

fn overlay_queue(b: &mut QueueSettings, f: &QueueSettings) {
    overlay!(b, f; lock_duration, max_size_in_megabytes, requires_duplicate_detection,
        requires_session, default_message_time_to_live, dead_lettering_on_message_expiration,
        duplicate_detection_history_time_window, max_delivery_count, enable_batched_operations,
        status, forward_to, forward_dead_lettered_messages_to, auto_delete_on_idle,
        enable_partitioning, enable_express, max_message_size_in_kilobytes, user_metadata);
}

fn overlay_topic(b: &mut TopicSettings, f: &TopicSettings) {
    overlay!(b, f; max_size_in_megabytes, requires_duplicate_detection,
        duplicate_detection_history_time_window, default_message_time_to_live,
        enable_batched_operations, status, auto_delete_on_idle, enable_partitioning,
        enable_express, support_ordering, max_message_size_in_kilobytes, user_metadata);
}

fn overlay_subscription(b: &mut SubscriptionSettings, f: &SubscriptionSettings) {
    overlay!(b, f; lock_duration, requires_session, default_message_time_to_live,
        dead_lettering_on_message_expiration, dead_lettering_on_filter_evaluation_exceptions,
        max_delivery_count, enable_batched_operations, status, forward_to,
        forward_dead_lettered_messages_to, auto_delete_on_idle, user_metadata);
}

fn exists(e: &Error) -> bool {
    matches!(e, Error::Conflict(m) if m.ends_with("already exists"))
}

fn skipped_existing(what: &str) -> ItemOutcome {
    ItemOutcome::Skipped {
        reason: format!("{what} already exists; use --overwrite to replace it"),
        expected: false,
    }
}

/// The rank of an import item: queues, then topics, then subscriptions.
fn rank(i: &ImportEntity) -> u8 {
    match i {
        ImportEntity::Queue { .. } => 0,
        ImportEntity::Topic { .. } => 1,
        ImportEntity::Subscription { .. } => 2,
    }
}

/// The rule a subscription is created with: `$Default` when the file has it, else the first.
fn initial_rule(rules: &[RuleSpec]) -> Option<usize> {
    if rules.is_empty() {
        None
    } else {
        Some(rules.iter().position(|r| r.name == "$Default").unwrap_or(0))
    }
}

impl Operator {
    /// Create the entities of an incumbent file (see [`crate::mgmt::incumbent`]): queues,
    /// then topics, then subscriptions with their initial rule, then their other rules. One
    /// confirmation covers the whole import and names every entity. An entity that exists is
    /// a skipped item, or with `overwrite` is updated with the file's settings on top of its
    /// own. Every rule filter is checked before the confirmation.
    pub async fn import_entities(
        &self,
        mut items: Vec<ImportEntity>,
        overwrite: bool,
        progress: &dyn Progress,
        cancel: &CancellationToken,
    ) -> Result<JobReport> {
        if items.is_empty() {
            return Err(Error::Invalid(
                "the file holds no queue, topic or subscription".into(),
            ));
        }
        for it in &items {
            if let ImportEntity::Subscription { rules, .. } = it {
                for r in rules {
                    validate_filter(&r.filter).map_err(|e| {
                        Error::Invalid(format!("{}, rule {}: {e}", it.entity(), r.name))
                    })?;
                }
            }
        }
        items.sort_by_key(rank);
        let entities: Vec<EntityRef> = items.iter().map(ImportEntity::entity).collect();
        let op = Operation::import_entities(entities, overwrite);
        let planned: usize = items
            .iter()
            .map(|i| match i {
                ImportEntity::Subscription { rules, .. } => 1 + rules.len().saturating_sub(1),
                _ => 1,
            })
            .sum();
        gated(&self.gate, &op, || async move {
            let m = self.mgmt();
            let mut job = JobRun::new(
                JobKind::ImportEntities,
                "import".into(),
                None,
                progress,
                Some(planned as u64),
            );
            // Rules still to add: (subscription, rule, the subscription existed).
            let mut later: Vec<(EntityRef, RuleSpec, bool)> = Vec::new();
            let mut cancelled = false;
            for it in &items {
                if cancel.is_cancelled() {
                    cancelled = true;
                    break;
                }
                let entity = it.entity();
                let created: Result<()> = match it {
                    ImportEntity::Queue { name, d, policies } => {
                        m.create_queue(name, &with_policies(d, policies)).await
                    }
                    ImportEntity::Topic { name, d, policies } => {
                        m.create_topic(name, &with_policies(d, policies)).await
                    }
                    ImportEntity::Subscription {
                        topic,
                        name,
                        d,
                        rules,
                    } => {
                        let first = initial_rule(rules).map(|i| Description::new(rules[i].clone()));
                        m.create_subscription_with_rule(topic, name, d, first.as_ref())
                            .await
                    }
                };
                let outcome = match created {
                    Ok(()) => {
                        if let ImportEntity::Subscription { rules, .. } = it {
                            let first = initial_rule(rules);
                            for (i, r) in rules.iter().enumerate() {
                                if Some(i) != first {
                                    later.push((entity.clone(), r.clone(), false));
                                }
                            }
                            succeeded(Some(match first {
                                Some(i) => format!("created with rule {}", rules[i].name),
                                None => "created".into(),
                            }))
                        } else {
                            succeeded(Some("created".into()))
                        }
                    }
                    Err(e) if exists(&e) && !overwrite => skipped_existing(&entity.to_string()),
                    Err(e) if exists(&e) => {
                        let replaced: Result<Vec<String>> = async {
                            match it {
                                ImportEntity::Queue { name, d, policies } => {
                                    let mut cur = m.get_queue_description(name).await?;
                                    let keys = merge(&mut cur, d, policies, overlay_queue);
                                    m.update_queue(name, &cur).await.map(|()| keys)
                                }
                                ImportEntity::Topic { name, d, policies } => {
                                    let mut cur = m.get_topic_description(name).await?;
                                    let keys = merge(&mut cur, d, policies, overlay_topic);
                                    m.update_topic(name, &cur).await.map(|()| keys)
                                }
                                ImportEntity::Subscription {
                                    topic,
                                    name,
                                    d,
                                    rules,
                                } => {
                                    let mut cur = m.get_subscription(topic, name).await?;
                                    merge(&mut cur, d, &[], overlay_subscription);
                                    m.update_subscription(topic, name, &cur).await?;
                                    for r in rules {
                                        later.push((entity.clone(), r.clone(), true));
                                    }
                                    Ok(Vec::new())
                                }
                            }
                        }
                        .await;
                        outcome(replaced.map(|keys| Some(overwritten(&keys))))
                    }
                    Err(e) => super::failed(&e),
                };
                job.item(entity_item(&entity, outcome));
            }
            for (sub, rule, existed) in later {
                if cancelled || cancel.is_cancelled() {
                    cancelled = true;
                    break;
                }
                let EntityRef::Subscription { topic, name } = &sub else {
                    continue;
                };
                let d = Description::new(rule.clone());
                let outcome = match m.create_rule(topic, name, &d).await {
                    Ok(()) => succeeded(Some(format!("rule {} created", rule.name))),
                    Err(e) if exists(&e) && existed && overwrite => outcome(
                        m.update_rule(topic, name, &d)
                            .await
                            .map(|()| Some(format!("rule {} updated (--overwrite)", rule.name))),
                    ),
                    Err(e) if exists(&e) => skipped_existing(&format!("rule {}", rule.name)),
                    Err(e) => failed_with(&e, format!("rule {}: {e}", rule.name)),
                };
                job.item(entity_item(&sub, outcome));
            }
            Ok(job.finish(cancelled))
        })
        .await
    }
}

fn no_policies() -> Error {
    Error::Invalid(
        "subscriptions have no SAS policies; add one to the topic or the namespace".into(),
    )
}

#[cfg(test)]
pub(crate) mod testkit {
    //! A management API on mockito behind an operator.

    use crate::Result;
    use crate::explorer::Explorer;
    use crate::namespace::{EmulatorPorts, Namespace};
    use crate::ops::Operator;
    use crate::safety::Gate;
    use crate::tokens::{ServiceBusToken, TokenKind, TokenSource};
    use async_trait::async_trait;
    use secrecy::SecretString;
    use std::sync::Arc;

    struct FixedToken;
    #[async_trait]
    impl TokenSource for FixedToken {
        async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
            Ok(ServiceBusToken {
                token: SecretString::from("SharedAccessSignature sr=x&sig=y&se=9999999999&skn=k"),
                kind: TokenKind::Sas,
                expires_on: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            })
        }
        fn describe(&self) -> String {
            "fixed".into()
        }
    }

    /// An operator whose management API is `server`.
    pub(crate) fn operator(server: &mockito::Server, gate: Gate) -> Operator {
        let port = server
            .host_with_port()
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .expect("mockito port");
        let ns = Namespace::emulator(
            "127.0.0.1",
            EmulatorPorts {
                amqp: 1,
                management: port,
            },
            Arc::new(FixedToken),
        );
        Operator::new(Arc::new(Explorer::new(ns).unwrap()), gate)
    }

    /// A queue entry with these counts.
    pub(crate) fn queue_entry(name: &str, active: u64) -> String {
        format!(
            r#"<entry xmlns="http://www.w3.org/2005/Atom"><title type="text">{name}</title><content type="application/xml"><QueueDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><LockDuration>PT1M</LockDuration><MaxDeliveryCount>10</MaxDeliveryCount><Status>Active</Status><CountDetails xmlns:d2p1="http://schemas.microsoft.com/netservices/2011/06/servicebus"><d2p1:ActiveMessageCount>{active}</d2p1:ActiveMessageCount><d2p1:DeadLetterMessageCount>0</d2p1:DeadLetterMessageCount><d2p1:ScheduledMessageCount>0</d2p1:ScheduledMessageCount><d2p1:TransferMessageCount>0</d2p1:TransferMessageCount><d2p1:TransferDeadLetterMessageCount>0</d2p1:TransferDeadLetterMessageCount></CountDetails></QueueDescription></content></entry>"#
        )
    }

    /// A subscription entry with these counts.
    pub(crate) fn subscription_entry(name: &str, active: u64) -> String {
        format!(
            r#"<entry xmlns="http://www.w3.org/2005/Atom"><title type="text">{name}</title><content type="application/xml"><SubscriptionDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><LockDuration>PT1M</LockDuration><MaxDeliveryCount>10</MaxDeliveryCount><Status>Active</Status><CountDetails xmlns:d2p1="http://schemas.microsoft.com/netservices/2011/06/servicebus"><d2p1:ActiveMessageCount>{active}</d2p1:ActiveMessageCount><d2p1:DeadLetterMessageCount>0</d2p1:DeadLetterMessageCount><d2p1:ScheduledMessageCount>0</d2p1:ScheduledMessageCount><d2p1:TransferMessageCount>0</d2p1:TransferMessageCount><d2p1:TransferDeadLetterMessageCount>0</d2p1:TransferDeadLetterMessageCount></CountDetails></SubscriptionDescription></content></entry>"#
        )
    }

    /// A feed of entries.
    pub(crate) fn feed(entries: &[String]) -> String {
        format!(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><title type="text">x</title>{}</feed>"#,
            entries.concat()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::activity::Outcome;
    use crate::jobs::ItemOutcome;
    use crate::mgmt::RuleFilter;
    use crate::ops::testkit::{deny_gate, fake_gate};
    use mockito::Matcher;

    #[tokio::test]
    async fn rename_refuses_a_queue_with_messages() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(queue_entry("a", 3))
            .create_async()
            .await;
        let put = server
            .mock("PUT", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let del = server
            .mock("DELETE", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let op = operator(&server, gate);
        let e = op
            .rename(EntityRef::queue("a"), "b".into())
            .await
            .unwrap_err();
        assert!(
            matches!(&e, Error::Conflict(m) if m.contains("3 messages would be lost; move them first")),
            "{e}"
        );
        assert!(audit.entries().is_empty(), "refused before the gate");
        put.assert_async().await;
        del.assert_async().await;
    }

    #[tokio::test]
    async fn duplicate_copies_settings_only() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry_full.xml"))
            .create_async()
            .await;
        let put = server
            .mock("PUT", "/b")
            .match_query(Matcher::Any)
            .match_header("if-match", Matcher::Missing)
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("<LockDuration>PT45S</LockDuration>".into()),
                Matcher::Regex("<MaxDeliveryCount>7</MaxDeliveryCount>".into()),
                Matcher::Regex("<KeyName>app</KeyName>".into()),
                Matcher::Regex("<FutureFlag>true</FutureFlag>".into()),
            ]))
            .with_status(201)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let op = operator(&server, gate);
        let r = op
            .duplicate(EntityRef::queue("a"), "b".into())
            .await
            .unwrap();
        put.assert_async().await;
        let reqs = r.items.clone();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].entity, "b");
        let ItemOutcome::Succeeded { detail: Some(d) } = &reqs[0].outcome else {
            panic!("{:?}", reqs[0])
        };
        assert!(d.contains("messages are not copied"), "{d}");
        // The keys are new: the source's never reach the copy.
        assert_eq!(audit.entries().len(), 2);
        assert!(
            audit.entries()[0]
                .summary
                .starts_with("Duplicate queue a as b on dev")
        );
    }

    /// A rule entry with a correlation filter whose properties are an int and a long.
    fn typed_rule_entry(name: &str) -> String {
        format!(
            r#"<entry xmlns="http://www.w3.org/2005/Atom"><title type="text">{name}</title><content type="application/xml"><RuleDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><Filter i:type="CorrelationFilter"><Label>orders</Label><Properties><KeyValueOfstringanyType><Key>priority</Key><Value xmlns:d6p1="http://www.w3.org/2001/XMLSchema" i:type="d6p1:int">5</Value></KeyValueOfstringanyType><KeyValueOfstringanyType><Key>tenant</Key><Value xmlns:d6p1="http://www.w3.org/2001/XMLSchema" i:type="d6p1:long">9000000000</Value></KeyValueOfstringanyType></Properties></Filter><Action i:type="EmptyRuleAction"/><CreatedAt>2026-09-01T08:00:00Z</CreatedAt><Name>{name}</Name></RuleDescription></content></entry>"#
        )
    }

    /// The body of a PUT that keeps both value types of [`typed_rule_entry`].
    fn keeps_typed_values() -> Matcher {
        Matcher::AllOf(vec![
            Matcher::Regex(r#"<Key>priority</Key><Value i:type="d6p1:int"[^>]*>5</Value>"#.into()),
            Matcher::Regex(
                r#"<Key>tenant</Key><Value i:type="d6p1:long"[^>]*>9000000000</Value>"#.into(),
            ),
        ])
    }

    /// Mocks for subscription `topic/name` with one rule, [`typed_rule_entry`] `only`.
    async fn typed_subscription(server: &mut mockito::Server, topic: &str, name: &str) {
        let base = format!("/{topic}/Subscriptions/{name}");
        for (path, body) in [
            (base.clone(), subscription_entry(name, 0)),
            (format!("{base}/Rules"), feed(&[typed_rule_entry("only")])),
            (format!("{base}/Rules/only"), typed_rule_entry("only")),
        ] {
            server
                .mock("GET", path.as_str())
                .match_query(Matcher::Any)
                .with_status(200)
                .with_body(body)
                .create_async()
                .await;
        }
    }

    #[tokio::test]
    async fn duplicate_keeps_the_value_types_of_the_first_rule() {
        // The first (here the only) rule is written into the new subscription's
        // DefaultRuleDescription: an int and a long correlation property stay typed, or the
        // copy would compare them as strings and stop matching.
        let mut server = mockito::Server::new_async().await;
        typed_subscription(&mut server, "events", "billing").await;
        let put = server
            .mock("PUT", "/events/Subscriptions/billing-copy")
            .match_query(Matcher::Any)
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("<DefaultRuleDescription>".into()),
                keeps_typed_values(),
            ]))
            .with_status(201)
            .create_async()
            .await;
        let (gate, _audit) = fake_gate();
        let r = operator(&server, gate)
            .duplicate(
                EntityRef::subscription("events", "billing"),
                "billing-copy".into(),
            )
            .await
            .unwrap();
        put.assert_async().await;
        assert_eq!(r.succeeded(), 1, "{:?}", r.items);
    }

    #[tokio::test]
    async fn rename_keeps_the_original_when_messages_arrive_after_the_confirmation() {
        // Empty when counted before the confirmation (and when read for the copy), 12
        // messages by the time the copy exists: the original is kept, never deleted.
        let mut server = mockito::Server::new_async().await;
        let _empty = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(queue_entry("a", 0))
            .expect(2)
            .create_async()
            .await;
        let _arrived = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(queue_entry("a", 12))
            .create_async()
            .await;
        let create = server
            .mock("PUT", "/b")
            .match_query(Matcher::Any)
            .with_status(201)
            .create_async()
            .await;
        let del = server
            .mock("DELETE", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let r = operator(&server, gate)
            .rename(EntityRef::queue("a"), "b".into())
            .await
            .unwrap();
        create.assert_async().await;
        del.assert_async().await;
        let failed: Vec<_> = r.failures().collect();
        assert_eq!(failed.len(), 1, "{:?}", r.items);
        assert_eq!(failed[0].entity, "a");
        assert_eq!(
            JobReport::reason(failed[0]),
            "b was created; a now holds 12 messages and was kept; move them to b, then delete a"
        );
        assert_eq!(audit.entries()[1].outcome, Outcome::PartlyFailed);
    }

    #[tokio::test]
    async fn rename_copies_a_topic_with_typed_rules_and_counts_again_before_the_delete() {
        let mut server = mockito::Server::new_async().await;
        let topic = server
            .mock("GET", "/events")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/topic_entry.xml"))
            .expect(3)
            .create_async()
            .await;
        let _subs = server
            .mock("GET", "/events/Subscriptions")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(feed(&[subscription_entry("billing", 0)]))
            .create_async()
            .await;
        typed_subscription(&mut server, "events", "billing").await;
        let _new_topic = server
            .mock("PUT", "/events-v2")
            .match_query(Matcher::Any)
            .with_status(201)
            .create_async()
            .await;
        let copy = server
            .mock("PUT", "/events-v2/Subscriptions/billing")
            .match_query(Matcher::Any)
            .match_body(keeps_typed_values())
            .with_status(201)
            .create_async()
            .await;
        let del = server
            .mock("DELETE", "/events")
            .match_query(Matcher::Any)
            .with_status(200)
            .create_async()
            .await;
        let (gate, _audit) = fake_gate();
        let r = operator(&server, gate)
            .rename(EntityRef::topic("events"), "events-v2".into())
            .await
            .unwrap();
        copy.assert_async().await;
        // Counted before the confirmation, read for the copy, counted before the delete.
        topic.assert_async().await;
        del.assert_async().await;
        assert_eq!(r.succeeded(), 2, "{:?}", r.items);
    }

    #[tokio::test]
    async fn duplicate_gives_the_copy_new_keys() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry_full.xml"))
            .create_async()
            .await;
        let put = server
            .mock("PUT", "/b")
            .match_query(Matcher::Any)
            .match_body(Matcher::Regex(
                "primary-key-value|secondary-key-value".into(),
            ))
            .expect(0)
            .create_async()
            .await;
        let _ok = server
            .mock("PUT", "/b")
            .match_query(Matcher::Any)
            .with_status(201)
            .create_async()
            .await;
        let (gate, _audit) = fake_gate();
        operator(&server, gate)
            .duplicate(EntityRef::queue("a"), "b".into())
            .await
            .unwrap();
        put.assert_async().await;
    }

    #[tokio::test]
    async fn delete_topic_summary_counts_its_subscriptions() {
        let mut server = mockito::Server::new_async().await;
        let _topic = server
            .mock("GET", "/events")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/topic_entry.xml"))
            .create_async()
            .await;
        let _subs = server
            .mock("GET", "/events/Subscriptions")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(feed(&[
                subscription_entry("a", 400),
                subscription_entry("b", 404),
                subscription_entry("c", 400),
            ]))
            .create_async()
            .await;
        let del = server
            .mock("DELETE", "/events")
            .match_query(Matcher::Any)
            .with_status(200)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let r = operator(&server, gate)
            .delete_entities(vec![EntityRef::topic("events")])
            .await
            .unwrap();
        del.assert_async().await;
        assert_eq!(r.succeeded(), 1);
        assert_eq!(
            audit.entries()[0].summary,
            "Delete topic events, its 3 subscriptions and their 1,204 messages on dev"
        );
    }

    #[tokio::test]
    async fn denied_change_does_nothing() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(queue_entry("a", 0))
            .create_async()
            .await;
        let put = server
            .mock("PUT", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let del = server
            .mock("DELETE", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let (gate, audit) = deny_gate();
        let op = operator(&server, gate);
        assert!(matches!(
            op.delete_entities(vec![EntityRef::queue("a")]).await,
            Err(Error::Denied(_))
        ));
        assert!(matches!(
            op.set_status(EntityRef::queue("a"), EntityStatus::Disabled)
                .await,
            Err(Error::Denied(_))
        ));
        put.assert_async().await;
        del.assert_async().await;
        let entries = audit.entries();
        assert_eq!(entries.len(), 2, "one denied entry per change");
        assert!(entries.iter().all(|e| e.outcome == Outcome::Denied));
    }

    #[tokio::test]
    async fn rename_rolls_back_when_the_delete_fails() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/a")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(queue_entry("a", 0))
            .create_async()
            .await;
        let create = server
            .mock("PUT", "/b")
            .match_query(Matcher::Any)
            .with_status(201)
            .create_async()
            .await;
        let _del = server
            .mock("DELETE", "/a")
            .match_query(Matcher::Any)
            .with_status(500)
            .with_body("<Error><Code>500</Code><Detail>boom</Detail></Error>")
            .create_async()
            .await;
        let undo = server.mock("DELETE", "/b").expect(0).create_async().await;
        let (gate, audit) = fake_gate();
        let r = operator(&server, gate)
            .rename(EntityRef::queue("a"), "b".into())
            .await
            .unwrap();
        create.assert_async().await;
        undo.assert_async().await;
        let failed: Vec<_> = r.failures().collect();
        assert_eq!(failed.len(), 1);
        let reason = JobReport::reason(failed[0]);
        assert!(
            reason.starts_with("b was created; a could not be deleted (")
                && reason.contains("boom")
                && reason.ends_with("); delete one of them"),
            "{reason}"
        );
        assert_eq!(
            audit.entries()[1].outcome,
            Outcome::PartlyFailed,
            "the copy was made, the delete was not"
        );
    }

    #[tokio::test]
    async fn invalid_rules_are_refused_before_the_gate() {
        let server = mockito::Server::new_async().await;
        let (gate, audit) = fake_gate();
        let op = operator(&server, gate);
        let empty = RuleSpec {
            name: "c".into(),
            filter: RuleFilter::Correlation {
                correlation_id: None,
                message_id: None,
                to: None,
                reply_to: None,
                subject: None,
                session_id: None,
                reply_to_session_id: None,
                content_type: None,
                properties: vec![],
            },
            action: None,
        };
        let sub = EntityRef::subscription("events", "billing");
        assert!(matches!(
            op.add_rule(sub.clone(), empty.clone()).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            op.create_entity(EntitySpec::Subscription {
                topic: "events".into(),
                name: "x".into(),
                d: Description::default(),
                rule: Some(empty),
            })
            .await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            op.add_policy(sub, "app".into(), vec![AccessRight::Listen])
                .await,
            Err(Error::Invalid(_))
        ));
        assert!(audit.entries().is_empty());
    }

    #[tokio::test]
    async fn policy_changes_read_modify_write_the_entity() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/orders")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry_full.xml"))
            .create_async()
            .await;
        let put = server
            .mock("PUT", "/orders")
            .match_query(Matcher::Any)
            .match_header("if-match", "*")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("<KeyName>app</KeyName><PrimaryKey>primary-key-value</PrimaryKey><SecondaryKey>secondary-key-value</SecondaryKey>".into()),
                Matcher::Regex("<KeyName>ops</KeyName>".into()),
                Matcher::Regex("<FutureFlag>true</FutureFlag>".into()),
            ]))
            .with_status(200)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let r = operator(&server, gate)
            .add_policy(
                EntityRef::queue("orders"),
                "ops".into(),
                vec![AccessRight::Listen, AccessRight::Send],
            )
            .await
            .unwrap();
        put.assert_async().await;
        assert_eq!(r.succeeded(), 1);
        for e in audit.entries() {
            let text = format!("{e:?}");
            assert!(!text.contains("primary-key-value") && !text.contains("secondary-key-value"));
        }
    }

    #[tokio::test]
    async fn import_skips_existing_and_creates_the_rest() {
        use crate::mgmt::incumbent::{ImportEntity, parse_entities};
        let mut items =
            parse_entities(include_str!("../../tests/fixtures/incumbent_entities.xml")).unwrap();
        items.insert(
            1,
            ImportEntity::Queue {
                name: "audit".into(),
                d: Description::default(),
                policies: Vec::new(),
            },
        );
        let mut server = mockito::Server::new_async().await;
        let mut conflict = |path: &str| {
            server
                .mock("PUT", path)
                .match_query(Matcher::Any)
                .match_header("if-match", Matcher::Missing)
                .with_status(409)
                .with_body("<Error><Code>409</Code><Detail>exists</Detail></Error>")
        };
        let orders = conflict("/orders").create_async().await;
        let events = conflict("/events").create_async().await;
        let audit_q = server
            .mock("PUT", "/audit")
            .match_query(Matcher::Any)
            .with_status(201)
            .create_async()
            .await;
        let billing = server
            .mock("PUT", "/events/Subscriptions/billing")
            .match_query(Matcher::Any)
            .match_header("if-match", Matcher::Missing)
            .match_body(Matcher::Regex(
                "<DefaultRuleDescription><Filter i:type=\"TrueFilter\">.*<Name>\\$Default</Name></DefaultRuleDescription>".into(),
            ))
            .with_status(201)
            .create_async()
            .await;
        let only_eu = server
            .mock("PUT", "/events/Subscriptions/billing/Rules/only-eu")
            .match_query(Matcher::Any)
            .with_status(201)
            .create_async()
            .await;
        let by_corr = server
            .mock("PUT", "/events/Subscriptions/billing/Rules/by-corr")
            .match_query(Matcher::Any)
            .match_body(Matcher::Regex("<Key>tenant</Key>".into()))
            .with_status(201)
            .create_async()
            .await;
        let updates = server
            .mock("PUT", Matcher::Any)
            .match_header("if-match", "*")
            .expect(0)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let progress = crate::jobs::RecordingProgress::default();
        let r = operator(&server, gate)
            .import_entities(
                items,
                false,
                &progress,
                &horizon_azure_kit::safety::CancellationToken::new(),
            )
            .await
            .unwrap();
        for m in [orders, events, audit_q, billing, only_eu, by_corr, updates] {
            m.assert_async().await;
        }
        let outcome = |entity: &str, detail: &str| {
            r.items
                .iter()
                .find(|i| {
                    i.entity == entity
                        && match &i.outcome {
                            ItemOutcome::Succeeded { detail: Some(d) }
                            | ItemOutcome::Skipped { reason: d, .. } => d.contains(detail),
                            _ => false,
                        }
                })
                .map(|i| i.outcome.clone())
        };
        assert!(matches!(
            outcome("orders", "already exists"),
            Some(ItemOutcome::Skipped { .. })
        ));
        assert!(matches!(
            outcome("events", "already exists"),
            Some(ItemOutcome::Skipped { .. })
        ));
        assert!(matches!(
            outcome("audit", "created"),
            Some(ItemOutcome::Succeeded { .. })
        ));
        assert!(matches!(
            outcome("events/Subscriptions/billing", "created"),
            Some(ItemOutcome::Succeeded { .. })
        ));
        assert!(outcome("events/Subscriptions/billing", "rule only-eu").is_some());
        assert!(outcome("events/Subscriptions/billing", "rule by-corr").is_some());
        assert_eq!(r.items.len(), 6, "{:#?}", r.items);
        let order: Vec<&str> = r.items.iter().map(|i| i.entity.as_str()).collect();
        assert_eq!(
            order,
            [
                "orders",
                "audit",
                "events",
                "events/Subscriptions/billing",
                "events/Subscriptions/billing",
                "events/Subscriptions/billing"
            ],
            "queues, topics, subscriptions, then rules"
        );
        let entries = audit.entries();
        assert_eq!(entries.len(), 2, "one gate for the whole import");
        assert_eq!(
            entries[0].summary,
            "Import 4 entities on dev: queue orders, queue audit, topic events, subscription \
             events/Subscriptions/billing; existing ones are skipped"
        );
    }

    #[tokio::test]
    async fn import_refuses_an_empty_correlation_filter_before_the_gate() {
        use crate::mgmt::incumbent::parse_entities;
        let xml = include_str!("../../tests/fixtures/incumbent_entities.xml")
            .replace(
                "<CorrelationId>abc</CorrelationId>",
                "<CorrelationId></CorrelationId>",
            )
            .replace("<Label>order.created</Label>", "<Label></Label>")
            .replace("<Property>tenant: acme</Property>", "");
        let items = parse_entities(&xml).unwrap();
        let server = mockito::Server::new_async().await;
        let (gate, audit) = fake_gate();
        let e = operator(&server, gate)
            .import_entities(
                items,
                false,
                &crate::jobs::NoProgress,
                &horizon_azure_kit::safety::CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(
            e.to_string().contains("by-corr") && e.to_string().contains("#794"),
            "{e}"
        );
        assert!(audit.entries().is_empty());
    }

    #[tokio::test]
    async fn import_with_overwrite_updates_on_top_of_the_existing_entity() {
        use crate::mgmt::incumbent::ImportEntity;
        let mut server = mockito::Server::new_async().await;
        let _create = server
            .mock("PUT", "/orders")
            .match_query(Matcher::Any)
            .match_header("if-match", Matcher::Missing)
            .with_status(409)
            .create_async()
            .await;
        let _get = server
            .mock("GET", "/orders")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry_full.xml"))
            .create_async()
            .await;
        let update = server
            .mock("PUT", "/orders")
            .match_query(Matcher::Any)
            .match_header("if-match", "*")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("<MaxDeliveryCount>2</MaxDeliveryCount>".into()),
                Matcher::Regex("<LockDuration>PT45S</LockDuration>".into()),
                Matcher::Regex("<FutureFlag>true</FutureFlag>".into()),
            ]))
            .with_status(200)
            .create_async()
            .await;
        let (gate, audit) = fake_gate();
        let r = operator(&server, gate)
            .import_entities(
                vec![ImportEntity::Queue {
                    name: "orders".into(),
                    d: Description::new(QueueSettings {
                        max_delivery_count: Some(2),
                        ..Default::default()
                    }),
                    policies: Vec::new(),
                }],
                true,
                &crate::jobs::NoProgress,
                &horizon_azure_kit::safety::CancellationToken::new(),
            )
            .await
            .unwrap();
        update.assert_async().await;
        assert_eq!(r.succeeded(), 1, "{:?}", r.items);
        assert!(
            audit.entries()[0]
                .summary
                .ends_with("existing ones are replaced")
        );
    }

    /// An incumbent file with queue `orders` and these policy elements.
    fn orders_file(policies: &str) -> String {
        format!(
            r#"<Entities xmlns="http://schemas.microsoft.com/servicebusexplorer" serviceBusNamespace="ns"><Queues><Queue><Path>orders</Path><MaxDeliveryCount>2</MaxDeliveryCount><AuthorizationRules>{policies}</AuthorizationRules></Queue></Queues></Entities>"#
        )
    }

    /// `orders` exists (`queue_entry_full.xml`: policy `app` with keys `primary-key-value`
    /// and `secondary-key-value`); import `file` with `--overwrite` and return the PUT body
    /// and the item's detail.
    async fn overwrite_orders_with(file: &str) -> (String, String) {
        let mut server = mockito::Server::new_async().await;
        let _create = server
            .mock("PUT", "/orders")
            .match_query(Matcher::Any)
            .match_header("if-match", Matcher::Missing)
            .with_status(409)
            .create_async()
            .await;
        let _get = server
            .mock("GET", "/orders")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry_full.xml"))
            .create_async()
            .await;
        let body = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen = body.clone();
        let update = server
            .mock("PUT", "/orders")
            .match_query(Matcher::Any)
            .match_header("if-match", "*")
            .match_request(move |req| {
                *seen.lock().unwrap() = req
                    .utf8_lossy_body()
                    .map(|b| b.into_owned())
                    .unwrap_or_default();
                true
            })
            .with_status(200)
            .create_async()
            .await;
        let (gate, _audit) = fake_gate();
        let items = crate::mgmt::incumbent::parse_entities(file).unwrap();
        let r = operator(&server, gate)
            .import_entities(
                items,
                true,
                &crate::jobs::NoProgress,
                &horizon_azure_kit::safety::CancellationToken::new(),
            )
            .await
            .unwrap();
        update.assert_async().await;
        assert_eq!(r.succeeded(), 1, "{:?}", r.items);
        let ItemOutcome::Succeeded {
            detail: Some(detail),
        } = &r.items[0].outcome
        else {
            panic!("{:?}", r.items)
        };
        (body.lock().unwrap().clone(), detail.clone())
    }

    #[tokio::test]
    async fn import_overwrite_keeps_the_keys_when_the_file_has_none() {
        // The default export leaves keys out. Restoring it with --overwrite updates the
        // rights of `app` and keeps its keys (rotating both would break every client at
        // once); `ops`, new to the queue, gets fresh keys.
        let (body, detail) = overwrite_orders_with(&orders_file(
            "<SharedAccessAuthorizationRule><Rights><Right>Listen</Right></Rights><KeyName>app</KeyName></SharedAccessAuthorizationRule>\
             <SharedAccessAuthorizationRule><Rights><Right>Send</Right></Rights><KeyName>ops</KeyName></SharedAccessAuthorizationRule>",
        ))
        .await;
        assert!(
            body.contains("<Rights><AccessRights>Listen</AccessRights></Rights><KeyName>app</KeyName><PrimaryKey>primary-key-value</PrimaryKey><SecondaryKey>secondary-key-value</SecondaryKey>"),
            "{body}"
        );
        let ops = body
            .split("<KeyName>ops</KeyName>")
            .nth(1)
            .expect("ops is added");
        let key = |tag: &str| {
            ops.split(&format!("<{tag}>"))
                .nth(1)
                .and_then(|s| s.split('<').next())
                .unwrap_or_default()
                .to_string()
        };
        let (p, s) = (key("PrimaryKey"), key("SecondaryKey"));
        assert!(
            p.len() >= 40 && s.len() >= 40 && p != s,
            "fresh keys: {p} {s}"
        );
        assert!(!body.contains("<KeyName>ops</KeyName><PrimaryKey>primary-key-value"));
        assert_eq!(detail, "updated (--overwrite)");
    }

    #[tokio::test]
    async fn import_overwrite_takes_the_keys_the_file_holds() {
        let (body, detail) = overwrite_orders_with(&orders_file(
            "<SharedAccessAuthorizationRule><Rights><Right>Listen</Right></Rights><KeyName>app</KeyName><PrimaryKey>file-primary</PrimaryKey><SecondaryKey>file-secondary</SecondaryKey></SharedAccessAuthorizationRule>",
        ))
        .await;
        assert!(
            body.contains("<KeyName>app</KeyName><PrimaryKey>file-primary</PrimaryKey><SecondaryKey>file-secondary</SecondaryKey>"),
            "{body}"
        );
        assert!(!body.contains("primary-key-value"));
        assert_eq!(
            detail,
            "updated (--overwrite); the keys of policy app were replaced from the file"
        );
    }

    #[tokio::test]
    async fn export_collects_a_selected_subscription_under_its_topic() {
        use crate::mgmt::incumbent::ExportEntity;
        let mut server = mockito::Server::new_async().await;
        let _topic = server
            .mock("GET", "/events")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/topic_entry.xml"))
            .create_async()
            .await;
        let _sub = server
            .mock("GET", "/events/Subscriptions/billing")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/subscription_entry.xml"))
            .create_async()
            .await;
        let _rules = server
            .mock("GET", "/events/Subscriptions/billing/Rules")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/rules_feed.xml"))
            .create_async()
            .await;
        let (gate, _) = fake_gate();
        let op = operator(&server, gate);
        let got = op
            .explorer()
            .collect_for_export(&[EntityRef::subscription("events", "billing")])
            .await
            .unwrap();
        let [
            ExportEntity::Topic {
                name,
                subscriptions,
                ..
            },
        ] = got.as_slice()
        else {
            panic!("{got:#?}")
        };
        assert_eq!(name, "events");
        assert_eq!(subscriptions.len(), 1);
        assert_eq!(subscriptions[0].rules.len(), 3);
        assert_eq!(subscriptions[0].rules[0].filter, RuleFilter::True);
    }
}
