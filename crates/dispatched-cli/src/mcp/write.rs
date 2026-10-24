//! The write tools. Each builds the core `Operation` the matching CLI command builds and runs it
//! through a gate with origin `mcp` and an [`McpPolicy`] over the call's client, so the human
//! confirms every change there and the activity log records it. Sizes are bounded per call.

use super::policy::McpPolicy;
use super::read::{Filters, View, entity_arg};
use super::{Server, ToolResult, invalid, reply};
use crate::commands::repair::{denied_reasons, nothing_to_purge};
use crate::commands::send::{SendArgs, scheduled_target, send_request_from};
use crate::commands::state::check_state_size;
use anyhow::Context as _;
use base64::Engine as _;
use dispatched_core::mgmt::{
    Description, EntityRef, RuleFilter, RuleSpec, SettingsChange, validate_filter,
};
use dispatched_core::ops::{
    LocateLimits, MessageSelection, ResubmitSpec, SelectSpec, default_target, send_target,
};
use dispatched_core::{
    EntityKind, EntityPath, EntitySpec, JobReport, NoProgress, Operator, PurgeScope,
};
use horizon_azure_kit::safety::CancellationToken;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, Peer, RoleServer, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// The most messages one `send_message` or `schedule_message` call makes.
pub const MAX_SEND: u64 = 100;
/// The most messages one delete, move, resubmit or cancel call names or matches.
pub const MAX_SELECT: u64 = 1_000;

/// A message to send, with the fields `dispatched send` takes (no files or templates).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct MessageArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// The queue or topic.
    pub entity: String,
    /// The body text (sent as UTF-8 bytes).
    pub body: Option<String>,
    /// ContentType, e.g. `application/json`.
    pub content_type: Option<String>,
    /// MessageId (a new random one per message when not given).
    pub message_id: Option<String>,
    /// SessionId (required by session-enabled entities).
    pub session_id: Option<String>,
    /// CorrelationId.
    pub correlation_id: Option<String>,
    /// Subject (Label).
    pub subject: Option<String>,
    /// To.
    pub to: Option<String>,
    /// ReplyTo.
    pub reply_to: Option<String>,
    /// ReplyToSessionId.
    pub reply_to_session_id: Option<String>,
    /// PartitionKey.
    pub partition_key: Option<String>,
    /// Time to live: `90s`, `15m`, `2h`, `7d`, `1h30m`, ISO 8601 or .NET TimeSpan text.
    pub time_to_live: Option<String>,
    /// Custom properties as `KEY=VALUE[:TYPE]` (string, int, long, double, bool, datetime,
    /// timespan, guid; string by default).
    #[serde(default)]
    pub properties: Vec<String>,
    /// How many copies (1 to 100, default 1).
    pub count: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleArgs {
    #[serde(flatten)]
    pub message: MessageArgs,
    /// When to enqueue: `+DURATION` (such as `+10m`) or an RFC 3339 time.
    pub at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CancelScheduledArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// The queue or topic the messages were scheduled on.
    pub entity: String,
    /// The sequence numbers `schedule_message` returned (1 to 1,000).
    pub sequence_numbers: Vec<i64>,
}

/// Which messages a delete, move or resubmit acts on: sequence numbers, or the matches of
/// filters up to `max`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Pick {
    /// These sequence numbers (1 to 1,000). Leave empty to select by filters instead.
    #[serde(default)]
    pub sequence_numbers: Vec<i64>,
    /// With filters: act on at most this many matches (1 to 1,000); required with filters.
    pub max: Option<u64>,
    /// Only messages of this session (exact session id); counts as a filter.
    pub session: Option<String>,
    #[serde(flatten)]
    pub filters: Filters,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Which messages (default active).
    #[serde(default)]
    pub sub_queue: View,
    #[serde(flatten)]
    pub pick: Pick,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// The source: `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Which messages of the source (default active).
    #[serde(default)]
    pub sub_queue: View,
    /// The queue or topic to move them to.
    pub to: String,
    /// Send a copy even when it must lose custom properties whose type cannot be copied;
    /// without it such a message fails and its original stays.
    #[serde(default)]
    pub drop_uncopyable_properties: bool,
    #[serde(flatten)]
    pub pick: Pick,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResubmitArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// The queue or subscription whose dead-letter queue holds the messages.
    pub entity: String,
    /// Take them from the transfer dead-letter queue instead.
    #[serde(default)]
    pub transfer: bool,
    /// The queue or topic the copies go to (default: the queue itself, or a subscription's
    /// topic).
    pub to: Option<String>,
    /// Remove each original once its copy is sent (default false: the originals stay).
    #[serde(default)]
    pub remove_original: bool,
    #[serde(flatten)]
    pub pick: Pick,
}

