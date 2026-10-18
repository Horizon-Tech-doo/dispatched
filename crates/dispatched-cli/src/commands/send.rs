//! `dispatched send`, `scheduled cancel` and `import`.

use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, StderrProgress, finish_job};
use crate::output::{sanitize, table, thousands};
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned};
use anyhow::Context as _;
use clap::{ArgGroup, Args, Subcommand};
use dispatched_core::messaging::{parse_duration, parse_prop_arg, parse_when, read_template};
use dispatched_core::ops::{ImportOptions, MAX_SELECTION, send_target};
use dispatched_core::{
    BodySource, Continuation, EntityPath, Explorer, ItemOutcome, JobReport, MessageIdSource,
    Mutation, OutgoingMessage, PeekOptions, PeekView, SendRequest, read_messages, stream_pages,
};
use std::path::PathBuf;
use time::OffsetDateTime;

/// The most messages one `send` makes.
pub const MAX_COUNT: u64 = 100_000;

/// `send` now (the planned kind and verb of [`dispatched_core::Operation::send`]).
pub const SEND: Planned = Planned {
    kind: Mutation::Send,
    verb: "send",
};
/// `send --schedule`.
pub const SCHEDULE: Planned = Planned {
    kind: Mutation::Send,
    verb: "schedule",
};
/// `scheduled cancel`.
pub const CANCEL_SCHEDULED: Planned = Planned {
    kind: Mutation::Delete,
    verb: "cancel-scheduled",
};
/// `import`.
pub const IMPORT: Planned = Planned {
    kind: Mutation::Send,
    verb: "import",
};

