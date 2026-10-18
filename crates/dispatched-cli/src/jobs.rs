//! Running jobs from the command line: progress on stderr, the result on stdout, the report
//! file, the exit code and Ctrl-C.

// Wired into the mutating commands by Tasks 19-21; until then only the tests use most of it.
#![allow(dead_code)]

use crate::Format;
use crate::output::{sanitize, table};
use anyhow::Context as _;
use dispatched_core::{ExportFormat, ItemOutcome, JobEvent, JobReport, Progress};
use horizon_azure_kit::safety::CancellationToken;
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::Path;

/// How a job ended, for the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobExit {
    /// Every item succeeded or was skipped (exit code 0).
    Ok,
    /// Some item failed or was not reached, or the job was cancelled (exit code 2).
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
    if report.cancelled || report.failed() > 0 {
        JobExit::Incomplete
    } else {
        JobExit::Ok
    }
}

/// Print the result (the summary line, a table of failures and every note; or the report as
/// JSON with `--format json`), write the report file when asked, and say how the job ended.
/// Everything printed or written is the scrubbed report.
pub fn finish_job(
    report: &JobReport,
    format: Format,
    report_path: Option<&Path>,
) -> anyhow::Result<JobExit> {
    finish_job_to(&mut std::io::stdout().lock(), report, format, report_path)
}

fn finish_job_to(
    out: &mut dyn Write,
    report: &JobReport,
    format: Format,
    report_path: Option<&Path>,
) -> anyhow::Result<JobExit> {
    let report = report.scrubbed();
    match format {
        Format::Table => {
            writeln!(out, "{}", sanitize(&report.summary_line()))?;
            let rows: Vec<Vec<String>> = report
                .failures()
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
        }
        // `--format ndjson` is refused before a job runs; the report prints as JSON.
        Format::Json | Format::Ndjson => {
            writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
        }
    }
    out.flush()?;
    if let Some(path) = report_path {
        report
            .write(path, report_format(path), false)
            .with_context(|| {
                format!(
                    "the job finished, but its report could not be written to {}",
                    path.display()
                )
            })?;
    }
    Ok(job_exit(&report))
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

/// A token that is cancelled when `signal` resolves.
pub fn cancel_on<F>(signal: F) -> CancellationToken
where
    F: Future<Output = ()> + Send + 'static,
{
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        signal.await;
        t.cancel();
    });
    token
}

/// A token that the first Ctrl-C cancels (the job stops between messages and reports what it
/// did); a second Ctrl-C ends the process at once.
pub fn cancel_on_ctrl_c() -> CancellationToken {
    cancel_on(async {
        if tokio::signal::ctrl_c().await.is_err() {
            // No signal handler: never cancel.
            std::future::pending::<()>().await;
        }
        tokio::spawn(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                std::process::exit(130);
            }
        });
    })
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
        finish_job_to(&mut Vec::new(), r, Format::Table, None).unwrap()
    }

    fn table_output(r: &JobReport) -> String {
        let mut out = Vec::new();
        finish_job_to(&mut out, r, Format::Table, None).unwrap();
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
        skipped.items.push(item(8, ItemOutcome::Missing));
        assert_eq!(
            finish_job_for_test(&skipped),
            JobExit::Ok,
            "skipped is fine"
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
        finish_job_to(&mut json, &r, Format::Json, None).unwrap();
        let json = String::from_utf8(json).unwrap();
        assert!(!json.contains("topsecretvalue"));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["notes"].as_array().unwrap().len(), 1, "notes in JSON too");

        let dir = tempfile::tempdir().unwrap();
        for name in ["r.json", "r.ndjson"] {
            let path = dir.path().join(name);
            let exit = finish_job_to(&mut Vec::new(), &r, Format::Table, Some(&path)).unwrap();
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

    #[test]
    fn progress_lines_every_hundred_and_at_the_end() {
        assert_eq!(progress_line(1, Some(250)), None);
        assert_eq!(progress_line(100, Some(250)).as_deref(), Some("100/250 …"));
        assert_eq!(progress_line(250, Some(250)).as_deref(), Some("250/250 …"));
        assert_eq!(progress_line(200, None).as_deref(), Some("200 …"));
        assert_eq!(progress_line(7, None), None);
    }

    #[tokio::test]
    async fn ctrl_c_cancels_the_job_token() {
        // The handler is a function of a signal future: `cancel_on(fut)`; resolving the future
        // cancels the token.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let token = cancel_on(async move {
            let _ = rx.await;
        });
        assert!(!token.is_cancelled());
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), token.cancelled())
            .await
            .unwrap();
    }
}