/// What a purge removes.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Active messages (not deferred, not scheduled).
    #[default]
    Active,
    /// The dead-letter queue.
    Dlq,
    /// The transfer dead-letter queue.
    TransferDlq,
    /// Active, deferred, dead-letter and transfer dead-letter messages (scheduled ones stay).
    All,
}

impl From<Scope> for PurgeScope {
    fn from(s: Scope) -> Self {
        match s {
            Scope::Active => PurgeScope::Active,
            Scope::Dlq => PurgeScope::DeadLetter,
            Scope::TransferDlq => PurgeScope::TransferDeadLetter,
            Scope::All => PurgeScope::All,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PurgeArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Which messages to remove (default active).
    #[serde(default)]
    pub scope: Scope,
}

/// Queue, topic or subscription.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Queue,
    Topic,
    Subscription,
}

impl From<Kind> for EntityKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Queue => EntityKind::Queue,
            Kind::Topic => EntityKind::Topic,
            Kind::Subscription => EntityKind::Subscription,
        }
    }
}

/// The settings object: the fields of `dispatched entity create|update`, by their service
/// names in snake_case. Durations take `90s`, `5m`, `1d`, ISO 8601 or `max` / `never`.
const SETTINGS_DOC: &str = "Settings, e.g. {\"lock_duration\": \"1m\", \"max_delivery_count\": 5}. \
    Fields: lock_duration, max_size_in_megabytes, requires_duplicate_detection, requires_session, \
    default_message_time_to_live, dead_lettering_on_message_expiration, \
    dead_lettering_on_filter_evaluation_exceptions, duplicate_detection_history_time_window, \
    max_delivery_count, enable_batched_operations, status (Active, Disabled, SendDisabled, \
    ReceiveDisabled), forward_to, forward_dead_lettered_messages_to (\"\" clears), \
    auto_delete_on_idle, enable_partitioning, enable_express, support_ordering, \
    max_message_size_in_kilobytes, user_metadata. Durations: 90s, 5m, 1d, ISO 8601, or max.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// What to create.
    pub kind: Kind,
    /// The name; for a subscription `topic/subscriptions/name`.
    pub entity: String,
    #[schemars(description = SETTINGS_DOC)]
    pub settings: Option<Value>,
    /// Subscriptions only: the SQL filter of the initial rule `$Default` (default: every
    /// message).
    pub rule_sql: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// What to change.
    pub kind: Kind,
    /// The name; for a subscription `topic/subscriptions/name`.
    pub entity: String,
    /// Only these settings change; the others keep their current values.
    #[schemars(description = SETTINGS_DOC)]
    pub settings: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityDeleteArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// What to delete.
    pub kind: Kind,
    /// The name; for a subscription `topic/subscriptions/name`.
    pub entity: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetStateArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// A session-enabled `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The session id.
    pub session: String,
    /// The new state as text. Give exactly one of `text`, `base64` or `clear`.
    pub text: Option<String>,
    /// The new state as base64 bytes.
    pub base64: Option<String>,
    /// Clear the state.
    #[serde(default)]
    pub clear: bool,
    /// Allow state above 256 KiB (Premium; the limit is then the entity's maximum message size).
    #[serde(default)]
    pub allow_large: bool,
}

/// An error message for invalid arguments built from an anyhow chain.
fn invalid_from(e: anyhow::Error) -> ErrorData {
    invalid(super::error_text(&e))
}

/// The CLI's send arguments for `m`, scheduled at `at`.
fn send_args(m: &MessageArgs, at: Option<&str>) -> Result<SendArgs, ErrorData> {
    let count = m.count.unwrap_or(1);
    if !(1..=MAX_SEND).contains(&count) {
        return Err(invalid(format!(
            "count {count}: send 1 to {MAX_SEND} messages per call"
        )));
    }
    Ok(SendArgs {
        entity: m.entity.clone(),
        body: m.body.clone(),
        file: Vec::new(),
        template: None,
        props: m.properties.clone(),
        message_id: m.message_id.clone(),
        increment_id: false,
        session_id: m.session_id.clone(),
        correlation_id: m.correlation_id.clone(),
        subject: m.subject.clone(),
        to: m.to.clone(),
        reply_to: m.reply_to.clone(),
        reply_to_session_id: m.reply_to_session_id.clone(),
        content_type: m.content_type.clone(),
        partition_key: m.partition_key.clone(),
        ttl: m.time_to_live.clone(),
        schedule: at.map(str::to_string),
        count,
        batch: false,
        mutate: Default::default(),
    })
}