/// `send`: messages to a queue or topic, now or scheduled.
#[derive(Args, Debug)]
pub struct SendArgs {
    /// Queue or topic.
    pub entity: String,
    /// The body text.
    #[arg(long, conflicts_with_all = ["file", "template"])]
    pub body: Option<String>,
    /// Body files, used round-robin (repeat, or list several). With --template, they replace
    /// its body.
    #[arg(long, value_name = "PATH", num_args = 1..)]
    pub file: Vec<PathBuf>,
    /// A BrokeredMessageTemplate file (JSON or XML); the flags override its fields.
    #[arg(long, value_name = "PATH")]
    pub template: Option<PathBuf>,
    /// A custom property, `KEY=VALUE[:TYPE]` (string, int, long, double, bool, datetime,
    /// timespan, guid; string by default). Repeat for several.
    #[arg(long = "prop", value_name = "KEY=VALUE[:TYPE]")]
    pub props: Vec<String>,
    /// MessageId (the same on every message unless --increment-id).
    #[arg(long)]
    pub message_id: Option<String>,
    /// Count the MessageId up per message (order-0009, order-0010, ...).
    #[arg(long, requires = "message_id")]
    pub increment_id: bool,
    /// SessionId.
    #[arg(long)]
    pub session_id: Option<String>,
    /// CorrelationId.
    #[arg(long)]
    pub correlation_id: Option<String>,
    /// Subject (Label).
    #[arg(long)]
    pub subject: Option<String>,
    /// To.
    #[arg(long)]
    pub to: Option<String>,
    /// ReplyTo.
    #[arg(long)]
    pub reply_to: Option<String>,
    /// ReplyToSessionId.
    #[arg(long)]
    pub reply_to_session_id: Option<String>,
    /// ContentType.
    #[arg(long)]
    pub content_type: Option<String>,
    /// PartitionKey.
    #[arg(long)]
    pub partition_key: Option<String>,
    /// Time to live: 90s, 15m, 2h, 7d, 1h30m, ISO 8601 or .NET TimeSpan text.
    #[arg(long, value_name = "DURATION")]
    pub ttl: Option<String>,
    /// Schedule the messages instead of sending them now: +DURATION (such as +10m) or an
    /// RFC 3339 time. Prints the new sequence numbers, which `scheduled cancel` takes.
    #[arg(long, value_name = "TIME")]
    pub schedule: Option<String>,
    /// How many messages (1 to 100,000).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..=MAX_COUNT))]
    pub count: u64,
    /// Send them all over one link, pipelined (not atomic), instead of one call per message.
    #[arg(long)]
    pub batch: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `scheduled`: scheduled messages.
#[derive(Subcommand, Debug)]
pub enum ScheduledCmd {
    /// Cancel scheduled messages by sequence number, or all of them.
    Cancel(ScheduledCancelArgs),
}

/// `scheduled cancel`.
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("which").required(true).args(["seq", "all"])))]
pub struct ScheduledCancelArgs {
    /// Queue or topic.
    pub entity: String,
    /// A sequence number `send --schedule` printed (repeat, or separate with commas).
    #[arg(long, value_name = "N", value_delimiter = ',')]
    pub seq: Vec<i64>,
    /// Every scheduled message (up to 100,000 at a time).
    #[arg(long)]
    pub all: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `import`: send the messages of an export file.
#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Queue or topic.
    pub entity: String,
    /// An `export` file (JSON or NDJSON).
    #[arg(long, value_name = "PATH")]
    pub file: PathBuf,
    /// Give each message a new random MessageId.
    #[arg(long)]
    pub new_message_id: bool,
    /// Clear SessionId (and a PartitionKey equal to it).
    #[arg(long)]
    pub no_sessions: bool,
    /// Keep the DeadLetterReason and DeadLetterErrorDescription properties.
    #[arg(long)]
    pub keep_dead_letter_properties: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// The send request the flags describe. Reads the template (a local file) but nothing else:
/// the target is checked with core's [`send_target`] (a subscription is refused), the time,
/// TTL and properties are parsed, and the message is validated, all before any connection.
pub fn send_request_from(a: &SendArgs) -> anyhow::Result<SendRequest> {
    let target = send_target(&a.entity)?.base();
    let mut template = match &a.template {
        Some(path) => read_template(path)
            .with_context(|| format!("--template {}", sanitize(&path.display().to_string())))?,
        None => OutgoingMessage::default(),
    };
    let body = match (&a.body, a.file.is_empty()) {
        (Some(text), _) => Some(BodySource::Text(text.clone())),
        (None, false) => Some(BodySource::Files(a.file.clone())),
        (None, true) => None,
    };
    let set = |field: &mut Option<String>, flag: &Option<String>| {
        if let Some(v) = flag {
            *field = Some(v.clone());
        }
    };
    set(&mut template.session_id, &a.session_id);
    set(&mut template.correlation_id, &a.correlation_id);
    set(&mut template.subject, &a.subject);
    set(&mut template.to, &a.to);
    set(&mut template.reply_to, &a.reply_to);
    set(&mut template.reply_to_session_id, &a.reply_to_session_id);
    set(&mut template.content_type, &a.content_type);
    set(&mut template.partition_key, &a.partition_key);
    if let Some(ttl) = &a.ttl {
        template.time_to_live =
            Some(parse_duration(ttl).map_err(|e| anyhow::anyhow!("--ttl: {e}"))?);
    }
    for prop in &a.props {
        let (key, value) =
            parse_prop_arg(prop).map_err(|e| anyhow::anyhow!("--prop {prop}: {e}"))?;
        template.properties.insert(key, value);
    }
    let ids = match (&a.message_id, a.increment_id) {
        (Some(base), true) => MessageIdSource::Increment { base: base.clone() },
        (Some(id), false) => MessageIdSource::Fixed(id.clone()),
        // The template's id, or a new one per message (as the Azure SDKs do).
        (None, _) if template.message_id.is_some() => MessageIdSource::Keep,
        (None, _) => MessageIdSource::Random,
    };
    if let Some(id) = ids.id_for(0) {
        template.message_id = Some(id);
    }
    template.validate()?;
    let schedule = match &a.schedule {
        // The core error names both forms (+DURATION and RFC 3339).
        Some(when) => Some(
            parse_when(when, OffsetDateTime::now_utc())
                .map_err(|e| anyhow::anyhow!("--schedule: {e}"))?,
        ),
        None => None,
    };
    Ok(SendRequest {
        target,
        template,
        body,
        ids,
        count: a.count,
        schedule,
        batch: a.batch,
    })
}

/// The note for several messages with one MessageId.
pub const DUPLICATE_ID_NOTE: &str =
    "Duplicate detection may drop copies with the same MessageId; use --increment-id";

/// [`DUPLICATE_ID_NOTE`] when `req` sends several messages with the same fixed MessageId
/// (from `--message-id` or the template).
fn duplicate_id_note(req: &SendRequest) -> Option<&'static str> {
    let fixed = match &req.ids {
        MessageIdSource::Fixed(_) => true,
        MessageIdSource::Keep => req.template.message_id.is_some(),
        MessageIdSource::Random | MessageIdSource::Increment { .. } => false,
    };
    (req.count > 1 && fixed).then_some(DUPLICATE_ID_NOTE)
}

