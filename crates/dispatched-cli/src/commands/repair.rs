//! `dispatched delete`, `move`, `resubmit`, `purge`, `deferred delete` and
//! `dlq summary|resubmit|move|delete|purge`.

use super::browse::{FilterArgs, build_filter, explorer_for, pick_view};
use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, StderrProgress, finish_job};
use crate::output::{sanitize, table, thousands};
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned};
use anyhow::Context as _;
use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use dispatched_core::messaging::{parse_duration, parse_prop_arg};
use dispatched_core::ops::{
    LocateLimits, MAX_SELECTION, MessageSelection, ResubmitSpec, SelectSpec, default_target,
    send_target,
};
use dispatched_core::{
    DlqSummary, Edits, EntityPath, Mutation, OutgoingBody, PeekView, PurgeScope, SubQueue,
};
use std::path::PathBuf;

/// Which messages a command acts on: `--seq` numbers, or `--all` matching the filter flags.
#[derive(Args, Debug, Clone)]
#[command(group(ArgGroup::new("which").required(true).args(["seqs", "all"])))]
pub struct WhichArgs {
    /// A sequence number (repeat, or separate with commas).
    #[arg(long = "seq", value_name = "N", value_delimiter = ',')]
    pub seqs: Vec<i64>,
    /// Every message of the view that matches the filter flags (at most --limit). The
    /// messages are found with a peek before the confirmation, which shows the exact count.
    #[arg(long, conflicts_with = "seqs")]
    pub all: bool,
    /// With --all: a property filter, e.g. "sys.DeliveryCount > 1 AND user.kind = 'x'".
    #[arg(long, conflicts_with = "seqs")]
    pub filter: Option<String>,
    /// With --all: keep messages whose body contains this text (case-insensitive).
    #[arg(long, conflicts_with = "seqs")]
    pub body_contains: Option<String>,
    /// With --all: keep messages whose body matches this regular expression.
    #[arg(long, conflicts_with = "seqs")]
    pub body_regex: Option<String>,
    /// With --all: keep messages whose JSON body has a value at this JSONPath.
    #[arg(long, conflicts_with = "seqs")]
    pub body_json: Option<String>,
    /// With --body-json: the value must equal this.
    #[arg(long, requires = "body_json", conflicts_with = "seqs")]
    pub body_json_eq: Option<String>,
    /// With --all: only messages of this session (exact session id).
    #[arg(long, conflicts_with = "seqs")]
    pub session: Option<String>,
    /// With --all: at most this many messages (1 to 100,000).
    #[arg(long, conflicts_with = "seqs", default_value_t = MAX_SELECTION, value_parser = clap::value_parser!(u64).range(1..=MAX_SELECTION))]
    pub limit: u64,
}

impl Default for WhichArgs {
    fn default() -> Self {
        Self {
            seqs: Vec::new(),
            all: false,
            filter: None,
            body_contains: None,
            body_regex: None,
            body_json: None,
            body_json_eq: None,
            session: None,
            limit: MAX_SELECTION,
        }
    }
}

impl WhichArgs {
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

/// The messages a delete, move or resubmit acts on: the entity, the view and which messages.
#[derive(Args, Debug, Clone, Default)]
pub struct SelectArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// View: active, scheduled, deferred, dlq or transfer-dlq.
    #[arg(long, conflicts_with_all = ["dlq", "transfer_dlq"])]
    pub view: Option<String>,
    /// Same as `--view dlq`.
    #[arg(long, conflicts_with = "transfer_dlq")]
    pub dlq: bool,
    /// Same as `--view transfer-dlq`.
    #[arg(long)]
    pub transfer_dlq: bool,
    #[command(flatten)]
    pub which: WhichArgs,
}

/// [`SelectArgs`] of a `dlq` subcommand: the dead-letter queue is implied, `--view` is not
/// taken.
#[derive(Args, Debug, Clone)]
pub struct DlqSelectArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The transfer dead-letter queue instead of the dead-letter queue.
    #[arg(long)]
    pub transfer_dlq: bool,
    #[command(flatten)]
    pub which: WhichArgs,
}

impl DlqSelectArgs {
    /// The same selection as top-level [`SelectArgs`].
    fn select(&self) -> SelectArgs {
        SelectArgs {
            entity: self.entity.clone(),
            view: None,
            dlq: !self.transfer_dlq,
            transfer_dlq: self.transfer_dlq,
            which: self.which.clone(),
        }
    }
}