/// The selection a [`Pick`] describes on `entity`'s `view`.
pub fn select_spec(entity: EntityPath, view: View, pick: &Pick) -> Result<SelectSpec, ErrorData> {
    let by_filter = pick.filters.any() || pick.session.is_some();
    let selection = match (pick.sequence_numbers.is_empty(), by_filter) {
        (false, false) => {
            if pick.sequence_numbers.len() as u64 > MAX_SELECT {
                return Err(invalid(format!(
                    "name 1 to {MAX_SELECT} sequence numbers per call"
                )));
            }
            if pick.max.is_some() {
                return Err(invalid("max goes with filters, not with sequence_numbers"));
            }
            MessageSelection::Sequences(pick.sequence_numbers.clone())
        }
        (true, true) => {
            let limit = pick.max.ok_or_else(|| {
                invalid(format!(
                    "selecting by filters needs max (1 to {MAX_SELECT}): the most messages to act on"
                ))
            })?;
            if !(1..=MAX_SELECT).contains(&limit) {
                return Err(invalid(format!("max {limit}: use 1 to {MAX_SELECT}")));
            }
            MessageSelection::Matching {
                filter: pick.filters.build(pick.session.as_deref())?,
                limit,
            }
        }
        (false, true) => {
            return Err(invalid("give sequence_numbers or filters, not both"));
        }
        (true, false) => {
            return Err(invalid(
                "give sequence_numbers, or filters (filter, body_contains, body_regex, body_json, \
                 session) with max",
            ));
        }
    };
    Ok(SelectSpec {
        entity,
        view: view.peek_view(),
        selection,
        limits: LocateLimits::default(),
    })
}

/// The settings change an object describes; unknown fields and bad values are invalid
/// arguments.
fn settings_arg(v: Option<&Value>) -> Result<SettingsChange, ErrorData> {
    match v {
        None | Some(Value::Null) => Ok(SettingsChange::default()),
        Some(v @ Value::Object(_)) => {
            serde_json::from_value(v.clone()).map_err(|e| invalid(format!("settings: {e}")))
        }
        Some(_) => Err(invalid("settings must be an object")),
    }
}

/// The entity a kind and path name; a bad pair is an invalid argument.
fn entity_ref(kind: Kind, path: &str) -> Result<EntityRef, ErrorData> {
    EntityRef::parse(kind.into(), path).map_err(|e| invalid(format!("entity: {e}")))
}

/// A job report as a result (the JSON of `--format json`).
fn report_json(report: JobReport) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(report)?)
}

impl Server {
    /// The operator for one write call: the target's connection, gated by the call's client
    /// with origin `mcp` and production read from the saved profiles now.
    fn operator(&self, profile: Option<&str>, peer: Peer<RoleServer>) -> anyhow::Result<Operator> {
        let target = self.target(profile)?;
        let profile = self.profile_ref(&target)?;
        let gate = crate::safety::mcp_gate(self.ctx(), &profile, Arc::new(McpPolicy::new(peer)))?;
        Ok(Operator::new(target.explorer.clone(), gate))
    }

    async fn send_as(
        &self,
        m: &MessageArgs,
        at: Option<&str>,
        peer: Peer<RoleServer>,
        cancel: &CancellationToken,
    ) -> ToolResult {
        let args = send_args(m, at)?;
        let req = send_request_from(&args).map_err(invalid_from)?;
        reply(
            async {
                let op = self.operator(m.profile.as_deref(), peer)?;
                report_json(op.send(req, &NoProgress, cancel).await?)
            }
            .await,
        )
    }
}

