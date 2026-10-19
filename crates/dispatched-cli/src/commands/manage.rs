//! `dispatched entity …`, `rule add|rm` and `policy …`: managing queues, topics,
//! subscriptions, rules and SAS policies. Every change goes through the operator's gate; the
//! reads (`entity export`, `policy ls`, `policy keys`) do not.

use super::browse::{check_output, explorer_for};
use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, StderrProgress, finish_job};
use crate::output::{sanitize, table};
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned};
use anyhow::Context as _;
use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use dispatched_core::messaging::parse_duration;
use dispatched_core::mgmt::incumbent::{ImportEntity, export_entities, parse_entities};
use dispatched_core::mgmt::policies::views;
use dispatched_core::mgmt::{
    AccessPolicy, AccessRight, CorrelationProperty, Description, EntityKind, EntityRef,
    EntityStatus, KeySlot, PolicyView, QueueSettings, RuleFilter, RuleSpec, SubscriptionSettings,
    TimeSpan, TopicSettings,
};
use dispatched_core::{EntityPath, EntitySpec, Explorer, ExportFormat, Mutation};
use secrecy::ExposeSecret;
use std::path::{Path, PathBuf};
use std::time::Duration;

const fn change(verb: &'static str) -> Planned {
    Planned {
        kind: Mutation::EntityChange,
        verb,
    }
}

/// `entity create`.
pub const CREATE: Planned = change("create-entity");
/// `entity update`.
pub const UPDATE: Planned = change("update-entity");
/// `entity delete`.
pub const DELETE: Planned = change("delete-entity");
/// `entity status`.
pub const STATUS: Planned = change("set-status");
/// `entity duplicate`.
pub const DUPLICATE: Planned = change("duplicate");
/// `entity rename`.
pub const RENAME: Planned = change("rename");
/// `entity import`.
pub const IMPORT: Planned = change("import-entities");
/// `rule add`.
pub const ADD_RULE: Planned = change("add-rule");
/// `rule rm`.
pub const REMOVE_RULE: Planned = change("remove-rule");
/// `policy add`.
pub const ADD_POLICY: Planned = change("add-policy");
/// `policy rm`.
pub const REMOVE_POLICY: Planned = change("remove-policy");
/// `policy renew`.
pub const RENEW_KEY: Planned = change("regenerate-key");

// ---------------------------------------------------------------------------------------------
// Arguments

/// `entity …`.
#[derive(Subcommand, Debug)]
pub enum EntityCmd {
    /// Create a queue, topic or subscription.
    Create(Box<CreateArgs>),
    /// Change settings of a queue, topic or subscription; only the given flags change.
    Update(Box<UpdateArgs>),
    /// Delete queues, topics or subscriptions with everything they hold.
    Delete(DeleteArgs),
    /// Set an entity's status.
    Status(StatusArgs),
    /// Create a copy of a queue (settings and policies, new keys) or a subscription (settings
    /// and rules). Messages are not copied.
    Duplicate(CopyArgs),
    /// Rename an empty queue or topic: create the copy, then delete the original.
    Rename(CopyArgs),
    /// Write entities to a ServiceBusExplorer-compatible XML file (read-only).
    Export(ExportArgs),
    /// Create the entities of a ServiceBusExplorer XML file.
    Import(ImportArgs),
}

/// What `entity create` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    Queue,
    Topic,
    Subscription,
}

/// Settings flags of `entity create` and `entity update`. On update only the flags given
/// change; the rest of the entity is read and written back as it is.
#[derive(Args, Debug, Default, Clone)]
pub struct SettingsArgs {
    /// How long a received message stays locked (queues, subscriptions): 30s, 1m, 5m.
    #[arg(long, value_name = "DURATION")]
    pub lock_duration: Option<String>,
    /// Maximum size in megabytes (queues, topics).
    #[arg(long, value_name = "MB", value_parser = clap::value_parser!(u64).range(1..))]
    pub max_size: Option<u64>,
    /// Default time to live of a message: 14d, 1h30m, ISO 8601, or `max`.
    #[arg(long, value_name = "DURATION")]
    pub ttl: Option<String>,
    /// Delete the entity after it has been idle this long: 7d, or `max` (never).
    #[arg(long, value_name = "DURATION")]
    pub auto_delete_on_idle: Option<String>,
    /// Detect duplicate MessageIds within this window (queues, topics). On create it turns
    /// duplicate detection on; on update it changes the window of an entity that has it.
    #[arg(long, value_name = "DURATION")]
    pub duplicate_detection: Option<String>,
    /// Deliveries before a message is dead-lettered (queues, subscriptions).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub max_delivery_count: Option<u32>,
    /// Forward every message to this queue or topic; `""` stops forwarding (update).
    #[arg(long, value_name = "ENTITY")]
    pub forward_to: Option<String>,
    /// Forward dead-lettered messages to this queue or topic; `""` stops it (update).
    #[arg(long = "forward-dlq-to", value_name = "ENTITY")]
    pub forward_dlq_to: Option<String>,
    /// Require sessions (create only; queues, subscriptions). `--sessions=false` to say no.
    #[arg(long, value_name = "BOOL", num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub sessions: Option<bool>,
    /// Partition the entity (create only; queues, topics).
    #[arg(long, value_name = "BOOL", num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub partitioning: Option<bool>,
    /// Batch store operations (the service default).
    #[arg(long, conflicts_with = "no_batched")]
    pub batched: bool,
    /// Do not batch store operations.
    #[arg(long)]
    pub no_batched: bool,
    /// Dead-letter messages that expire (queues, subscriptions). `=false` to turn it off.
    #[arg(long, value_name = "BOOL", num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub dead_letter_on_expiration: Option<bool>,
    /// User metadata (free text); `""` clears it (update).
    #[arg(long, value_name = "TEXT")]
    pub metadata: Option<String>,
}

