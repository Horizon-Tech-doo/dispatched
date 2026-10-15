//! `dispatched ls`, `peek` and `export`.

use crate::Format;
use crate::context::Ctx;
use crate::output::{ellipsize, sanitize, table};
use crate::resolve::{Selection, needs_sign_in, resolve};
use anyhow::Context as _;
use clap::Args;
use dispatched_core::messaging::DEFAULT_SCAN_LIMIT;
pub use dispatched_core::namespace::known_entities;
use dispatched_core::{
    EntityCounts, EntityPath, Explorer, ExportFormat, ExportedMessage, MessageBody, MessageFilter,
    Namespace, PeekOptions, PeekPage, PeekView, PeekedMessage, QueueInfo, SubscriptionInfo,
    TopicInfo, write_messages,
};
use std::future::Future;
use std::io::Write;
use time::format_description::well_known::Rfc3339;

/// The namespace the user selected. The sign-in (a keychain read) is only
/// looked up when the selection uses Entra.
fn namespace(ctx: &Ctx, sel: &Selection<'_>) -> anyhow::Result<Namespace> {
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
            session: None,
        }
    }
}

pub fn build_filter(a: &FilterArgs<'_>) -> anyhow::Result<MessageFilter> {
    if a.body_json_eq.is_some() && a.body_json.is_none() {
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

/// How a scan ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// The end of the entity was reached.
    End,
    /// Stopped early; continue from this sequence number.
    More(i64),
    /// The scan made no progress, so the end of the entity was NOT confirmed.
    Stalled,
    /// `export` collected everything asked for (`--top`); no continuation is known
    /// and none is needed.
    Done,
}

pub fn continuation(page: &PeekPage) -> Continuation {
    if page.exhausted {
        Continuation::End
    } else {
        match page.next_sequence {
            Some(n) => Continuation::More(n),
            None => Continuation::Stalled,
        }
    }
}

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

pub async fn peek(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ViewArgs,
    top: u32,
    format: Format,
) -> anyhow::Result<()> {
    let q = Query::open(ctx, sel, args)?;
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

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

/// The largest number of matches requested in one export page.
const EXPORT_PAGE: u32 = 1000;

/// Messages gathered over one or more pages.
pub struct Collected {
    pub messages: Vec<PeekedMessage>,
    pub scanned: u64,
    pub end: Continuation,
}

/// Page until `top` matches are collected or the entity is exhausted. Each
/// page's `next_sequence` feeds the next request. `fetch(from, max)` returns one page.
pub async fn collect_pages<F, Fut>(
    top: u32,
    from_seq: i64,
    mut fetch: F,
) -> anyhow::Result<Collected>
where
    F: FnMut(i64, u32) -> Fut,
    Fut: Future<Output = dispatched_core::Result<PeekPage>>,
{
    let top = top as usize;
    let mut messages: Vec<PeekedMessage> = Vec::new();
    let mut scanned = 0u64;
    let mut from = from_seq;
    let end = loop {
        let remaining = (top - messages.len()) as u32;
        let page = fetch(from, remaining.min(EXPORT_PAGE)).await?;
        scanned += u64::from(page.scanned);
        let end = continuation(&page);
        messages.extend(page.messages);
        messages.truncate(top);
        match end {
            Continuation::More(n) if messages.len() < top => {
                if n <= from {
                    // A page must move forward; never loop on one position.
                    break Continuation::Stalled;
                }
                from = n;
            }
            // Asked-for count reached: complete, even if the last page made no
            // progress past it.
            Continuation::Stalled if messages.len() >= top => break Continuation::Done,
            other => break other,
        }
    };
    Ok(Collected {
        messages,
        scanned,
        end,
    })
}

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

/// An existing target that is not a regular file (a device such as
/// `/dev/null` or `/dev/stdout`, a FIFO): it is written to directly, and
/// "overwrite" does not apply to it.
fn is_special_target(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| !m.is_file() && !m.is_dir())
}

/// Fail early (before any network work) when `path` exists and `force` is off.
/// Devices and pipes are not "existing files" and never need `--force`.
pub fn check_output(path: &str, force: bool) -> anyhow::Result<()> {
    let p = std::path::Path::new(path);
    if path != "-" && !force && p.exists() && !is_special_target(p) {
        anyhow::bail!(exists_message(path));
    }
    Ok(())
}

/// Removes the temporary file on drop unless disarmed.
struct TempFile(Option<std::path::PathBuf>);

impl TempFile {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn temp_path(path: &std::path::Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    path.with_file_name(format!(".{name}.dispatched-tmp-{}", std::process::id()))
}

/// Produce `path` atomically: `write` fills a sibling temp file, which is
/// flushed and synced, then put in place. With `force` it is renamed over the
/// target. Without it, a hard link creates the target only if absent
/// (atomic; fails with `AlreadyExists`), and the temp file is then removed;
/// where hard links are unsupported it falls back to an existence check
/// followed by a rename. On any error the temp file is removed and an
/// existing target is left untouched. An existing non-regular target (device,
/// FIFO) is written directly, without a temp file, rename or `--force`.
pub fn write_atomic(
    path: &std::path::Path,
    force: bool,
    write: impl FnOnce(&mut dyn Write) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let shown = path.display();
    if is_special_target(path) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .with_context(|| format!("cannot write {shown}; check the path and permissions"))?;
        let mut w = std::io::BufWriter::new(file);
        write(&mut w)?;
        return w.flush().with_context(|| format!("cannot write {shown}"));
    }
    let exists = || anyhow::anyhow!(exists_message(&path.to_string_lossy()));
    let tmp = temp_path(path);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("cannot write {shown}; check the path and permissions"))?;
    let mut guard = TempFile(Some(tmp.clone()));
    let mut w = std::io::BufWriter::new(file);
    write(&mut w)?;
    w.flush().with_context(|| format!("cannot write {shown}"))?;
    let file = w
        .into_inner()
        .map_err(|e| e.into_error())
        .with_context(|| format!("cannot write {shown}"))?;
    file.sync_all()
        .with_context(|| format!("cannot sync {shown}"))?;
    drop(file);
    if force {
        std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {shown}"))?;
        guard.disarm();
        return Ok(());
    }
    match std::fs::hard_link(&tmp, path) {
        // The temp name is dropped (guard) and the target keeps the data.
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(exists()),
        Err(_) => {
            // No hard links here: best-effort check, then rename.
            if path.exists() {
                return Err(exists());
            }
            std::fs::rename(&tmp, path).with_context(|| format!("cannot write {shown}"))?;
            guard.disarm();
            Ok(())
        }
    }
}

