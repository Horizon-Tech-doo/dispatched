//! `dispatched listen`: a live receive stream until Ctrl-C.
//!
//! Each message is printed as it arrives: one line (table) or one JSON object per line
//! (`--format json`, the `peek --format json` message shape with `"type": "message"`). After
//! Ctrl-C the summary follows: the summary line and notes, or one JSON object with
//! `"type": "summary"`.

use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, arm_ctrl_c, report_format};
use crate::output::{ellipsize, sanitize};
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned, check_report_path, cli_gate};
use clap::Args;
use dispatched_core::ops::listen::{MAX_CONCURRENCY, MAX_PREFETCH};
use dispatched_core::{
    EntityPath, Explorer, ExportedMessage, JobEvent, JobReport, ListenMode, ListenOutcome,
    ListenRequest, ListenSessions, MessageBody, Mutation, Operator, PeekedMessage, Progress,
    SubQueue,
};
use horizon_azure_kit::safety::CancellationToken;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use time::format_description::well_known::Rfc3339;

/// `listen`.
#[derive(Args, Debug)]
pub struct ListenArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Listen to the dead-letter queue.
    #[arg(long)]
    pub dlq: bool,
    /// Listen to this session only.
    #[arg(long, conflicts_with = "each_session")]
    pub session: Option<String>,
    /// Take each session in turn: the next available one, until it goes quiet.
    #[arg(long)]
    pub each_session: bool,
    /// Peek-lock each message and complete it (removes it). Without --complete or --delete
    /// the listener only watches: nothing is locked or removed.
    #[arg(long, conflicts_with = "delete")]
    pub complete: bool,
    /// Receive and delete: each message is removed as it is delivered.
    #[arg(long)]
    pub delete: bool,
    /// Receivers to run at once (1 to 8).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=MAX_CONCURRENCY as i64))]
    pub concurrency: u8,
    /// With --delete: messages each receiver asks for at once (0 to 100).
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=MAX_PREFETCH as i64))]
    pub prefetch: u32,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `listen --complete`.
pub const LISTEN_COMPLETE: Planned = Planned {
    kind: Mutation::Settle,
    verb: "listen-complete",
};
/// `listen --delete`.
pub const LISTEN_DELETE: Planned = Planned {
    kind: Mutation::Delete,
    verb: "listen-delete",
};

/// The listen request the flags describe (no I/O).
pub fn listen_request_from(a: &ListenArgs) -> anyhow::Result<ListenRequest> {
    if a.prefetch > 0 && !a.delete {
        anyhow::bail!("--prefetch goes with --delete: peek-lock receives one message at a time");
    }
    Ok(ListenRequest {
        entity: EntityPath::parse(&a.entity)?,
        sub_queue: if a.dlq {
            SubQueue::DeadLetter
        } else {
            SubQueue::Active
        },
        sessions: match (&a.session, a.each_session) {
            (Some(id), _) => ListenSessions::Id(id.clone()),
            (None, true) => ListenSessions::Each,
            (None, false) => ListenSessions::None,
        },
        mode: match (a.complete, a.delete) {
            (true, _) => ListenMode::Complete,
            (_, true) => ListenMode::Delete,
            _ => ListenMode::Watch,
        },
        concurrency: a.concurrency,
        prefetch: a.prefetch,
    })
}

/// The planned kind and verb of a listener that removes messages; `None` for watching.
pub fn planned_listen(mode: ListenMode) -> Option<Planned> {
    match mode {
        ListenMode::Watch => None,
        ListenMode::Complete => Some(LISTEN_COMPLETE),
        ListenMode::Delete => Some(LISTEN_DELETE),
    }
}

/// Prints each message as it arrives; failures go to stderr.
struct Stream {
    format: Format,
    /// stdout stopped taking output (a closed pipe): stop writing, warn once at the end.
    broken: AtomicBool,
}

fn body_cell(body: &MessageBody) -> String {
    match body.as_text() {
        Some(text) => ellipsize(&text, 60),
        None => format!("<{} bytes>", body.len()),
    }
}

/// The table line of one message.
fn message_line(m: &PeekedMessage) -> String {
    format!(
        "{:<8} {:<25} {:<36} {:<16} {}",
        m.sequence_number,
        m.enqueued_time
            .and_then(|t| t.format(&Rfc3339).ok())
            .unwrap_or_default(),
        ellipsize(m.message_id.as_deref().unwrap_or(""), 36),
        ellipsize(m.session_id.as_deref().unwrap_or(""), 16),
        body_cell(&m.body)
    )
}

/// The table header.
const HEADER: &str =
    "SEQ      ENQUEUED                  MESSAGE ID                           SESSION          BODY";

/// The JSON line of one message.
fn message_json(m: &PeekedMessage) -> anyhow::Result<String> {
    let mut v = serde_json::to_value(ExportedMessage::new(m))?;
    if let Some(fields) = v.as_object_mut() {
        fields.insert("type".into(), "message".into());
    }
    Ok(serde_json::to_string(&v)?)
}

