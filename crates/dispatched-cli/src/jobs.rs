//! Running jobs from the command line: progress on stderr, the result on stdout, the report
//! file, the exit code and Ctrl-C.

use crate::Format;
use crate::output::{sanitize, table};
use crate::safety::MutateArgs;
use dispatched_core::{
    ExportFormat, ExportedMessage, ItemOutcome, JobEvent, JobReport, PeekedMessage, Progress,
};
use horizon_azure_kit::safety::CancellationToken;
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::Path;

/// How a job ended, for the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobExit {
    /// Every item succeeded, or was skipped as the command promised (exit code 0).
    Ok,
    /// Some item failed, was not reached, was missing or was skipped, or the job was cancelled
    /// (exit code 2).
    Incomplete,
}

/// Progress on stderr: a line per 100 items and one at the end, only when stderr is a terminal.
pub struct StderrProgress {
    tty: bool,
}

impl StderrProgress {
    /// Progress for this process (silent unless stderr is a terminal).
    pub fn new() -> Self {
        Self {
            tty: std::io::stderr().is_terminal(),
        }
    }
}

impl Default for StderrProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress for StderrProgress {
    fn event(&self, e: &JobEvent) {
        if !self.tty {
            return;
        }
        if let JobEvent::Progress { done, total } = e
            && let Some(line) = progress_line(*done, *total)
        {
            eprintln!("{line}");
        }
    }
}

/// The progress line for `done` items, or `None` between hundreds.
fn progress_line(done: u64, total: Option<u64>) -> Option<String> {
    let at_end = total == Some(done);
    if !(at_end || (done > 0 && done.is_multiple_of(100))) {
        return None;
    }
    Some(match total {
        Some(t) => format!("{done}/{t} \u{2026}"),
        None => format!("{done} \u{2026}"),
    })
}

/// Whether the job finished every item.
fn job_exit(report: &JobReport) -> JobExit {
    if report.cancelled || report.incomplete() > 0 {
        JobExit::Incomplete
    } else {
        JobExit::Ok
    }
}

/// Write the `--report` file when asked (replacing it only with `--force-report`), then print
/// the result (the summary line, a table of the items that did not succeed and every note; or
/// the report as JSON with `--format json`), and say how the job ended. Everything printed or
/// written is the scrubbed report. The report file comes first: it may be the only full copy of
/// what a destructive read removed, so a closed stdout (`| head`) must never lose it. The job
/// already ran, so nothing here fails the command: a result or report that cannot be written
/// is a warning on stderr and the exit is [`JobExit::Incomplete`] (exit 2).
pub fn finish_job(report: &JobReport, format: Format, args: &MutateArgs) -> JobExit {
    finish_job_with(report, format, args, None, &Extras::default())
}

/// [`finish_job`] with the messages a job received (`receive`) and table text around the
/// result. With messages, the JSON result is one document with a `messages` array in the
/// `peek --format json` shape, and the `--report` file holds the messages in full too
/// ([`JobReport::write_with_messages`]), so a receive that removed messages can always keep a
/// full copy of them.
pub fn finish_job_with(
    report: &JobReport,
    format: Format,
    args: &MutateArgs,
    messages: Option<&[PeekedMessage]>,
    extras: &Extras,
) -> JobExit {
    finish_job_to(
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
        report,
        format,
        args,
        messages,
        extras,
    )
}

/// Table-format text a command prints around the job result (already terminal-safe).
#[derive(Default)]
pub struct Extras {
    /// Before the summary line (`receive`: the messages table).
    pub before: Option<String>,
    /// After the notes (`send --schedule`: the scheduled sequence numbers).
    pub after: Option<String>,
}