#[tool_router(router = write_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Send messages to a queue or topic (1 to 100 copies). The human confirms in \
            this client first. Returns the job report: succeeded, failed [{seq, reason}], skipped \
            and one item per message.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn send_message(
        &self,
        Parameters(a): Parameters<MessageArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        self.send_as(&a, None, peer, &cancel).await
    }

    #[tool(
        description = "Schedule messages on a queue or topic for later (`at`: +10m or an RFC 3339 \
            time). The human confirms first. The report's items carry the new sequence numbers, \
            which cancel_scheduled takes.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn schedule_message(
        &self,
        Parameters(a): Parameters<ScheduleArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        self.send_as(&a.message, Some(&a.at), peer, &cancel).await
    }

    #[tool(
        description = "Cancel scheduled messages of a queue or topic by sequence number (1 to \
            1,000). The human confirms first. Returns the job report.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn cancel_scheduled(
        &self,
        Parameters(a): Parameters<CancelScheduledArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        let entity = scheduled_target(&a.entity).map_err(invalid_from)?;
        let n = a.sequence_numbers.len() as u64;
        if !(1..=MAX_SELECT).contains(&n) {
            return Err(invalid(format!(
                "name 1 to {MAX_SELECT} sequence numbers per call"
            )));
        }
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                let r = op
                    .cancel_scheduled(&entity, &a.sequence_numbers, &NoProgress, &cancel)
                    .await?;
                report_json(r)
            }
            .await,
        )
    }

    #[tool(
        description = "Send copies of dead-lettered messages back to a queue or topic (default: \
            the entity itself, or a subscription's topic), by sequence number or by filters with \
            `max` (at most 1,000), optionally removing the originals. The human confirms first. \
            Returns the job report.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn resubmit_dlq(
        &self,
        Parameters(a): Parameters<ResubmitArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        let entity = entity_arg(&a.entity, "entity")?;
        let view = if a.transfer {
            View::TransferDlq
        } else {
            View::Dlq
        };
        let target = match &a.to {
            Some(to) => send_target(to)
                .map_err(|e| invalid(format!("to: {e}")))?
                .base(),
            None => default_target(&entity),
        };
        let spec = ResubmitSpec {
            source: select_spec(entity, view, &a.pick)?,
            target,
            edits: Default::default(),
            new_message_id: false,
            remove_original: a.remove_original,
            keep_dead_letter_properties: false,
            drop_uncopyable_properties: false,
        };
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                report_json(op.resubmit(spec, &NoProgress, &cancel).await?)
            }
            .await,
        )
    }

    #[tool(
        description = "Move messages to another queue or topic, unchanged (each is sent first, then \
            removed), by sequence number or by filters with `max` (at most 1,000). The human \
            confirms first, for the source and on production also the target. Returns the job \
            report.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn move_messages(
        &self,
        Parameters(a): Parameters<MoveArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        let entity = entity_arg(&a.entity, "entity")?;
        let to = send_target(&a.to)
            .map_err(|e| invalid(format!("to: {e}")))?
            .base();
        let source = select_spec(entity, a.sub_queue, &a.pick)?;
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                let r = op
                    .move_messages(
                        source,
                        to,
                        a.drop_uncopyable_properties,
                        &NoProgress,
                        &cancel,
                    )
                    .await?;
                report_json(r)
            }
            .await,
        )
    }

    #[tool(
        description = "Delete messages by sequence number or by filters with `max` (at most \
            1,000): active and deferred ones are completed, scheduled ones cancelled. The human \
            confirms first. This can't be undone. Returns the job report.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn delete_messages(
        &self,
        Parameters(a): Parameters<DeleteArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        let entity = entity_arg(&a.entity, "entity")?;
        let spec = select_spec(entity, a.sub_queue, &a.pick)?;
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                report_json(op.delete(spec, &NoProgress, &cancel).await?)
            }
            .await,
        )
    }

    #[tool(
        description = "Remove every message of a queue or subscription in `scope` (active by \
            default; dlq, transfer_dlq or all). The messages are counted first and the human \
            confirms that count; never more are removed. This can't be undone. Returns the job \
            report.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn purge(
        &self,
        Parameters(a): Parameters<PurgeArgs>,
        peer: Peer<RoleServer>,
        cancel: CancellationToken,
    ) -> ToolResult {
        let entity = entity_arg(&a.entity, "entity")?;
        let scope = PurgeScope::from(a.scope);
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                let plan = op
                    .plan_purge(&entity, scope)
                    .await
                    .with_context(|| format!("counting the messages of {}", entity.base()))?;
                if nothing_to_purge(&plan) {
                    return Ok(json!({
                        "entity": plan.target().address_text(),
                        "succeeded": 0,
                        "failed": [],
                        "skipped": 0,
                        "notes": ["nothing to purge; nothing was changed"],
                    }));
                }
                let report = op.purge(vec![plan], &NoProgress, &cancel).await?;
                // One entity: its denial is the answer, not a job.
                if let Some(denied) = denied_reasons(&report).first() {
                    anyhow::bail!("{denied}");
                }
                report_json(report)
            }
            .await,
        )
    }

    #[tool(
        description = "Create a queue, topic or subscription with the given settings (a \
            subscription can get an initial SQL rule). The human confirms first. Returns the job \
            report.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn entity_create(
        &self,
        Parameters(a): Parameters<CreateArgs>,
        peer: Peer<RoleServer>,
    ) -> ToolResult {
        let target = entity_ref(a.kind, &a.entity)?;
        let change = settings_arg(a.settings.as_ref())?;
        let rule = match (&a.rule_sql, &target) {
            (None, _) => None,
            (Some(sql), EntityRef::Subscription { .. }) => {
                let filter = RuleFilter::Sql {
                    expression: sql.clone(),
                };
                validate_filter(&filter).map_err(|e| invalid(format!("rule_sql: {e}")))?;
                Some(RuleSpec {
                    name: "$Default".into(),
                    filter,
                    action: None,
                })
            }
            (Some(_), _) => return Err(invalid("rule_sql is for subscriptions")),
        };
        let spec = (|| -> dispatched_core::Result<EntitySpec> {
            Ok(match &target {
                EntityRef::Queue { name } => {
                    let mut d = Description::default();
                    change.apply_queue(&mut d.settings, true)?;
                    EntitySpec::Queue(name.clone(), d)
                }
                EntityRef::Topic { name } => {
                    let mut d = Description::default();
                    change.apply_topic(&mut d.settings, true)?;
                    EntitySpec::Topic(name.clone(), d)
                }
                EntityRef::Subscription { topic, name } => {
                    let mut d = Description::default();
                    change.apply_subscription(&mut d.settings, true)?;
                    EntitySpec::Subscription {
                        topic: topic.clone(),
                        name: name.clone(),
                        d,
                        rule,
                    }
                }
            })
        })()
        .map_err(|e| invalid(format!("settings: {e}")))?;
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                report_json(op.create_entity(spec).await?)
            }
            .await,
        )
    }

    #[tool(
        description = "Change settings of a queue, topic or subscription; only the given settings \
            change (read after the confirmation, so other changes meanwhile are kept). The human \
            confirms first. Returns the job report.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn entity_update(
        &self,
        Parameters(a): Parameters<UpdateArgs>,
        peer: Peer<RoleServer>,
    ) -> ToolResult {
        let target = entity_ref(a.kind, &a.entity)?;
        let change = settings_arg(Some(&a.settings))?;
        if change.is_empty() {
            return Err(invalid("settings: nothing to change"));
        }
        if let Some(i) = change.issues(target.kind(), false).into_iter().next() {
            return Err(invalid(format!("settings: {}: {}", i.field, i.message)));
        }
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                report_json(op.update_settings(target, change).await?)
            }
            .await,
        )
    }

    #[tool(
        description = "Delete a queue, topic (with its subscriptions) or subscription and every \
            message it holds. It is counted first and the human confirms what it holds. This \
            can't be undone. Returns the job report.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn entity_delete(
        &self,
        Parameters(a): Parameters<EntityDeleteArgs>,
        peer: Peer<RoleServer>,
    ) -> ToolResult {
        let target = entity_ref(a.kind, &a.entity)?;
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                report_json(op.delete_entities(vec![target]).await?)
            }
            .await,
        )
    }

    #[tool(
        description = "Replace or clear the state of one session (`text`, `base64` or `clear`). \
            The human confirms first. A session held by a running receiver is refused and nothing \
            changes.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn set_session_state(
        &self,
        Parameters(a): Parameters<SetStateArgs>,
        peer: Peer<RoleServer>,
    ) -> ToolResult {
        let entity = entity_arg(&a.entity, "entity")?;
        let state = match (&a.text, &a.base64, a.clear) {
            (Some(t), None, false) => Some(t.clone().into_bytes()),
            (None, Some(b), false) => Some(
                base64::engine::general_purpose::STANDARD
                    .decode(b.trim())
                    .map_err(|e| invalid(format!("base64: {e}")))?,
            ),
            (None, None, true) => None,
            _ => return Err(invalid("give exactly one of text, base64 or clear")),
        };
        if let Some(s) = &state {
            check_state_size(s, a.allow_large).map_err(invalid_from)?;
        }
        reply(
            async {
                let op = self.operator(a.profile.as_deref(), peer)?;
                let replaced = state.is_some();
                op.set_session_state(&entity, &a.session, state).await?;
                Ok(json!({
                    "entity": entity.base(),
                    "session_id": a.session,
                    "done": if replaced { "state replaced" } else { "state cleared" },
                }))
            }
            .await,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(seqs: &[i64], filter: Option<&str>, max: Option<u64>) -> Pick {
        Pick {
            sequence_numbers: seqs.to_vec(),
            max,
            session: None,
            filters: Filters {
                filter: filter.map(str::to_string),
                ..Filters::default()
            },
        }
    }

    fn spec(p: &Pick) -> Result<SelectSpec, String> {
        select_spec(EntityPath::queue("q"), View::Dlq, p).map_err(|e| e.message.to_string())
    }

    #[test]
    fn a_selection_is_sequence_numbers_or_bounded_filters() {
        let s = spec(&pick(&[3, 1], None, None)).unwrap();
        assert!(matches!(s.selection, MessageSelection::Sequences(ref v) if v == &[3, 1]));
        assert_eq!(s.view, dispatched_core::PeekView::DeadLetter);

        let s = spec(&pick(&[], Some("sys.DeliveryCount > 1"), Some(10))).unwrap();
        assert!(matches!(
            s.selection,
            MessageSelection::Matching { limit: 10, .. }
        ));

        let e = spec(&pick(&[], Some("sys.DeliveryCount > 1"), None)).unwrap_err();
        assert!(e.contains("needs max"), "{e}");
        let e = spec(&pick(&[], Some("sys.DeliveryCount > 1"), Some(1_001))).unwrap_err();
        assert!(e.contains("1 to 1000"), "{e}");
        let e = spec(&pick(&[1], Some("sys.DeliveryCount > 1"), Some(1))).unwrap_err();
        assert!(e.contains("not both"), "{e}");
        let e = spec(&pick(&[], None, None)).unwrap_err();
        assert!(e.contains("give sequence_numbers"), "{e}");
        let e = spec(&pick(&[1], None, Some(5))).unwrap_err();
        assert!(e.contains("max goes with filters"), "{e}");
        let many: Vec<i64> = (0..1_001).collect();
        assert!(spec(&pick(&many, None, None)).is_err());
        let e = spec(&pick(&[], Some("this is not a filter ((("), Some(1))).unwrap_err();
        assert!(!e.is_empty());
    }

    #[test]
    fn send_counts_are_bounded_and_flags_map_to_the_cli() {
        let m = MessageArgs {
            entity: "orders".into(),
            body: Some("hi".into()),
            properties: vec!["k=1:int".into()],
            count: Some(2),
            ..MessageArgs::default()
        };
        let a = send_args(&m, Some("+10m")).unwrap();
        assert_eq!(a.count, 2);
        assert_eq!(a.schedule.as_deref(), Some("+10m"));
        let req = send_request_from(&a).unwrap();
        assert!(req.schedule.is_some());
        assert_eq!(req.count, 2);

        for bad in [0, MAX_SEND + 1] {
            let m = MessageArgs {
                entity: "orders".into(),
                count: Some(bad),
                ..MessageArgs::default()
            };
            assert!(send_args(&m, None).is_err());
        }
        // A subscription is not a send target.
        let m = MessageArgs {
            entity: "t/subscriptions/s".into(),
            ..MessageArgs::default()
        };
        assert!(send_request_from(&send_args(&m, None).unwrap()).is_err());
    }

    #[test]
    fn settings_are_an_object_of_known_fields() {
        let c = settings_arg(Some(
            &json!({"lock_duration": "1m", "max_delivery_count": 5}),
        ))
        .unwrap();
        assert_eq!(c.max_delivery_count, Some(5));
        assert_eq!(c.lock_duration, Some(std::time::Duration::from_secs(60)));
        assert!(settings_arg(Some(&json!({"lock": "1m"}))).is_err());
        assert!(settings_arg(Some(&json!([1]))).is_err());
        assert!(settings_arg(None).unwrap().is_empty());
        assert!(entity_ref(Kind::Subscription, "orders").is_err());
        assert!(entity_ref(Kind::Queue, "t/subscriptions/s").is_err());
    }
}
