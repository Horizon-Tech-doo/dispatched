//! `dispatched ls`, `peek` and `export`.

use crate::Format;
use crate::context::Ctx;
use crate::output::{ellipsize, sanitize, table, thousands};
use crate::resolve::{Selection, needs_sign_in, resolve};
use clap::Args;
use dispatched_core::messaging::DEFAULT_SCAN_LIMIT;
pub use dispatched_core::namespace::known_entities;
use dispatched_core::{
    Continuation, EntityCounts, EntityPath, Explorer, ExportFormat, ExportedMessage, FileExport,
    MessageBody, MessageFilter, Namespace, PeekOptions, PeekPage, PeekView, PeekedMessage,
    QueueInfo, Streamed, SubscriptionInfo, TopicInfo, continuation, stream_pages, write_messages,
};
use horizon_azure_kit::safety::CancellationToken;
use std::future::Future;
use time::format_description::well_known::Rfc3339;

/// The namespace the user selected. The sign-in (a keychain read) is only
/// looked up when the selection uses Entra.
pub(crate) fn namespace(ctx: &Ctx, sel: &Selection<'_>) -> anyhow::Result<Namespace> {
    let sign_in = if needs_sign_in(sel, ctx)? {
        ctx.current_sign_in()?
    } else {
        None
    };
    resolve(
        sel,
        ctx,
        ctx.secrets.as_ref(),
        &ctx.session,
        sign_in.as_ref(),
    )
}

/// An explorer for the namespace the user selected.
pub(crate) fn explorer_for(ctx: &Ctx, sel: &Selection<'_>) -> anyhow::Result<Explorer> {
    Ok(Explorer::new(namespace(ctx, sel)?)?)
}

/// The entities pinned on the profile the selection resolves to. Empty when
/// the selection is a connection string or a bare namespace.
fn pins_for(ctx: &Ctx, sel: &Selection<'_>) -> anyhow::Result<Vec<String>> {
    if sel.connection_string.is_some() || sel.namespace.is_some() {
        return Ok(Vec::new());
    }
    let profile = match sel.profile {
        Some(name) => ctx.store()?.get(name)?,
        None => ctx.store()?.default_profile()?,
    };
    Ok(profile.map(|p| p.pinned_entities).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Flags shared by `peek` and `export`
// ---------------------------------------------------------------------------

/// Which entity and view to read, and how to filter it.
#[derive(Args, Debug, Default)]
pub struct ViewArgs {
    /// `queue` or `topic/subscriptions/subscription`. Defaults to the
    /// connection string's EntityPath, then to the profile's single pinned entity.
    pub entity: Option<String>,
    /// View: active, scheduled, deferred, dlq or transfer-dlq.
    #[arg(long, conflicts_with_all = ["dlq", "transfer_dlq"])]
    pub view: Option<String>,
    /// Same as `--view dlq`.
    #[arg(long, conflicts_with = "transfer_dlq")]
    pub dlq: bool,
    /// Same as `--view transfer-dlq`.
    #[arg(long)]
    pub transfer_dlq: bool,
    /// Start at this sequence number.
    #[arg(long, default_value_t = 0)]
    pub from_seq: i64,
    /// Examine at most this many messages (matching or not) per peek call; for
    /// `export`, per page of the export.
    #[arg(long, default_value_t = DEFAULT_SCAN_LIMIT, value_parser = clap::value_parser!(u32).range(1..))]
    pub scan_limit: u32,
    /// Property filter, e.g. "sys.DeliveryCount > 1 AND user.kind = 'x'".
    #[arg(long)]
    pub filter: Option<String>,
    /// Keep messages whose body contains this text (case-insensitive).
    #[arg(long)]
    pub body_contains: Option<String>,
    /// Keep messages whose body matches this regular expression.
    #[arg(long)]
    pub body_regex: Option<String>,
    /// Keep messages whose JSON body has a value at this JSONPath, e.g. $.order.status.
    #[arg(long)]
    pub body_json: Option<String>,
    /// With --body-json: the value must equal this.
    #[arg(long, requires = "body_json")]
    pub body_json_eq: Option<String>,
    /// Only messages of this session (exact session id).
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Default, Clone, Copy)]
pub struct FilterArgs<'a> {
    pub filter: Option<&'a str>,
    pub body_contains: Option<&'a str>,
    pub body_regex: Option<&'a str>,
    pub body_json: Option<&'a str>,
    pub body_json_eq: Option<&'a str>,
    pub session: Option<&'a str>,
}

impl ViewArgs {
    fn filter_args(&self) -> FilterArgs<'_> {
        FilterArgs {
            filter: self.filter.as_deref(),
            body_contains: self.body_contains.as_deref(),
            body_regex: self.body_regex.as_deref(),
            body_json: self.body_json.as_deref(),
            body_json_eq: self.body_json_eq.as_deref(),
            session: self.session.as_deref(),
        }
    }
}

pub fn build_filter(a: &FilterArgs<'_>) -> anyhow::Result<MessageFilter> {
    let set = |s: Option<&str>| s.is_some_and(|s| !s.trim().is_empty());
    if set(a.body_json_eq) && !set(a.body_json) {
        anyhow::bail!("--body-json-eq needs --body-json PATH");
    }
    let spec = dispatched_core::FilterSpec {
        property: a.filter.map(str::to_string),
        body_contains: a.body_contains.map(str::to_string),
        body_regex: a.body_regex.map(str::to_string),
        body_json_path: a.body_json.map(str::to_string),
        body_json_equals: a.body_json_eq.map(str::to_string),
        session_id: a.session.map(str::to_string),
    };
    Ok(spec.build()?)
}

pub fn pick_view(view: Option<&str>, dlq: bool, transfer_dlq: bool) -> anyhow::Result<PeekView> {
    Ok(match (view, dlq, transfer_dlq) {
        (Some(v), false, false) => PeekView::parse(v)?,
        (None, true, false) => PeekView::DeadLetter,
        (None, false, true) => PeekView::TransferDeadLetter,
        (None, false, false) => PeekView::Active,
        _ => anyhow::bail!("use only one of --view, --dlq, --transfer-dlq"),
    })
}

pub fn default_entity(
    explicit: Option<&str>,
    ns: &Namespace,
    pins: &[String],
) -> anyhow::Result<EntityPath> {
    if let Some(e) = explicit {
        return Ok(EntityPath::parse(e)?);
    }
    if let Some(e) = ns.entity_path() {
        return Ok(EntityPath::parse(e)?);
    }
    if let [only] = pins {
        return Ok(EntityPath::parse(only)?);
    }
    anyhow::bail!("entity required: pass <queue> or <topic>/subscriptions/<sub>")
}