/// `entity create`.
#[derive(Args, Debug)]
pub struct CreateArgs {
    /// What to create.
    #[arg(value_enum)]
    pub kind: Kind,
    /// Its name; a subscription is `topic/subscriptions/name`.
    pub name: String,
    #[command(flatten)]
    pub settings: SettingsArgs,
    /// Subscriptions: the initial rule `$Default` with this SQL filter (instead of matching
    /// every message).
    #[arg(long, value_name = "EXPR", conflicts_with_all = ["rule_correlation", "rule_property"])]
    pub rule_sql: Option<String>,
    /// Subscriptions: the initial rule `$Default` with a correlation filter, FIELD=VALUE
    /// (correlation-id, message-id, to, reply-to, subject, session-id, reply-to-session-id,
    /// content-type); repeat for several.
    #[arg(long, value_name = "FIELD=VALUE")]
    pub rule_correlation: Vec<String>,
    /// Subscriptions: a user property the initial correlation filter matches, KEY=VALUE.
    #[arg(long, value_name = "KEY=VALUE")]
    pub rule_property: Vec<String>,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `entity update`.
#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// `queue`, `topic` or `topic/subscriptions/subscription`.
    pub entity: String,
    #[command(flatten)]
    pub settings: SettingsArgs,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `entity delete`.
#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// Queues, topics or `topic/subscriptions/subscription`s.
    #[arg(required = true)]
    pub entities: Vec<String>,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `entity status`.
#[derive(Args, Debug)]
pub struct StatusArgs {
    /// `queue`, `topic` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// active, disabled, send-disabled or receive-disabled.
    #[arg(value_parser = parse_status)]
    pub status: EntityStatus,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `entity duplicate` and `entity rename`.
#[derive(Args, Debug)]
pub struct CopyArgs {
    /// `queue`, `topic` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The new name (a subscription stays under its topic).
    pub new_name: String,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `entity export`.
#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Queues, topics or subscriptions to export; none means every queue and topic.
    pub entities: Vec<String>,
    /// Output file, or `-` for stdout.
    #[arg(long)]
    pub out: String,
    /// Write the SAS keys of the policies too (the file then holds secrets).
    #[arg(long)]
    pub include_keys: bool,
    /// Replace the output file if it exists.
    #[arg(long)]
    pub force: bool,
}

/// `entity import`.
#[derive(Args, Debug)]
pub struct ImportArgs {
    /// A ServiceBusExplorer entity file (`<Entities>`).
    pub file: PathBuf,
    /// Replace existing entities with the file's settings (otherwise they are skipped).
    #[arg(long)]
    pub overwrite: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `rule add`.
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("filter").required(true).multiple(true).args(["sql", "correlation", "property"])))]
pub struct RuleAddArgs {
    /// `topic/subscriptions/subscription`.
    pub subscription: String,
    /// The rule name.
    pub name: String,
    /// A SQL filter, e.g. `user.region = 'eu'` (`1=1` matches every message).
    #[arg(long, value_name = "EXPR", conflicts_with_all = ["correlation", "property"])]
    pub sql: Option<String>,
    /// A correlation filter field, FIELD=VALUE (correlation-id, message-id, to, reply-to,
    /// subject, session-id, reply-to-session-id, content-type); repeat for several.
    #[arg(long, value_name = "FIELD=VALUE")]
    pub correlation: Vec<String>,
    /// A user property the correlation filter matches, KEY=VALUE; repeat for several.
    #[arg(long, value_name = "KEY=VALUE")]
    pub property: Vec<String>,
    /// A SQL action run on matching messages, e.g. `SET sys.Label = 'eu'`.
    #[arg(long, value_name = "SQL")]
    pub action: Option<String>,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `rule rm`.
#[derive(Args, Debug)]
pub struct RuleRmArgs {
    /// `topic/subscriptions/subscription`.
    pub subscription: String,
    /// The rule name.
    pub name: String,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `policy …`.
#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    /// List the SAS policies of a queue or topic (names and rights, never keys).
    Ls {
        /// Queue or topic.
        entity: String,
    },
    /// Add a SAS policy with new keys.
    Add(PolicyAddArgs),
    /// Remove a SAS policy.
    Rm(PolicyNameArgs),
    /// Replace one key of a SAS policy with a new one.
    Renew(PolicyRenewArgs),
    /// Print the keys of a SAS policy (secrets: shown only on this request).
    Keys {
        /// Queue or topic.
        entity: String,
        /// The policy name.
        name: String,
    },
}

/// `policy add`.
#[derive(Args, Debug)]
pub struct PolicyAddArgs {
    /// Queue or topic.
    pub entity: String,
    /// The policy name.
    pub name: String,
    /// listen, send and manage, comma separated (manage needs listen and send).
    #[arg(long, value_delimiter = ',', required = true, value_parser = parse_right)]
    pub rights: Vec<AccessRight>,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `policy rm`.
#[derive(Args, Debug)]
pub struct PolicyNameArgs {
    /// Queue or topic.
    pub entity: String,
    /// The policy name.
    pub name: String,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// Which key `policy renew` replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Slot {
    Primary,
    Secondary,
}

impl From<Slot> for KeySlot {
    fn from(s: Slot) -> Self {
        match s {
            Slot::Primary => KeySlot::Primary,
            Slot::Secondary => KeySlot::Secondary,
        }
    }
}

/// `policy renew`.
#[derive(Args, Debug)]
pub struct PolicyRenewArgs {
    /// Queue or topic.
    pub entity: String,
    /// The policy name.
    pub name: String,
    /// The key to replace.
    #[arg(value_enum)]
    pub slot: Slot,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

fn parse_status(s: &str) -> Result<EntityStatus, String> {
    s.parse().map_err(|e: dispatched_core::Error| e.to_string())
}

fn parse_right(s: &str) -> Result<AccessRight, String> {
    s.parse().map_err(|e: dispatched_core::Error| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Settings

/// A duration flag: the `parse_duration` syntax, or `max` / `never` for the longest.
fn duration(flag: &str, text: &str) -> anyhow::Result<Duration> {
    if matches!(text.trim().to_ascii_lowercase().as_str(), "max" | "never") {
        return Ok(TimeSpan::MAX);
    }
    parse_duration(text).with_context(|| format!("--{flag}"))
}

fn opt_duration(flag: &str, text: &Option<String>) -> anyhow::Result<Option<Duration>> {
    text.as_deref().map(|t| duration(flag, t)).transpose()
}

/// A name flag where `""` means "none".
fn name_or_clear(v: &Option<String>) -> Option<Option<String>> {
    v.as_ref().map(|s| {
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    })
}

impl SettingsArgs {
    /// `Some(true)` for `--batched`, `Some(false)` for `--no-batched`.
    fn batched(&self) -> Option<bool> {
        match (self.batched, self.no_batched) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }

    /// The flags given that `kind` does not have, as `--flag` names.
    fn not_for(&self, kind: EntityKind) -> Vec<&'static str> {
        let given = [
            ("--lock-duration", self.lock_duration.is_some()),
            ("--max-size", self.max_size.is_some()),
            ("--duplicate-detection", self.duplicate_detection.is_some()),
            ("--max-delivery-count", self.max_delivery_count.is_some()),
            ("--forward-to", self.forward_to.is_some()),
            ("--forward-dlq-to", self.forward_dlq_to.is_some()),
            ("--sessions", self.sessions.is_some()),
            ("--partitioning", self.partitioning.is_some()),
            (
                "--dead-letter-on-expiration",
                self.dead_letter_on_expiration.is_some(),
            ),
        ];
        let missing: &[&str] = match kind {
            EntityKind::Queue => &[],
            EntityKind::Topic => &[
                "--lock-duration",
                "--max-delivery-count",
                "--forward-to",
                "--forward-dlq-to",
                "--sessions",
                "--dead-letter-on-expiration",
            ],
            EntityKind::Subscription => &["--max-size", "--duplicate-detection", "--partitioning"],
        };
        given
            .into_iter()
            .filter(|(flag, set)| *set && missing.contains(flag))
            .map(|(flag, _)| flag)
            .collect()
    }

    /// Refuse flags `kind` does not have and, on update, the ones fixed at creation.
    fn check(&self, kind: EntityKind, create: bool) -> anyhow::Result<()> {
        let wrong = self.not_for(kind);
        if !wrong.is_empty() {
            anyhow::bail!("{} does not apply to a {}", wrong.join(", "), kind.as_str());
        }
        if !create {
            let fixed: Vec<&str> = [
                ("--sessions", self.sessions.is_some()),
                ("--partitioning", self.partitioning.is_some()),
            ]
            .into_iter()
            .filter(|(_, set)| *set)
            .map(|(f, _)| f)
            .collect();
            if !fixed.is_empty() {
                anyhow::bail!(
                    "{} cannot be changed after the entity is created; create a new one (`entity \
                     duplicate` copies the other settings)",
                    fixed.join(", ")
                );
            }
        }
        Ok(())
    }

    /// Whether any setting is given.
    fn any(&self) -> bool {
        self.lock_duration.is_some()
            || self.max_size.is_some()
            || self.ttl.is_some()
            || self.auto_delete_on_idle.is_some()
            || self.duplicate_detection.is_some()
            || self.max_delivery_count.is_some()
            || self.forward_to.is_some()
            || self.forward_dlq_to.is_some()
            || self.sessions.is_some()
            || self.partitioning.is_some()
            || self.batched().is_some()
            || self.dead_letter_on_expiration.is_some()
            || self.metadata.is_some()
    }

    /// Put the given flags on a queue's settings (`create`: a new queue).
    pub fn apply_queue(&self, s: &mut QueueSettings, create: bool) -> anyhow::Result<()> {
        self.check(EntityKind::Queue, create)?;
        set(
            &mut s.lock_duration,
            opt_duration("lock-duration", &self.lock_duration)?,
        );
        set(&mut s.max_size_in_megabytes, self.max_size);
        set(
            &mut s.default_message_time_to_live,
            opt_duration("ttl", &self.ttl)?,
        );
        set(
            &mut s.auto_delete_on_idle,
            opt_duration("auto-delete-on-idle", &self.auto_delete_on_idle)?,
        );
        if let Some(w) = opt_duration("duplicate-detection", &self.duplicate_detection)? {
            duplicate_window(
                &mut s.requires_duplicate_detection,
                &mut s.duplicate_detection_history_time_window,
                w,
                create,
            )?;
        }
        set(&mut s.max_delivery_count, self.max_delivery_count);
        if let Some(v) = name_or_clear(&self.forward_to) {
            s.forward_to = v;
        }
        if let Some(v) = name_or_clear(&self.forward_dlq_to) {
            s.forward_dead_lettered_messages_to = v;
        }
        set(&mut s.requires_session, self.sessions);
        set(&mut s.enable_partitioning, self.partitioning);
        set(&mut s.enable_batched_operations, self.batched());
        set(
            &mut s.dead_lettering_on_message_expiration,
            self.dead_letter_on_expiration,
        );
        if let Some(v) = metadata(&self.metadata) {
            s.user_metadata = v;
        }
        Ok(())
    }

    /// Put the given flags on a topic's settings.
    pub fn apply_topic(&self, s: &mut TopicSettings, create: bool) -> anyhow::Result<()> {
        self.check(EntityKind::Topic, create)?;
        set(&mut s.max_size_in_megabytes, self.max_size);
        set(
            &mut s.default_message_time_to_live,
            opt_duration("ttl", &self.ttl)?,
        );
        set(
            &mut s.auto_delete_on_idle,
            opt_duration("auto-delete-on-idle", &self.auto_delete_on_idle)?,
        );
        if let Some(w) = opt_duration("duplicate-detection", &self.duplicate_detection)? {
            duplicate_window(
                &mut s.requires_duplicate_detection,
                &mut s.duplicate_detection_history_time_window,
                w,
                create,
            )?;
        }
        set(&mut s.enable_partitioning, self.partitioning);
        set(&mut s.enable_batched_operations, self.batched());
        if let Some(v) = metadata(&self.metadata) {
            s.user_metadata = v;
        }
        Ok(())
    }

    /// Put the given flags on a subscription's settings.
    pub fn apply_subscription(
        &self,
        s: &mut SubscriptionSettings,
        create: bool,
    ) -> anyhow::Result<()> {
        self.check(EntityKind::Subscription, create)?;
        set(
            &mut s.lock_duration,
            opt_duration("lock-duration", &self.lock_duration)?,
        );
        set(
            &mut s.default_message_time_to_live,
            opt_duration("ttl", &self.ttl)?,
        );
        set(
            &mut s.auto_delete_on_idle,
            opt_duration("auto-delete-on-idle", &self.auto_delete_on_idle)?,
        );
        set(&mut s.max_delivery_count, self.max_delivery_count);
        if let Some(v) = name_or_clear(&self.forward_to) {
            s.forward_to = v;
        }
        if let Some(v) = name_or_clear(&self.forward_dlq_to) {
            s.forward_dead_lettered_messages_to = v;
        }
        set(&mut s.requires_session, self.sessions);
        set(&mut s.enable_batched_operations, self.batched());
        set(
            &mut s.dead_lettering_on_message_expiration,
            self.dead_letter_on_expiration,
        );
        if let Some(v) = metadata(&self.metadata) {
            s.user_metadata = v;
        }
        Ok(())
    }
}

/// Set `field` when the flag was given.
fn set<T>(field: &mut Option<T>, flag: Option<T>) {
    if flag.is_some() {
        *field = flag;
    }
}

/// `--metadata`: `""` clears.
fn metadata(v: &Option<String>) -> Option<Option<String>> {
    v.as_ref().map(|s| (!s.is_empty()).then(|| s.clone()))
}

/// `--duplicate-detection`: on create, detection on with this window; on update, the window
/// of an entity that has detection (it cannot be turned on later).
fn duplicate_window(
    requires: &mut Option<bool>,
    window: &mut Option<Duration>,
    w: Duration,
    create: bool,
) -> anyhow::Result<()> {
    if create {
        *requires = Some(true);
    } else if *requires != Some(true) {
        anyhow::bail!(
            "duplicate detection is off on this entity and can only be turned on when it is \
             created"
        );
    }
    *window = Some(w);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Rules

/// One correlation field by its flag name (any case; `-` and `_` ignored; `label` for subject).
fn correlation_field<'a>(
    filter: &'a mut RuleFilter,
    name: &str,
) -> anyhow::Result<&'a mut Option<String>> {
    let RuleFilter::Correlation {
        correlation_id,
        message_id,
        to,
        reply_to,
        subject,
        session_id,
        reply_to_session_id,
        content_type,
        ..
    } = filter
    else {
        unreachable!("built as a correlation filter")
    };
    let key: String = name
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    Ok(match key.as_str() {
        "correlationid" => correlation_id,
        "messageid" => message_id,
        "to" => to,
        "replyto" => reply_to,
        "subject" | "label" => subject,
        "sessionid" => session_id,
        "replytosessionid" => reply_to_session_id,
        "contenttype" => content_type,
        _ => anyhow::bail!(
            "`{name}` is not a correlation field; use correlation-id, message-id, to, reply-to, \
             subject, session-id, reply-to-session-id or content-type (user properties go in \
             --property)"
        ),
    })
}

fn pair(arg: &str) -> anyhow::Result<(&str, &str)> {
    match arg.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => Ok((k.trim(), v)),
        _ => anyhow::bail!("`{arg}` is not KEY=VALUE"),
    }
}

/// A correlation filter from `FIELD=VALUE` fields and `KEY=VALUE` user properties.
pub fn correlation_filter(fields: &[String], properties: &[String]) -> anyhow::Result<RuleFilter> {
    let mut filter = RuleFilter::Correlation {
        correlation_id: None,
        message_id: None,
        to: None,
        reply_to: None,
        subject: None,
        session_id: None,
        reply_to_session_id: None,
        content_type: None,
        properties: Vec::new(),
    };
    for f in fields {
        let (k, v) = pair(f)?;
        *correlation_field(&mut filter, k)? = Some(v.to_string());
    }
    if let RuleFilter::Correlation { properties: p, .. } = &mut filter {
        for arg in properties {
            let (k, v) = pair(arg)?;
            p.push(CorrelationProperty {
                name: k.to_string(),
                value: v.to_string(),
            });
        }
    }
    Ok(filter)
}

/// The rule `rule add` describes.
pub fn rule_spec(a: &RuleAddArgs) -> anyhow::Result<RuleSpec> {
    let filter = match &a.sql {
        Some(sql) => RuleFilter::Sql {
            expression: sql.clone(),
        },
        None => correlation_filter(&a.correlation, &a.property)?,
    };
    Ok(RuleSpec {
        name: a.name.clone(),
        filter,
        action: a.action.clone(),
    })
}

/// The initial rule of `entity create subscription`, if a filter flag is given.
fn initial_rule(a: &CreateArgs) -> anyhow::Result<Option<RuleSpec>> {
    let filter = match &a.rule_sql {
        Some(sql) => RuleFilter::Sql {
            expression: sql.clone(),
        },
        None if a.rule_correlation.is_empty() && a.rule_property.is_empty() => return Ok(None),
        None => correlation_filter(&a.rule_correlation, &a.rule_property)?,
    };
    Ok(Some(RuleSpec {
        name: "$Default".into(),
        filter,
        action: None,
    }))
}

// ---------------------------------------------------------------------------------------------
// Entities

/// The entity `entity create` describes, with its settings (no I/O).
pub fn create_spec(a: &CreateArgs) -> anyhow::Result<EntitySpec> {
    let rule = initial_rule(a)?;
    if rule.is_some() && a.kind != Kind::Subscription {
        anyhow::bail!("--rule-sql, --rule-correlation and --rule-property are for subscriptions");
    }
    Ok(match a.kind {
        Kind::Queue => {
            let mut s = QueueSettings::default();
            a.settings.apply_queue(&mut s, true)?;
            EntitySpec::Queue(plain_name(&a.name)?, Description::new(s))
        }
        Kind::Topic => {
            let mut s = TopicSettings::default();
            a.settings.apply_topic(&mut s, true)?;
            EntitySpec::Topic(plain_name(&a.name)?, Description::new(s))
        }
        Kind::Subscription => {
            let EntityRef::Subscription { topic, name } =
                EntityRef::parse(EntityKind::Subscription, &a.name)?
            else {
                unreachable!("parsed as a subscription")
            };
            let mut s = SubscriptionSettings::default();
            a.settings.apply_subscription(&mut s, true)?;
            EntitySpec::Subscription {
                topic,
                name,
                d: Description::new(s),
                rule,
            }
        }
    })
}

/// A queue or topic name: no `/subscriptions/`.
fn plain_name(name: &str) -> anyhow::Result<String> {
    let p = EntityPath::parse(name)?;
    if p.subscription.is_some() {
        anyhow::bail!("`{name}` is a subscription path; use `entity create subscription`");
    }
    Ok(p.entity)
}

/// The entity path a create, of `kind` named `name`, makes (for the production check).
fn created_path(a: &CreateArgs) -> anyhow::Result<EntityPath> {
    Ok(create_spec(a)?.entity().entity_path())
}

/// The path of `new_name` next to `source` (same topic for a subscription).
fn sibling(source: &EntityPath, new_name: &str) -> anyhow::Result<EntityPath> {
    if new_name.contains('/') || new_name.trim().is_empty() {
        anyhow::bail!("give the new name only, without a path (`{new_name}`)");
    }
    Ok(match &source.subscription {
        Some(_) => EntityPath::subscription(&source.entity, new_name),
        None => EntityPath::queue(new_name),
    })
}

/// A queue, topic or subscription at `path`; the kind of a queue or topic is asked of the
/// management API.
async fn entity_ref(ex: &Explorer, path: &EntityPath) -> anyhow::Result<EntityRef> {
    if let Some(sub) = &path.subscription {
        return Ok(EntityRef::subscription(&path.entity, sub));
    }
    Ok(
        match ex.management_client().entity_kind(&path.entity).await? {
            EntityKind::Topic => EntityRef::topic(&path.entity),
            _ => EntityRef::queue(&path.entity),
        },
    )
}

/// The entity's description with the given flags on top, as an update.
async fn update_spec(
    ex: &Explorer,
    entity: &EntityRef,
    flags: &SettingsArgs,
) -> anyhow::Result<EntitySpec> {
    let m = ex.management_client();
    Ok(match entity {
        EntityRef::Queue { name } => {
            let mut d = m.get_queue_description(name).await?;
            flags.apply_queue(&mut d.settings, false)?;
            EntitySpec::Queue(name.clone(), d)
        }
        EntityRef::Topic { name } => {
            let mut d = m.get_topic_description(name).await?;
            flags.apply_topic(&mut d.settings, false)?;
            EntitySpec::Topic(name.clone(), d)
        }
        EntityRef::Subscription { topic, name } => {
            let mut d = m.get_subscription(topic, name).await?;
            flags.apply_subscription(&mut d.settings, false)?;
            EntitySpec::Subscription {
                topic: topic.clone(),
                name: name.clone(),
                d,
                rule: None,
            }
        }
    })
}

fn parse_paths(entities: &[String]) -> anyhow::Result<Vec<EntityPath>> {
    entities.iter().map(|e| Ok(EntityPath::parse(e)?)).collect()
}

pub async fn entity(
    ctx: &Ctx,
    sel: &Selection<'_>,
    cmd: &EntityCmd,
    format: Format,
) -> anyhow::Result<JobExit> {
    match cmd {
        EntityCmd::Create(a) => {
            let spec = create_spec(a)?;
            let path = created_path(a)?;
            let (operator, _cancel) =
                operator_for(ctx, sel, &a.mutate, CREATE, std::slice::from_ref(&path))?;
            let result = operator.create_entity(spec).await;
            operator.explorer().close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
        EntityCmd::Update(a) => {
            if !a.settings.any() {
                anyhow::bail!("give at least one setting to change");
            }
            let path = EntityPath::parse(&a.entity)?;
            let (operator, _cancel) =
                operator_for(ctx, sel, &a.mutate, UPDATE, std::slice::from_ref(&path))?;
            let ex = operator.explorer();
            let result = async {
                let target = entity_ref(ex, &path).await?;
                let spec = update_spec(ex, &target, &a.settings).await?;
                Ok::<_, anyhow::Error>(operator.update_entity(spec).await?)
            }
            .await;
            ex.close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
        EntityCmd::Delete(a) => {
            let paths = parse_paths(&a.entities)?;
            let (operator, _cancel) = operator_for(ctx, sel, &a.mutate, DELETE, &paths)?;
            let ex = operator.explorer();
            let result = async {
                let mut targets = Vec::new();
                for p in &paths {
                    targets.push(entity_ref(ex, p).await?);
                }
                Ok::<_, anyhow::Error>(operator.delete_entities(targets).await?)
            }
            .await;
            ex.close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
        EntityCmd::Status(a) => {
            let path = EntityPath::parse(&a.entity)?;
            let (operator, _cancel) =
                operator_for(ctx, sel, &a.mutate, STATUS, std::slice::from_ref(&path))?;
            let ex = operator.explorer();
            let result = async {
                let target = entity_ref(ex, &path).await?;
                Ok::<_, anyhow::Error>(operator.set_status(target, a.status).await?)
            }
            .await;
            ex.close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
        EntityCmd::Duplicate(a) | EntityCmd::Rename(a) => {
            let rename = matches!(cmd, EntityCmd::Rename(_));
            let source = EntityPath::parse(&a.entity)?;
            let target = sibling(&source, &a.new_name)?;
            // Production confirms every entity the change touches: the copy, and for a
            // rename the original it deletes.
            let (planned, touched) = if rename {
                (RENAME, vec![source.clone(), target])
            } else {
                (DUPLICATE, vec![target])
            };
            let (operator, _cancel) = operator_for(ctx, sel, &a.mutate, planned, &touched)?;
            let ex = operator.explorer();
            let result = async {
                let from = entity_ref(ex, &source).await?;
                let new_name = a.new_name.trim().to_string();
                Ok::<_, anyhow::Error>(if rename {
                    operator.rename(from, new_name).await?
                } else {
                    operator.duplicate(from, new_name).await?
                })
            }
            .await;
            ex.close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
        EntityCmd::Export(a) => {
            export(ctx, sel, a, format).await?;
            Ok(JobExit::Ok)
        }
        EntityCmd::Import(a) => {
            let items = read_import(&a.file)?;
            let paths: Vec<EntityPath> = items.iter().map(|i| i.entity().entity_path()).collect();
            let (operator, cancel) = operator_for(ctx, sel, &a.mutate, IMPORT, &paths)?;
            let result = operator
                .import_entities(items, a.overwrite, &StderrProgress::new(), &cancel)
                .await;
            operator.explorer().close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
    }
}

/// Read and parse an entity file (before connecting: a bad file changes nothing).
fn read_import(file: &Path) -> anyhow::Result<Vec<ImportEntity>> {
    let shown = sanitize(&file.display().to_string());
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {shown}"))?;
    parse_entities(&text).with_context(|| format!("reading {shown}"))
}

/// The namespace name of a host: `myns` for `myns.servicebus.windows.net`.
fn namespace_name(host: &str) -> &str {
    host.split('.').next().unwrap_or(host)
}

/// The warning `--include-keys` prints.
pub const KEYS_WARNING: &str =
    "warning: the file holds SAS keys; anyone who reads it can use them. Keep it secret";

async fn export(
    ctx: &Ctx,
    sel: &Selection<'_>,
    a: &ExportArgs,
    format: Format,
) -> anyhow::Result<()> {
    // Before any connection.
    check_output(&a.out, a.force)?;
    let paths = parse_paths(&a.entities)?;
    let ex = explorer_for(ctx, sel)?;
    let result = async {
        let mut refs = Vec::new();
        for p in &paths {
            refs.push(entity_ref(&ex, p).await?);
        }
        Ok::<_, anyhow::Error>(ex.collect_for_export(&refs).await?)
    }
    .await;
    let ns = namespace_name(ex.namespace().host()).to_string();
    ex.close().await;
    let entities = result?;
    let xml = export_entities(&ns, &entities, a.include_keys)?;
    if a.include_keys {
        eprintln!("{KEYS_WARNING}");
    }
    let count: usize = entities
        .iter()
        .map(|e| match e {
            dispatched_core::mgmt::incumbent::ExportEntity::Queue { .. } => 1,
            dispatched_core::mgmt::incumbent::ExportEntity::Topic { subscriptions, .. } => {
                1 + subscriptions.len()
            }
        })
        .sum();
    if a.out == "-" {
        print!("{xml}");
        return Ok(());
    }
    let path = Path::new(&a.out);
    // With keys, the file is owner-only from its first byte (the temp file included).
    let mut file = if a.include_keys {
        dispatched_core::FileExport::create_private(path, ExportFormat::Json, a.force)?
    } else {
        dispatched_core::FileExport::create(path, ExportFormat::Json, a.force)?
    };
    file.write_raw(xml.as_bytes())?;
    file.finish()?;
    let line = format!(
        "Exported {count} {} to {}",
        if count == 1 { "entity" } else { "entities" },
        sanitize(&a.out)
    );
    match format {
        Format::Json => println!(
            "{}",
            serde_json::json!({"entities": count, "out": a.out, "include_keys": a.include_keys})
        ),
        _ => println!("{line}"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Rules and policies

fn subscription_ref(path: &str) -> anyhow::Result<EntityRef> {
    Ok(EntityRef::parse(EntityKind::Subscription, path)?)
}

pub async fn rule_add(
    ctx: &Ctx,
    sel: &Selection<'_>,
    a: &RuleAddArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let sub = subscription_ref(&a.subscription)?;
    let rule = rule_spec(a)?;
    dispatched_core::mgmt::validate_filter(&rule.filter)?;
    let (operator, _cancel) = operator_for(
        ctx,
        sel,
        &a.mutate,
        ADD_RULE,
        std::slice::from_ref(&sub.entity_path()),
    )?;
    let result = operator.add_rule(sub, rule).await;
    operator.explorer().close().await;
    Ok(finish_job(&result?, format, &a.mutate))
}

pub async fn rule_rm(
    ctx: &Ctx,
    sel: &Selection<'_>,
    a: &RuleRmArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let sub = subscription_ref(&a.subscription)?;
    let (operator, _cancel) = operator_for(
        ctx,
        sel,
        &a.mutate,
        REMOVE_RULE,
        std::slice::from_ref(&sub.entity_path()),
    )?;
    let result = operator.remove_rule(sub, a.name.clone()).await;
    operator.explorer().close().await;
    Ok(finish_job(&result?, format, &a.mutate))
}

/// A queue or topic path for a policy command; subscriptions have no policies.
fn policy_path(entity: &str) -> anyhow::Result<EntityPath> {
    let p = EntityPath::parse(entity)?;
    if p.subscription.is_some() {
        anyhow::bail!(
            "subscriptions have no SAS policies; use the policies of their topic or the namespace"
        );
    }
    Ok(p)
}

/// The policies of a queue or topic, keys included (callers decide what to show).
async fn read_policies(ex: &Explorer, path: &EntityPath) -> anyhow::Result<Vec<AccessPolicy>> {
    let m = ex.management_client();
    Ok(match entity_ref(ex, path).await? {
        EntityRef::Topic { name } => m.get_topic_description(&name).await?.policies,
        EntityRef::Queue { name } => m.get_queue_description(&name).await?.policies,
        EntityRef::Subscription { .. } => Vec::new(),
    })
}

fn rights_text(rights: &[AccessRight]) -> String {
    rights
        .iter()
        .map(|r| r.as_str().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `policy ls` output: names and rights only. Built from [`PolicyView`]s, which have no keys.
pub fn policies_output(policies: &[AccessPolicy], format: Format) -> anyhow::Result<String> {
    let list: Vec<PolicyView> = views(policies);
    Ok(match format {
        Format::Json | Format::Ndjson => serde_json::to_string_pretty(&list)?,
        Format::Table => {
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|p| vec![p.key_name.clone(), rights_text(&p.rights)])
                .collect();
            table(&["NAME", "RIGHTS"], &rows)
        }
    })
}

/// The warning `policy keys` prints on stderr.
pub const POLICY_KEYS_WARNING: &str =
    "warning: the output holds SAS keys; anyone who has them can use the policy's rights";

pub async fn policy(
    ctx: &Ctx,
    sel: &Selection<'_>,
    cmd: &PolicyCmd,
    format: Format,
) -> anyhow::Result<JobExit> {
    match cmd {
        PolicyCmd::Ls { entity } => {
            let path = policy_path(entity)?;
            let ex = explorer_for(ctx, sel)?;
            let result = read_policies(&ex, &path).await;
            ex.close().await;
            let policies = result?;
            println!("{}", policies_output(&policies, format)?);
            if policies.is_empty() && matches!(format, Format::Table) {
                eprintln!("(no policies)");
            }
            Ok(JobExit::Ok)
        }
        PolicyCmd::Keys { entity, name } => {
            let path = policy_path(entity)?;
            let ex = explorer_for(ctx, sel)?;
            let result = read_policies(&ex, &path).await;
            ex.close().await;
            let policies = result?;
            let p = policies
                .iter()
                .find(|p| p.key_name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    anyhow::anyhow!("{} has no policy {}", path.base(), sanitize(name))
                })?;
            eprintln!("{POLICY_KEYS_WARNING}");
            let (primary, secondary) = (
                p.primary_key.expose_secret(),
                p.secondary_key.expose_secret(),
            );
            match format {
                Format::Json | Format::Ndjson => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "key_name": p.key_name,
                        "rights": p.rights,
                        "primary_key": primary,
                        "secondary_key": secondary,
                    }))?
                ),
                Format::Table => println!(
                    "{}",
                    table(
                        &["NAME", "RIGHTS", "PRIMARY KEY", "SECONDARY KEY"],
                        &[vec![
                            p.key_name.clone(),
                            rights_text(&p.rights),
                            primary.to_string(),
                            secondary.to_string(),
                        ]],
                    )
                ),
            }
            Ok(JobExit::Ok)
        }
        PolicyCmd::Add(a) => {
            let change = PolicyChange::Add(a.name.clone(), a.rights.clone());
            run_policy(ctx, sel, &a.mutate, &a.entity, change, format).await
        }
        PolicyCmd::Rm(a) => {
            let change = PolicyChange::Remove(a.name.clone());
            run_policy(ctx, sel, &a.mutate, &a.entity, change, format).await
        }
        PolicyCmd::Renew(a) => {
            let change = PolicyChange::Renew(a.name.clone(), a.slot.into());
            run_policy(ctx, sel, &a.mutate, &a.entity, change, format).await
        }
    }
}

/// A change to the policies of a queue or topic.
enum PolicyChange {
    Add(String, Vec<AccessRight>),
    Remove(String),
    Renew(String, KeySlot),
}

impl PolicyChange {
    fn planned(&self) -> Planned {
        match self {
            PolicyChange::Add(..) => ADD_POLICY,
            PolicyChange::Remove(_) => REMOVE_POLICY,
            PolicyChange::Renew(..) => RENEW_KEY,
        }
    }
}

/// A policy change on a queue or topic: gate, resolve its kind, change it.
async fn run_policy(
    ctx: &Ctx,
    sel: &Selection<'_>,
    mutate: &MutateArgs,
    entity: &str,
    change: PolicyChange,
    format: Format,
) -> anyhow::Result<JobExit> {
    let path = policy_path(entity)?;
    let (operator, _cancel) = operator_for(
        ctx,
        sel,
        mutate,
        change.planned(),
        std::slice::from_ref(&path),
    )?;
    let ex = operator.explorer();
    let result = async {
        let target = entity_ref(ex, &path).await?;
        Ok::<_, anyhow::Error>(match change {
            PolicyChange::Add(name, rights) => operator.add_policy(target, name, rights).await?,
            PolicyChange::Remove(name) => operator.remove_policy(target, name).await?,
            PolicyChange::Renew(name, slot) => operator.regenerate_key(target, name, slot).await?,
        })
    }
    .await;
    ex.close().await;
    Ok(finish_job(&result?, format, mutate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use secrecy::SecretString;

    fn try_parse(args: &[&str]) -> Result<crate::Cli, clap::Error> {
        crate::Cli::try_parse_from(std::iter::once("dispatched").chain(args.iter().copied()))
    }

    fn entity_cmd(args: &[&str]) -> EntityCmd {
        match try_parse(args).unwrap().command {
            crate::Command::Entity { cmd } => cmd,
            _ => panic!("not entity"),
        }
    }

    fn create(args: &[&str]) -> CreateArgs {
        match entity_cmd(args) {
            EntityCmd::Create(a) => *a,
            _ => panic!("not create"),
        }
    }

    #[test]
    fn every_command_parses() {
        for ok in [
            &[
                "entity",
                "create",
                "queue",
                "orders",
                "--sessions",
                "--max-size",
                "2048",
            ][..],
            &[
                "entity",
                "create",
                "subscription",
                "t/subscriptions/s",
                "--rule-sql",
                "1=1",
            ],
            &["entity", "update", "orders", "--ttl", "7d", "--no-batched"],
            &["entity", "delete", "a", "t", "t/subscriptions/s", "--yes"],
            &["entity", "status", "orders", "send-disabled"],
            &["entity", "duplicate", "orders", "orders-copy"],
            &[
                "entity",
                "rename",
                "orders",
                "orders2",
                "--confirm-prod",
                "orders",
            ],
            &["entity", "export", "--out", "e.xml"],
            &[
                "entity",
                "export",
                "orders",
                "t",
                "--out",
                "-",
                "--include-keys",
                "--force",
            ],
            &["entity", "import", "e.xml", "--overwrite", "--yes"],
            &[
                "rule",
                "add",
                "t/subscriptions/s",
                "eu",
                "--sql",
                "user.region = 'eu'",
            ],
            &[
                "rule",
                "add",
                "t/subscriptions/s",
                "eu",
                "--correlation",
                "subject=eu",
                "--property",
                "region=eu",
                "--action",
                "SET sys.Label = 'x'",
            ],
            &["rule", "rm", "t/subscriptions/s", "eu"],
            &["policy", "ls", "orders"],
            &["policy", "add", "orders", "app", "--rights", "listen,send"],
            &["policy", "rm", "orders", "app"],
            &["policy", "renew", "orders", "app", "secondary"],
            &["policy", "keys", "orders", "app"],
        ] {
            assert!(try_parse(ok).is_ok(), "{ok:?}: {:?}", try_parse(ok).err());
        }
        for bad in [
            &["entity", "create", "exchange", "x"][..],
            &["entity", "delete"],
            &["entity", "status", "orders", "paused"],
            &["entity", "export"],
            &["entity", "update", "orders", "--batched", "--no-batched"],
            &[
                "entity",
                "create",
                "queue",
                "q",
                "--max-delivery-count",
                "0",
            ],
            &["rule", "add", "t/subscriptions/s", "eu"],
            &[
                "rule",
                "add",
                "t/subscriptions/s",
                "eu",
                "--sql",
                "1=1",
                "--correlation",
                "to=x",
            ],
            &["policy", "add", "orders", "app"],
            &["policy", "add", "orders", "app", "--rights", "listen,own"],
            &["policy", "renew", "orders", "app", "tertiary"],
        ] {
            assert!(try_parse(bad).is_err(), "{bad:?}");
        }
        // `--sessions` takes no value unless written `--sessions=false`, so a name after it
        // is the positional.
        let a = create(&["entity", "create", "queue", "--sessions", "orders"]);
        assert_eq!(
            (a.name.as_str(), a.settings.sessions),
            ("orders", Some(true))
        );
        let a = create(&["entity", "create", "queue", "orders", "--sessions=false"]);
        assert_eq!(a.settings.sessions, Some(false));
        let EntityCmd::Status(s) = entity_cmd(&["entity", "status", "orders", "receive-disabled"])
        else {
            panic!("not status")
        };
        assert_eq!(s.status, EntityStatus::ReceiveDisabled);
    }

    #[test]
    fn create_flags_map_to_settings() {
        let a = create(&[
            "entity",
            "create",
            "queue",
            "orders",
            "--lock-duration",
            "1m",
            "--max-size",
            "2048",
            "--ttl",
            "max",
            "--auto-delete-on-idle",
            "7d",
            "--duplicate-detection",
            "10m",
            "--max-delivery-count",
            "5",
            "--forward-to",
            "archive",
            "--forward-dlq-to",
            "dead",
            "--sessions",
            "--partitioning",
            "--no-batched",
            "--dead-letter-on-expiration",
            "--metadata",
            "team a",
        ]);
        let EntitySpec::Queue(name, d) = create_spec(&a).unwrap() else {
            panic!("a queue")
        };
        assert_eq!(name, "orders");
        let s = d.settings;
        assert_eq!(s.lock_duration, Some(Duration::from_secs(60)));
        assert_eq!(s.max_size_in_megabytes, Some(2048));
        assert_eq!(s.default_message_time_to_live, Some(TimeSpan::MAX));
        assert_eq!(s.auto_delete_on_idle, Some(Duration::from_secs(7 * 86_400)));
        assert_eq!(s.requires_duplicate_detection, Some(true));
        assert_eq!(
            s.duplicate_detection_history_time_window,
            Some(Duration::from_secs(600))
        );
        assert_eq!(s.max_delivery_count, Some(5));
        assert_eq!(s.forward_to.as_deref(), Some("archive"));
        assert_eq!(s.forward_dead_lettered_messages_to.as_deref(), Some("dead"));
        assert_eq!(s.requires_session, Some(true));
        assert_eq!(s.enable_partitioning, Some(true));
        assert_eq!(s.enable_batched_operations, Some(false));
        assert_eq!(s.dead_lettering_on_message_expiration, Some(true));
        assert_eq!(s.user_metadata.as_deref(), Some("team a"));
        // Nothing given: every setting is the service's default.
        let EntitySpec::Topic(_, d) =
            create_spec(&create(&["entity", "create", "topic", "events"])).unwrap()
        else {
            panic!("a topic")
        };
        assert_eq!(d.settings, TopicSettings::default());
        // A subscription with a correlation rule as its `$Default`.
        let a = create(&[
            "entity",
            "create",
            "subscription",
            "events/subscriptions/eu",
            "--lock-duration",
            "30s",
            "--rule-correlation",
            "subject=eu",
            "--rule-property",
            "tier=gold",
        ]);
        let EntitySpec::Subscription {
            topic,
            name,
            d,
            rule,
        } = create_spec(&a).unwrap()
        else {
            panic!("a subscription")
        };
        assert_eq!((topic.as_str(), name.as_str()), ("events", "eu"));
        assert_eq!(d.settings.lock_duration, Some(Duration::from_secs(30)));
        let rule = rule.unwrap();
        assert_eq!(rule.name, "$Default");
        let RuleFilter::Correlation {
            subject,
            properties,
            ..
        } = rule.filter
        else {
            panic!("correlation")
        };
        assert_eq!(subject.as_deref(), Some("eu"));
        assert_eq!(properties[0].name, "tier");
        // Flags a kind does not have are refused, naming them.
        let e = create_spec(&create(&[
            "entity",
            "create",
            "topic",
            "events",
            "--lock-duration",
            "1m",
            "--sessions",
        ]))
        .unwrap_err()
        .to_string();
        assert_eq!(e, "--lock-duration, --sessions does not apply to a topic");
        assert!(
            create_spec(&create(&[
                "entity",
                "create",
                "subscription",
                "t/subscriptions/s",
                "--max-size",
                "1"
            ]))
            .is_err()
        );
        assert!(
            create_spec(&create(&[
                "entity",
                "create",
                "queue",
                "q",
                "--rule-sql",
                "1=1"
            ]))
            .is_err(),
            "rules are for subscriptions"
        );
        assert!(
            create_spec(&create(&[
                "entity", "create", "queue", "q", "--ttl", "soon"
            ]))
            .is_err()
        );
    }

    #[test]
    fn update_changes_only_given_flags() {
        let before = QueueSettings {
            lock_duration: Some(Duration::from_secs(30)),
            max_size_in_megabytes: Some(1024),
            requires_duplicate_detection: Some(true),
            duplicate_detection_history_time_window: Some(Duration::from_secs(60)),
            max_delivery_count: Some(10),
            requires_session: Some(true),
            forward_to: Some("archive".into()),
            enable_batched_operations: Some(true),
            user_metadata: Some("keep me".into()),
            ..QueueSettings::default()
        };
        let EntityCmd::Update(a) = entity_cmd(&[
            "entity",
            "update",
            "orders",
            "--max-delivery-count",
            "3",
            "--duplicate-detection",
            "5m",
            "--forward-to",
            "",
        ]) else {
            panic!("not update")
        };
        let mut s = before.clone();
        a.settings.apply_queue(&mut s, false).unwrap();
        let expected = QueueSettings {
            max_delivery_count: Some(3),
            duplicate_detection_history_time_window: Some(Duration::from_secs(300)),
            forward_to: None,
            ..before.clone()
        };
        assert_eq!(s, expected);
        // Settings fixed at creation are refused, and so is turning on duplicate detection.
        for args in [
            &["entity", "update", "orders", "--sessions=false"][..],
            &["entity", "update", "orders", "--partitioning"],
        ] {
            let EntityCmd::Update(a) = entity_cmd(args) else {
                panic!("not update")
            };
            let e = a
                .settings
                .apply_queue(&mut before.clone(), false)
                .unwrap_err()
                .to_string();
            assert!(
                e.contains("cannot be changed after the entity is created"),
                "{e}"
            );
        }
        let EntityCmd::Update(a) =
            entity_cmd(&["entity", "update", "orders", "--duplicate-detection", "5m"])
        else {
            panic!("not update")
        };
        let mut off = QueueSettings {
            requires_duplicate_detection: Some(false),
            ..QueueSettings::default()
        };
        assert!(a.settings.apply_queue(&mut off, false).is_err());
        let EntityCmd::Update(a) = entity_cmd(&["entity", "update", "orders"]) else {
            panic!("not update")
        };
        assert!(!a.settings.any(), "an update with no setting is refused");
    }

    #[test]
    fn policy_ls_prints_no_keys() {
        let policies = vec![
            AccessPolicy {
                key_name: "app".into(),
                rights: vec![AccessRight::Listen, AccessRight::Send],
                primary_key: SecretString::from("PRIMARYSECRETKEYVALUE=".to_string()),
                secondary_key: SecretString::from("SECONDARYSECRETKEYVALUE=".to_string()),
            },
            AccessPolicy {
                key_name: "admin".into(),
                rights: vec![AccessRight::Listen, AccessRight::Send, AccessRight::Manage],
                primary_key: SecretString::from("ADMINPRIMARY=".to_string()),
                secondary_key: SecretString::from("ADMINSECONDARY=".to_string()),
            },
        ];
        for format in [Format::Table, Format::Json] {
            let out = policies_output(&policies, format).unwrap();
            for secret in [
                "PRIMARYSECRETKEYVALUE",
                "SECONDARYSECRETKEYVALUE",
                "ADMINPRIMARY",
                "ADMINSECONDARY",
            ] {
                assert!(!out.contains(secret), "{out}");
            }
            assert!(out.contains("app") && out.contains("admin"));
        }
        let table = policies_output(&policies, Format::Table).unwrap();
        assert!(table.contains("listen, send, manage"), "{table}");
        let json: serde_json::Value =
            serde_json::from_str(&policies_output(&policies, Format::Json).unwrap()).unwrap();
        assert_eq!(json[0]["key_name"], "app");
        assert_eq!(json[0]["rights"], serde_json::json!(["listen", "send"]));
        assert!(policy_path("t/subscriptions/s").is_err(), "no policies");
    }

    #[tokio::test]
    async fn export_refuses_existing_file_without_force() {
        let ctx = crate::context::Ctx::for_test_with_profiles(&[]);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("entities.xml");
        std::fs::write(&out, "keep").unwrap();
        // The host does not resolve: a command that connected would fail differently.
        let cs =
            "Endpoint=sb://dev.servicebus.invalid/;SharedAccessKeyName=k;SharedAccessKey=topsecret";
        let e = crate::run_for_test(
            &ctx,
            [
                "dispatched",
                "--connection-string",
                cs,
                "entity",
                "export",
                "--out",
                out.to_str().unwrap(),
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("exists; pass --force"), "{e}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "keep");
    }

    #[tokio::test]
    async fn entity_changes_on_production_need_every_entity_confirmed() {
        let ctx = crate::context::Ctx::for_test_with_profiles(&[(
            "prod-weu",
            "prod.servicebus.invalid",
            true,
        )]);
        let cs = "Endpoint=sb://prod.servicebus.invalid/;SharedAccessKeyName=k;SharedAccessKey=topsecret";
        for (args, verb, missing) in [
            (
                vec!["entity", "delete", "a", "b", "--yes", "--confirm-prod", "a"],
                "delete-entity",
                "b",
            ),
            (
                vec!["entity", "rename", "a", "c", "--yes", "--confirm-prod", "a"],
                "rename",
                "c",
            ),
            (
                vec!["entity", "duplicate", "a", "c", "--yes"],
                "duplicate",
                "c",
            ),
            (
                vec![
                    "rule",
                    "add",
                    "t/subscriptions/s",
                    "r",
                    "--sql",
                    "1=1",
                    "--yes",
                ],
                "add-rule",
                "t/Subscriptions/s",
            ),
            (
                vec!["policy", "add", "q", "p", "--rights", "listen", "--yes"],
                "add-policy",
                "q",
            ),
            (
                vec!["listen", "q", "--delete", "--yes"],
                "listen-delete",
                "q",
            ),
        ] {
            let mut argv = vec!["dispatched", "--connection-string", cs];
            argv.extend(args);
            let cli = crate::Cli::try_parse_from(argv).unwrap();
            let e = crate::run_command(&ctx, cli).await.unwrap_err().to_string();
            assert!(
                e.contains(&format!("--confirm-prod {missing}")),
                "{verb}: {e}"
            );
            let log = ctx
                .activity()
                .unwrap()
                .list(&dispatched_core::ActivityQuery::default())
                .unwrap();
            assert_eq!(log[0].action, verb);
            assert!(!format!("{:?}", log[0]).contains("topsecret"));
        }
    }

    #[test]
    fn planned_kinds_match_the_operations() {
        use dispatched_core::Operation;
        let q = EntityRef::queue("q");
        let sub = EntityRef::subscription("t", "s");
        for (planned, op) in [
            (CREATE, Operation::create_entity(&q, None)),
            (UPDATE, Operation::update_entity(&q)),
            (
                DELETE,
                Operation::delete_entities(vec![dispatched_core::DeleteTarget {
                    entity: q.clone(),
                    messages: dispatched_core::CountEstimate::Exact(0),
                    subscriptions: None,
                }]),
            ),
            (STATUS, Operation::set_status(&q, EntityStatus::Disabled)),
            (DUPLICATE, Operation::duplicate(&q, &q.renamed("c"))),
            (RENAME, Operation::rename(&q, &q.renamed("c"))),
            (IMPORT, Operation::import_entities(vec![q.clone()], false)),
            (ADD_RULE, Operation::add_rule(&sub, "r")),
            (REMOVE_RULE, Operation::remove_rule(&sub, "r")),
            (
                ADD_POLICY,
                Operation::add_policy(&q, "p", &[AccessRight::Listen]),
            ),
            (REMOVE_POLICY, Operation::remove_policy(&q, "p")),
            (
                RENEW_KEY,
                Operation::regenerate_key(&q, "p", KeySlot::Primary),
            ),
        ] {
            assert_eq!((planned.kind, planned.verb), (op.mutation, op.verb()));
        }
    }

    #[test]
    fn correlation_filters_take_fields_and_properties() {
        let f = correlation_filter(
            &[
                "CorrelationId=c1".into(),
                "reply_to=r".into(),
                "label=l".into(),
            ],
            &["region=eu=west".into()],
        )
        .unwrap();
        let RuleFilter::Correlation {
            correlation_id,
            reply_to,
            subject,
            properties,
            ..
        } = f
        else {
            panic!("correlation")
        };
        assert_eq!(correlation_id.as_deref(), Some("c1"));
        assert_eq!(reply_to.as_deref(), Some("r"));
        assert_eq!(subject.as_deref(), Some("l"));
        assert_eq!(
            (properties[0].name.as_str(), properties[0].value.as_str()),
            ("region", "eu=west"),
            "split on the first `=`"
        );
        assert!(correlation_filter(&["region=eu".into()], &[]).is_err());
        assert!(correlation_filter(&["to".into()], &[]).is_err());
        assert!(correlation_filter(&[], &["=x".into()]).is_err());
        // An empty correlation filter is refused before anything connects (#794).
        let empty = correlation_filter(&["to=".into()], &[]).unwrap();
        assert!(dispatched_core::mgmt::validate_filter(&empty).is_err());
    }

    #[test]
    fn copies_stay_next_to_their_source() {
        assert_eq!(
            sibling(&EntityPath::parse("t/subscriptions/s").unwrap(), "s2").unwrap(),
            EntityPath::subscription("t", "s2")
        );
        assert_eq!(
            sibling(&EntityPath::queue("q"), "q2").unwrap(),
            EntityPath::queue("q2")
        );
        assert!(sibling(&EntityPath::queue("q"), "t/subscriptions/x").is_err());
        assert_eq!(namespace_name("myns.servicebus.windows.net"), "myns");
        assert_eq!(namespace_name("localhost"), "localhost");
    }
}
