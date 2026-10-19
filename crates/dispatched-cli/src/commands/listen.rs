//! `dispatched listen`: a live receive stream until Ctrl-C.
//!
//! Each message is printed as it arrives: one line (table) or one JSON object per line
//! (`--format json`, the `peek --format json` message shape with `"type": "message"`). After
//! Ctrl-C the summary follows: the summary line and notes, or one JSON object with
//! `"type": "summary"`.

use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, arm_ctrl_c, clear_second_press, on_second_press, report_format};
use crate::output::{ellipsize, sanitize};
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned, check_report_path, cli_gate};
use clap::Args;
use dispatched_core::ops::listen::{MAX_CONCURRENCY, MAX_PREFETCH};
use dispatched_core::{
    EntityPath, Explorer, ExportedMessage, ItemResult, JobEvent, JobKind, JobReport, ListenMode,
    ListenOutcome, ListenRequest, ListenSessions, MessageBody, Mutation, Operator, PeekedMessage,
    Progress, SubQueue,
};
use horizon_azure_kit::safety::CancellationToken;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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

/// Prints each message as it arrives; failures go to stderr. In a mode that removes messages
/// it is also the account of what was removed: a stdout that stops taking output (a closed
/// pipe) cancels the listener at once, every received message that was not printed is
/// counted, and with `--report` every received message is kept in full for the report file
/// (§14 Decision 17: a destructive read offers a full copy).
struct Stream {
    format: Format,
    /// The entity listened to (for the report a second Ctrl-C writes).
    entity: String,
    out: Mutex<Box<dyn Write + Send>>,
    /// The listener's token: cancelled on the first failed write.
    cancel: CancellationToken,
    /// stdout stopped taking output: nothing more is written to it.
    broken: AtomicBool,
    /// Received messages that were not printed.
    unshown: AtomicU64,
    /// Every received message, in full, when a removing listener writes a `--report`.
    kept: Option<Mutex<Vec<PeekedMessage>>>,
    /// Failed items so far (for the report a second Ctrl-C writes).
    items: Mutex<Vec<ItemResult>>,
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
    fn new(
        format: Format,
        entity: String,
        out: Box<dyn Write + Send>,
        cancel: CancellationToken,
        keep: bool,
    ) -> Self {
        Self {
            format,
            entity,
            out: Mutex::new(out),
            cancel,
            broken: AtomicBool::new(false),
            unshown: AtomicU64::new(0),
            kept: keep.then(|| Mutex::new(Vec::new())),
            items: Mutex::new(Vec::new()),
        }
    }