/// Everything a `peek`/`export` needs, with the namespace resolved and the
/// flags validated (flags first, so a typo fails before any keychain access).
struct Query {
    explorer: Explorer,
    entity: EntityPath,
    view: PeekView,
    filter: MessageFilter,
    from_seq: i64,
    scan_limit: u32,
}

impl Query {
    fn open(ctx: &Ctx, sel: &Selection<'_>, args: &ViewArgs) -> anyhow::Result<Self> {
        let view = pick_view(args.view.as_deref(), args.dlq, args.transfer_dlq)?;
        let filter = build_filter(&args.filter_args())?;
        let ns = namespace(ctx, sel)?;
        let pins = if args.entity.is_some() || ns.entity_path().is_some() {
            Vec::new()
        } else {
            pins_for(ctx, sel)?
        };
        let explorer = Explorer::new(ns)?;
        let entity = default_entity(args.entity.as_deref(), explorer.namespace(), &pins)?;
        Ok(Self {
            explorer,
            entity,
            view,
            filter,
            from_seq: args.from_seq,
            scan_limit: args.scan_limit,
        })
    }

    fn options(&self, max: u32, from: i64) -> PeekOptions {
        PeekOptions::new(max)
            .from_sequence(from)
            .scan_limit(self.scan_limit)
            .filter(self.filter.clone())
    }
}

// ---------------------------------------------------------------------------
// Where a scan ended
// ---------------------------------------------------------------------------

/// What to tell the user (on stderr) about how a scan ended.
pub struct ScanSummary {
    pub scanned: u64,
    pub matched: usize,
    /// The match limit that was asked for.
    pub top: u32,
    /// The per-peek-call scan limit, when it can end the scan (`peek`).
    pub scan_limit: Option<u32>,
    pub end: Continuation,
}

pub fn scan_notes(s: &ScanSummary) -> Vec<String> {
    let ScanSummary {
        scanned,
        matched,
        top,
        scan_limit,
        end,
    } = *s;
    match end {
        Continuation::End => vec!["(end of entity)".into()],
        Continuation::More(n) => {
            let limit_hit = scan_limit.is_some_and(|l| scanned >= u64::from(l))
                && (matched as u64) < u64::from(top);
            if limit_hit {
                vec![format!(
                    "(scan limit reached: scanned {scanned} message(s), {matched} matched; continue with --from-seq {n})"
                )]
            } else {
                vec![format!(
                    "(more: continue with --from-seq {n}; scanned {scanned})"
                )]
            }
        }
        Continuation::Done => Vec::new(),
        Continuation::Stalled => vec![format!(
            "warning: the scan stopped without progress after {scanned} message(s), so the end of the entity was not confirmed; \
             the service returned nothing new from this position. Try again, or pass a different --from-seq"
        )],
    }
}

fn print_notes(notes: &[String]) {
    for n in notes {
        eprintln!("{}", sanitize(n));
    }
}

// ---------------------------------------------------------------------------
// ls
// ---------------------------------------------------------------------------

const APPROX_FOOTNOTE: &str = "~ counted by peeking (the service reports no counts)";
const FALLBACK_NOTE: &str = "listing needs Manage,EntityRead; showing known entities";

/// A count cell: `?` when unknown, `~N` when counted by peeking, `~N+` when
/// that count hit its cap.
fn count_cell(counts: Option<&EntityCounts>, pick: fn(&EntityCounts) -> u64) -> String {
    match counts {
        None => "?".into(),
        Some(c) => {
            let n = pick(c);
            match (c.approximate, c.truncated) {
                (_, true) => format!("~{n}+"),
                (true, false) => format!("~{n}"),
                (false, false) => n.to_string(),
            }
        }
    }
}

/// Whether any count was rendered with a leading `~`.
fn any_approximate(queues: &[QueueInfo], subs: &[&SubscriptionInfo]) -> bool {
    let flagged =
        |c: &Option<EntityCounts>| c.as_ref().is_some_and(|c| c.approximate || c.truncated);
    queues.iter().any(|q| flagged(&q.counts)) || subs.iter().any(|s| flagged(&s.counts))
}

fn queues_table(queues: &[QueueInfo]) -> String {
    let rows: Vec<Vec<String>> = queues
        .iter()
        .map(|q| {
            vec![
                q.name.clone(),
                count_cell(q.counts.as_ref(), |c| c.active),
                count_cell(q.counts.as_ref(), |c| c.dead_letter),
                count_cell(q.counts.as_ref(), |c| c.scheduled),
                if q.requires_session {
                    "yes".into()
                } else {
                    String::new()
                },
                q.status.clone().unwrap_or_default(),
            ]
        })
        .collect();
    table(
        &["QUEUE", "ACTIVE", "DLQ", "SCHEDULED", "SESSIONS", "STATUS"],
        &rows,
    )
}

fn subscriptions_table(subs: &[&SubscriptionInfo]) -> String {
    let rows: Vec<Vec<String>> = subs
        .iter()
        .map(|s| {
            vec![
                format!("{}/subscriptions/{}", s.topic, s.name),
                count_cell(s.counts.as_ref(), |c| c.active),
                count_cell(s.counts.as_ref(), |c| c.dead_letter),
                s.status.clone().unwrap_or_default(),
            ]
        })
        .collect();
    table(&["SUBSCRIPTION", "ACTIVE", "DLQ", "STATUS"], &rows)
}

/// The queues table, or a note when the namespace has none.
fn queues_section(queues: &[QueueInfo]) -> String {
    if queues.is_empty() {
        "(no queues)".into()
    } else {
        queues_table(queues)
    }
}

/// The subscriptions table, or a note when there are no topics or no
/// subscriptions to show.
fn subscriptions_section(topic_count: usize, subs: &[&SubscriptionInfo]) -> String {
    if topic_count == 0 {
        "(no topics)".into()
    } else if subs.is_empty() {
        "(no subscriptions)".into()
    } else {
        subscriptions_table(subs)
    }
}