fn finish_job_to(
    out: &mut dyn Write,
    err: &mut dyn Write,
    report: &JobReport,
    format: Format,
    args: &MutateArgs,
    messages: Option<&[PeekedMessage]>,
    extras: &Extras,
) -> JobExit {
    let report = report.scrubbed();
    let mut exit = job_exit(&report);
    // The report first (see `finish_job`).
    if let Some(path) = &args.report
        && let Err(e) = match messages {
            Some(m) => report.write_with_messages(path, report_format(path), args.force_report, m),
            None => report.write(path, report_format(path), args.force_report),
        }
    {
        let _ = writeln!(
            err,
            "warning: the report could not be written to {}: {}",
            sanitize(&path.display().to_string()),
            sanitize(&e.to_string())
        );
        exit = JobExit::Incomplete;
    }
    if let Err(e) = print_result(out, &report, format, messages, extras) {
        let _ = writeln!(err, "warning: the result could not be printed: {e}");
        exit = JobExit::Incomplete;
    }
    exit
}

fn print_result(
    out: &mut dyn Write,
    report: &JobReport,
    format: Format,
    messages: Option<&[PeekedMessage]>,
    extras: &Extras,
) -> anyhow::Result<()> {
    match format {
        Format::Table => {
            if let Some(text) = &extras.before {
                writeln!(out, "{text}")?;
            }
            writeln!(out, "{}", sanitize(&report.summary_line()))?;
            // Every item that did not succeed, with its reason: skipped and missing ones too.
            let rows: Vec<Vec<String>> = report
                .not_succeeded()
                .map(|i| {
                    vec![
                        i.sequence_number.map(|s| s.to_string()).unwrap_or_default(),
                        i.message_id.clone().unwrap_or_default(),
                        status(&i.outcome).into(),
                        JobReport::reason(i),
                    ]
                })
                .collect();
            if !rows.is_empty() {
                writeln!(
                    out,
                    "{}",
                    table(&["SEQUENCE", "MESSAGE ID", "STATUS", "REASON"], &rows)
                )?;
            }
            for note in &report.notes {
                writeln!(out, "note: {}", sanitize(note))?;
            }
            if let Some(text) = &extras.after {
                writeln!(out, "{text}")?;
            }
        }
        // `--format ndjson` is refused before a job runs; the report prints as JSON.
        Format::Json | Format::Ndjson => {
            let mut value = serde_json::to_value(report)?;
            if let (Some(messages), Some(fields)) = (messages, value.as_object_mut()) {
                let list: Vec<_> = messages.iter().map(ExportedMessage::new).collect();
                fields.insert("messages".into(), serde_json::to_value(list)?);
            }
            writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
        }
    }
    out.flush()?;
    Ok(())
}

/// Print the one-line result of a mutating command that runs no job (`session state set`).
/// The change was made, so a closed stdout is a warning and exit 2, never a panic.
pub fn print_done(line: &str) -> JobExit {
    print_done_to(
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
        line,
    )
}

fn print_done_to(out: &mut dyn Write, err: &mut dyn Write, line: &str) -> JobExit {
    match writeln!(out, "{line}").and_then(|()| out.flush()) {
        Ok(()) => JobExit::Ok,
        Err(e) => {
            let _ = writeln!(err, "warning: the result could not be printed: {e}");
            JobExit::Incomplete
        }
    }
}

/// NDJSON for `.ndjson` and `.jsonl`, JSON otherwise.
fn report_format(path: &Path) -> ExportFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("ndjson") || e.eq_ignore_ascii_case("jsonl") => {
            ExportFormat::Ndjson
        }
        _ => ExportFormat::Json,
    }
}

fn status(o: &ItemOutcome) -> &'static str {
    match o {
        ItemOutcome::Failed { .. } => "failed",
        ItemOutcome::NotReached { .. } => "not reached",
        ItemOutcome::Succeeded { .. } => "succeeded",
        ItemOutcome::Skipped { .. } => "skipped",
        ItemOutcome::Missing => "missing",
    }
}

/// Something that reports Ctrl-C presses (`None`: no more will come).
trait Presses: Send + 'static {
    fn next_press(&mut self) -> impl Future<Output = Option<()>> + Send;
}

#[cfg(unix)]
impl Presses for tokio::signal::unix::Signal {
    fn next_press(&mut self) -> impl Future<Output = Option<()>> + Send {
        self.recv()
    }
}

