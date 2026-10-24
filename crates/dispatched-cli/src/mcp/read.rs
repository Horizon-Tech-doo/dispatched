//! The read tools: profiles, entities, counts, peeks, the dead-letter summary and sessions.
//! Nothing here is gated or logged; peeks never lock, receive or settle.

use super::{Server, ToolResult, invalid, reply};
use crate::commands::browse::{FilterArgs, build_filter, forbidden_without_known, topics_outcome};
use crate::commands::inspect::details_of;
use base64::Engine as _;
use dispatched_core::messaging::DEFAULT_SCAN_LIMIT;
use dispatched_core::profiles::ProfileAuth;
use dispatched_core::{
    EntityKind, EntityPath, ExportedMessage, MessageBody, MessageFilter, PeekOptions, PeekPage,
    PeekView, PeekedMessage, SubQueue,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The most bytes of a body (or session state) a result carries.
pub const BODY_CAP: usize = 64 * 1024;
/// Messages a peek returns unless asked for fewer or more.
pub const DEFAULT_TOP: u32 = 50;
/// The most messages one peek or search returns.
pub const MAX_TOP: u32 = 500;
/// The most messages one search examines.
pub const MAX_SCAN: u32 = 100_000;

const BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Which messages of an entity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// Active messages.
    #[default]
    Active,
    /// The dead-letter queue.
    Dlq,
    /// The transfer dead-letter queue.
    TransferDlq,
    /// Scheduled messages (not yet enqueued).
    Scheduled,
    /// Deferred messages.
    Deferred,
}

impl View {
    pub fn peek_view(self) -> PeekView {
        match self {
            View::Active => PeekView::Active,
            View::Dlq => PeekView::DeadLetter,
            View::TransferDlq => PeekView::TransferDeadLetter,
            View::Scheduled => PeekView::Scheduled,
            View::Deferred => PeekView::Deferred,
        }
    }
}

/// Message filters, in the filter language of `dispatched peek`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Filters {
    /// Property filter, e.g. `sys.DeliveryCount > 1 AND user.kind = 'x'` (`sys.` for system
    /// properties, `user.` for custom ones).
    pub filter: Option<String>,
    /// Keep messages whose body contains this text (case-insensitive).
    pub body_contains: Option<String>,
    /// Keep messages whose body matches this regular expression.
    pub body_regex: Option<String>,
    /// Keep messages whose JSON body has a value at this JSONPath, e.g. `$.order.status`.
    pub body_json: Option<String>,
    /// With `body_json`: the value must equal this.
    pub body_json_eq: Option<String>,
}

impl Filters {
    /// Whether any filter is given.
    pub fn any(&self) -> bool {
        [
            &self.filter,
            &self.body_contains,
            &self.body_regex,
            &self.body_json,
        ]
        .iter()
        .any(|f| f.as_deref().is_some_and(|s| !s.trim().is_empty()))
    }

    /// The core filter, with `session`; a bad expression is an invalid argument.
    pub fn build(&self, session: Option<&str>) -> Result<MessageFilter, rmcp::ErrorData> {
        build_filter(&FilterArgs {
            filter: self.filter.as_deref(),
            body_contains: self.body_contains.as_deref(),
            body_regex: self.body_regex.as_deref(),
            body_json: self.body_json.as_deref(),
            body_json_eq: self.body_json_eq.as_deref(),
            session,
        })
        .map_err(|e| invalid(format!("{e:#}")))
    }
}

/// An entity path argument; a bad one is an invalid argument.
pub fn entity_arg(name: &str, what: &str) -> Result<EntityPath, rmcp::ErrorData> {
    EntityPath::parse(name).map_err(|e| invalid(format!("{what}: {e}")))
}