/// Known entities as tables, every count `?`.
pub fn fallback_tables(known: &[EntityPath]) -> String {
    let queues: Vec<Vec<String>> = known
        .iter()
        .filter(|e| e.subscription.is_none())
        .map(|e| {
            vec![
                e.entity.clone(),
                "?".into(),
                "?".into(),
                "?".into(),
                String::new(),
                String::new(),
            ]
        })
        .collect();
    let subs: Vec<Vec<String>> = known
        .iter()
        .filter_map(|e| {
            e.subscription.as_ref().map(|s| {
                vec![
                    format!("{}/subscriptions/{s}", e.entity),
                    "?".into(),
                    "?".into(),
                    String::new(),
                ]
            })
        })
        .collect();
    let mut parts = Vec::new();
    if !queues.is_empty() {
        parts.push(table(
            &["QUEUE", "ACTIVE", "DLQ", "SCHEDULED", "SESSIONS", "STATUS"],
            &queues,
        ));
    }
    if !subs.is_empty() {
        parts.push(table(&["SUBSCRIPTION", "ACTIVE", "DLQ", "STATUS"], &subs));
    }
    parts.join("\n\n")
}

pub fn fallback_json(known: &[EntityPath]) -> serde_json::Value {
    let queues: Vec<_> = known
        .iter()
        .filter(|e| e.subscription.is_none())
        .map(|e| serde_json::json!({ "name": e.entity, "counts": null }))
        .collect();
    let subs: Vec<_> = known
        .iter()
        .filter_map(|e| {
            e.subscription
                .as_ref()
                .map(|s| serde_json::json!({ "topic": e.entity, "name": s, "counts": null }))
        })
        .collect();
    serde_json::json!({ "queues": queues, "subscriptions": subs, "note": FALLBACK_NOTE })
}

pub async fn ls(
    ctx: &Ctx,
    sel: &Selection<'_>,
    topics: bool,
    format: Format,
) -> anyhow::Result<()> {
    let ns = namespace(ctx, sel)?;
    let pins = pins_for(ctx, sel)?;
    let explorer = Explorer::new(ns)?;
    let result = list(&explorer, topics, format, &pins).await;
    explorer.close().await;
    result
}

type TopicListing = Vec<(TopicInfo, Vec<SubscriptionInfo>)>;

const LIST_FORBIDDEN_HINT: &str = "this key can't list entities; peek a known entity with `dispatched peek <entity>` or pin it with `dispatched profile pin <profile> <entity>`";
const TOPICS_SKIPPED_NOTE: &str =
    "topics and subscriptions skipped: listing them needs Manage,EntityRead";

/// The original listing error with a hint on what to do instead.
pub fn forbidden_without_known(e: dispatched_core::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}; {LIST_FORBIDDEN_HINT}")
}