#[cfg(windows)]
impl Presses for tokio::signal::windows::CtrlC {
    fn next_press(&mut self) -> impl Future<Output = Option<()>> + Send {
        self.recv()
    }
}

/// Ctrl-C presses, with the handler registered **now** (not when a task first polls), so no
/// press after this call falls through to the default handling. `None` when no handler can be
/// installed (then nothing is ever cancelled).
fn ctrl_c_presses() -> Option<impl Presses> {
    #[cfg(unix)]
    return tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok();
    #[cfg(windows)]
    return tokio::signal::windows::ctrl_c().ok();
}

/// The first press cancels `token`; the second calls `second` (ending the process).
fn watch<P: Presses>(
    token: CancellationToken,
    mut presses: P,
    second: impl FnOnce() + Send + 'static,
) {
    tokio::spawn(async move {
        if presses.next_press().await.is_none() {
            return;
        }
        token.cancel();
        if presses.next_press().await.is_some() {
            second();
        }
    });
}

/// A token that the first Ctrl-C cancels (the job stops between messages and reports what it
/// did); a second Ctrl-C ends the process at once.
pub fn cancel_on_ctrl_c() -> CancellationToken {
    let token = CancellationToken::new();
    arm_ctrl_c(&token);
    token
}

/// From now on, the first Ctrl-C cancels `token` and a second ends the process (exit 130).
/// The handler is registered before this returns. Mutating commands call this once the gate
/// confirmed ([`crate::safety::cli_gate`]), so a Ctrl-C at the confirmation prompt still ends
/// the process.
pub fn arm_ctrl_c(token: &CancellationToken) {
    if let Some(presses) = ctrl_c_presses() {
        watch(token.clone(), presses, || std::process::exit(130));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{ItemResult, JobKind, NotReached};
    use std::time::Duration;
    use time::OffsetDateTime;

    fn item(seq: i64, outcome: ItemOutcome) -> ItemResult {
        ItemResult {
            sequence_number: Some(seq),
            message_id: Some(format!("m{seq}")),
            session_id: None,
            entity: "orders".into(),
            new_sequence_number: None,
            outcome,
        }
    }

    /// `ok` succeeded items and `failed` failed ones.
    fn report(ok: u64, failed: u64) -> JobReport {
        let mut items = Vec::new();
        for i in 0..ok {
            items.push(item(i as i64 + 1, ItemOutcome::Succeeded { detail: None }));
        }
        for i in 0..failed {
            items.push(item(
                100 + i as i64,
                ItemOutcome::Failed {
                    kind: "lock_lost",
                    reason: "the lock expired; run again for the failed sequence numbers".into(),
                },
            ));
        }
        JobReport {
            op_id: Some("op-1".into()),
            kind: JobKind::Delete,
            entity: "orders".into(),
            target: None,
            started_at: OffsetDateTime::UNIX_EPOCH,
            finished_at: OffsetDateTime::UNIX_EPOCH,
            cancelled: false,
            items,
            notes: Vec::new(),
        }
    }

    fn finish_job_for_test(r: &JobReport) -> JobExit {
        finish_job_to(
            &mut Vec::new(),
            &mut Vec::new(),
            r,
            Format::Table,
            &MutateArgs::default(),
            None,
            &Extras::default(),
        )
    }

    fn to_file(path: &Path, force_report: bool) -> MutateArgs {
        MutateArgs {
            report: Some(path.to_path_buf()),
            force_report,
            ..MutateArgs::default()
        }
    }

    fn table_output(r: &JobReport) -> String {
        let mut out = Vec::new();
        finish_job_to(
            &mut out,
            &mut Vec::new(),
            r,
            Format::Table,
            &MutateArgs::default(),
            None,
            &Extras::default(),
        );
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn job_exit_codes() {
        assert_eq!(finish_job_for_test(&report(1, 0)), JobExit::Ok);
        assert_eq!(finish_job_for_test(&report(1, 1)), JobExit::Incomplete);
        assert_eq!(
            finish_job_for_test(&report(0, 2)),
            JobExit::Incomplete,
            "all failed"
        );
        let mut cancelled = report(1, 0);
        cancelled.cancelled = true;
        assert_eq!(
            finish_job_for_test(&cancelled),
            JobExit::Incomplete,
            "cancelled exits 2"
        );
        let mut not_reached = report(1, 0);
        not_reached.items.push(item(
            7,
            ItemOutcome::NotReached {
                why: NotReached::Budget,
            },
        ));
        assert_eq!(finish_job_for_test(&not_reached), JobExit::Incomplete);
        let mut skipped = report(1, 0);
        skipped.items.push(item(
            8,
            ItemOutcome::Skipped {
                reason: "session locked by another receiver".into(),
                expected: false,
            },
        ));
        assert_eq!(
            finish_job_for_test(&skipped),
            JobExit::Incomplete,
            "a skip leaves work undone"
        );
        let mut missing = report(1, 0);
        missing.items.push(item(9, ItemOutcome::Missing));
        assert_eq!(
            finish_job_for_test(&missing),
            JobExit::Incomplete,
            "a missing message was not acted on"
        );
        let mut asked = report(1, 0);
        asked.items.push(item(
            10,
            ItemOutcome::Skipped {
                reason: "scheduled messages are not moved".into(),
                expected: true,
            },
        ));
        assert_eq!(
            finish_job_for_test(&asked),
            JobExit::Ok,
            "a skip the command promised (scheduled messages) is not incomplete"
        );
    }

    #[test]
    fn table_output_lists_skipped_and_missing_items_with_reasons() {
        let mut r = report(1, 0);
        r.items.push(item(
            8,
            ItemOutcome::Skipped {
                reason: "session B is locked by another receiver".into(),
                expected: false,
            },
        ));
        r.items.push(item(9, ItemOutcome::Missing));
        let text = table_output(&r);
        assert!(text.contains("1 skipped, 1 missing"), "{text}");
        assert!(
            text.lines().any(|l| l.starts_with('8')
                && l.contains("skipped")
                && l.contains("session B is locked")),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|l| l.starts_with('9') && l.contains("missing") && l.contains("not found")),
            "{text}"
        );
    }

    #[test]
    fn table_output_has_the_summary_every_failure_and_every_note() {
        let mut r = report(1, 6);
        r.notes = vec![
            "up to 3 more messages may have been removed after the wait and are not in this result"
                .into(),
            "2 messages could not be unlocked".into(),
        ];
        let text = table_output(&r);
        assert!(text.starts_with(&r.summary_line()), "{text}");
        for seq in 100..106 {
            assert!(
                text.lines()
                    .any(|l| l.starts_with(&seq.to_string()) && l.contains("the lock expired")),
                "failure {seq} in the table: {text}"
            );
        }
        for note in &r.notes {
            assert!(text.contains(note.as_str()), "note missing: {text}");
        }
    }

    #[test]
    fn output_and_report_files_are_scrubbed() {
        let mut r = report(0, 1);
        let secret = "SharedAccessKey=topsecretvalue";
        r.items[0].outcome = ItemOutcome::Failed {
            kind: "network",
            reason: format!("broker said Endpoint=sb://x/;{secret}"),
        };
        r.notes = vec![format!("note with {secret}")];
        assert!(!table_output(&r).contains("topsecretvalue"));
        let mut json = Vec::new();
        finish_job_to(
            &mut json,
            &mut Vec::new(),
            &r,
            Format::Json,
            &MutateArgs::default(),
            None,
            &Extras::default(),
        );
        let json = String::from_utf8(json).unwrap();
        assert!(!json.contains("topsecretvalue"));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["notes"].as_array().unwrap().len(), 1, "notes in JSON too");

        let dir = tempfile::tempdir().unwrap();
        for name in ["r.json", "r.ndjson"] {
            let path = dir.path().join(name);
            let exit = finish_job_to(
                &mut Vec::new(),
                &mut Vec::new(),
                &r,
                Format::Table,
                &to_file(&path, false),
                None,
                &Extras::default(),
            );
            assert_eq!(exit, JobExit::Incomplete);
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains("topsecretvalue"));
            if name.ends_with(".ndjson") {
                assert_eq!(text.lines().count(), 2, "one item and the summary line");
                assert!(text.lines().last().unwrap().contains(r#""type":"summary""#));
            } else {
                assert!(
                    serde_json::from_str::<serde_json::Value>(&text).unwrap()["items"].is_array()
                );
            }
        }
    }

    fn received(seq: i64, body: &str) -> PeekedMessage {
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
            body: dispatched_core::MessageBody::Text(body.into()),
        }
    }

    #[test]
    fn json_result_carries_the_messages_in_one_document() {
        let messages = [received(7, "hello")];
        let mut out = Vec::new();
        finish_job_to(
            &mut out,
            &mut Vec::new(),
            &report(1, 0),
            Format::Json,
            &MutateArgs::default(),
            Some(&messages),
            &Extras::default(),
        );
        let v: serde_json::Value = serde_json::from_slice(&out).expect("one JSON document");
        assert_eq!(v["messages"][0]["sequence_number"], 7);
        assert!(v["items"].is_array() && v["succeeded"] == 1, "{v}");
        // The table leaves them to the caller.
        let mut out = Vec::new();
        finish_job_to(
            &mut out,
            &mut Vec::new(),
            &report(1, 0),
            Format::Table,
            &MutateArgs::default(),
            Some(&messages),
            &Extras::default(),
        );
        assert!(!String::from_utf8(out).unwrap().contains("hello"));
    }

    #[test]
    fn a_receive_report_file_keeps_the_full_messages() {
        let long = "b".repeat(300);
        let messages = [received(7, &long)];
        let dir = tempfile::tempdir().unwrap();
        for name in ["r.json", "r.ndjson"] {
            let path = dir.path().join(name);
            let exit = finish_job_to(
                &mut Vec::new(),
                &mut Vec::new(),
                &report(1, 0),
                Format::Table,
                &to_file(&path, false),
                Some(&messages),
                &Extras::default(),
            );
            assert_eq!(exit, JobExit::Ok);
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(&long), "{name} holds the whole body");
        }
        // Other jobs write the bare report.
        let path = dir.path().join("plain.json");
        finish_job_to(
            &mut Vec::new(),
            &mut Vec::new(),
            &report(1, 0),
            Format::Table,
            &to_file(&path, false),
            None,
            &Extras::default(),
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v.get("messages").is_none());
    }

    /// A stdout whose reader went away: every write fails as a closed pipe does. Without
    /// SIGPIPE ignored the process would die at the first write, so it notes whether the
    /// report file existed by then.
    struct ClosedPipe<'a> {
        report: &'a Path,
        report_at_first_write: Option<bool>,
    }

    impl Write for ClosedPipe<'_> {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            self.report_at_first_write
                .get_or_insert_with(|| self.report.exists());
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn a_closed_stdout_after_a_change_is_a_warning() {
        let path = Path::new("unused");
        let mut out = ClosedPipe {
            report: path,
            report_at_first_write: None,
        };
        let mut err = Vec::new();
        assert_eq!(
            print_done_to(&mut out, &mut err, "Set it"),
            JobExit::Incomplete
        );
        assert!(
            String::from_utf8(err)
                .unwrap()
                .starts_with("warning: the result")
        );
        let mut out = Vec::new();
        assert_eq!(
            print_done_to(&mut out, &mut Vec::new(), "Set it"),
            JobExit::Ok
        );
        assert_eq!(out, b"Set it\n");
    }

    #[test]
    fn a_closed_stdout_never_loses_the_report_file() {
        // `receive --delete --report removed.json | head`: the report is the only full copy of
        // what was removed, so it is written before anything goes to stdout.
        let messages = [received(7, "only copy")];
        let dir = tempfile::tempdir().unwrap();
        for (i, format) in [Format::Table, Format::Json].into_iter().enumerate() {
            let path = dir.path().join(format!("r{i}.json"));
            let mut out = ClosedPipe {
                report: &path,
                report_at_first_write: None,
            };
            let mut err = Vec::new();
            let exit = finish_job_to(
                &mut out,
                &mut err,
                &report(1, 0),
                format,
                &to_file(&path, false),
                Some(&messages),
                &Extras {
                    before: Some("SEQUENCE ...".into()),
                    after: None,
                },
            );
            assert_eq!(
                out.report_at_first_write,
                Some(true),
                "the report is on disk before stdout is touched"
            );
            assert_eq!(exit, JobExit::Incomplete, "the result was not printed");
            let text = std::fs::read_to_string(&path).expect("the report exists");
            assert!(text.contains("only copy"), "{text}");
            let err = String::from_utf8(err).unwrap();
            assert!(
                err.starts_with("warning: the result could not be printed"),
                "{err}"
            );
        }
    }

    /// Presses fed from a channel.
    struct Fed(tokio::sync::mpsc::UnboundedReceiver<()>);

    impl Presses for Fed {
        fn next_press(&mut self) -> impl Future<Output = Option<()>> + Send {
            self.0.recv()
        }
    }

    #[tokio::test]
    async fn ctrl_c_cancels_the_job_token() {
        // The first press cancels the token; the second runs the exit.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (exited_tx, exited) = tokio::sync::oneshot::channel::<()>();
        let token = CancellationToken::new();
        watch(token.clone(), Fed(rx), move || {
            let _ = exited_tx.send(());
        });
        assert!(!token.is_cancelled());
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), token.cancelled())
            .await
            .unwrap();
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), exited)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn arming_registers_the_handler_before_it_returns() {
        // Registration happens in the call, not in the spawned task.
        assert!(ctrl_c_presses().is_some());
        let token = CancellationToken::new();
        arm_ctrl_c(&token);
        assert!(!token.is_cancelled());
    }

    #[test]
    fn progress_lines_every_hundred_and_at_the_end() {
        assert_eq!(progress_line(1, Some(250)), None);
        assert_eq!(progress_line(100, Some(250)).as_deref(), Some("100/250 …"));
        assert_eq!(progress_line(250, Some(250)).as_deref(), Some("250/250 …"));
        assert_eq!(progress_line(200, None).as_deref(), Some("200 …"));
        assert_eq!(progress_line(7, None), None);
    }

    #[test]
    fn a_report_that_cannot_be_written_after_the_job_is_a_warning_and_exit_2() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("r.json");
        std::fs::write(&existing, "old").unwrap();
        // The file appeared after the pre-check: the job still ran, so no error, exit 2.
        let mut err = Vec::new();
        let exit = finish_job_to(
            &mut Vec::new(),
            &mut err,
            &report(2, 0),
            Format::Table,
            &to_file(&existing, false),
            None,
            &Extras::default(),
        );
        assert_eq!(exit, JobExit::Incomplete);
        let err = String::from_utf8(err).unwrap();
        assert!(
            err.starts_with(&format!(
                "warning: the report could not be written to {}",
                existing.display()
            )),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "old");
        // A missing directory too.
        let exit = finish_job_to(
            &mut Vec::new(),
            &mut Vec::new(),
            &report(2, 0),
            Format::Table,
            &to_file(&dir.path().join("gone/r.json"), false),
            None,
            &Extras::default(),
        );
        assert_eq!(exit, JobExit::Incomplete);
        // --force-report replaces it.
        let exit = finish_job_to(
            &mut Vec::new(),
            &mut Vec::new(),
            &report(2, 0),
            Format::Table,
            &to_file(&existing, true),
            None,
            &Extras::default(),
        );
        assert_eq!(exit, JobExit::Ok);
        assert!(
            std::fs::read_to_string(&existing)
                .unwrap()
                .contains("\"items\"")
        );
    }
}