impl Stream {
    fn write(&self, line: &str) {
        if self.broken.load(Ordering::Relaxed) {
            return;
        }
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            self.broken.store(true, Ordering::Relaxed);
        }
    }
}

impl Progress for Stream {
    fn event(&self, e: &JobEvent) {
        match e {
            JobEvent::Received(m) => match self.format {
                Format::Table => self.write(&message_line(m)),
                _ => match message_json(m) {
                    Ok(line) => self.write(&line),
                    Err(e) => eprintln!("warning: a message could not be printed: {e}"),
                },
            },
            JobEvent::Item(i) => eprintln!(
                "failed: {}{}",
                i.sequence_number
                    .map(|s| format!("sequence {s}: "))
                    .unwrap_or_default(),
                sanitize(&JobReport::reason(i))
            ),
            _ => {}
        }
    }
}

/// The operator for a watch: no confirmation (nothing changes), but the same selection, and
/// Ctrl-C stops it.
fn watch_operator(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &MutateArgs,
) -> anyhow::Result<(Operator, CancellationToken)> {
    check_report_path(args)?;
    let resolved = super::browse::mutating_target(ctx, sel)?;
    let (gate, cancel) = cli_gate(ctx, &resolved, args)?;
    arm_ctrl_c(&cancel);
    let explorer = Arc::new(Explorer::new(resolved.namespace)?);
    Ok((Operator::new(explorer, gate), cancel))
}

/// The table note of a listener that removes messages without a JSON copy.
pub const SHORTENED_NOTE: &str =
    "bodies are shortened; use --format json to keep full copies of the messages removed";

pub async fn listen(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ListenArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let req = listen_request_from(args)?;
    let (operator, cancel) = match planned_listen(req.mode) {
        Some(planned) => operator_for(
            ctx,
            sel,
            &args.mutate,
            planned,
            std::slice::from_ref(&req.entity),
        )?,
        None => watch_operator(ctx, sel, &args.mutate)?,
    };
    let removes = req.mode != ListenMode::Watch;
    if let Format::Table = format {
        if removes {
            eprintln!("{SHORTENED_NOTE}");
        }
        eprintln!(
            "listening on {}; Ctrl-C stops",
            req.entity.address(req.sub_queue)
        );
    }
    let stream = Stream {
        format,
        broken: AtomicBool::new(false),
    };
    if let Format::Table = format {
        stream.write(HEADER);
    }
    let result = operator.listen(req, &stream, &cancel).await;
    operator.explorer().close().await;
    let outcome = result?;
    Ok(finish(
        &outcome,
        format,
        &args.mutate,
        stream.broken.load(Ordering::Relaxed),
    ))
}

/// Write the `--report` file (first: the job already ran), then print the summary.
fn finish(outcome: &ListenOutcome, format: Format, args: &MutateArgs, broken: bool) -> JobExit {
    let report = outcome.report.scrubbed();
    let mut exit = if report.incomplete() > 0 {
        JobExit::Incomplete
    } else {
        JobExit::Ok
    };
    if let Some(path) = &args.report
        && let Err(e) = report.write(path, report_format(path), args.force_report)
    {
        eprintln!(
            "warning: the report could not be written to {}: {}",
            sanitize(&path.display().to_string()),
            sanitize(&e.to_string())
        );
        exit = JobExit::Incomplete;
    }
    let printed = match format {
        Format::Table => {
            let mut text = sanitize(&report.summary_line());
            // The first note is the counts, already in the summary line.
            for note in report.notes.iter().skip(1) {
                text.push_str(&format!("\nnote: {}", sanitize(note)));
            }
            text
        }
        _ => summary_json(&report, outcome),
    };
    let mut out = std::io::stdout().lock();
    if broken
        || writeln!(out, "{printed}")
            .and_then(|()| out.flush())
            .is_err()
    {
        eprintln!("warning: the result could not be printed: stdout is closed");
        exit = JobExit::Incomplete;
    }
    exit
}