/// A refused topic listing is not fatal once the queues were listed:
/// `Ok(None)` means skip the topics. Other errors still fail the command.
pub fn topics_outcome(
    r: dispatched_core::Result<TopicListing>,
) -> anyhow::Result<Option<TopicListing>> {
    match r {
        Ok(t) => Ok(Some(t)),
        Err(dispatched_core::Error::Forbidden { .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn fetch_topics(explorer: &Explorer) -> dispatched_core::Result<TopicListing> {
    let mut out = Vec::new();
    for t in explorer.list_topics().await? {
        let subs = explorer.list_subscriptions(&t.name).await?;
        out.push((t, subs));
    }
    Ok(out)
}

async fn list(
    explorer: &Explorer,
    topics: bool,
    format: Format,
    pins: &[String],
) -> anyhow::Result<()> {
    let queues = match explorer.list_queues().await {
        Ok(q) => q,
        Err(e @ dispatched_core::Error::Forbidden { .. }) => {
            let known = known_entities(explorer.namespace().entity_path(), pins);
            if known.is_empty() {
                return Err(forbidden_without_known(e));
            }
            eprintln!("{FALLBACK_NOTE}");
            if let Format::Json = format {
                println!("{}", serde_json::to_string_pretty(&fallback_json(&known))?);
            } else {
                println!("{}", fallback_tables(&known));
            }
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let skipped;
    let topic_list = if topics {
        let listing = topics_outcome(fetch_topics(explorer).await)?;
        skipped = listing.is_none();
        listing.unwrap_or_default()
    } else {
        skipped = false;
        Vec::new()
    };
    if skipped {
        eprintln!("{TOPICS_SKIPPED_NOTE}");
    }
    if let Format::Json = format {
        let mut value = serde_json::json!({ "queues": queues });
        if skipped {
            value["topics_skipped"] = serde_json::Value::String(TOPICS_SKIPPED_NOTE.into());
        } else if topics {
            let list: Vec<_> = topic_list
                .iter()
                .map(|(t, subs)| serde_json::json!({ "topic": t, "subscriptions": subs }))
                .collect();
            value["topics"] = serde_json::Value::Array(list);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let subs: Vec<&SubscriptionInfo> = topic_list.iter().flat_map(|(_, s)| s.iter()).collect();
    println!("{}", queues_section(&queues));
    if topics && !skipped {
        println!("\n{}", subscriptions_section(topic_list.len(), &subs));
    }
    if any_approximate(&queues, &subs) {
        eprintln!("{APPROX_FOOTNOTE}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// peek
// ---------------------------------------------------------------------------

fn body_cell(body: &MessageBody) -> String {
    match body.as_text() {
        Some(text) => ellipsize(&text, 60),
        None => format!("<{} bytes>", body.len()),
    }
}

/// The peek table. `STATE` appears for the scheduled and deferred views,
/// `REASON` for the dead-letter views. Every cell is sanitised by `table`.
pub fn messages_table(messages: &[PeekedMessage], view: PeekView) -> String {
    let state = matches!(view, PeekView::Scheduled | PeekView::Deferred);
    let reason = matches!(view, PeekView::DeadLetter | PeekView::TransferDeadLetter);
    let mut headers = vec!["SEQ"];
    if state {
        headers.push("STATE");
    }
    headers.extend(["ENQUEUED", "MESSAGE ID", "DELIV", "SUBJECT"]);
    if reason {
        headers.push("REASON");
    }
    headers.push("BODY");
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            let mut row = vec![m.sequence_number.to_string()];
            if state {
                row.push(m.state.as_str().to_string());
            }
            row.push(
                m.enqueued_time
                    .and_then(|t| t.format(&Rfc3339).ok())
                    .unwrap_or_default(),
            );
            row.push(ellipsize(m.message_id.as_deref().unwrap_or(""), 36));
            row.push(m.delivery_count.to_string());
            row.push(ellipsize(m.subject.as_deref().unwrap_or(""), 24));
            if reason {
                row.push(ellipsize(m.dead_letter_reason.as_deref().unwrap_or(""), 32));
            }
            row.push(body_cell(&m.body));
            row
        })
        .collect();
    table(&headers, &rows)
}

/// `peek --format json`. `exhausted` is an addition to the documented shape:
/// with `next_sequence: null` it tells "end of entity" from "stopped without
/// progress".
pub fn peek_json(page: &PeekPage) -> serde_json::Value {
    serde_json::json!({
        "messages": page.messages.iter().map(ExportedMessage::new).collect::<Vec<_>>(),
        "scanned": page.scanned,
        "next_sequence": page.next_sequence,
        "exhausted": page.exhausted,
    })
}

/// The most `peek --from-end` returns; core pages the tail in the same size.
pub const MAX_TAIL: u32 = 5_000;
/// The smallest scan budget for `peek --from-end`: finding the end and reading back
/// from it needs more than a forward peek's default.
pub const TAIL_MIN_SCAN: u32 = 100_000;

/// What to tell the user (on stderr) about a tail peek. `limit` is the scan budget it ran with.
pub fn tail_notes(page: &PeekPage, top: u32, capped: bool, limit: u32) -> Vec<String> {
    let n = page.messages.len();
    let mut notes = Vec::new();
    if capped {
        notes.push(format!(
            "(--top is limited to {MAX_TAIL} with --from-end; showing at most {MAX_TAIL})"
        ));
    }
    if !page.exhausted {
        notes.push(format!(
            "warning: scanned {} message(s) without reaching the end of the entity, so these {n} \
             match(es) are not the last ones. Narrow the filter or raise --scan-limit above {}",
            page.scanned,
            thousands(u64::from(TAIL_MIN_SCAN))
        ));
    } else if (n as u64) >= u64::from(top) {
        notes.push(format!(
            "(last {n} matching message(s); scanned {})",
            page.scanned
        ));
    } else if page.scanned >= limit {
        // The budget ended the search, not the start of the entity: older messages may match.
        let what = match n {
            0 => "no matching messages".to_string(),
            n => format!("{n} matching message(s)"),
        };
        notes.push(format!(
            "({what} in the newest {}; older messages were not examined)",
            thousands(u64::from(limit))
        ));
    } else if n == 0 {
        notes.push(format!("(no matching messages; scanned {})", page.scanned));
    } else {
        notes.push(format!(
            "(only {n} matching message(s) found, {top} asked for; scanned {})",
            page.scanned
        ));
    }
    notes
}

pub async fn peek(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ViewArgs,
    top: u32,
    from_end: bool,
    format: Format,
) -> anyhow::Result<()> {
    let q = Query::open(ctx, sel, args)?;
    if from_end {
        return peek_tail(q, top, format).await;
    }
    let page = q
        .explorer
        .peek(&q.entity, q.view, &q.options(top, q.from_seq))
        .await;
    q.explorer.close().await;
    let page = page?;
    let end = continuation(&page);
    let notes = scan_notes(&ScanSummary {
        scanned: u64::from(page.scanned),
        matched: page.messages.len(),
        top,
        scan_limit: Some(q.scan_limit),
        end,
    });
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(&peek_json(&page))?);
        if end != Continuation::End {
            print_notes(&notes);
        }
    } else {
        println!("{}", messages_table(&page.messages, q.view));
        if page.messages.is_empty() {
            eprintln!("(no messages)");
        }
        print_notes(&notes);
    }
    Ok(())
}

/// `peek --from-end`: the last `top` matches, oldest first, with no continuation.
async fn peek_tail(q: Query, top: u32, format: Format) -> anyhow::Result<()> {
    let capped = top > MAX_TAIL;
    let top = top.min(MAX_TAIL);
    let limit = q.scan_limit.max(TAIL_MIN_SCAN);
    let opts = PeekOptions::new(top)
        .scan_limit(limit)
        .filter(q.filter.clone());
    let page = q.explorer.peek_tail(&q.entity, q.view, &opts).await;
    q.explorer.close().await;
    let page = page?;
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(&peek_json(&page))?);
    } else {
        // An empty tail says so once, in the note below.
        println!("{}", messages_table(&page.messages, q.view));
    }
    print_notes(&tail_notes(&page, top, capped, limit));
    Ok(())
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

/// `--format` for `export`: JSON unless NDJSON is asked for.
pub fn export_format(format: Option<Format>) -> anyhow::Result<ExportFormat> {
    match format {
        None | Some(Format::Json) => Ok(ExportFormat::Json),
        Some(Format::Ndjson) => Ok(ExportFormat::Ndjson),
        Some(Format::Table) => {
            anyhow::bail!("export writes json or ndjson; pass --format json or --format ndjson")
        }
    }
}

fn exists_message(path: &str) -> String {
    format!("{path} exists; pass --force to overwrite it, or choose another --out")
}

/// Fail early (before any network work) when `path` exists and `force` is off.
/// Devices and pipes are not "existing files" and never need `--force`.
pub fn check_output(path: &str, force: bool) -> anyhow::Result<()> {
    let p = std::path::Path::new(path);
    if path != "-" && !force && p.exists() && !dispatched_core::export::is_special(p) {
        anyhow::bail!(exists_message(path));
    }
    Ok(())
}

/// An export whose scan stopped without progress is incomplete: the file is
/// written, but the command must not look like a clean success.
pub fn check_complete(end: Continuation, written: usize) -> anyhow::Result<()> {
    if end == Continuation::Stalled {
        anyhow::bail!(
            "export incomplete: the scan stopped without progress, so the end of the entity was not confirmed ({written} message(s) were written)"
        );
    }
    Ok(())
}

/// A token that Ctrl-C cancels. A second Ctrl-C, while a page is still loading, ends the
/// process at once.
fn cancel_on_ctrl_c() -> CancellationToken {
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            t.cancel();
            if tokio::signal::ctrl_c().await.is_ok() {
                std::process::exit(130);
            }
        }
    });
    token
}

/// Core refuses to replace a file that appeared while the export ran; say how to allow it.
fn finish_error(e: dispatched_core::Error, force: bool, out: &str) -> anyhow::Error {
    match e {
        dispatched_core::Error::Invalid(_) if !force => anyhow::anyhow!(exists_message(out)),
        e => e.into(),
    }
}

/// Stream the pages to `out` (`-` is stdout). A cancelled or failed export leaves no file and
/// an existing `out` untouched.
async fn export_stream<F, Fut>(
    top: u32,
    from_seq: i64,
    out: &str,
    format: ExportFormat,
    force: bool,
    cancel: &CancellationToken,
    fetch: F,
) -> anyhow::Result<Streamed>
where
    F: FnMut(i64, u32) -> Fut,
    Fut: Future<Output = dispatched_core::Result<PeekPage>>,
{
    let top = Some(u64::from(top));
    let streamed = if out == "-" {
        let mut all = Vec::new();
        let s = stream_pages(top, from_seq, Some(cancel), fetch, |m, _| {
            all.extend(m);
            Ok(())
        })
        .await;
        s.and_then(|s| write_messages(std::io::stdout().lock(), &all, format).map(|_| s))
            .map_err(anyhow::Error::from)
    } else {
        let mut file = FileExport::create(std::path::Path::new(out), format, force)?;
        let s = stream_pages(top, from_seq, Some(cancel), fetch, |m, _| file.write(&m)).await;
        match s {
            Ok(s) => file
                .finish()
                .map(|_| s)
                .map_err(|e| finish_error(e, force, out)),
            Err(e) => Err(e.into()),
        }
    };
    streamed.map_err(|e| match e.downcast_ref::<dispatched_core::Error>() {
        Some(dispatched_core::Error::Cancelled) => {
            anyhow::anyhow!("Export cancelled; nothing was written.")
        }
        _ => e,
    })
}

pub async fn export(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ViewArgs,
    top: u32,
    out: &str,
    format: ExportFormat,
    force: bool,
) -> anyhow::Result<()> {
    check_output(out, force)?;
    let q = Query::open(ctx, sel, args)?;
    let qr = &q;
    let cancel = cancel_on_ctrl_c();
    let result = export_stream(top, q.from_seq, out, format, force, &cancel, |from, max| {
        let opts = qr.options(max, from);
        async move { qr.explorer.peek(&qr.entity, qr.view, &opts).await }
    })
    .await;
    q.explorer.close().await;
    let s = result?;
    let n = s.written as usize;
    let dest = if out == "-" {
        "stdout".to_string()
    } else {
        sanitize(out)
    };
    eprintln!("Exported {n} message(s) to {dest}");
    print_notes(&scan_notes(&ScanSummary {
        scanned: s.scanned,
        matched: n,
        top,
        scan_limit: None,
        end: s.end,
    }));
    check_complete(s.end, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{MessageState, PropValue};
    use std::collections::BTreeMap;

    fn msg(seq: i64) -> PeekedMessage {
        PeekedMessage {
            sequence_number: seq,
            state: MessageState::Active,
            message_id: Some(format!("id-{seq}")),
            correlation_id: None,
            session_id: None,
            subject: None,
            content_type: None,
            to: None,
            reply_to: None,
            enqueued_time: None,
            scheduled_enqueue_time: None,
            locked_until: None,
            delivery_count: 1,
            time_to_live_ms: None,
            dead_letter_reason: None,
            dead_letter_description: None,
            dead_letter_source: None,
            application_properties: BTreeMap::<String, PropValue>::new(),
            body: MessageBody::Text("hello".into()),
        }
    }

    fn page(seqs: &[i64], scanned: u32, next: Option<i64>, exhausted: bool) -> PeekPage {
        PeekPage {
            messages: seqs.iter().map(|s| msg(*s)).collect(),
            scanned,
            next_sequence: next,
            exhausted,
        }
    }

    // --- brief tests ---

    #[test]
    fn filter_flags_combine() {
        let f = build_filter(&FilterArgs {
            filter: Some("sys.DeliveryCount > 1"),
            body_contains: Some("x"),
            body_json: Some("$.a"),
            body_json_eq: Some("1"),
            ..Default::default()
        })
        .unwrap();
        assert!(f.property.is_some());
        assert_eq!(f.body.len(), 2);
        assert!(
            build_filter(&FilterArgs {
                body_json_eq: Some("1"),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            build_filter(&FilterArgs {
                filter: Some("a ="),
                ..Default::default()
            })
            .unwrap_err()
            .to_string()
            .contains("column")
        );
        assert!(
            build_filter(&FilterArgs {
                body_regex: Some("("),
                ..Default::default()
            })
            .is_err()
        );
        assert!(build_filter(&FilterArgs::default()).unwrap().is_empty());
        let blank = build_filter(&FilterArgs {
            filter: Some(""),
            body_regex: Some("  "),
            body_json: Some(" "),
            body_json_eq: Some(" "),
            ..Default::default()
        })
        .unwrap();
        assert!(blank.is_empty(), "blank flags mean no condition");
    }

    #[test]
    fn views_and_aliases() {
        assert_eq!(
            pick_view(Some("scheduled"), false, false).unwrap(),
            PeekView::Scheduled
        );
        assert_eq!(pick_view(None, true, false).unwrap(), PeekView::DeadLetter);
        assert_eq!(
            pick_view(None, false, true).unwrap(),
            PeekView::TransferDeadLetter
        );
        assert_eq!(pick_view(None, false, false).unwrap(), PeekView::Active);
        assert!(pick_view(Some("active"), true, false).is_err());
        assert!(pick_view(None, true, true).is_err());
    }

    #[test]
    fn entity_defaults() {
        let cs = dispatched_core::ConnectionString::parse("Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=orders").unwrap();
        let ns = Namespace::from_connection_string(&cs).unwrap();
        assert_eq!(default_entity(None, &ns, &[]).unwrap().base(), "orders");
        // An explicit entity wins over the connection string's EntityPath.
        assert_eq!(
            default_entity(Some("other"), &ns, &[]).unwrap().base(),
            "other"
        );
        let cs2 = dispatched_core::ConnectionString::parse(
            "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v",
        )
        .unwrap();
        let ns2 = Namespace::from_connection_string(&cs2).unwrap();
        assert_eq!(
            default_entity(None, &ns2, &["events/subscriptions/all".into()])
                .unwrap()
                .base(),
            "events/Subscriptions/all"
        );
        let err = default_entity(None, &ns2, &[]).unwrap_err().to_string();
        assert!(err.contains("entity required"), "{err}");
        // Two pins are ambiguous.
        assert!(default_entity(None, &ns2, &["a".into(), "b".into()]).is_err());
    }

    // --- how a scan ended ---

    fn summary(scanned: u64, matched: usize, limit: Option<u32>, end: Continuation) -> ScanSummary {
        ScanSummary {
            scanned,
            matched,
            top: 20,
            scan_limit: limit,
            end,
        }
    }

    #[test]
    fn notes_for_end_more_and_stalled() {
        let end = scan_notes(&summary(5, 5, Some(10_000), Continuation::End));
        assert_eq!(end, vec!["(end of entity)"]);

        let more = scan_notes(&summary(20, 20, Some(10_000), Continuation::More(21)));
        assert_eq!(
            more,
            vec!["(more: continue with --from-seq 21; scanned 20)"]
        );

        let stalled = scan_notes(&summary(0, 0, Some(10_000), Continuation::Stalled));
        assert_eq!(stalled.len(), 1);
        assert!(stalled[0].starts_with("warning:"), "{stalled:?}");
        assert!(stalled[0].contains("without progress"));
        assert!(stalled[0].contains("not confirmed"));
        // Never presented as a clean end.
        assert!(!stalled[0].contains("end of entity)"));
    }

    #[test]
    fn scan_limit_with_few_matches_says_how_many_were_scanned_and_how_to_continue() {
        let n = scan_notes(&summary(
            10_000,
            1,
            Some(10_000),
            Continuation::More(10_001),
        ));
        assert_eq!(n.len(), 1);
        assert!(n[0].contains("scan limit reached"), "{n:?}");
        assert!(n[0].contains("scanned 10000"));
        assert!(n[0].contains("1 matched"));
        assert!(n[0].contains("--from-seq 10001"));
        // With no matches at all.
        let n = scan_notes(&summary(250, 0, Some(250), Continuation::More(251)));
        assert!(n[0].contains("0 matched") && n[0].contains("--from-seq 251"));
        // Reaching --top is not a scan-limit stop, even if the counts coincide.
        let n = scan_notes(&summary(20, 20, Some(20), Continuation::More(21)));
        assert!(n[0].starts_with("(more:"), "{n:?}");
        // Export has no scan limit.
        let n = scan_notes(&summary(50_000, 3, None, Continuation::More(50_001)));
        assert!(n[0].starts_with("(more:"), "{n:?}");
    }

    #[test]
    fn tail_notes_say_whether_the_end_was_reached() {
        let lim = TAIL_MIN_SCAN;
        let done = tail_notes(&page(&[8, 9], 40, None, true), 2, false, lim);
        assert_eq!(done, vec!["(last 2 matching message(s); scanned 40)"]);
        let few = tail_notes(&page(&[9], 40, None, true), 5, false, lim);
        assert_eq!(
            few,
            vec!["(only 1 matching message(s) found, 5 asked for; scanned 40)"]
        );
        let cut = tail_notes(&page(&[1], 100_000, None, false), 5, false, lim);
        assert_eq!(cut.len(), 1);
        assert!(
            cut[0].starts_with("warning:") && cut[0].contains("without reaching the end"),
            "{cut:?}"
        );
        assert!(
            cut[0].contains("--scan-limit above 100,000"),
            "the hint names the floor: {cut:?}"
        );
        assert!(!cut[0].contains("last 1 matching"), "{cut:?}");
        let capped = tail_notes(&page(&[1], 10, None, true), MAX_TAIL, true, lim);
        assert!(capped[0].contains("5000"), "{capped:?}");
    }

    #[test]
    fn a_short_tail_that_hit_the_scan_limit_does_not_claim_the_whole_entity() {
        let lim = TAIL_MIN_SCAN;
        let n = tail_notes(&page(&[7, 8, 9], 100_000, None, true), 5, false, lim);
        assert_eq!(
            n,
            vec!["(3 matching message(s) in the newest 100,000; older messages were not examined)"]
        );
        assert!(!n[0].contains("only"), "{n:?}");
        let none = tail_notes(&page(&[], 100_000, None, true), 5, false, lim);
        assert_eq!(
            none,
            vec!["(no matching messages in the newest 100,000; older messages were not examined)"]
        );
        // A raised --scan-limit is the limit that counts.
        let n = tail_notes(&page(&[9], 250_000, None, true), 5, false, 250_000);
        assert!(n[0].contains("newest 250,000"), "{n:?}");
        // Reaching --top is a clean tail whatever was scanned.
        let full = tail_notes(&page(&[8, 9], 100_000, None, true), 2, false, lim);
        assert_eq!(full, vec!["(last 2 matching message(s); scanned 100000)"]);
    }

    #[test]
    fn an_empty_tail_is_one_line() {
        let n = tail_notes(&page(&[], 40, None, true), 5, false, TAIL_MIN_SCAN);
        assert_eq!(n, vec!["(no matching messages; scanned 40)"]);
    }

    // --- tables and JSON ---

    #[test]
    fn state_column_for_scheduled_and_deferred_reason_for_dlq() {
        let mut m = msg(7);
        m.state = MessageState::Scheduled;
        m.dead_letter_reason = Some("MaxDelivery".into());
        let header = |v| {
            messages_table(std::slice::from_ref(&m), v)
                .lines()
                .next()
                .unwrap()
                .to_string()
        };
        let h = header(PeekView::Scheduled);
        assert!(h.contains("STATE") && !h.contains("REASON"), "{h}");
        assert!(header(PeekView::Deferred).contains("STATE"));
        let h = header(PeekView::DeadLetter);
        assert!(h.contains("REASON") && !h.contains("STATE"), "{h}");
        assert!(header(PeekView::TransferDeadLetter).contains("REASON"));
        let h = header(PeekView::Active);
        assert!(!h.contains("STATE") && !h.contains("REASON"), "{h}");
        let t = messages_table(std::slice::from_ref(&m), PeekView::Scheduled);
        assert!(t.lines().nth(1).unwrap().contains("scheduled"));
    }

    #[test]
    fn peek_table_neutralises_terminal_control_characters() {
        let mut m = msg(1);
        m.message_id = Some("\x1b[31mid\x1b[0m".into());
        m.subject = Some("a\u{202e}b\nc".into());
        m.body = MessageBody::Text("x\x07y\x1b]8;;http://e\x07z".into());
        let t = messages_table(&[m], PeekView::Active);
        assert!(
            !t.chars()
                .any(|c| (c.is_control() && c != '\n') || c == '\u{202e}')
        );
        assert_eq!(t.lines().count(), 2, "{t:?}");
    }

    #[test]
    fn peek_json_has_the_documented_shape_and_stays_faithful() {
        let mut m = msg(3);
        m.body = MessageBody::Text("a\x1b[31m\u{202e}b".into());
        let p = PeekPage {
            messages: vec![m],
            scanned: 9,
            next_sequence: Some(4),
            exhausted: false,
        };
        let v = peek_json(&p);
        assert_eq!(v["scanned"], 9);
        assert_eq!(v["next_sequence"], 4);
        assert_eq!(v["exhausted"], false);
        assert_eq!(v["messages"][0]["schema"], dispatched_core::EXPORT_SCHEMA);
        assert_eq!(v["messages"][0]["sequence_number"], 3);
        // Control characters survive in the value (serde escapes them on output).
        let text = serde_json::to_string(&v).unwrap();
        assert!(!text.contains('\x1b'));
        assert!(text.contains("\\u001b"));
        let back: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back, v);
        let null = peek_json(&page(&[], 0, None, true));
        assert!(null["next_sequence"].is_null());
        assert_eq!(null["exhausted"], true);
    }

    // --- export output ---

    #[test]
    fn export_format_maps_the_global_flag() {
        assert_eq!(export_format(None).unwrap(), ExportFormat::Json);
        assert_eq!(
            export_format(Some(Format::Json)).unwrap(),
            ExportFormat::Json
        );
        assert_eq!(
            export_format(Some(Format::Ndjson)).unwrap(),
            ExportFormat::Ndjson
        );
        assert!(export_format(Some(Format::Table)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn export_to_dev_null_needs_no_force() {
        check_output("/dev/null", false).unwrap();
        let mut f = FileExport::create(
            std::path::Path::new("/dev/null"),
            ExportFormat::Ndjson,
            false,
        )
        .unwrap();
        f.write(&[msg(1), msg(2)]).unwrap();
        assert_eq!(f.finish().unwrap(), 2);
    }

    #[test]
    fn export_refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let p = path.to_str().unwrap();
        std::fs::write(&path, "keep me").unwrap();
        let e = check_output(p, false).unwrap_err().to_string();
        assert!(e.contains("--force") && e.contains("exists"), "{e}");
        let e = FileExport::create(&path, ExportFormat::Json, false)
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("already exists"), "{e}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
        // stdout is never "existing".
        check_output("-", false).unwrap();
        // --force replaces it.
        check_output(p, true).unwrap();
        let mut f = FileExport::create(&path, ExportFormat::Json, true).unwrap();
        f.write(&[msg(1), msg(2)]).unwrap();
        assert_eq!(f.finish().unwrap(), 2);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    // --- export streaming ---

    fn one_page(seqs: &[i64], next: Option<i64>, exhausted: bool) -> PeekPage {
        PeekPage {
            messages: seqs.iter().map(|s| msg(*s)).collect(),
            scanned: seqs.len() as u32,
            next_sequence: next,
            exhausted,
        }
    }

    fn dir_names(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn a_cancelled_export_says_so_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let mut calls = 0;
        // The first page arrives, then Ctrl-C: the temp file must be gone, the target never made.
        let e = export_stream(
            10,
            0,
            path.to_str().unwrap(),
            ExportFormat::Json,
            false,
            &cancel,
            |_, _| {
                calls += 1;
                c.cancel();
                async { Ok(one_page(&[1, 2], Some(3), false)) }
            },
        )
        .await
        .unwrap_err();
        assert_eq!(e.to_string(), "Export cancelled; nothing was written.");
        assert_eq!(calls, 1);
        assert!(dir_names(dir.path()).is_empty());
        // An existing target stays as it was, with --force too.
        std::fs::write(&path, "keep").unwrap();
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let e = export_stream(
            10,
            0,
            path.to_str().unwrap(),
            ExportFormat::Json,
            true,
            &cancel,
            |_, _| {
                c.cancel();
                async { Ok(one_page(&[1], Some(2), false)) }
            },
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("cancelled"), "{e}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
        assert_eq!(dir_names(dir.path()), ["out.json"]);
    }

    #[tokio::test]
    async fn a_file_that_appears_during_the_export_is_kept_and_the_error_names_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("race.json");
        let p = path.clone();
        let e = export_stream(
            10,
            0,
            path.to_str().unwrap(),
            ExportFormat::Json,
            false,
            &CancellationToken::new(),
            |_, _| {
                std::fs::write(&p, "theirs").unwrap();
                async { Ok(one_page(&[1], None, true)) }
            },
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("--force"), "{e}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
        assert_eq!(dir_names(dir.path()), ["race.json"]);
    }

    #[tokio::test]
    async fn a_good_export_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.ndjson");
        let s = export_stream(
            10,
            0,
            path.to_str().unwrap(),
            ExportFormat::Ndjson,
            false,
            &CancellationToken::new(),
            |_, _| async { Ok(one_page(&[1, 2], None, true)) },
        )
        .await
        .unwrap();
        assert_eq!((s.written, s.end), (2, Continuation::End));
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
        assert_eq!(dir_names(dir.path()), ["ok.ndjson"]);
    }

    // --- ls ---

    fn counts(active: u64, approximate: bool, truncated: bool) -> EntityCounts {
        EntityCounts {
            active,
            approximate,
            truncated,
            ..Default::default()
        }
    }

    fn queue(name: &str, counts: Option<EntityCounts>) -> QueueInfo {
        QueueInfo {
            name: name.into(),
            status: None,
            counts,
            size_in_bytes: None,
            requires_session: false,
            max_delivery_count: None,
            lock_duration: None,
            forward_to: None,
        }
    }

    #[test]
    fn count_cells() {
        let pick: fn(&EntityCounts) -> u64 = |c| c.active;
        assert_eq!(count_cell(None, pick), "?");
        assert_eq!(count_cell(Some(&counts(5, false, false)), pick), "5");
        assert_eq!(count_cell(Some(&counts(5, true, false)), pick), "~5");
        assert_eq!(count_cell(Some(&counts(5, true, true)), pick), "~5+");
    }

    #[test]
    fn footnote_only_when_something_was_approximate() {
        let exact = [queue("a", Some(counts(1, false, false))), queue("b", None)];
        assert!(!any_approximate(&exact, &[]));
        let approx = [queue("a", Some(counts(1, true, false)))];
        assert!(any_approximate(&approx, &[]));
        let table = queues_table(&approx);
        assert!(table.contains("~1"), "{table}");
        let truncated = [queue("a", Some(counts(1, true, true)))];
        assert!(any_approximate(&truncated, &[]));
        assert_eq!(
            APPROX_FOOTNOTE,
            "~ counted by peeking (the service reports no counts)"
        );
    }

    #[test]
    fn empty_listings_say_so_instead_of_printing_a_bare_header() {
        assert_eq!(queues_section(&[]), "(no queues)");
        assert_eq!(subscriptions_section(0, &[]), "(no topics)");
        assert_eq!(subscriptions_section(2, &[]), "(no subscriptions)");
    }

    #[test]
    fn fallback_lists_known_entities_with_unknown_counts() {
        let k = known_entities(Some("orders"), &["events/subscriptions/all".to_string()]);
        let t = fallback_tables(&k);
        assert!(t.contains("QUEUE") && t.contains("orders"), "{t}");
        assert!(t.contains("SUBSCRIPTION") && t.contains("events/subscriptions/all"));
        let orders = t.lines().find(|l| l.starts_with("orders")).unwrap();
        assert_eq!(orders.matches('?').count(), 3, "{orders}");
        let j = fallback_json(&k);
        assert_eq!(j["queues"][0]["name"], "orders");
        assert!(j["queues"][0]["counts"].is_null());
        assert_eq!(j["subscriptions"][0]["topic"], "events");
        assert_eq!(
            j["note"],
            "listing needs Manage,EntityRead; showing known entities"
        );
    }

    #[test]
    fn connection_string_selection_never_opens_the_store() {
        use dispatched_core::profiles::ProfileStore;
        use horizon_azure_kit::secrets::MemoryStore;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let opens = Arc::new(AtomicUsize::new(0));
        let counter = opens.clone();
        let ctx = Ctx::lazy(
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(ProfileStore::open_in_memory()?)
            }),
            Arc::new(MemoryStore::new()),
        );
        let cs = "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=q";
        let by_cs = Selection {
            connection_string: Some(cs),
            namespace: None,
            profile: None,
        };
        let ns = namespace(&ctx, &by_cs).unwrap();
        assert_eq!(ns.entity_path(), Some("q"));
        assert!(pins_for(&ctx, &by_cs).unwrap().is_empty());
        assert_eq!(
            opens.load(Ordering::SeqCst),
            0,
            "store opened for a connection string"
        );
        // A profile selection (or the default profile) does open it.
        let none = Selection {
            connection_string: None,
            namespace: None,
            profile: None,
        };
        assert!(namespace(&ctx, &none).is_err());
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn pins_come_from_the_selected_profile_only() {
        use dispatched_core::profiles::ProfileStore;
        use horizon_azure_kit::secrets::MemoryStore;
        use std::sync::Arc;
        let ctx = Ctx::from_parts(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        );
        let cs = "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v";
        let mut a = ctx
            .store()
            .unwrap()
            .add_connection_string("a", cs, ctx.secrets.as_ref(), false)
            .unwrap();
        a.pinned_entities = vec!["orders".into()];
        ctx.store().unwrap().update(&a).unwrap();
        ctx.store()
            .unwrap()
            .add_connection_string("b", cs, ctx.secrets.as_ref(), false)
            .unwrap();
        let by_profile = |name| Selection {
            connection_string: None,
            namespace: None,
            profile: name,
        };
        assert_eq!(pins_for(&ctx, &by_profile(Some("a"))).unwrap(), ["orders"]);
        assert!(pins_for(&ctx, &by_profile(Some("b"))).unwrap().is_empty());
        // No selection: the default profile.
        assert!(pins_for(&ctx, &by_profile(None)).unwrap().is_empty());
        ctx.store().unwrap().set_default(Some("a")).unwrap();
        assert_eq!(pins_for(&ctx, &by_profile(None)).unwrap(), ["orders"]);
        // A connection string or bare namespace has no profile.
        let cs_sel = Selection {
            connection_string: Some("x"),
            namespace: None,
            profile: None,
        };
        assert!(pins_for(&ctx, &cs_sel).unwrap().is_empty());
    }

    #[test]
    fn an_incomplete_export_fails_but_a_full_one_does_not() {
        // Fewer than --top and no progress: end not confirmed -> non-zero.
        let e = check_complete(Continuation::Stalled, 1).unwrap_err();
        assert!(e.to_string().contains("export incomplete"), "{e}");
        assert!(e.to_string().contains("1 message(s) were written"));
        // --top reached: complete even though the last page made no progress.
        check_complete(Continuation::Done, 3).unwrap();
        assert!(scan_notes(&summary(3, 3, None, Continuation::Done)).is_empty());
        // End, More: fine.
        check_complete(Continuation::End, 0).unwrap();
        check_complete(Continuation::More(5), 3).unwrap();
    }

    #[test]
    fn forbidden_listing_with_nothing_known_keeps_the_error_and_adds_a_hint() {
        let e = forbidden_without_known(dispatched_core::Error::Forbidden {
            needed: Some("Manage,EntityRead".into()),
            detail: "Unauthorized".into(),
        });
        let t = e.to_string();
        assert!(
            t.starts_with("access denied (needs Manage,EntityRead): Unauthorized"),
            "{t}"
        );
        assert!(t.contains("dispatched peek <entity>"));
        assert!(t.contains("dispatched profile pin <profile> <entity>"));
    }

    #[test]
    fn refused_topic_listing_is_skipped_but_other_errors_fail() {
        let forbidden = dispatched_core::Error::Forbidden {
            needed: None,
            detail: "no".into(),
        };
        assert!(topics_outcome(Err(forbidden)).unwrap().is_none());
        assert!(topics_outcome(Ok(Vec::new())).unwrap().is_some());
        assert!(topics_outcome(Err(dispatched_core::Error::Network("down".into()))).is_err());
        assert!(TOPICS_SKIPPED_NOTE.contains("Manage,EntityRead"));
    }

    #[test]
    fn viewargs_parse_and_conflict() {
        use clap::Parser;
        #[derive(Parser)]
        struct T {
            #[command(flatten)]
            v: ViewArgs,
        }
        let t = T::try_parse_from(["t", "q", "--dlq", "--filter", "x = 1"]).unwrap();
        assert_eq!(t.v.entity.as_deref(), Some("q"));
        assert!(t.v.dlq);
        assert_eq!(t.v.scan_limit, DEFAULT_SCAN_LIMIT);
        assert!(T::try_parse_from(["t", "--view", "active", "--dlq"]).is_err());
        assert!(T::try_parse_from(["t", "--dlq", "--transfer-dlq"]).is_err());
        assert!(T::try_parse_from(["t", "--body-json-eq", "1"]).is_err());
        assert!(T::try_parse_from(["t", "--scan-limit", "0"]).is_err());
        assert!(T::try_parse_from(["t"]).unwrap().v.entity.is_none());
    }
}