/// Error kinds of a sender link that failed or lost its access during the job.
const LINK_KINDS: [&str; 5] = ["network", "protocol", "auth", "forbidden", "timeout"];

/// A note explaining failed messages of a send or import whose link failed: a long send can
/// outlive the access it was opened with, and messages in flight may have arrived.
pub fn link_failure_note(report: &JobReport) -> Option<String> {
    let n = report
        .items
        .iter()
        .filter(
            |i| matches!(&i.outcome, ItemOutcome::Failed { kind, .. } if LINK_KINDS.contains(kind)),
        )
        .count() as u64;
    (n > 0).then(|| {
        let what = if n == 1 {
            "1 message was".to_string()
        } else {
            format!("{} messages were", thousands(n))
        };
        format!(
            "{what} not confirmed because the connection to the entity failed or its access \
             expired during the job (a long send can outlive its access token); some may still \
             have arrived, so check the entity before sending the failed ones again"
        )
    })
}

/// The new sequence numbers of a scheduled send, one row per scheduled message (failed ones
/// are in the failures table). `None` when nothing was scheduled.
pub fn scheduled_table(report: &JobReport) -> Option<String> {
    let rows: Vec<Vec<String>> = report
        .items
        .iter()
        .filter_map(|i| {
            i.new_sequence_number
                .map(|seq| vec![seq.to_string(), i.message_id.clone().unwrap_or_default()])
        })
        .collect();
    (!rows.is_empty()).then(|| table(&["SEQUENCE", "MESSAGE ID"], &rows))
}

pub async fn send(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &SendArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let req = send_request_from(args)?;
    let entity = send_target(&req.target)?;
    let scheduled = req.schedule.is_some();
    let planned = if scheduled { SCHEDULE } else { SEND };
    let note = duplicate_id_note(&req);
    let (operator, cancel) = operator_for(ctx, sel, &args.mutate, planned, &[entity])?;
    let result = operator.send(req, &StderrProgress::new(), &cancel).await;
    operator.explorer().close().await;
    let mut report = result?;
    report.notes.extend(note.map(str::to_string));
    report.notes.extend(link_failure_note(&report));
    let exit = finish_job(&report, format, &args.mutate);
    if scheduled
        && matches!(format, Format::Table)
        && let Some(t) = scheduled_table(&report)
    {
        println!("{t}");
        eprintln!(
            "(cancel with `dispatched scheduled cancel {} --seq N`)",
            sanitize(&report.entity)
        );
    }
    Ok(exit)
}

pub async fn scheduled(
    ctx: &Ctx,
    sel: &Selection<'_>,
    cmd: &ScheduledCmd,
    format: Format,
) -> anyhow::Result<JobExit> {
    let ScheduledCmd::Cancel(args) = cmd;
    // Scheduled messages live on the queue or topic.
    let entity = send_target(&args.entity)?;
    let (operator, cancel) = operator_for(
        ctx,
        sel,
        &args.mutate,
        CANCEL_SCHEDULED,
        std::slice::from_ref(&entity),
    )?;
    let progress = StderrProgress::new();
    let result = async {
        let (seqs, complete) = if args.all {
            scheduled_sequences(operator.explorer(), &entity).await?
        } else {
            (args.seq.clone(), true)
        };
        if seqs.is_empty() {
            return Ok(None);
        }
        operator
            .cancel_scheduled(&entity, &seqs, &progress, &cancel)
            .await
            .map(|r| Some((r, complete)))
            .map_err(anyhow::Error::from)
    }
    .await;
    operator.explorer().close().await;
    let Some((mut report, complete)) = result? else {
        eprintln!("(no scheduled messages; nothing was changed)");
        return Ok(JobExit::Ok);
    };
    if !complete {
        report.notes.push(format!(
            "the scheduled view was not read to the end (at most {} are cancelled at a time); \
             more scheduled messages may remain: run `scheduled cancel --all` again",
            thousands(MAX_SELECTION)
        ));
    }
    Ok(finish_job(&report, format, &args.mutate))
}