/// The text of `text` cut to at most `cap` bytes, at a character boundary.
pub fn cut_text(text: &str, cap: usize) -> (&str, bool) {
    if text.len() <= cap {
        return (text, false);
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Bytes as text (when UTF-8) or base64, cut at [`BODY_CAP`]: `{size, text | base64,
/// truncated}`.
pub fn bytes_json(bytes: &[u8], text: Option<String>) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    out.insert("size".into(), bytes.len().into());
    match text {
        Some(t) => {
            let (t, truncated) = cut_text(&t, BODY_CAP);
            out.insert("text".into(), t.into());
            out.insert("truncated".into(), truncated.into());
        }
        None => {
            let cut = &bytes[..bytes.len().min(BODY_CAP)];
            out.insert("base64".into(), BASE64.encode(cut).into());
            out.insert("truncated".into(), (bytes.len() > BODY_CAP).into());
        }
    }
    out
}

/// A body for an agent: its kind (as in `peek --format json`), its size in bytes, and the text
/// as the CLI decodes it, or base64 for binary that is not UTF-8, cut at 64 KiB.
pub fn body_json(body: &MessageBody) -> Value {
    let kind = match body {
        MessageBody::Binary(_) => "binary",
        MessageBody::Text(_) => "text",
        MessageBody::Value(_) => "value",
        MessageBody::Sequence(_) => "sequence",
        MessageBody::Empty => "empty",
    };
    let mut out = match body {
        MessageBody::Binary(b) => bytes_json(b, body.as_text()),
        MessageBody::Empty => {
            let mut m = serde_json::Map::new();
            m.insert("size".into(), 0.into());
            m.insert("truncated".into(), false.into());
            m
        }
        _ => {
            let text = body.as_text().unwrap_or_default();
            bytes_json(text.as_bytes(), Some(text.clone()))
        }
    };
    out.insert("kind".into(), kind.into());
    Value::Object(out)
}

/// A message in the `peek --format json` shape, with the body as [`body_json`].
pub fn message_json(mut m: PeekedMessage) -> Value {
    let body = std::mem::replace(&mut m.body, MessageBody::Empty);
    let mut v = serde_json::to_value(ExportedMessage::new(&m)).unwrap_or(Value::Null);
    if let Some(fields) = v.as_object_mut() {
        fields.remove("body_text");
        fields.insert("body".into(), body_json(&body));
    }
    v
}

/// A peek page in the `peek --format json` shape.
pub fn page_json(page: PeekPage) -> Value {
    json!({
        "scanned": page.scanned,
        "next_sequence": page.next_sequence,
        "exhausted": page.exhausted,
        "messages": page.messages.into_iter().map(message_json).collect::<Vec<_>>(),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListEntitiesArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// Also list topics and their subscriptions.
    #[serde(default)]
    pub topics: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// `queue`, `topic` or `topic/subscriptions/subscription`.
    pub entity: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PeekArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Which messages (default active).
    #[serde(default)]
    pub sub_queue: View,
    /// Most messages to return (default 50, at most 500).
    pub top: Option<u32>,
    /// Start at this sequence number (`next_sequence` of an earlier call continues it).
    pub from_sequence: Option<i64>,
    /// Only messages of this session (exact session id).
    pub session: Option<String>,
    #[serde(flatten)]
    pub filters: Filters,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Which messages (default active).
    #[serde(default)]
    pub sub_queue: View,
    /// Most matches to return (default 50, at most 500).
    pub top: Option<u32>,
    /// Most messages to examine, matching or not (default 10,000, at most 100,000).
    pub scan_limit: Option<u32>,
    /// Start at this sequence number (`next_sequence` of an earlier call continues it).
    pub from_sequence: Option<i64>,
    /// Only messages of this session (exact session id).
    pub session: Option<String>,
    /// At least one of `filter`, `body_contains`, `body_regex` or `body_json` is required.
    #[serde(flatten)]
    pub filters: Filters,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DlqSummaryArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Summarize the transfer dead-letter queue instead.
    #[serde(default)]
    pub transfer: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionStateArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// A session-enabled `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The session id.
    pub session: String,
}

/// `top`, checked: 1 to [`MAX_TOP`], [`DEFAULT_TOP`] when not given.
pub fn top_arg(top: Option<u32>) -> Result<u32, rmcp::ErrorData> {
    match top.unwrap_or(DEFAULT_TOP) {
        n @ 1..=MAX_TOP => Ok(n),
        n => Err(invalid(format!("top {n}: ask for 1 to {MAX_TOP} messages"))),
    }
}

/// `scan_limit`, checked: 1 to [`MAX_SCAN`], the CLI's default when not given.
pub fn scan_arg(scan: Option<u32>) -> Result<u32, rmcp::ErrorData> {
    match scan.unwrap_or(DEFAULT_SCAN_LIMIT) {
        n @ 1..=MAX_SCAN => Ok(n),
        n => Err(invalid(format!(
            "scan_limit {n}: examine 1 to {MAX_SCAN} messages per call"
        ))),
    }
}

fn auth_text(a: &ProfileAuth) -> &'static str {
    match a {
        ProfileAuth::ConnectionString => "connection-string",
        ProfileAuth::Entra { .. } => "entra",
    }
}

#[tool_router(router = read_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "List the saved connection profiles: name, namespace host, how it signs in, \
            whether it is the emulator or marked production, and which is the default. No secrets.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_profiles(&self) -> ToolResult {
        reply((|| {
            let store = self.ctx().store()?;
            let default = store.default_profile()?.map(|p| p.id);
            let profiles: Vec<Value> = store
                .list()?
                .iter()
                .map(|p| {
                    json!({
                        "name": p.name,
                        "host": p.host,
                        "auth": auth_text(&p.auth),
                        "emulator": p.emulator,
                        "production": p.production,
                        "default": default.as_deref() == Some(p.id.as_str()),
                    })
                })
                .collect();
            let server_default = match &self.inner.default {
                Ok(t) => json!({ "name": t.name, "host": t.host }),
                Err(e) => json!({ "error": e }),
            };
            Ok(json!({ "profiles": profiles, "server_default": server_default }))
        })())
    }

    #[tool(
        description = "List queues with their message counts; with `topics`, also topics and their \
            subscriptions with counts. Counts marked approximate were counted by peeking.",
        annotations(read_only_hint = true)
    )]
    async fn list_entities(&self, Parameters(a): Parameters<ListEntitiesArgs>) -> ToolResult {
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let ex = &t.explorer;
                let queues = match ex.list_queues().await {
                    Ok(q) => q,
                    Err(e @ dispatched_core::Error::Forbidden { .. }) => {
                        return Err(forbidden_without_known(e));
                    }
                    Err(e) => return Err(e.into()),
                };
                let mut value = json!({ "queues": queues });
                if a.topics {
                    let listing = async {
                        let mut out = Vec::new();
                        for topic in ex.list_topics().await? {
                            let subs = ex.list_subscriptions(&topic.name).await?;
                            out.push((topic, subs));
                        }
                        Ok(out)
                    }
                    .await;
                    match topics_outcome(listing)? {
                        Some(list) => {
                            value["topics"] = list
                                .iter()
                                .map(|(t, subs)| json!({ "topic": t, "subscriptions": subs }))
                                .collect();
                        }
                        None => {
                            value["topics_skipped"] =
                                "topics and subscriptions skipped: listing them needs \
                                 Manage,EntityRead"
                                    .into();
                        }
                    }
                }
                Ok(value)
            }
            .await,
        )
    }

    #[tool(
        description = "An entity's properties (status, lock duration, forwarding, sessions, ...) and \
            message counts, as `dispatched show` gives them.",
        annotations(read_only_hint = true)
    )]
    async fn get_entity(&self, Parameters(a): Parameters<EntityArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let d =
                    details_of(&path, |kind| t.explorer.entity_details(kind, &a.entity)).await?;
                Ok(serde_json::to_value(d)?)
            }
            .await,
        )
    }

    #[tool(
        description = "Message counts of a queue or subscription (active, dead-letter, scheduled, \
            transfer, transfer dead-letter), or of every subscription of a topic.",
        annotations(read_only_hint = true)
    )]
    async fn get_counts(&self, Parameters(a): Parameters<EntityArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let d =
                    details_of(&path, |kind| t.explorer.entity_details(kind, &a.entity)).await?;
                if d.kind != EntityKind::Topic {
                    return Ok(json!({
                        "entity": path.base(),
                        "kind": d.kind,
                        "counts": d.counts,
                    }));
                }
                let subs = t.explorer.list_subscriptions(&d.name).await?;
                Ok(json!({
                    "entity": path.base(),
                    "kind": d.kind,
                    "subscriptions": subs
                        .iter()
                        .map(|s| json!({ "name": s.name, "counts": s.counts }))
                        .collect::<Vec<_>>(),
                }))
            }
            .await,
        )
    }

    #[tool(
        description = "Peek messages without removing or locking them. Returns at most `top` \
            (default 50, at most 500) with their properties and bodies (text, or base64 for \
            binary; cut at 64 KiB with `truncated: true`). Continue with `from_sequence` = \
            `next_sequence`. Filters use the language of `dispatched peek`.",
        annotations(read_only_hint = true)
    )]
    async fn peek_messages(&self, Parameters(a): Parameters<PeekArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        let top = top_arg(a.top)?;
        let filter = a.filters.build(a.session.as_deref())?;
        let opts = PeekOptions::new(top)
            .from_sequence(a.from_sequence.unwrap_or(0))
            .filter(filter);
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let page = t
                    .explorer
                    .peek(&path, a.sub_queue.peek_view(), &opts)
                    .await?;
                Ok(page_json(page))
            }
            .await,
        )
    }

    #[tool(
        description = "Search messages by property and body filters (at least one is required), \
            examining up to `scan_limit` messages (default 10,000, at most 100,000) without \
            removing or locking them. Returns at most `top` matches (default 50, at most 500); \
            continue with `from_sequence` = `next_sequence`.",
        annotations(read_only_hint = true)
    )]
    async fn search_messages(&self, Parameters(a): Parameters<SearchArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        if !a.filters.any() {
            return Err(invalid(
                "give at least one of filter, body_contains, body_regex or body_json \
                 (peek_messages reads without one)",
            ));
        }
        let top = top_arg(a.top)?;
        let scan = scan_arg(a.scan_limit)?;
        let filter = a.filters.build(a.session.as_deref())?;
        let opts = PeekOptions::new(top)
            .from_sequence(a.from_sequence.unwrap_or(0))
            .scan_limit(scan)
            .filter(filter);
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let page = t
                    .explorer
                    .peek(&path, a.sub_queue.peek_view(), &opts)
                    .await?;
                Ok(page_json(page))
            }
            .await,
        )
    }

    #[tool(
        description = "Dead-lettered messages grouped by reason and description, with the count \
            and the first and last sequence number of each group (from a peek of up to 10,000 \
            messages), as `dispatched dlq summary` gives them.",
        annotations(read_only_hint = true)
    )]
    async fn get_dlq_summary(&self, Parameters(a): Parameters<DlqSummaryArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        let sub_queue = if a.transfer {
            SubQueue::TransferDeadLetter
        } else {
            SubQueue::DeadLetter
        };
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                Ok(serde_json::to_value(
                    t.explorer.dlq_summary(&path, sub_queue).await?,
                )?)
            }
            .await,
        )
    }

    #[tool(
        description = "Sessions of a session-enabled queue or subscription, with message counts \
            (no session is locked).",
        annotations(read_only_hint = true)
    )]
    async fn list_sessions(&self, Parameters(a): Parameters<EntityArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                Ok(serde_json::to_value(
                    t.explorer.list_sessions(&path).await?,
                )?)
            }
            .await,
        )
    }

    #[tool(
        description = "The state of one session (text, or base64 when binary; cut at 64 KiB). \
            Reading it takes the session lock briefly without locking any message; a session \
            held by a running receiver is reported as locked.",
        annotations(read_only_hint = true)
    )]
    async fn get_session_state(&self, Parameters(a): Parameters<SessionStateArgs>) -> ToolResult {
        let path = entity_arg(&a.entity, "entity")?;
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let state = t.explorer.session_state(&path, &a.session).await?;
                let mut out = serde_json::Map::new();
                out.insert("session_id".into(), a.session.clone().into());
                match state {
                    None => {
                        out.insert("state".into(), Value::Null);
                    }
                    Some(bytes) => {
                        let text = String::from_utf8(bytes.clone()).ok();
                        out.insert("state".into(), Value::Object(bytes_json(&bytes, text)));
                    }
                }
                Ok(Value::Object(out))
            }
            .await,
        )
    }
}