/// Write the export to `path` (`-` is stdout), atomically for regular files
/// (see [`write_atomic`]).
pub fn write_output(
    path: &str,
    force: bool,
    messages: &[PeekedMessage],
    format: ExportFormat,
) -> anyhow::Result<usize> {
    if path == "-" {
        return Ok(write_messages(std::io::stdout().lock(), messages, format)?);
    }
    let mut n = 0;
    write_atomic(std::path::Path::new(path), force, |w| {
        n = write_messages(w, messages, format)?;
        Ok(())
    })?;
    Ok(n)
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
    let collected = collect_pages(top, q.from_seq, |from, max| {
        let opts = qr.options(max, from);
        async move { qr.explorer.peek(&qr.entity, qr.view, &opts).await }
    })
    .await;
    q.explorer.close().await;
    let collected = collected?;
    let n = write_output(out, force, &collected.messages, format)?;
    let dest = if out == "-" {
        "stdout".to_string()
    } else {
        sanitize(out)
    };
    eprintln!("Exported {n} message(s) to {dest}");
    print_notes(&scan_notes(&ScanSummary {
        scanned: collected.scanned,
        matched: n,
        top,
        scan_limit: None,
        end: collected.end,
    }));
    check_complete(collected.end, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{MessageState, PropValue};
    use std::cell::RefCell;
    use std::collections::{BTreeMap, VecDeque};

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

    #[test]
    fn continuation_tells_end_more_and_stalled_apart() {
        assert_eq!(continuation(&page(&[], 0, None, true)), Continuation::End);
        assert_eq!(
            continuation(&page(&[1], 1, Some(2), false)),
            Continuation::More(2)
        );
        // No progress: next_sequence None, not exhausted.
        assert_eq!(
            continuation(&page(&[], 0, None, false)),
            Continuation::Stalled
        );
    }

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

    // --- export paging ---

    /// A fake service: hands out the queued pages and records each request.
    struct Fake {
        pages: RefCell<VecDeque<PeekPage>>,
        requests: RefCell<Vec<(i64, u32)>>,
    }

    impl Fake {
        fn new(pages: Vec<PeekPage>) -> Self {
            Self {
                pages: RefCell::new(pages.into()),
                requests: RefCell::new(Vec::new()),
            }
        }
        async fn fetch(&self, from: i64, max: u32) -> dispatched_core::Result<PeekPage> {
            self.requests.borrow_mut().push((from, max));
            Ok(self
                .pages
                .borrow_mut()
                .pop_front()
                .expect("unexpected request"))
        }
    }

    #[tokio::test]
    async fn export_pages_until_the_entity_is_exhausted() {
        let fake = Fake::new(vec![
            page(&[1, 2], 5, Some(6), false),
            page(&[8], 3, Some(9), false),
            page(&[], 0, None, true),
        ]);
        let c = collect_pages(10, 0, |f, m| fake.fetch(f, m)).await.unwrap();
        let seqs: Vec<i64> = c.messages.iter().map(|m| m.sequence_number).collect();
        assert_eq!(seqs, vec![1, 2, 8]);
        assert_eq!(c.scanned, 8);
        assert_eq!(c.end, Continuation::End);
        // Each page's next_sequence feeds the next request; max is what remains.
        assert_eq!(*fake.requests.borrow(), vec![(0, 10), (6, 8), (9, 7)]);
    }

    #[tokio::test]
    async fn export_stops_at_top_and_reports_where_to_continue() {
        let fake = Fake::new(vec![page(&[1, 2, 3], 3, Some(4), false)]);
        let c = collect_pages(3, 0, |f, m| fake.fetch(f, m)).await.unwrap();
        assert_eq!(c.messages.len(), 3);
        assert_eq!(c.end, Continuation::More(4));
        assert_eq!(fake.requests.borrow().len(), 1);
    }

    #[tokio::test]
    async fn export_caps_page_size_and_truncates_to_top() {
        let fake = Fake::new(vec![page(&[1, 2, 3], 3, Some(4), true)]);
        let c = collect_pages(2, 5, |f, m| fake.fetch(f, m)).await.unwrap();
        assert_eq!(c.messages.len(), 2);
        let fake = Fake::new(vec![page(&[], 0, None, true)]);
        collect_pages(5_000, 0, |f, m| fake.fetch(f, m))
            .await
            .unwrap();
        assert_eq!(*fake.requests.borrow(), vec![(0, EXPORT_PAGE)]);
    }

    #[tokio::test]
    async fn export_without_progress_is_stalled_not_end() {
        let fake = Fake::new(vec![
            page(&[1], 1, Some(2), false),
            page(&[], 0, None, false),
        ]);
        let c = collect_pages(10, 0, |f, m| fake.fetch(f, m)).await.unwrap();
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.end, Continuation::Stalled);
    }

    #[tokio::test]
    async fn export_never_loops_on_a_page_that_does_not_advance() {
        // next_sequence equal to the request position would loop forever.
        let fake = Fake::new(vec![page(&[], 4, Some(0), false)]);
        let c = collect_pages(10, 0, |f, m| fake.fetch(f, m)).await.unwrap();
        assert_eq!(c.end, Continuation::Stalled);
        assert_eq!(fake.requests.borrow().len(), 1);
    }

    #[tokio::test]
    async fn export_propagates_errors_from_a_page() {
        let r = collect_pages(10, 0, |_, _| async {
            Err::<PeekPage, _>(dispatched_core::Error::Network("boom".into()))
        })
        .await;
        assert!(r.is_err());
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
        assert_eq!(
            write_output("/dev/null", false, &[msg(1), msg(2)], ExportFormat::Ndjson).unwrap(),
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_to_a_fifo_writes_directly() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        match std::process::Command::new("mkfifo").arg(&fifo).status() {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("skipping: mkfifo is not available");
                return;
            }
        }
        let reader = {
            let fifo = fifo.clone();
            std::thread::spawn(move || std::fs::read_to_string(fifo).unwrap())
        };
        let p = fifo.to_str().unwrap();
        // No --force, and no temp file or rename: the FIFO itself is written.
        check_output(p, false).unwrap();
        assert_eq!(
            write_output(p, false, &[msg(1), msg(2)], ExportFormat::Ndjson).unwrap(),
            2
        );
        let got = reader.join().unwrap();
        assert_eq!(got.lines().count(), 2, "{got}");
        assert_eq!(dir_names(dir.path()), ["pipe"]);
        assert!(
            !std::fs::metadata(&fifo).unwrap().is_file(),
            "the FIFO must not be replaced by a regular file"
        );
    }

    #[test]
    fn export_refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let p = path.to_str().unwrap();
        std::fs::write(&path, "keep me").unwrap();
        let e = check_output(p, false).unwrap_err().to_string();
        assert!(e.contains("--force") && e.contains("exists"), "{e}");
        let e = write_output(p, false, &[msg(1)], ExportFormat::Json)
            .unwrap_err()
            .to_string();
        assert!(e.contains("--force"), "{e}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
        // stdout is never "existing".
        check_output("-", false).unwrap();
        // --force replaces it.
        check_output(p, true).unwrap();
        assert_eq!(
            write_output(p, true, &[msg(1), msg(2)], ExportFormat::Json).unwrap(),
            2
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn export_creates_new_files_and_writes_ndjson_byte_faithfully() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.ndjson");
        let p = path.to_str().unwrap();
        let mut m = msg(1);
        m.body = MessageBody::Text("a\x1b[31m\u{202e}b".into());
        write_output(p, false, &[m, msg(2)], ExportFormat::Ndjson).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        // Not sanitised: serde escapes the control characters, the value is intact.
        assert!(text.contains("\\u001b"));
        let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["body"]["value"], "a\x1b[31m\u{202e}b");
        // An unwritable path says what to check.
        let bad = dir.path().join("no/such/dir/out.json");
        let e = format!(
            "{:#}",
            write_output(bad.to_str().unwrap(), false, &[], ExportFormat::Json).unwrap_err()
        );
        assert!(e.contains("check the path"), "{e}");
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

    fn dir_names(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn failed_write_leaves_the_original_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        std::fs::write(&path, "original").unwrap();
        for force in [false, true] {
            let err = write_atomic(&path, force, |w| {
                w.write_all(b"partial")?;
                anyhow::bail!("disk full")
            })
            .unwrap_err();
            assert!(err.to_string().contains("disk full"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
            assert_eq!(dir_names(dir.path()), ["out.json"]);
        }
        // A writer that fails at the I/O level.
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("no space"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let err = write_atomic(&path, true, |w| {
            std::io::copy(&mut &b"data"[..], &mut Failing)?;
            w.write_all(b"never")?;
            Ok(())
        })
        .unwrap_err();
        assert!(err.to_string().contains("no space"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(dir_names(dir.path()), ["out.json"]);
    }

    #[test]
    fn failed_export_to_a_new_path_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.json");
        assert!(write_atomic(&path, false, |_| anyhow::bail!("boom")).is_err());
        assert!(dir_names(dir.path()).is_empty());
        // An unwritable (missing) directory fails without creating anything.
        let missing = dir.path().join("nope/out.json");
        let e = format!(
            "{:#}",
            write_atomic(&missing, true, |_| Ok(())).unwrap_err()
        );
        assert!(e.contains("check the path"), "{e}");
        assert!(dir_names(dir.path()).is_empty());
    }

    #[test]
    fn a_target_that_appears_during_the_write_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let err = write_atomic(&path, false, |w| {
            w.write_all(b"ours")?;
            // Someone else creates the target while we write.
            std::fs::write(&path, "theirs")?;
            Ok(())
        })
        .unwrap_err();
        assert!(err.to_string().contains("--force"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
        assert_eq!(dir_names(dir.path()), ["out.json"]);
    }

    #[test]
    fn successful_atomic_writes_leave_only_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        write_atomic(&path, false, |w| Ok(w.write_all(b"one")?)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
        assert_eq!(dir_names(dir.path()), ["out.json"]);
        write_atomic(&path, true, |w| Ok(w.write_all(b"two")?)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(dir_names(dir.path()), ["out.json"]);
    }

    #[tokio::test]
    async fn short_export_is_stalled_but_a_full_one_is_not() {
        // Fewer than --top and no progress: end not confirmed -> non-zero.
        let fake = Fake::new(vec![page(&[1], 1, None, false)]);
        let c = collect_pages(3, 0, |f, m| fake.fetch(f, m)).await.unwrap();
        assert_eq!(c.end, Continuation::Stalled);
        let e = check_complete(c.end, c.messages.len()).unwrap_err();
        assert!(e.to_string().contains("export incomplete"), "{e}");
        assert!(e.to_string().contains("1 message(s) were written"));
        // --top reached: complete even though the last page made no progress.
        let fake = Fake::new(vec![page(&[1, 2, 3], 3, None, false)]);
        let c = collect_pages(3, 0, |f, m| fake.fetch(f, m)).await.unwrap();
        assert_eq!(c.end, Continuation::Done);
        check_complete(c.end, 3).unwrap();
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