/// `delete`.
#[derive(Args, Debug)]
pub struct DeleteArgs {
    #[command(flatten)]
    pub select: SelectArgs,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `move`.
#[derive(Args, Debug)]
pub struct MoveArgs {
    #[command(flatten)]
    pub select: SelectArgs,
    /// The queue or topic to move the messages to.
    #[arg(long, value_name = "TARGET")]
    pub to: String,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// Changes made to each resubmitted copy.
#[derive(Args, Debug, Clone, Default)]
pub struct EditArgs {
    /// Give each copy a new random MessageId.
    #[arg(long)]
    pub new_message_id: bool,
    /// Remove each original once its copy is sent (sent first, then removed: a message may
    /// arrive twice).
    #[arg(long)]
    pub remove_original: bool,
    /// Keep DeadLetterReason and DeadLetterErrorDescription on copies from a dead-letter queue.
    #[arg(long)]
    pub keep_dead_letter_properties: bool,
    /// Replace the body with this text.
    #[arg(long, value_name = "TEXT", conflicts_with = "set_body_file")]
    pub set_body: Option<String>,
    /// Replace the body with the bytes of this file.
    #[arg(long, value_name = "PATH")]
    pub set_body_file: Option<PathBuf>,
    /// Set a custom property, `KEY=VALUE[:TYPE]` (string by default). Repeat for several.
    #[arg(long = "set-prop", value_name = "KEY=VALUE[:TYPE]")]
    pub set_props: Vec<String>,
    /// Remove a custom property. Repeat for several.
    #[arg(long = "remove-prop", value_name = "KEY")]
    pub remove_props: Vec<String>,
    /// Subject (Label).
    #[arg(long, value_name = "T")]
    pub set_subject: Option<String>,
    /// ContentType.
    #[arg(long, value_name = "T")]
    pub set_content_type: Option<String>,
    /// SessionId (a PartitionKey equal to the old SessionId follows it).
    #[arg(long, value_name = "ID")]
    pub set_session_id: Option<String>,
    /// ReplyToSessionId.
    #[arg(long, value_name = "ID")]
    pub set_reply_to_session_id: Option<String>,
    /// PartitionKey.
    #[arg(long, value_name = "KEY")]
    pub set_partition_key: Option<String>,
    /// CorrelationId.
    #[arg(long, value_name = "ID")]
    pub set_correlation_id: Option<String>,
    /// Time to live: 90s, 15m, 2h, 7d, 1h30m, ISO 8601 or .NET TimeSpan text.
    #[arg(long, value_name = "DURATION")]
    pub set_ttl: Option<String>,
}

/// `resubmit`.
#[derive(Args, Debug)]
pub struct ResubmitArgs {
    #[command(flatten)]
    pub select: SelectArgs,
    /// The queue or topic the copies go to (default: the queue itself, or a subscription's
    /// topic).
    #[arg(long, value_name = "TARGET")]
    pub to: Option<String>,
    #[command(flatten)]
    pub edits: EditArgs,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// What `purge --scope` removes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ScopeArg {
    /// Active messages (not deferred, not scheduled).
    #[default]
    Active,
    /// The dead-letter queue.
    Dlq,
    /// The transfer dead-letter queue.
    TransferDlq,
    /// Active, deferred, dead-letter and transfer dead-letter messages (scheduled ones are
    /// kept).
    All,
}

impl From<ScopeArg> for PurgeScope {
    fn from(s: ScopeArg) -> Self {
        match s {
            ScopeArg::Active => PurgeScope::Active,
            ScopeArg::Dlq => PurgeScope::DeadLetter,
            ScopeArg::TransferDlq => PurgeScope::TransferDeadLetter,
            ScopeArg::All => PurgeScope::All,
        }
    }
}

/// `purge`.
#[derive(Args, Debug)]
pub struct PurgeArgs {
    /// Queues or subscriptions; each is counted, then confirmed and purged on its own.
    #[arg(required = true, value_name = "ENTITY")]
    pub entities: Vec<String>,
    /// Which messages to remove.
    #[arg(long, value_enum, default_value_t = ScopeArg::Active)]
    pub scope: ScopeArg,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `deferred delete`: `delete --view deferred` by sequence number.
#[derive(Args, Debug)]
pub struct DeferredDeleteArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// A sequence number (repeat, or separate with commas).
    #[arg(long = "seq", value_name = "N", value_delimiter = ',', required = true)]
    pub seqs: Vec<i64>,
    /// The session the messages are in. Optional: each message's session is read from the
    /// entity before it is received.
    #[arg(long)]
    pub session: Option<String>,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

impl DeferredDeleteArgs {
    /// The same selection as `delete --view deferred --seq N...`.
    fn select(&self) -> SelectArgs {
        SelectArgs {
            entity: self.entity.clone(),
            view: Some("deferred".into()),
            which: WhichArgs {
                seqs: self.seqs.clone(),
                session: self.session.clone(),
                ..WhichArgs::default()
            },
            ..SelectArgs::default()
        }
    }
}

/// `dlq`: the dead-letter queue.
#[derive(Subcommand, Debug)]
pub enum DlqCmd {
    /// Dead-lettered messages grouped by reason and description.
    Summary(DlqSummaryArgs),
    /// `resubmit` from the dead-letter queue.
    Resubmit(Box<DlqResubmitArgs>),
    /// `move` from the dead-letter queue.
    Move(DlqMoveArgs),
    /// `delete` from the dead-letter queue.
    Delete(DlqDeleteArgs),
    /// `purge --scope dlq`.
    Purge(DlqPurgeArgs),
}

/// `dlq summary`.
#[derive(Args, Debug)]
pub struct DlqSummaryArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The transfer dead-letter queue instead.
    #[arg(long)]
    pub transfer_dlq: bool,
}

/// `dlq resubmit`.
#[derive(Args, Debug)]
pub struct DlqResubmitArgs {
    #[command(flatten)]
    pub select: DlqSelectArgs,
    /// The queue or topic the copies go to (default: the queue itself, or a subscription's
    /// topic).
    #[arg(long, value_name = "TARGET")]
    pub to: Option<String>,
    #[command(flatten)]
    pub edits: EditArgs,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `dlq move`.
#[derive(Args, Debug)]
pub struct DlqMoveArgs {
    #[command(flatten)]
    pub select: DlqSelectArgs,
    /// The queue or topic to move the messages to.
    #[arg(long, value_name = "TARGET")]
    pub to: String,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `dlq delete`.
#[derive(Args, Debug)]
pub struct DlqDeleteArgs {
    #[command(flatten)]
    pub select: DlqSelectArgs,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `dlq purge`.
#[derive(Args, Debug)]
pub struct DlqPurgeArgs {
    /// Queues or subscriptions; each is counted, then confirmed and purged on its own.
    #[arg(required = true, value_name = "ENTITY")]
    pub entities: Vec<String>,
    /// The transfer dead-letter queue instead.
    #[arg(long)]
    pub transfer_dlq: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

impl DlqResubmitArgs {
    fn normal(&self) -> ResubmitArgs {
        ResubmitArgs {
            select: self.select.select(),
            to: self.to.clone(),
            edits: self.edits.clone(),
            mutate: self.mutate.clone(),
        }
    }
}

impl DlqMoveArgs {
    fn normal(&self) -> MoveArgs {
        MoveArgs {
            select: self.select.select(),
            to: self.to.clone(),
            mutate: self.mutate.clone(),
        }
    }
}

impl DlqDeleteArgs {
    fn normal(&self) -> DeleteArgs {
        DeleteArgs {
            select: self.select.select(),
            mutate: self.mutate.clone(),
        }
    }
}

impl DlqPurgeArgs {
    fn normal(&self) -> PurgeArgs {
        PurgeArgs {
            entities: self.entities.clone(),
            scope: if self.transfer_dlq {
                ScopeArg::TransferDlq
            } else {
                ScopeArg::Dlq
            },
            mutate: self.mutate.clone(),
        }
    }
}

/// The selection the flags describe (no I/O): the view (`--view`, `--dlq`, `--transfer-dlq`),
/// and `--seq` numbers or `--all` with the filter flags.
pub fn select_spec_from(a: &SelectArgs) -> anyhow::Result<SelectSpec> {
    let entity = EntityPath::parse(&a.entity)?;
    let view = pick_view(a.view.as_deref(), a.dlq, a.transfer_dlq)?;
    let w = &a.which;
    let selection = match (w.seqs.is_empty(), w.all) {
        (false, false) => MessageSelection::Sequences(w.seqs.clone()),
        (true, true) => MessageSelection::Matching {
            filter: build_filter(&w.filter_args())?,
            limit: w.limit,
        },
        // clap requires exactly one; kept so the function is total.
        _ => anyhow::bail!("pass --seq N (repeat for several) or --all"),
    };
    Ok(SelectSpec {
        entity,
        view,
        selection,
        limits: LocateLimits::default(),
    })
}

/// The edits the flags describe. Reads `--set-body-file` (a local file) but nothing else.
fn edits_from(a: &EditArgs) -> anyhow::Result<Edits> {
    let body = match (&a.set_body, &a.set_body_file) {
        (Some(text), _) => Some(OutgoingBody::Bytes(text.as_bytes().to_vec())),
        (None, Some(path)) => {
            let shown = sanitize(&path.display().to_string());
            Some(OutgoingBody::Bytes(
                std::fs::read(path).with_context(|| format!("--set-body-file {shown}"))?,
            ))
        }
        (None, None) => None,
    };
    let mut set = std::collections::BTreeMap::new();
    for prop in &a.set_props {
        let (key, value) =
            parse_prop_arg(prop).map_err(|e| anyhow::anyhow!("--set-prop {prop}: {e}"))?;
        set.insert(key, value);
    }
    let time_to_live = match &a.set_ttl {
        Some(ttl) => Some(Some(
            parse_duration(ttl).map_err(|e| anyhow::anyhow!("--set-ttl: {e}"))?,
        )),
        None => None,
    };
    let to = |v: &Option<String>| v.clone().map(Some);
    Ok(Edits {
        body,
        set,
        remove: a.remove_props.clone(),
        subject: to(&a.set_subject),
        content_type: to(&a.set_content_type),
        session_id: to(&a.set_session_id),
        reply_to_session_id: to(&a.set_reply_to_session_id),
        partition_key: to(&a.set_partition_key),
        correlation_id: to(&a.set_correlation_id),
        time_to_live,
        ..Edits::default()
    })
}

/// The resubmit the flags describe (no I/O beyond reading `--set-body-file`): the target is
/// `--to` or the source's default (the queue itself, or a subscription's topic), checked with
/// core's [`send_target`].
pub fn resubmit_spec_from(a: &ResubmitArgs) -> anyhow::Result<ResubmitSpec> {
    let source = select_spec_from(&a.select)?;
    let target = match &a.to {
        Some(to) => send_target(to)?.base(),
        None => default_target(&source.entity),
    };
    Ok(ResubmitSpec {
        source,
        target,
        edits: edits_from(&a.edits)?,
        new_message_id: a.edits.new_message_id,
        remove_original: a.edits.remove_original,
        keep_dead_letter_properties: a.edits.keep_dead_letter_properties,
    })
}

/// `delete` (the planned kind and verb of [`dispatched_core::Operation::delete`]).
pub const DELETE: Planned = Planned {
    kind: Mutation::Delete,
    verb: "delete",
};
/// `delete --view scheduled`: cancelling.
pub const DELETE_SCHEDULED: Planned = Planned {
    kind: Mutation::Delete,
    verb: "cancel-scheduled",
};
/// `move`.
pub const MOVE: Planned = Planned {
    kind: Mutation::Delete,
    verb: "move",
};
/// `purge`.
pub const PURGE: Planned = Planned {
    kind: Mutation::Purge,
    verb: "purge",
};

/// The planned kind and verb of a delete in `view`.
fn planned_delete(view: PeekView) -> Planned {
    if view == PeekView::Scheduled {
        DELETE_SCHEDULED
    } else {
        DELETE
    }
}

/// The planned kind and verb of a resubmit: removing the originals is a delete, else a send.
fn planned_resubmit(remove_original: bool) -> Planned {
    Planned {
        kind: if remove_original {
            Mutation::Delete
        } else {
            Mutation::Send
        },
        verb: "resubmit",
    }
}

/// A sequence number with thousands separators.
fn seq_text(n: i64) -> String {
    u64::try_from(n).map_or_else(|_| n.to_string(), thousands)
}

/// The `dlq summary` table: one row per group in the summary's order (largest first), then
/// how many messages were examined.
pub fn render_dlq_summary(s: &DlqSummary) -> String {
    let scanned = format!(
        "Scanned {} {}{}",
        thousands(s.scanned),
        if s.scanned == 1 {
            "message"
        } else {
            "messages"
        },
        if s.truncated {
            " (capped): the counts cover only these"
        } else {
            ""
        }
    );
    if s.groups.is_empty() {
        return format!("(no dead-lettered messages)\n{scanned}");
    }
    let rows: Vec<Vec<String>> = s
        .groups
        .iter()
        .map(|g| {
            vec![
                thousands(g.count),
                g.reason.clone().unwrap_or_else(|| "(none)".into()),
                g.description.clone().unwrap_or_else(|| "(none)".into()),
                seq_text(g.first_sequence),
                seq_text(g.last_sequence),
            ]
        })
        .collect();
    let t = table(
        &["Count", "Reason", "Description", "First seq", "Last seq"],
        &rows,
    );
    format!("{t}\n{scanned}")
}

/// The entities named on the command line, parsed, without repeats (compared on the address,
/// ignoring ASCII case).
fn entities_from(names: &[String]) -> anyhow::Result<Vec<EntityPath>> {
    let mut out: Vec<EntityPath> = Vec::new();
    for name in names {
        let path = EntityPath::parse(name)?;
        if !out
            .iter()
            .any(|p| p.base().eq_ignore_ascii_case(&path.base()))
        {
            out.push(path);
        }
    }
    Ok(out)
}

/// The source and, for a move or resubmit, the target: the precheck needs every entity the
/// command changes.
fn named(source: &EntityPath, target: Option<&str>) -> anyhow::Result<Vec<EntityPath>> {
    let mut out = vec![source.clone()];
    if let Some(t) = target {
        let t = EntityPath::parse(t)?;
        if !t.base().eq_ignore_ascii_case(&source.base()) {
            out.push(t);
        }
    }
    Ok(out)
}

pub async fn delete(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &DeleteArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let spec = select_spec_from(&args.select)?;
    let planned = planned_delete(spec.view);
    let entities = named(&spec.entity, None)?;
    let (operator, cancel) = operator_for(ctx, sel, &args.mutate, planned, &entities)?;
    // An --all selection is resolved inside `delete`, after the precheck.
    let result = operator.delete(spec, &StderrProgress::new(), &cancel).await;
    operator.explorer().close().await;
    Ok(finish_job(&result?, format, &args.mutate))
}

pub async fn move_messages(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &MoveArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let spec = select_spec_from(&args.select)?;
    let target = send_target(&args.to)?.base();
    let entities = named(&spec.entity, Some(&target))?;
    let (operator, cancel) = operator_for(ctx, sel, &args.mutate, MOVE, &entities)?;
    let result = operator
        .move_messages(spec, target, &StderrProgress::new(), &cancel)
        .await;
    operator.explorer().close().await;
    Ok(finish_job(&result?, format, &args.mutate))
}

pub async fn resubmit(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ResubmitArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let spec = resubmit_spec_from(args)?;
    let planned = planned_resubmit(spec.remove_original);
    let entities = named(&spec.source.entity, Some(&spec.target))?;
    let (operator, cancel) = operator_for(ctx, sel, &args.mutate, planned, &entities)?;
    let result = operator
        .resubmit(spec, &StderrProgress::new(), &cancel)
        .await;
    operator.explorer().close().await;
    Ok(finish_job(&result?, format, &args.mutate))
}

pub async fn purge(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &PurgeArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let entities = entities_from(&args.entities)?;
    let scope = PurgeScope::from(args.scope);
    // The precheck names every entity before anything is counted.
    let (operator, cancel) = operator_for(ctx, sel, &args.mutate, PURGE, &entities)?;
    let result = async {
        // Count every entity first: a count that fails changes nothing anywhere.
        let mut plans = Vec::with_capacity(entities.len());
        for entity in &entities {
            let plan = operator.plan_purge(entity, scope).await.with_context(|| {
                format!("counting the messages of {}", sanitize(&entity.base()))
            })?;
            plans.push(plan);
        }
        // One confirmation per entity, each with its own count; an unknown count is refused
        // before it is put to the confirmation.
        operator
            .purge(plans, &StderrProgress::new(), &cancel)
            .await
            .map_err(anyhow::Error::from)
    }
    .await;
    operator.explorer().close().await;
    Ok(finish_job(&result?, format, &args.mutate))
}

pub async fn deferred_delete(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &DeferredDeleteArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let delete_args = DeleteArgs {
        select: args.select(),
        mutate: args.mutate.clone(),
    };
    delete(ctx, sel, &delete_args, format).await
}

pub async fn dlq(
    ctx: &Ctx,
    sel: &Selection<'_>,
    cmd: &DlqCmd,
    format: Format,
) -> anyhow::Result<JobExit> {
    match cmd {
        DlqCmd::Summary(a) => {
            dlq_summary(ctx, sel, a, format).await?;
            Ok(JobExit::Ok)
        }
        DlqCmd::Resubmit(a) => resubmit(ctx, sel, &a.normal(), format).await,
        DlqCmd::Move(a) => move_messages(ctx, sel, &a.normal(), format).await,
        DlqCmd::Delete(a) => delete(ctx, sel, &a.normal(), format).await,
        DlqCmd::Purge(a) => purge(ctx, sel, &a.normal(), format).await,
    }
}

/// `dlq summary`: a read (no gate, no activity entry).
async fn dlq_summary(
    ctx: &Ctx,
    sel: &Selection<'_>,
    a: &DlqSummaryArgs,
    format: Format,
) -> anyhow::Result<()> {
    let entity = EntityPath::parse(&a.entity)?;
    let sub_queue = if a.transfer_dlq {
        SubQueue::TransferDeadLetter
    } else {
        SubQueue::DeadLetter
    };
    let explorer = explorer_for(ctx, sel)?;
    let result = explorer.dlq_summary(&entity, sub_queue).await;
    explorer.close().await;
    let summary = result?;
    match format {
        Format::Json | Format::Ndjson => println!("{}", serde_json::to_string_pretty(&summary)?),
        Format::Table => println!("{}", render_dlq_summary(&summary)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Ctx;
    use crate::run_for_test;
    use clap::Parser;
    use dispatched_core::ReasonGroup;
    use dispatched_core::messaging::TypedValue;

    fn try_parse<const N: usize>(args: [&str; N]) -> Result<crate::Cli, clap::Error> {
        crate::Cli::try_parse_from(std::iter::once("dispatched").chain(args))
    }

    /// The arguments of a repair command line, in the top-level command's shape (a `dlq` or
    /// `deferred` subcommand is normalized the way it runs).
    trait FromCommand: Sized {
        fn from_command(c: crate::Command) -> Option<Self>;
    }

    impl FromCommand for SelectArgs {
        fn from_command(c: crate::Command) -> Option<Self> {
            use crate::Command as C;
            Some(match c {
                C::Delete(a) => a.select,
                C::Move(a) => a.select,
                C::Resubmit(a) => a.select,
                C::Dlq { cmd } => match cmd {
                    DlqCmd::Delete(a) => a.normal().select,
                    DlqCmd::Move(a) => a.normal().select,
                    DlqCmd::Resubmit(a) => a.normal().select,
                    _ => return None,
                },
                C::Deferred {
                    cmd: crate::commands::receive::DeferredCmd::Delete(a),
                } => a.select(),
                _ => return None,
            })
        }
    }

    impl FromCommand for ResubmitArgs {
        fn from_command(c: crate::Command) -> Option<Self> {
            match c {
                crate::Command::Resubmit(a) => Some(*a),
                crate::Command::Dlq {
                    cmd: DlqCmd::Resubmit(a),
                } => Some(a.normal()),
                _ => None,
            }
        }
    }

    fn parse<T: FromCommand, const N: usize>(args: [&str; N]) -> T {
        T::from_command(try_parse(args).unwrap().command).expect("a repair command")
    }

    #[test]
    fn select_args_need_seq_or_all_and_map_views() {
        assert!(try_parse(["delete", "q"]).is_err());
        let s =
            select_spec_from(&parse(["dlq", "delete", "q", "--seq", "4", "--seq", "9"])).unwrap();
        assert_eq!(s.view, PeekView::DeadLetter);
        assert!(matches!(s.selection, MessageSelection::Sequences(ref v) if v == &[4, 9]));
        assert!(try_parse(["dlq", "delete", "q", "--view", "active", "--seq", "1"]).is_err());
        let s = select_spec_from(&parse([
            "delete",
            "q",
            "--all",
            "--filter",
            "user.kind = 'x'",
        ]))
        .unwrap();
        assert!(matches!(
            s.selection,
            MessageSelection::Matching { limit: 100_000, .. }
        ));
    }

    #[test]
    fn resubmit_edits_build() {
        let spec = resubmit_spec_from(&parse([
            "dlq",
            "resubmit",
            "q",
            "--seq",
            "1",
            "--set-prop",
            "retry=1:int",
            "--remove-prop",
            "DeadLetterReason",
            "--set-subject",
            "again",
            "--new-message-id",
            "--remove-original",
        ]))
        .unwrap();
        assert_eq!(spec.target, "q");
        assert!(spec.new_message_id && spec.remove_original);
        assert_eq!(spec.edits.set["retry"], TypedValue::Int(1));
        assert_eq!(spec.edits.subject, Some(Some("again".into())));
    }

    #[test]
    fn move_needs_a_target_and_purge_takes_many_entities() {
        assert!(try_parse(["move", "q", "--dlq", "--seq", "1"]).is_err());
        assert!(
            try_parse([
                "purge",
                "a",
                "b",
                "c",
                "--scope",
                "all",
                "--yes",
                "--confirm-prod",
                "a",
                "--confirm-prod",
                "b"
            ])
            .is_ok()
        );
        assert!(try_parse(["purge", "a", "--scope", "everything"]).is_err());
    }

    #[test]
    fn resubmit_can_change_session_reply_session_and_partition_key() {
        let spec = resubmit_spec_from(&parse([
            "resubmit",
            "q",
            "--seq",
            "1",
            "--set-session-id",
            "S2",
            "--set-reply-to-session-id",
            "R2",
            "--set-partition-key",
            "S2",
        ]))
        .unwrap();
        assert_eq!(spec.edits.session_id, Some(Some("S2".into())));
        assert_eq!(spec.edits.reply_to_session_id, Some(Some("R2".into())));
        assert_eq!(spec.edits.partition_key, Some(Some("S2".into())));
    }

    #[test]
    fn deferred_delete_is_delete_in_the_deferred_view() {
        let s = select_spec_from(&parse([
            "deferred",
            "delete",
            "q",
            "--seq",
            "7",
            "--session",
            "A",
        ]))
        .unwrap();
        assert_eq!(s.view, PeekView::Deferred);
        assert!(matches!(s.selection, MessageSelection::Sequences(ref v) if v == &[7]));
        assert!(
            try_parse(["deferred", "delete", "q"]).is_err(),
            "--seq is required"
        );
    }

    #[tokio::test]
    async fn purge_on_production_without_flags_never_connects() {
        // The saved production profile points at an unroutable host: a refusal that is not a
        // network error proves the check ran before counting (plan_purge) or any connection.
        let ctx = Ctx::for_test_with_profiles(&[("prod", "unroutable.invalid", true)])
            .signed_in_for_test();
        let e = run_for_test(
            &ctx,
            [
                "dispatched",
                "--profile",
                "prod",
                "purge",
                "orders",
                "--scope",
                "all",
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("--confirm-prod orders"), "{e}");
        assert!(
            !e.to_lowercase().contains("network") && !e.to_lowercase().contains("connect to"),
            "{e}"
        );
        let e = run_for_test(
            &ctx,
            [
                "dispatched",
                "--profile",
                "prod",
                "delete",
                "orders",
                "--all",
                "--filter",
                "user.k = 'x'",
                "--yes",
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("--confirm-prod orders"),
            "an --all selection is not resolved first: {e}"
        );
    }

    #[test]
    fn dlq_summary_table_lists_groups_by_count() {
        let g = |reason: &str, description: &str, count, first, last| ReasonGroup {
            reason: Some(reason.into()),
            description: Some(description.into()),
            count,
            first_sequence: first,
            last_sequence: last,
            oldest: None,
            newest: None,
        };
        let summary = DlqSummary {
            groups: vec![
                g("MaxDeliveryCountExceeded", "x", 1_200, 3, 9_000),
                g("Manual", "bad id", 4, 12, 40),
            ],
            scanned: 10_000,
            truncated: true,
        };
        let t = render_dlq_summary(&summary);
        let lines: Vec<&str> = t.lines().collect();
        assert!(
            lines[0].contains("Count")
                && lines[0].contains("Reason")
                && lines[0].contains("Description")
                && lines[0].contains("First seq")
                && lines[0].contains("Last seq"),
            "{t}"
        );
        assert!(
            lines[1].contains("1,200")
                && lines[1].contains("MaxDeliveryCountExceeded")
                && lines[1].contains("9,000"),
            "{t}"
        );
        assert!(
            lines[2].contains("Manual") && lines[2].contains("bad id"),
            "rows keep the summary's order: {t}"
        );
        assert!(t.contains("Scanned 10,000 messages (capped)"), "{t}");
        let full = DlqSummary {
            truncated: false,
            scanned: 2,
            ..summary
        };
        assert!(!render_dlq_summary(&full).contains("capped"));
    }

    #[test]
    fn an_empty_summary_says_so() {
        let t = render_dlq_summary(&DlqSummary {
            groups: Vec::new(),
            scanned: 0,
            truncated: false,
        });
        assert_eq!(t, "(no dead-lettered messages)\nScanned 0 messages");
        let g = ReasonGroup {
            reason: None,
            description: None,
            count: 1,
            first_sequence: 5,
            last_sequence: 5,
            oldest: None,
            newest: None,
        };
        let t = render_dlq_summary(&DlqSummary {
            groups: vec![g],
            scanned: 1,
            truncated: false,
        });
        assert!(t.lines().nth(1).unwrap().contains("(none)"), "{t}");
        assert!(t.ends_with("Scanned 1 message"), "{t}");
    }

    #[test]
    fn planned_kinds_match_the_operations() {
        use dispatched_core::{Operation, Target};
        let t = |q| Target::new(EntityPath::queue("q"), q);
        for (planned, op) in [
            (
                planned_delete(PeekView::DeadLetter),
                Operation::delete(t(SubQueue::DeadLetter), &[1]),
            ),
            (
                planned_delete(PeekView::Scheduled),
                Operation::cancel_scheduled(t(SubQueue::Active), 1),
            ),
            (MOVE, Operation::move_to(t(SubQueue::DeadLetter), "r", 1)),
            (
                planned_resubmit(true),
                Operation::resubmit(t(SubQueue::DeadLetter), "r", 1, true),
            ),
            (
                planned_resubmit(false),
                Operation::resubmit(t(SubQueue::DeadLetter), "r", 1, false),
            ),
            (
                PURGE,
                Operation::purge(
                    t(SubQueue::Active),
                    dispatched_core::CountEstimate::Exact(1),
                ),
            ),
        ] {
            assert_eq!((planned.kind, planned.verb), (op.mutation, op.verb()));
        }
    }

    #[test]
    fn filters_go_with_all_and_dlq_commands_imply_the_dead_letter_queue() {
        assert!(
            try_parse(["delete", "q", "--seq", "1", "--filter", "user.k = 'x'"]).is_err(),
            "a filter needs --all"
        );
        assert!(try_parse(["delete", "q", "--seq", "1", "--all"]).is_err());
        assert!(try_parse(["delete", "q", "--all", "--limit", "0"]).is_err());
        assert!(try_parse(["delete", "q", "--all", "--limit", "100001"]).is_err());
        let s = select_spec_from(&parse([
            "delete",
            "q",
            "--all",
            "--session",
            "A",
            "--limit",
            "5",
            "--view",
            "deferred",
        ]))
        .unwrap();
        assert_eq!(s.view, PeekView::Deferred);
        assert!(
            matches!(s.selection, MessageSelection::Matching { limit: 5, ref filter } if filter.session_id.as_deref() == Some("A"))
        );
        let s = select_spec_from(&parse([
            "dlq",
            "move",
            "q",
            "--transfer-dlq",
            "--seq",
            "1",
            "--to",
            "r",
        ]))
        .unwrap();
        assert_eq!(s.view, PeekView::TransferDeadLetter);
        assert!(try_parse(["dlq", "move", "q", "--seq", "1", "--to", "r", "--dlq"]).is_err());
        let s = select_spec_from(&parse(["move", "q", "--seq", "1,2", "--to", "r"])).unwrap();
        assert_eq!(s.view, PeekView::Active);
        assert!(matches!(s.selection, MessageSelection::Sequences(ref v) if v == &[1, 2]));
        let crate::Command::Dlq {
            cmd: DlqCmd::Purge(p),
        } = try_parse(["dlq", "purge", "a", "b", "--transfer-dlq"])
            .unwrap()
            .command
        else {
            panic!("not dlq purge")
        };
        let p = p.normal();
        assert_eq!(p.entities, ["a", "b"]);
        assert_eq!(PurgeScope::from(p.scope), PurgeScope::TransferDeadLetter);
        let crate::Command::Purge(p) = try_parse(["purge", "a"]).unwrap().command else {
            panic!("not purge")
        };
        assert_eq!(
            PurgeScope::from(p.scope),
            PurgeScope::Active,
            "active by default"
        );
        assert!(try_parse(["purge"]).is_err(), "an entity is required");
        assert!(try_parse(["dlq", "summary", "q", "--transfer-dlq"]).is_ok());
    }

    #[test]
    fn resubmit_targets_bodies_and_ttl() {
        let spec = resubmit_spec_from(&parse([
            "resubmit",
            "events/subscriptions/all",
            "--dlq",
            "--seq",
            "3",
            "--set-body",
            "hi",
            "--set-ttl",
            "90s",
            "--set-content-type",
            "text/plain",
            "--set-correlation-id",
            "c",
            "--keep-dead-letter-properties",
        ]))
        .unwrap();
        assert_eq!(
            spec.target, "events",
            "a subscription's copies go to its topic"
        );
        assert_eq!(spec.source.view, PeekView::DeadLetter);
        assert_eq!(spec.edits.body, Some(OutgoingBody::Bytes(b"hi".to_vec())));
        assert_eq!(
            spec.edits.time_to_live,
            Some(Some(std::time::Duration::from_secs(90)))
        );
        assert_eq!(spec.edits.content_type, Some(Some("text/plain".into())));
        assert_eq!(spec.edits.correlation_id, Some(Some("c".into())));
        assert!(spec.keep_dead_letter_properties && !spec.remove_original);
        assert_eq!(spec.edits.session_id, None, "unchanged unless asked");
        let e = resubmit_spec_from(&parse([
            "resubmit",
            "q",
            "--seq",
            "1",
            "--to",
            "t/subscriptions/s",
        ]))
        .unwrap_err()
        .to_string();
        assert!(e.contains("topic"), "{e}");
        assert!(
            try_parse([
                "resubmit",
                "q",
                "--seq",
                "1",
                "--set-body",
                "a",
                "--set-body-file",
                "f"
            ])
            .is_err()
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.json");
        std::fs::write(&file, b"{\"a\":1}").unwrap();
        let spec = resubmit_spec_from(&parse([
            "resubmit",
            "q",
            "--seq",
            "1",
            "--set-body-file",
            file.to_str().unwrap(),
        ]))
        .unwrap();
        assert_eq!(
            spec.edits.body,
            Some(OutgoingBody::Bytes(b"{\"a\":1}".to_vec()))
        );
        let missing = dir.path().join("missing");
        let e = resubmit_spec_from(&parse([
            "resubmit",
            "q",
            "--seq",
            "1",
            "--set-body-file",
            missing.to_str().unwrap(),
        ]))
        .unwrap_err();
        assert!(format!("{e:#}").contains("--set-body-file"), "{e:#}");
        assert!(
            resubmit_spec_from(&parse(["resubmit", "q", "--seq", "1", "--set-ttl", "soon"]))
                .is_err()
        );
        assert!(
            resubmit_spec_from(&parse([
                "resubmit",
                "q",
                "--seq",
                "1",
                "--set-prop",
                "novalue"
            ]))
            .is_err()
        );
    }

    /// Every repair command on production without the flags is refused before any I/O, logged
    /// once as the precheck's own denial, and never shows the key.
    #[tokio::test]
    async fn repair_commands_on_production_without_flags_never_connect() {
        let ctx = Ctx::for_test_with_profiles(&[("prod-weu", "prod.servicebus.invalid", true)]);
        let cs = "Endpoint=sb://prod.servicebus.invalid/;SharedAccessKeyName=k;SharedAccessKey=topsecret";
        for (args, verb, kind, named) in [
            (
                vec!["delete", "orders", "--dlq", "--seq", "3"],
                "delete",
                Mutation::Delete,
                "orders",
            ),
            (
                vec!["deferred", "delete", "orders", "--seq", "3"],
                "delete",
                Mutation::Delete,
                "orders",
            ),
            (
                vec!["delete", "orders", "--view", "scheduled", "--seq", "3"],
                "cancel-scheduled",
                Mutation::Delete,
                "orders",
            ),
            (
                vec![
                    "dlq",
                    "delete",
                    "orders",
                    "--all",
                    "--yes",
                    "--confirm-prod",
                    "events",
                ],
                "delete",
                Mutation::Delete,
                "orders",
            ),
            // The target is named too: copies are sent to it.
            (
                vec![
                    "move",
                    "orders",
                    "--dlq",
                    "--seq",
                    "3",
                    "--to",
                    "retry",
                    "--yes",
                    "--confirm-prod",
                    "orders",
                ],
                "move",
                Mutation::Delete,
                "retry",
            ),
            (
                vec![
                    "dlq",
                    "resubmit",
                    "orders",
                    "--all",
                    "--yes",
                    "--confirm-prod",
                    "orders",
                    "--to",
                    "retry",
                ],
                "resubmit",
                Mutation::Send,
                "retry",
            ),
            (
                vec![
                    "dlq",
                    "resubmit",
                    "orders",
                    "--seq",
                    "1",
                    "--remove-original",
                ],
                "resubmit",
                Mutation::Delete,
                "orders",
            ),
            // Every purged entity needs its own --confirm-prod, before any count.
            (
                vec![
                    "purge",
                    "orders",
                    "events",
                    "--scope",
                    "all",
                    "--yes",
                    "--confirm-prod",
                    "orders",
                ],
                "purge",
                Mutation::Purge,
                "events",
            ),
            (
                vec!["dlq", "purge", "orders"],
                "purge",
                Mutation::Purge,
                "orders",
            ),
        ] {
            let mut argv = vec!["dispatched", "--connection-string", cs];
            argv.extend(args.iter().copied());
            let list = || {
                ctx.activity()
                    .unwrap()
                    .list(&dispatched_core::ActivityQuery::default())
                    .unwrap()
            };
            let before = list().len();
            let cli = crate::Cli::try_parse_from(argv).unwrap();
            let e = crate::run_command(&ctx, cli).await.unwrap_err().to_string();
            assert!(
                e.contains(&format!("--confirm-prod {named}")),
                "{args:?}: {e}"
            );
            assert!(!e.contains("topsecret"));
            let log = list();
            assert_eq!(log.len(), before + 1, "{args:?}: one entry, the precheck's");
            assert_eq!(log[0].action, verb, "{args:?}");
            assert_eq!(
                log[0].kind,
                dispatched_core::OpKind::Mutating(kind),
                "{args:?}"
            );
            assert_eq!(log[0].phase, dispatched_core::Phase::Finished);
            assert_eq!(
                log[0].summary,
                format!("{verb} on {named} refused before connecting")
            );
            assert_eq!(log[0].error.as_deref(), Some(e.as_str()));
            assert!(!format!("{:?}", log[0]).contains("topsecret"));
        }
    }

    #[test]
    fn repeated_purge_entities_are_purged_once() {
        let e =
            entities_from(&["orders".into(), "ORDERS".into(), "t/subscriptions/s".into()]).unwrap();
        assert_eq!(
            e,
            [
                EntityPath::queue("orders"),
                EntityPath::subscription("t", "s")
            ]
        );
    }
}
