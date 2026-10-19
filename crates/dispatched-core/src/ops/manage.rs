//! Gated entity management: create, update, delete, status, duplicate, rename, rules and SAS
//! policies. Each change is one `Mutation::EntityChange` operation through the safety gate;
//! everything that can be refused without changing anything is refused before it.

use super::{Operator, gated};
use crate::diagnostics::error_kind;
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, NoProgress};
use crate::mgmt::policies::{self, KeySlot};
use crate::mgmt::{
    AccessRight, Description, EntityCounts, EntityKind, EntityRef, EntityStatus, ManagementClient,
    QueueSettings, RuleSpec, SubscriptionSettings, TopicSettings, validate_filter,
};
use crate::safety::{CountEstimate, DeleteTarget, Operation, count_text};
use crate::{Error, Result};

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
    let first = rules.first().map(|r| r.settings.clone());
    m.create_subscription_with_rule(to_topic, to_name, &d, first.as_ref())
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
    /// delete would lose them.
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
                let item = match delete_one(m, &source).await {
                    Ok(()) => succeeded(Some(format!("deleted after the copy to {new_name}"))),
                    Err(e) => {
                        let reason = format!(
                            "{} was created; {} could not be deleted ({e}); delete one of them",
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
}