    /// Write one line; whether it was written.
    fn write(&self, line: &str) -> bool {
        if self.broken.load(Ordering::Relaxed) {
            return false;
        }
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            // Nobody reads what follows: stop receiving now rather than remove messages
            // nobody sees (`| head`, a reader that failed).
            self.broken.store(true, Ordering::Relaxed);
            self.cancel.cancel();
            return false;
        }
        true
    }

    fn unshown(&self) -> u64 {
        self.unshown.load(Ordering::Relaxed)
    }

    fn kept(&self) -> Option<Vec<PeekedMessage>> {
        self.kept
            .as_ref()
            .map(|k| k.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

impl Progress for Stream {
    fn event(&self, e: &JobEvent) {
        match e {
            JobEvent::Received(m) => {
                if let Some(kept) = &self.kept {
                    kept.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((**m).clone());
                }
                let shown = match self.format {
                    Format::Table => self.write(&message_line(m)),
                    _ => match message_json(m) {
                        Ok(line) => self.write(&line),
                        Err(e) => {
                            eprintln!("warning: a message could not be printed: {e}");
                            false
                        }
                    },
                };
                if !shown {
                    self.unshown.fetch_add(1, Ordering::Relaxed);
                }
            }
            JobEvent::Item(i) => {
                self.items
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(i.clone());
                eprintln!(
                    "failed: {}{}",
                    i.sequence_number
                        .map(|s| format!("sequence {s}: "))
                        .unwrap_or_default(),
                    sanitize(&JobReport::reason(i))
                )
            }
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

/// The table note of a listener that removes messages with no `--report`.
pub const SHORTENED_NOTE: &str = "bodies are shortened; use --format json or --report FILE to keep full copies of the messages removed";

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
        if removes && args.mutate.report.is_none() {
            eprintln!("{SHORTENED_NOTE}");
        }
        eprintln!(
            "listening on {}; Ctrl-C stops",
            req.entity.address(req.sub_queue)
        );
    }
    let stream = Arc::new(Stream::new(
        format,
        req.entity.address(req.sub_queue),
        Box::new(std::io::stdout()),
        cancel.clone(),
        removes && args.mutate.report.is_some(),
    ));
    if removes {
        // A second Ctrl-C ends the process while the receivers close: what was received is
        // written to the report, and what the receivers may still hold is counted.
        let (stream, mutate) = (stream.clone(), args.mutate.clone());
        let at_risk = held_at_most(&req);
        on_second_press(move || {
            stream.write_interrupted(&mutate, at_risk, &mut std::io::stderr().lock())
        });
    }
    if let Format::Table = format {
        stream.write(HEADER);
    }
    let result = operator.listen(req, stream.as_ref(), &cancel).await;
    operator.explorer().close().await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(e) => {
            clear_second_press();
            return Err(e.into());
        }
    };
    let exit = finish(
        &outcome,
        format,
        &args.mutate,
        &stream,
        &mut std::io::stderr().lock(),
    );
    clear_second_press();
    Ok(exit)
}

/// The most removed messages the receivers of `req` can hold that were not handed out yet:
/// each receiver's credit (receive-and-delete), or the one message each completes before it
/// is shown (peek-lock).
fn held_at_most(req: &ListenRequest) -> u64 {
    let per_receiver = match req.mode {
        ListenMode::Delete => u64::from(req.prefetch.max(1)),
        _ => 1,
    };
    u64::from(req.concurrency) * per_receiver
}

/// `1 removed message was` / `3 removed messages were`.
fn removed_were(n: u64) -> String {
    if n == 1 {
        "1 removed message was".into()
    } else {
        format!("{n} removed messages were")
    }
}

impl Stream {
    /// What a second Ctrl-C writes before the process ends: the `--report` file with every
    /// message received so far, or (without one) the count of removed messages not shown.
    /// `at_risk` is the most the receivers may still hold. Never fails: the process is ending.
    fn write_interrupted(&self, args: &MutateArgs, at_risk: u64, err: &mut dyn Write) {
        let unshown = self.unshown();
        let (Some(path), Some(messages)) = (&args.report, self.kept()) else {
            let shown = if unshown > 0 {
                format!(
                    "{} not shown, and up to {at_risk} more",
                    removed_were(unshown)
                )
            } else {
                format!("up to {at_risk} removed messages")
            };
            let _ = writeln!(
                err,
                "warning: interrupted while the receivers closed; {shown} the receivers held are in no output (pass --report FILE to keep them)"
            );
            return;
        };
        let items = self.items.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let report = crate::jobs::interrupted_report(
            JobKind::Listen,
            self.entity.clone(),
            items,
            &format!(
                "interrupted by a second Ctrl-C while the receivers closed; up to {at_risk} more removed messages they held are not in this report"
            ),
        );
        let shown = sanitize(&path.display().to_string());
        let n = messages.len();
        let received = if n == 1 {
            "1 message received was".to_string()
        } else {
            format!("{n} messages received were")
        };
        match report.write_with_messages(path, report_format(path), args.force_report, &messages) {
            Ok(()) => {
                let _ = writeln!(
                    err,
                    "interrupted; {received} written to {shown}; up to {at_risk} more removed messages the receivers held are in no output"
                );
            }
            Err(e) => {
                let _ = writeln!(
                    err,
                    "warning: interrupted; {received} not kept: the report could not be written to {shown}: {}",
                    sanitize(&e.to_string())
                );
            }
        }
    }
}

/// Write the `--report` file (first: the job already ran), then print the summary.
fn finish(
    outcome: &ListenOutcome,
    format: Format,
    args: &MutateArgs,
    stream: &Stream,
    err: &mut dyn Write,
) -> JobExit {
    let report = outcome.report.scrubbed();
    let mut exit = if report.incomplete() > 0 {
        JobExit::Incomplete
    } else {
        JobExit::Ok
    };
    let kept = stream.kept();
    let mut report_written = None;
    if let Some(path) = &args.report {
        let written = match &kept {
            Some(m) => report.write_with_messages(path, report_format(path), args.force_report, m),
            None => report.write(path, report_format(path), args.force_report),
        };
        let shown = sanitize(&path.display().to_string());
        match written {
            Ok(()) => report_written = Some(shown),
            Err(e) => {
                let _ = writeln!(
                    err,
                    "warning: the report could not be written to {shown}: {}",
                    sanitize(&e.to_string())
                );
                exit = JobExit::Incomplete;
            }
        }
    }
    let printed = match format {
        Format::Table => summary_text(&report),
        _ => summary_json(&report, outcome),
    };
    if stream.write(&printed) {
        return exit;
    }
    // stdout is closed: nothing more goes to it. The summary and its notes (uncertain or
    // undecodable removals included) go to stderr, with what the closed stdout did not show.
    let _ = writeln!(err, "{}", summary_text(&report));
    let unshown = stream.unshown();
    let what = match (&report_written, &kept) {
        (Some(path), Some(_)) if unshown > 0 => format!(
            "note: stdout closed, so the listener stopped; {} not shown; the report {path} holds them in full",
            removed_were(unshown)
        ),
        _ if unshown > 0 => format!(
            "warning: stdout closed, so the listener stopped; {} not shown (pass --report FILE to keep them)",
            removed_were(unshown)
        ),
        _ => "warning: the result could not be printed: stdout is closed".into(),
    };
    let _ = writeln!(err, "{what}");
    JobExit::Incomplete
}

/// The summary line and the notes after the first (the counts, already in the line).
fn summary_text(report: &JobReport) -> String {
    let mut text = sanitize(&report.summary_line());
    for note in report.notes.iter().skip(1) {
        text.push_str(&format!("\nnote: {}", sanitize(note)));
    }
    text
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

    /// stdout after the reader went away: every write fails.
    struct ClosedPipe;
    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    /// A stdout that takes `lines` lines, then closes (`| head -n lines`).
    struct HeadPipe {
        lines: usize,
        taken: Arc<Mutex<Vec<u8>>>,
    }
    impl Write for HeadPipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut taken = self.taken.lock().unwrap();
            if taken.iter().filter(|b| **b == b'\n').count() >= self.lines {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            taken.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn removed_outcome(n: u64) -> ListenOutcome {
        let mut report = crate::jobs::interrupted_report(
            dispatched_core::JobKind::Listen,
            "orders".into(),
            Vec::new(),
            &format!("{n} messages received, {n} removed"),
        );
        report.cancelled = false;
        ListenOutcome {
            report,
            received: n,
            removed: n,
        }
    }

    fn received(stream: &Stream, body: &str) {
        let mut m = message(7);
        m.body = MessageBody::Text(body.into());
        stream.event(&JobEvent::Received(Box::new(m)));
    }

    #[test]
    fn a_closed_stdout_stops_a_removing_listener_and_counts_what_it_did_not_show() {
        // `listen orders --delete --format json | head -n 2`: the third line fails. The
        // listener is cancelled at once, and every message received from then on (the ones
        // the receivers' close returns) is counted, never printed and never lost silently.
        let taken = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let stream = Stream::new(
            Format::Json,
            "orders".into(),
            Box::new(HeadPipe {
                lines: 2,
                taken: taken.clone(),
            }),
            cancel.clone(),
            false,
        );
        received(&stream, "one");
        received(&stream, "two");
        assert!(!cancel.is_cancelled());
        received(&stream, "three");
        assert!(
            cancel.is_cancelled(),
            "the listener stops at the failed write"
        );
        received(&stream, "four");
        received(&stream, "five");
        assert_eq!(stream.unshown(), 3);
        let mut err = Vec::new();
        let exit = finish(
            &removed_outcome(5),
            Format::Json,
            &MutateArgs::default(),
            &stream,
            &mut err,
        );
        assert_eq!(exit, JobExit::Incomplete);
        let out = String::from_utf8(taken.lock().unwrap().clone()).unwrap();
        assert_eq!(
            out.lines().count(),
            2,
            "nothing more on the closed stdout: {out}"
        );
        let err = String::from_utf8(err).unwrap();
        assert!(
            err.contains("Listened to orders: 5 messages received, 5 removed"),
            "the summary goes to stderr: {err}"
        );
        assert!(
            err.contains(
                "warning: stdout closed, so the listener stopped; 3 removed messages were not shown (pass --report FILE to keep them)"
            ),
            "{err}"
        );
    }

    #[test]
    fn with_a_report_the_messages_a_closed_stdout_missed_are_kept_in_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("removed.json");
        let args = MutateArgs {
            report: Some(path.clone()),
            ..MutateArgs::default()
        };
        let stream = Stream::new(
            Format::Table,
            "orders".into(),
            Box::new(ClosedPipe),
            CancellationToken::new(),
            true,
        );
        received(&stream, "first");
        received(&stream, "second");
        let mut err = Vec::new();
        let exit = finish(&removed_outcome(2), Format::Table, &args, &stream, &mut err);
        assert_eq!(exit, JobExit::Incomplete);
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let bodies: Vec<_> = report["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["body"]["value"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(bodies, ["first", "second"], "{report}");
        let err = String::from_utf8(err).unwrap();
        assert!(
            err.contains("2 removed messages were not shown; the report")
                && err.contains("holds them in full"),
            "{err}"
        );
    }

    #[test]
    fn a_removing_listener_report_keeps_every_message_in_full() {
        // stdout works: the report still holds full copies (the table shortens bodies).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("removed.ndjson");
        let args = MutateArgs {
            report: Some(path.clone()),
            ..MutateArgs::default()
        };
        let taken = Arc::new(Mutex::new(Vec::new()));
        let stream = Stream::new(
            Format::Table,
            "orders".into(),
            Box::new(HeadPipe { lines: 100, taken }),
            CancellationToken::new(),
            true,
        );
        received(&stream, &"x".repeat(200));
        let exit = finish(
            &removed_outcome(1),
            Format::Table,
            &args,
            &stream,
            &mut Vec::new(),
        );
        assert_eq!(exit, JobExit::Ok);
        let text = std::fs::read_to_string(&path).unwrap();
        let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["type"], "message");
        assert_eq!(first["body"]["value"].as_str().unwrap().len(), 200);
    }

    #[test]
    fn a_second_ctrl_c_accounts_for_what_the_listener_removed() {
        // The process ends while the receivers close: what was received is written to the
        // report, and the messages the receivers may still hold are counted on stderr.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("removed.json");
        let args = MutateArgs {
            report: Some(path.clone()),
            ..MutateArgs::default()
        };
        let stream = Stream::new(
            Format::Json,
            "orders".into(),
            Box::new(ClosedPipe),
            CancellationToken::new(),
            true,
        );
        received(&stream, "kept");
        let mut err = Vec::new();
        stream.write_interrupted(&args, 800, &mut err);
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(report["messages"][0]["body"]["value"], "kept");
        assert_eq!(report["cancelled"], true);
        let err = String::from_utf8(err).unwrap();
        assert!(
            err.contains("1 message received was written to")
                && err.contains(
                    "up to 800 more removed messages the receivers held are in no output"
                ),
            "{err}"
        );
        // Without --report: counted on stderr.
        let stream = Stream::new(
            Format::Json,
            "orders".into(),
            Box::new(ClosedPipe),
            CancellationToken::new(),
            false,
        );
        received(&stream, "lost");
        let mut err = Vec::new();
        stream.write_interrupted(&MutateArgs::default(), 8, &mut err);
        let err = String::from_utf8(err).unwrap();
        assert!(
            err.contains("warning: interrupted while the receivers closed; 1 removed message was not shown, and up to 8 more the receivers held are in no output (pass --report FILE to keep them)"),
            "{err}"
        );
    }

    fn message(seq: i64) -> PeekedMessage {
        PeekedMessage {
            sequence_number: seq,
            state: dispatched_core::MessageState::Active,
            message_id: None,
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
            body: MessageBody::Text(String::new()),
        }
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