/// The sequence numbers of the scheduled messages of `entity`, at most [`MAX_SELECTION`],
/// and whether the view was read to its end.
async fn scheduled_sequences(
    explorer: &Explorer,
    entity: &EntityPath,
) -> anyhow::Result<(Vec<i64>, bool)> {
    let mut seqs = Vec::new();
    let streamed = stream_pages(
        Some(MAX_SELECTION),
        0,
        None,
        |from, max| {
            let opts = PeekOptions::new(max).from_sequence(from);
            async move { explorer.peek(entity, PeekView::Scheduled, &opts).await }
        },
        |page, _| {
            seqs.extend(page.iter().map(|m| m.sequence_number));
            Ok(())
        },
    )
    .await
    .context("listing the scheduled messages")?;
    Ok((seqs, streamed.end == Continuation::End))
}

pub async fn import(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ImportArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let entity = send_target(&args.entity)?;
    let shown = sanitize(&args.file.display().to_string());
    let messages = read_messages(&args.file).with_context(|| format!("--file {shown}"))?;
    if messages.is_empty() {
        anyhow::bail!("{shown} holds no messages; nothing was sent");
    }
    let opts = ImportOptions {
        new_message_id: args.new_message_id,
        keep_sessions: !args.no_sessions,
        drop_dead_letter_properties: !args.keep_dead_letter_properties,
    };
    let (operator, cancel) = operator_for(
        ctx,
        sel,
        &args.mutate,
        IMPORT,
        std::slice::from_ref(&entity),
    )?;
    let result = operator
        .import(
            entity.base(),
            messages,
            opts,
            &StderrProgress::new(),
            &cancel,
        )
        .await;
    operator.explorer().close().await;
    let mut report = result?;
    report.notes.extend(link_failure_note(&report));
    Ok(finish_job(&report, format, &args.mutate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use dispatched_core::MessageIdSource;
    use dispatched_core::messaging::TypedValue;
    use std::time::Duration;
    use time::OffsetDateTime;
    use time::macros::datetime;

    fn try_parse<const N: usize>(args: [&str; N]) -> Result<crate::Cli, clap::Error> {
        crate::Cli::try_parse_from(std::iter::once("dispatched").chain(args))
    }

    fn parse<const N: usize>(args: [&str; N]) -> SendArgs {
        match try_parse(args).unwrap().command {
            crate::Command::Send(a) => *a,
            _ => panic!("not send"),
        }
    }

    #[test]
    fn send_flags_build_the_request() {
        let req = send_request_from(&parse([
            "send",
            "orders",
            "--body",
            "{}",
            "--prop",
            "n=1:int",
            "--message-id",
            "m-01",
            "--increment-id",
            "--ttl",
            "2h",
            "--count",
            "3",
            "--content-type",
            "application/json",
        ]))
        .unwrap();
        assert_eq!(req.target, "orders");
        assert_eq!(req.count, 3);
        assert!(matches!(req.ids, MessageIdSource::Increment { ref base } if base == "m-01"));
        assert_eq!(req.template.time_to_live, Some(Duration::from_secs(7200)));
        assert_eq!(req.template.properties["n"], TypedValue::Int(1));
    }

    #[test]
    fn send_refuses_conflicting_bodies_and_subscription_targets() {
        assert!(try_parse(["send", "q", "--body", "a", "--file", "f"]).is_err());
        assert!(
            send_request_from(&parse(["send", "events/subscriptions/all", "--body", "x"]))
                .unwrap_err()
                .to_string()
                .contains("send to the topic")
        );
        assert!(try_parse(["send", "q", "--increment-id"]).is_err());
    }

    #[test]
    fn scheduled_cancel_needs_seq_or_all() {
        assert!(try_parse(["scheduled", "cancel", "q"]).is_err());
        assert!(try_parse(["scheduled", "cancel", "q", "--seq", "5", "--seq", "6"]).is_ok());
        assert!(try_parse(["scheduled", "cancel", "q", "--all", "--seq", "5"]).is_err());
    }

    #[test]
    fn schedule_time_accepts_relative_and_absolute() {
        let before = OffsetDateTime::now_utc();
        let rel = send_request_from(&parse(["send", "q", "--body", "x", "--schedule", "+10m"]))
            .unwrap()
            .schedule
            .unwrap();
        assert!(
            rel >= before + time::Duration::minutes(10)
                && rel <= OffsetDateTime::now_utc() + time::Duration::minutes(10)
        );
        let abs = send_request_from(&parse([
            "send",
            "q",
            "--body",
            "x",
            "--schedule",
            "2030-01-01T00:00:00Z",
        ]))
        .unwrap()
        .schedule;
        assert_eq!(abs, Some(datetime!(2030-01-01 0:00 UTC)));
        let e = send_request_from(&parse([
            "send",
            "q",
            "--body",
            "x",
            "--schedule",
            "yesterday",
        ]))
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("+DURATION") && e.contains("RFC 3339"),
            "names both forms: {e}"
        );
    }

    const PROD_CS: &str = "Endpoint=sb://prod.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=topsecret";

    fn prod_ctx() -> crate::context::Ctx {
        crate::context::Ctx::for_test_with_profiles(&[(
            "prod-weu",
            "prod.servicebus.windows.net",
            true,
        )])
    }

    fn log(ctx: &crate::context::Ctx) -> Vec<dispatched_core::ActivityEntry> {
        ctx.activity()
            .unwrap()
            .list(&dispatched_core::ActivityQuery::default())
            .unwrap()
    }

    #[test]
    fn planned_kinds_match_the_operations() {
        use dispatched_core::{Operation, SubQueue, Target};
        let t = || Target::new(EntityPath::queue("q"), SubQueue::Active);
        let at = Some(OffsetDateTime::now_utc());
        for (planned, op) in [
            (SEND, Operation::send(t(), 1, None)),
            (SCHEDULE, Operation::send(t(), 1, at)),
            (CANCEL_SCHEDULED, Operation::cancel_scheduled(t(), 1)),
            (IMPORT, Operation::import(t(), 1)),
        ] {
            assert_eq!((planned.kind, planned.verb), (op.mutation, op.verb()));
        }
    }

    #[tokio::test]
    async fn send_on_production_without_flags_never_connects() {
        let ctx = prod_ctx();
        for (args, verb) in [
            (vec!["send", "orders", "--body", "x"], "send"),
            (
                vec!["send", "orders", "--body", "x", "--schedule", "+1h"],
                "schedule",
            ),
            (
                vec!["scheduled", "cancel", "orders", "--all"],
                "cancel-scheduled",
            ),
        ] {
            let mut argv = vec!["dispatched", "--connection-string", PROD_CS];
            argv.extend(args);
            let cli = crate::Cli::try_parse_from(argv).unwrap();
            let e = crate::run_command(&ctx, cli).await.unwrap_err().to_string();
            assert!(e.contains("--confirm-prod orders"), "{e}");
            assert!(!e.contains("topsecret"));
            let entry = log(&ctx).into_iter().next().unwrap();
            assert_eq!(entry.action, verb);
            assert_eq!(entry.outcome, dispatched_core::Outcome::Denied);
        }
        let mut kinds: Vec<_> = log(&ctx).iter().map(|e| e.kind).collect();
        kinds.dedup();
        assert_eq!(
            kinds,
            [
                dispatched_core::OpKind::Mutating(Mutation::Delete),
                dispatched_core::OpKind::Mutating(Mutation::Send)
            ],
            "newest first"
        );
    }

    #[test]
    fn several_messages_with_one_id_get_the_duplicate_note() {
        let note = |args: &[&str]| {
            let mut argv = vec!["send", "q", "--body", "x"];
            argv.extend(args);
            let cli =
                crate::Cli::try_parse_from(std::iter::once("dispatched").chain(argv)).unwrap();
            let crate::Command::Send(a) = cli.command else {
                panic!()
            };
            duplicate_id_note(&send_request_from(&a).unwrap())
        };
        assert_eq!(
            note(&["--message-id", "m", "--count", "2"]),
            Some(DUPLICATE_ID_NOTE)
        );
        assert_eq!(note(&["--message-id", "m"]), None, "one message");
        assert_eq!(
            note(&["--message-id", "m", "--increment-id", "--count", "2"]),
            None
        );
        assert_eq!(note(&["--count", "2"]), None, "random ids");
        let random = send_request_from(&parse(["send", "q", "--body", "x"])).unwrap();
        assert_eq!(random.ids, MessageIdSource::Random);
        assert!(random.template.message_id.is_some());
    }

    #[test]
    fn template_fields_are_overridden_by_flags_and_files_replace_its_body() {
        let dir = tempfile::tempdir().unwrap();
        let tpl = dir.path().join("t.json");
        std::fs::write(
            &tpl,
            r#"{"messageId":"t-1","subject":"from-template","to":"x","body":"tb","properties":[{"key":"k","type":"Int32","value":"5"}]}"#,
        )
        .unwrap();
        let tpl_s = tpl.to_str().unwrap();
        let req = send_request_from(&parse([
            "send",
            "q",
            "--template",
            tpl_s,
            "--subject",
            "flag",
            "--prop",
            "k=7:int",
            "--file",
            "a.bin",
            "b.bin",
        ]))
        .unwrap();
        assert_eq!(req.template.subject.as_deref(), Some("flag"));
        assert_eq!(req.template.to.as_deref(), Some("x"));
        assert_eq!(req.template.message_id.as_deref(), Some("t-1"));
        assert_eq!(req.ids, MessageIdSource::Keep);
        assert_eq!(req.template.properties["k"], TypedValue::Int(7));
        assert_eq!(
            req.body,
            Some(BodySource::Files(vec!["a.bin".into(), "b.bin".into()]))
        );
        assert!(try_parse(["send", "q", "--body", "a", "--template", tpl_s]).is_err());
        assert!(try_parse(["send", "q", "--count", "0"]).is_err());
        assert!(try_parse(["send", "q", "--count", "100001"]).is_err());
        let e = send_request_from(&parse(["send", "q", "--ttl", "0s"]))
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("--ttl"), "{e}");
    }

    fn item(seq: Option<i64>, id: &str, outcome: ItemOutcome) -> dispatched_core::ItemResult {
        dispatched_core::ItemResult {
            sequence_number: None,
            message_id: Some(id.into()),
            session_id: None,
            entity: "q".into(),
            new_sequence_number: seq,
            outcome,
        }
    }

    fn report(items: Vec<dispatched_core::ItemResult>) -> JobReport {
        JobReport {
            op_id: None,
            kind: dispatched_core::JobKind::Send,
            entity: "q".into(),
            target: None,
            started_at: OffsetDateTime::UNIX_EPOCH,
            finished_at: OffsetDateTime::UNIX_EPOCH,
            cancelled: false,
            items,
            notes: Vec::new(),
        }
    }

    #[test]
    fn a_partly_failed_schedule_still_prints_the_new_sequence_numbers() {
        let failed = ItemOutcome::Failed {
            kind: "network",
            reason: "the connection was lost".into(),
        };
        let r = report(vec![
            item(Some(41), "m-1", ItemOutcome::Succeeded { detail: None }),
            item(Some(42), "m-2", ItemOutcome::Succeeded { detail: None }),
            item(None, "m-3", failed),
        ]);
        let t = scheduled_table(&r).unwrap();
        assert!(
            t.lines().any(|l| l.starts_with("41") && l.contains("m-1")),
            "{t}"
        );
        assert!(
            t.lines().any(|l| l.starts_with("42") && l.contains("m-2")),
            "{t}"
        );
        assert!(!t.contains("m-3"));
        assert_eq!(scheduled_table(&report(Vec::new())), None);
    }

    #[test]
    fn link_failures_are_explained() {
        let fail = |kind| ItemOutcome::Failed {
            kind,
            reason: "x".into(),
        };
        let r = report(vec![
            item(None, "a", ItemOutcome::Succeeded { detail: None }),
            item(None, "b", fail("auth")),
            item(None, "c", fail("network")),
            item(None, "d", fail("io")),
        ]);
        let note = link_failure_note(&r).unwrap();
        assert!(note.starts_with("2 messages were not confirmed"), "{note}");
        assert!(
            note.contains("access token") && note.contains("check the entity"),
            "{note}"
        );
        assert_eq!(
            link_failure_note(&report(vec![item(None, "d", fail("io"))])),
            None
        );
    }
}