/// The closing JSON line: the report with the counts, `"type": "summary"`.
fn summary_json(report: &JobReport, outcome: &ListenOutcome) -> String {
    let mut v = serde_json::to_value(report).unwrap_or_default();
    if let Some(fields) = v.as_object_mut() {
        fields.insert("type".into(), "summary".into());
        fields.insert("received".into(), outcome.received.into());
        fields.insert("removed".into(), outcome.removed.into());
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn try_parse<const N: usize>(args: [&str; N]) -> Result<crate::Cli, clap::Error> {
        crate::Cli::try_parse_from(std::iter::once("dispatched").chain(args))
    }

    fn parse<const N: usize>(args: [&str; N]) -> ListenArgs {
        match try_parse(args).unwrap().command {
            crate::Command::Listen(a) => a,
            _ => panic!("not listen"),
        }
    }

    #[test]
    fn listen_flags_parse() {
        let r = listen_request_from(&parse(["listen", "orders"])).unwrap();
        assert_eq!(r.mode, ListenMode::Watch);
        assert_eq!(r.sessions, ListenSessions::None);
        assert_eq!((r.concurrency, r.prefetch), (1, 0));
        let r = listen_request_from(&parse([
            "listen",
            "events/subscriptions/all",
            "--dlq",
            "--delete",
            "--concurrency",
            "4",
            "--prefetch",
            "50",
        ]))
        .unwrap();
        assert_eq!(r.entity, EntityPath::subscription("events", "all"));
        assert_eq!(r.sub_queue, SubQueue::DeadLetter);
        assert_eq!(r.mode, ListenMode::Delete);
        assert_eq!((r.concurrency, r.prefetch), (4, 50));
        let r =
            listen_request_from(&parse(["listen", "q", "--complete", "--each-session"])).unwrap();
        assert_eq!(
            (r.mode, r.sessions),
            (ListenMode::Complete, ListenSessions::Each)
        );
        let r = listen_request_from(&parse(["listen", "q", "--session", "s1"])).unwrap();
        assert_eq!(r.sessions, ListenSessions::Id("s1".into()));
        for bad in [
            &["listen", "q", "--complete", "--delete"][..],
            &["listen", "q", "--session", "s", "--each-session"],
            &["listen", "q", "--delete", "--prefetch", "101"],
            &["listen", "q", "--concurrency", "0"],
            &["listen", "q", "--concurrency", "9"],
        ] {
            assert!(
                crate::Cli::try_parse_from(
                    std::iter::once("dispatched").chain(bad.iter().copied())
                )
                .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn prefetch_goes_with_delete_only() {
        for args in [
            ["listen", "q", "--complete", "--prefetch", "5"],
            ["listen", "q", "--dlq", "--prefetch", "5"],
        ] {
            let e = listen_request_from(&parse(args)).unwrap_err().to_string();
            assert!(e.contains("--prefetch goes with --delete"), "{e}");
        }
    }

    #[test]
    fn planned_kinds_match_the_operations() {
        use dispatched_core::{Operation, Target};
        let t = || Target::new(EntityPath::queue("q"), SubQueue::Active);
        assert_eq!(planned_listen(ListenMode::Watch), None);
        for (mode, delete) in [(ListenMode::Complete, false), (ListenMode::Delete, true)] {
            let planned = planned_listen(mode).unwrap();
            let op = Operation::listen(t(), delete);
            assert_eq!((planned.kind, planned.verb), (op.mutation, op.verb()));
        }
    }

    #[tokio::test]
    async fn listen_delete_needs_yes_without_a_terminal() {
        let ctx = crate::context::Ctx::for_test_with_profiles(&[]).interactive_for_test(false);
        let cs =
            "Endpoint=sb://dev.servicebus.invalid/;SharedAccessKeyName=k;SharedAccessKey=topsecret";
        for args in [
            vec!["listen", "orders", "--delete"],
            vec!["listen", "orders", "--complete"],
        ] {
            let mut argv = vec!["dispatched", "--connection-string", cs];
            argv.extend(args);
            let cli = crate::Cli::try_parse_from(argv).unwrap();
            let e = crate::run_command(&ctx, cli).await.unwrap_err().to_string();
            assert!(
                e.contains("needs a confirmation and there is no terminal to ask on"),
                "{e}"
            );
            assert!(!e.contains("topsecret"));
        }
        let log = ctx
            .activity()
            .unwrap()
            .list(&dispatched_core::ActivityQuery::default())
            .unwrap();
        assert_eq!(log[0].action, "listen-complete");
        assert_eq!(log[1].action, "listen-delete");
    }

    #[test]
    fn messages_print_as_lines_and_json_objects() {
        let m = PeekedMessage {
            sequence_number: 42,
            state: dispatched_core::MessageState::Active,
            message_id: Some("m-1".into()),
            correlation_id: None,
            session_id: None,
            subject: None,
            content_type: None,
            to: None,
            reply_to: None,
            reply_to_session_id: None,
            partition_key: None,
            expires_at: None,
            enqueued_time: None,
            scheduled_enqueue_time: None,
            locked_until: None,
            delivery_count: 0,
            time_to_live: None,
            dead_letter_reason: None,
            dead_letter_description: None,
            dead_letter_source: None,
            application_properties: Default::default(),
            body: MessageBody::Text("hello".into()),
        };
        let line = message_line(&m);
        assert!(line.starts_with("42 "), "{line}");
        assert!(line.contains("m-1"));
        let v: serde_json::Value = serde_json::from_str(&message_json(&m).unwrap()).unwrap();
        assert_eq!(v["type"], "message");
        assert_eq!(v["sequence_number"], 42);
    }
}
