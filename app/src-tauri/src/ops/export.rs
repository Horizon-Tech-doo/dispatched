//! Export a view, or selected messages, to a JSON or NDJSON file as a cancellable job.

use crate::error::CommandError;
use crate::events::{ActivityError, Events, JobEvent, tracked};
use crate::jobs::Jobs;
use crate::ops::browse::SCAN_PER_CALL;
use dispatched_core::messaging::SequenceCollection;
use dispatched_core::{
    Continuation, EntityPath, Error, Explorer, ExportFormat, FileExport, FilterSpec, MessageFilter,
    PeekOptions, PeekPage, PeekView, stream_pages,
};
use horizon_azure_kit::safety::CancellationToken;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Most messages examined to find selected sequence numbers again.
const SELECTED_BUDGET: u64 = 1_000_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ExportScope {
    All {
        #[serde(default)]
        filter: FilterSpec,
        #[serde(default)]
        from_sequence: i64,
        #[serde(default)]
        max: Option<u64>,
    },
    Selected {
        sequences: Vec<i64>,
    },
}

/// What to export and where. `pick` names a path the save dialog returned (`pick_export_path`);
/// the webview never sends a path, so it can only make Rust write where the user chose.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub entity: String,
    pub view: PeekView,
    pub scope: ExportScope,
    pub format: String,
    pub pick: String,
    #[serde(default)]
    pub total_hint: Option<u64>,
}

/// A path the save dialog returned. `name` and `dir` are for display only.
#[derive(Debug, Serialize)]
pub struct PickedPath {
    pub pick: String,
    pub name: String,
    pub dir: String,
}

#[derive(Debug, Serialize)]
pub struct JobStarted {
    pub id: String,
}

#[derive(Debug)]
pub enum Scope {
    All {
        filter: MessageFilter,
        from: i64,
        max: Option<u64>,
    },
    Selected(Vec<i64>),
}

#[derive(Debug)]
pub struct ExportPlan {
    pub entity: EntityPath,
    pub view: PeekView,
    pub scope: Scope,
    pub file: FileExport,
    pub path: PathBuf,
    /// `Export {entity} ({view})`, without the file.
    pub title: String,
    pub file_name: String,
    pub total: Option<u64>,
}

#[derive(Debug)]
pub struct ExportSummary {
    /// Messages written.
    pub written: u64,
    /// Selected messages that were no longer there (0 for an export of a whole view).
    pub missing: u64,
    pub path: PathBuf,
    pub summary: String,
}

/// Reads an export needs (the explorer; a fake in tests).
#[async_trait::async_trait]
pub trait ExportSource: Send + Sync {
    async fn page(
        &self,
        entity: &EntityPath,
        view: PeekView,
        opts: &PeekOptions,
    ) -> dispatched_core::Result<PeekPage>;
    async fn sequences(
        &self,
        entity: &EntityPath,
        view: PeekView,
        sequences: &[i64],
        budget: u64,
        cancel: &CancellationToken,
    ) -> dispatched_core::Result<SequenceCollection>;
}

#[async_trait::async_trait]
impl ExportSource for Explorer {
    async fn page(
        &self,
        entity: &EntityPath,
        view: PeekView,
        opts: &PeekOptions,
    ) -> dispatched_core::Result<PeekPage> {
        self.peek(entity, view, opts).await
    }
    async fn sequences(
        &self,
        entity: &EntityPath,
        view: PeekView,
        sequences: &[i64],
        budget: u64,
        cancel: &CancellationToken,
    ) -> dispatched_core::Result<SequenceCollection> {
        self.collect_sequences(entity, view, sequences, budget, Some(cancel))
            .await
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The view as the UI names it, so a job's title reads like the view's own tab.
fn view_label(view: PeekView) -> &'static str {
    match view {
        PeekView::Active => "active messages",
        PeekView::Scheduled => "scheduled messages",
        PeekView::Deferred => "deferred messages",
        PeekView::DeadLetter => "dead-letter queue",
        PeekView::TransferDeadLetter => "transfer dead-letter queue",
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn messages(n: u64) -> String {
    format!(
        "{} {}",
        thousands(n),
        if n == 1 { "message" } else { "messages" }
    )
}

/// `{entity-slug}-{view}-{yyyyMMdd-HHmm}.{ext}`; anything but letters, digits, `.`, `_`, `-` becomes `-`.
pub fn suggested_file_name(
    entity: &str,
    view: PeekView,
    format: &str,
    now: time::OffsetDateTime,
) -> String {
    let slug: String = entity
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let ext = if format.eq_ignore_ascii_case("ndjson") {
        "ndjson"
    } else {
        "json"
    };
    format!(
        "{slug}-{}-{:04}{:02}{:02}-{:02}{:02}.{ext}",
        view.as_str(),
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute()
    )
}

/// Validate a request and open the export file at `path`, which the save dialog returned. A file
/// already there is replaced when the export finishes: the dialog asked "replace?" for it.
pub fn plan(req: ExportRequest, path: PathBuf) -> Result<ExportPlan, CommandError> {
    let entity = EntityPath::parse(&req.entity)?;
    let format = ExportFormat::parse(&req.format)?;
    if !path.is_absolute() {
        return Err(CommandError::new(
            "invalid",
            "Choose where to save the export.",
        ));
    }
    let scope = match req.scope {
        ExportScope::All {
            filter,
            from_sequence,
            max,
        } => Scope::All {
            filter: filter.build()?,
            from: from_sequence.max(0),
            max,
        },
        ExportScope::Selected { sequences } if sequences.is_empty() => {
            return Err(CommandError::new(
                "invalid",
                "Select the messages to export.",
            ));
        }
        // Each message once, so the job's total is the number of distinct messages asked for.
        ExportScope::Selected { mut sequences } => {
            sequences.sort_unstable();
            sequences.dedup();
            Scope::Selected(sequences)
        }
    };
    let total = match &scope {
        Scope::Selected(s) => Some(s.len() as u64),
        Scope::All { max, .. } => match (*max, req.total_hint) {
            (Some(m), Some(h)) => Some(m.min(h)),
            (m, h) => m.or(h),
        },
    };
    let file = FileExport::create(&path, format, true)?;
    let title = format!("Export {entity} ({})", view_label(req.view));
    let file_name = file_name(&path);
    Ok(ExportPlan {
        entity,
        view: req.view,
        scope,
        file,
        path,
        title,
        file_name,
        total,
    })
}

fn cancelled(e: Error) -> CommandError {
    match e {
        Error::Cancelled => CommandError::new("cancelled", "Export cancelled."),
        other => other.into(),
    }
}

/// Write the export, reporting progress after every page.
pub async fn run(
    plan: ExportPlan,
    source: &dyn ExportSource,
    cancel: &CancellationToken,
    ev: &dyn Events,
    id: &str,
) -> Result<ExportSummary, CommandError> {
    let ExportPlan {
        entity,
        view,
        scope,
        mut file,
        path,
        total,
        ..
    } = plan;
    let mut note = String::new();
    let mut missing = 0u64;
    match scope {
        Scope::All { filter, from, max } => {
            let fetch = |from: i64, n: u32| {
                let opts = PeekOptions::new(n)
                    .from_sequence(from)
                    .scan_limit(SCAN_PER_CALL)
                    .filter(filter.clone());
                let entity = &entity;
                async move { source.page(entity, view, &opts).await }
            };
            let streamed = stream_pages(max, from, Some(cancel), fetch, |page, scanned| {
                file.write(&page)?;
                ev.job(JobEvent::Progress {
                    id: id.to_string(),
                    done: file.written(),
                    total,
                    scanned,
                });
                Ok(())
            })
            .await
            .map_err(cancelled)?;
            if streamed.end == Continuation::Stalled {
                note =
                    "; the scan stopped early, so the end of the entity was not confirmed".into();
            }
        }
        Scope::Selected(sequences) => {
            let found = source
                .sequences(&entity, view, &sequences, SELECTED_BUDGET, cancel)
                .await
                .map_err(cancelled)?;
            if cancel.is_cancelled() {
                return Err(CommandError::new("cancelled", "Export cancelled."));
            }
            file.write(&found.messages)?;
            ev.job(JobEvent::Progress {
                id: id.to_string(),
                done: file.written(),
                total,
                scanned: found.scanned,
            });
            missing = found.missing.len() as u64;
            match found.missing.len() {
                0 => {}
                1 => note = "; 1 selected message was no longer there".into(),
                n => note = format!("; {n} selected messages were no longer there"),
            }
        }
    }
    // A cancel that arrived during the last read still wins: no file is put in place.
    if cancel.is_cancelled() {
        return Err(CommandError::new("cancelled", "Export cancelled."));
    }
    let written = file.finish()?;
    Ok(ExportSummary {
        written,
        missing,
        summary: format!(
            "Exported {} to {}{note}",
            messages(written),
            file_name(&path)
        ),
        path,
    })
}

/// Remember a path the save dialog returned and describe it for display.
pub fn remember_pick(jobs: &Jobs, path: PathBuf) -> PickedPath {
    let name = file_name(&path);
    let dir = path
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    PickedPath {
        pick: jobs.pick(path),
        name,
        dir,
    }
}

/// Validate, then run the export in the background. Returns at once with the job id.
pub fn start(
    jobs: &Arc<Jobs>,
    source: Arc<dyn ExportSource>,
    ev: Arc<dyn Events>,
    req: ExportRequest,
) -> Result<JobStarted, CommandError> {
    let pick = req.pick.clone();
    let path = jobs
        .take_pick(&pick)
        .ok_or_else(|| CommandError::new("invalid", "Choose where to save the export."))?;
    // Spent only when the export starts: a request refused here (a filter that does not
    // compile) can be fixed and sent again with the same pick.
    let plan = match plan(req, path.clone()) {
        Ok(plan) => plan,
        Err(e) => {
            jobs.restore_pick(&pick, path);
            return Err(e);
        }
    };
    let (id, cancel) = jobs.start();
    ev.job(JobEvent::Started {
        id: id.clone(),
        title: plan.title.clone(),
        total: plan.total,
        file_name: Some(plan.file_name.clone()),
    });
    let (jobs, job_id) = (jobs.clone(), id.clone());
    tokio::spawn(async move {
        // The activity log names the file; the job row has it on its own line.
        let failed = format!("{} to {} failed", plan.title, plan.file_name);
        let result = tracked(
            &*ev,
            &failed,
            run(plan, &*source, &cancel, &*ev, &job_id),
            |s| s.summary.clone(),
        )
        .await;
        match result {
            Ok(s) => {
                jobs.finish(&job_id, Some(s.path.clone()));
                ev.job(JobEvent::Finished {
                    id: job_id,
                    error: None,
                    summary: Some(s.summary),
                    output: Some(s.path.display().to_string()),
                    written: Some(s.written),
                    missing: Some(s.missing),
                    file_name: Some(file_name(&s.path)),
                });
            }
            Err(e) => {
                jobs.finish(&job_id, None);
                ev.job(JobEvent::Finished {
                    id: job_id,
                    error: Some(ActivityError {
                        kind: e.kind.to_string(),
                        message: e.message,
                    }),
                    summary: None,
                    output: None,
                    written: None,
                    missing: None,
                    file_name: None,
                });
            }
        }
    });
    Ok(JobStarted { id })
}

/// Explorer's one argument that selects `file`: `/select,"<path>"`. Explorer parses its own command
/// line, so the quoted path must reach it as typed (see `reveal`), not re-escaped by the runtime.
/// The path is shown the way Explorer reads it: backslashes, and no verbatim (`\\?\`) prefix, which
/// Explorer doesn't understand (the save dialog returns plain paths; this guards against any other).
pub fn explorer_select_arg(file: &Path) -> OsString {
    let mut arg = OsString::from("/select,\"");
    match file.to_str() {
        Some(text) => {
            let plain = match text.strip_prefix(r"\\?\UNC\") {
                Some(rest) => format!(r"\\{rest}"),
                None => text.strip_prefix(r"\\?\").unwrap_or(text).to_owned(),
            };
            arg.push(plain.replace('/', "\\"));
        }
        // Not valid Unicode (possible on Windows): as it is.
        None => arg.push(file),
    }
    arg.push("\"");
    arg
}

/// Explorer from the Windows directory (`%WINDIR%`), so a program of that name earlier on the search
/// path or in the current directory is never run. Without it, the bare name.
#[cfg(any(windows, test))]
pub fn explorer_path(windir: Option<OsString>) -> PathBuf {
    match windir.filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("explorer.exe"),
        None => PathBuf::from("explorer.exe"),
    }
}

/// The program and arguments that show `file` in the platform's file manager: selected in
/// Finder (`open -R`) or Explorer (`/select,`); elsewhere its folder is opened.
pub fn reveal_command(file: &Path, os: &str) -> (&'static str, Vec<OsString>) {
    match os {
        "macos" => ("open", vec!["-R".into(), file.into()]),
        "windows" => ("explorer.exe", vec![explorer_select_arg(file)]),
        _ => (
            "xdg-open",
            vec![file.parent().unwrap_or(file).as_os_str().to_owned()],
        ),
    }
}

/// The process that shows the file. On Windows the single `/select,"<path>"` argument goes in raw, from
/// Explorer in `%WINDIR%`; elsewhere the arguments are ordinary.
#[cfg(windows)]
fn reveal_process(_program: &str, args: Vec<OsString>) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new(explorer_path(std::env::var_os("WINDIR")));
    for arg in args {
        cmd.raw_arg(arg);
    }
    cmd
}

#[cfg(not(windows))]
fn reveal_process(program: &str, args: Vec<OsString>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd
}

/// Show a finished export's file in the file manager. Only files this app wrote can be shown.
pub fn reveal(jobs: &Jobs, id: &str) -> Result<(), CommandError> {
    let file = jobs
        .output(id)
        .ok_or_else(|| CommandError::new("not_found", "That export has no file to show."))?;
    if !file.is_file() {
        return Err(CommandError::new(
            "not_found",
            "The exported file is no longer there. It was moved or deleted.",
        ));
    }
    let os = std::env::consts::OS;
    let (program, args) = reveal_command(&file, os);
    let status = reveal_process(program, args)
        .status()
        .map_err(|e| CommandError::new("io", format!("Couldn't show the file: {e}")))?;
    // Explorer exits with 1 even when it showed the file.
    if !status.success() && os != "windows" {
        return Err(CommandError::new(
            "io",
            format!("Couldn't show the file: {program} exited with {status}."),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use dispatched_core::{MessageBody, MessageState, PeekPage, PeekedMessage};
    use std::path::Path;
    use std::sync::Mutex;

    fn msg(seq: i64) -> PeekedMessage {
        PeekedMessage {
            sequence_number: seq,
            state: MessageState::Active,
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
            time_to_live_ms: None,
            dead_letter_reason: None,
            dead_letter_description: None,
            dead_letter_source: None,
            application_properties: Default::default(),
            body: MessageBody::Text(format!("secret-body-{seq}")),
        }
    }

    /// Pages of 2 over messages 1..=n; `sequences` finds what exists.
    struct Fake {
        n: i64,
        calls: Mutex<u32>,
        cancel_after: Option<(u32, CancellationToken)>,
    }

    fn fake(n: i64) -> Fake {
        Fake {
            n,
            calls: Mutex::new(0),
            cancel_after: None,
        }
    }

    #[async_trait::async_trait]
    impl ExportSource for Fake {
        async fn page(
            &self,
            _e: &EntityPath,
            _v: PeekView,
            opts: &PeekOptions,
        ) -> dispatched_core::Result<PeekPage> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if let Some((after, token)) = &self.cancel_after
                && *calls >= *after
            {
                token.cancel();
            }
            let from = opts.from_sequence.max(1);
            let messages: Vec<PeekedMessage> = (from..=self.n)
                .take(2.min(opts.max as usize))
                .map(msg)
                .collect();
            let last = messages.last().map(|m| m.sequence_number).unwrap_or(from);
            let exhausted = last >= self.n;
            Ok(PeekPage {
                scanned: messages.len() as u32,
                next_sequence: (!exhausted).then_some(last + 1),
                exhausted,
                messages,
            })
        }
        async fn sequences(
            &self,
            _e: &EntityPath,
            _v: PeekView,
            seqs: &[i64],
            _b: u64,
            c: &CancellationToken,
        ) -> dispatched_core::Result<SequenceCollection> {
            if let Some((_, token)) = &self.cancel_after {
                token.cancel();
            }
            let _ = c;
            let (found, missing): (Vec<i64>, Vec<i64>) = seqs.iter().partition(|s| **s <= self.n);
            Ok(SequenceCollection {
                messages: found.into_iter().map(msg).collect(),
                missing,
                scanned: 0,
            })
        }
    }

    fn all() -> ExportScope {
        ExportScope::All {
            filter: Default::default(),
            from_sequence: 0,
            max: None,
        }
    }

    fn request(pick: &str, scope: ExportScope) -> ExportRequest {
        ExportRequest {
            entity: "orders".into(),
            view: PeekView::DeadLetter,
            scope,
            format: "json".into(),
            pick: pick.into(),
            total_hint: Some(5),
        }
    }

    /// Plan an export of `scope` to `dir/out.json`, as if the dialog had returned it.
    fn plan_in(dir: &Path, scope: ExportScope) -> Result<ExportPlan, CommandError> {
        plan(request("pick-x", scope), dir.join("out.json"))
    }

    #[tokio::test]
    async fn exports_every_page_with_progress_and_a_summary() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let plan = plan_in(dir.path(), all()).unwrap();
        let s = run(plan, &fake(5), &CancellationToken::new(), &ev, "j1")
            .await
            .unwrap();
        assert_eq!(s.written, 5);
        assert_eq!(s.summary, "Exported 5 messages to out.json");
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("out.json")).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 5);
        let jobs = ev.jobs.lock().unwrap();
        let done: Vec<(u64, u64)> = jobs
            .iter()
            .filter_map(|j| match j {
                JobEvent::Progress { done, scanned, .. } => Some((*done, *scanned)),
                _ => None,
            })
            .collect();
        // Written and examined so far: the fake reads two messages a page.
        assert_eq!(done, [(2, 2), (4, 4), (5, 5)]);
        assert_eq!(s.missing, 0);
    }

    #[tokio::test]
    async fn cancel_stops_the_export_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let token = CancellationToken::new();
        let plan = plan_in(dir.path(), all()).unwrap();
        let fake = Fake {
            n: 100,
            calls: Mutex::new(0),
            cancel_after: Some((2, token.clone())),
        };
        let e = run(plan, &fake, &token, &ev, "j1").await.unwrap_err();
        assert_eq!(e.kind, "cancelled");
        assert_eq!(e.message, "Export cancelled.");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "temp file removed"
        );
    }

    #[tokio::test]
    async fn a_cancel_during_the_last_page_still_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let token = CancellationToken::new();
        // Three messages are two pages; the cancel arrives while the second (last) one is read.
        let fake = Fake {
            n: 3,
            calls: Mutex::new(0),
            cancel_after: Some((2, token.clone())),
        };
        let e = run(
            plan_in(dir.path(), all()).unwrap(),
            &fake,
            &token,
            &ev,
            "j1",
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            ("cancelled", "Export cancelled.")
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        // The same for selected messages.
        let token = CancellationToken::new();
        let fake = Fake {
            n: 3,
            calls: Mutex::new(0),
            cancel_after: Some((1, token.clone())),
        };
        let plan = plan_in(
            dir.path(),
            ExportScope::Selected {
                sequences: vec![1, 2],
            },
        )
        .unwrap();
        assert_eq!(
            run(plan, &fake, &token, &ev, "j2").await.unwrap_err().kind,
            "cancelled"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn selected_export_summary_counts_missing_messages() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let plan = plan_in(
            dir.path(),
            ExportScope::Selected {
                sequences: vec![1, 2, 9],
            },
        )
        .unwrap();
        let s = run(plan, &fake(5), &CancellationToken::new(), &ev, "j1")
            .await
            .unwrap();
        assert_eq!((s.written, s.missing), (2, 1));
        assert_eq!(
            s.summary,
            "Exported 2 messages to out.json; 1 selected message was no longer there"
        );
    }

    #[test]
    fn plan_refuses_bad_input_before_any_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = request("p", all());
        r.format = "csv".into();
        assert_eq!(
            plan(r, dir.path().join("out.json")).unwrap_err().kind,
            "invalid"
        );
        assert_eq!(
            plan(request("p", all()), "relative.json".into())
                .unwrap_err()
                .kind,
            "invalid"
        );
        assert_eq!(
            plan_in(dir.path(), ExportScope::Selected { sequences: vec![] })
                .unwrap_err()
                .kind,
            "invalid"
        );
        let e = plan(request("p", all()), dir.path().to_path_buf()).unwrap_err();
        assert!(e.message.contains("is a directory"), "{}", e.message);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "nothing created"
        );
    }

    #[tokio::test]
    async fn a_picked_file_that_exists_is_replaced() {
        // The save dialog asked "replace?" for it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("out.json"), "old").unwrap();
        let s = run(
            plan_in(dir.path(), all()).unwrap(),
            &fake(1),
            &CancellationToken::new(),
            &Recorder::default(),
            "j1",
        )
        .await
        .unwrap();
        assert_eq!(s.written, 1);
        assert!(
            std::fs::read_to_string(dir.path().join("out.json"))
                .unwrap()
                .contains("dispatched.message/v1")
        );
    }

    #[test]
    fn a_request_cannot_name_a_path() {
        let base = serde_json::json!({"entity": "orders", "view": "dead-letter", "scope": {"kind": "all"}, "format": "json"});
        let mut with_pick = base.clone();
        with_pick["pick"] = "pick-1".into();
        let r: ExportRequest = serde_json::from_value(with_pick.clone()).unwrap();
        assert_eq!((r.pick.as_str(), r.total_hint), ("pick-1", None));
        let mut with_path = with_pick.clone();
        with_path["path"] = "/etc/passwd".into();
        assert!(
            serde_json::from_value::<ExportRequest>(with_path).is_err(),
            "a path is refused"
        );
        let mut with_overwrite = with_pick;
        with_overwrite["overwrite"] = true.into();
        assert!(serde_json::from_value::<ExportRequest>(with_overwrite).is_err());
        assert!(
            serde_json::from_value::<ExportRequest>(base).is_err(),
            "a pick is required"
        );
    }

    #[tokio::test]
    async fn export_events_name_the_file_not_the_messages() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        run(
            plan_in(dir.path(), all()).unwrap(),
            &fake(3),
            &CancellationToken::new(),
            &ev,
            "j1",
        )
        .await
        .unwrap();
        let events = serde_json::to_string(&*ev.jobs.lock().unwrap()).unwrap();
        assert!(!events.contains("secret-body"), "{events}");
    }

    #[test]
    fn suggested_names_are_safe_and_dated() {
        let at = time::macros::datetime!(2026-10-16 09:41 UTC);
        assert_eq!(
            suggested_file_name(
                "events/subscriptions/audit",
                PeekView::DeadLetter,
                "json",
                at
            ),
            "events-subscriptions-audit-dead-letter-20261016-0941.json"
        );
        assert_eq!(
            suggested_file_name("a b:c", PeekView::Active, "ndjson", at),
            "a-b-c-active-20261016-0941.ndjson"
        );
    }

    #[test]
    fn counts_are_grouped_and_singular() {
        assert_eq!(messages(1), "1 message");
        assert_eq!(messages(1_284), "1,284 messages");
        assert_eq!(messages(1_000_000), "1,000,000 messages");
    }

    #[test]
    fn selected_totals_count_each_sequence_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan_in(
            dir.path(),
            ExportScope::Selected {
                sequences: vec![3, 1, 3, 2, 1],
            },
        )
        .unwrap();
        assert_eq!(p.total, Some(3));
        assert!(matches!(&p.scope, Scope::Selected(s) if s == &[1, 2, 3]));
    }

    #[test]
    fn a_pick_describes_the_file_for_display() {
        let jobs = Jobs::default();
        let p = remember_pick(&jobs, PathBuf::from("/data/exports/out.json"));
        assert!(p.pick.starts_with("pick-"));
        assert_eq!(
            (p.name.as_str(), p.dir.as_str()),
            ("out.json", "/data/exports")
        );
        assert_eq!(
            serde_json::to_value(&p).unwrap().as_object().unwrap().len(),
            3
        );
    }

    #[test]
    fn reveal_selects_the_file_on_each_platform() {
        let f = Path::new("/data/exports/out.json");
        let (prog, args) = reveal_command(f, "macos");
        assert_eq!((prog, args), ("open", vec![OsString::from("-R"), f.into()]));
        let (prog, args) = reveal_command(f, "windows");
        assert_eq!(
            (prog, args),
            (
                "explorer.exe",
                vec![OsString::from("/select,\"\\data\\exports\\out.json\"")]
            )
        );
        let (prog, args) = reveal_command(f, "linux");
        assert_eq!(
            (prog, args),
            ("xdg-open", vec![OsString::from("/data/exports")])
        );
    }

    #[test]
    fn explorer_gets_one_raw_select_argument_with_the_path_quoted() {
        // Explorer parses its own command line: `/select,` and the quoted path are one argument, so a
        // path with spaces or commas stays whole. Built here on every platform, passed raw on Windows.
        let f = Path::new(r"C:\Users\Ana Maria\My exports, final\out.json");
        assert_eq!(
            explorer_select_arg(f),
            OsString::from(r#"/select,"C:\Users\Ana Maria\My exports, final\out.json""#)
        );
    }

    #[test]
    fn explorer_gets_a_plain_backslash_path_even_from_a_verbatim_or_slashed_one() {
        let arg = |p: &str| explorer_select_arg(Path::new(p));
        // The save dialog hands back `C:\...`, but a verbatim (`\\?\`) or forward-slash path must not reach
        // Explorer as is: it doesn't understand the prefix, and `/select,` wants backslashes.
        assert_eq!(
            arg(r"\\?\C:\data\out.json"),
            OsString::from(r#"/select,"C:\data\out.json""#)
        );
        assert_eq!(
            arg(r"\\?\UNC\server\share\out.json"),
            OsString::from(r#"/select,"\\server\share\out.json""#)
        );
        assert_eq!(
            arg("C:/data/sub/out.json"),
            OsString::from(r#"/select,"C:\data\sub\out.json""#)
        );
    }

    #[test]
    fn explorer_is_resolved_from_the_windows_directory() {
        assert_eq!(
            explorer_path(Some(OsString::from("C:\\Windows"))),
            Path::new("C:\\Windows").join("explorer.exe")
        );
        // Without %WINDIR%, the bare name is left to the system's own search.
        assert_eq!(explorer_path(None), PathBuf::from("explorer.exe"));
        assert_eq!(
            explorer_path(Some(OsString::new())),
            PathBuf::from("explorer.exe")
        );
    }

    #[test]
    fn reveal_needs_a_file_this_app_wrote() {
        let jobs = Jobs::default();
        assert_eq!(reveal(&jobs, "job-nope").unwrap_err().kind, "not_found");
        let (id, _) = jobs.start();
        jobs.finish(&id, Some("/nowhere/gone.json".into()));
        assert_eq!(
            reveal(&jobs, &id).unwrap_err().kind,
            "not_found",
            "the file was moved or deleted"
        );
    }

    /// Pages that take a moment each, so a job can be cancelled while it runs.
    struct Slow(i64);

    #[async_trait::async_trait]
    impl ExportSource for Slow {
        async fn page(
            &self,
            e: &EntityPath,
            v: PeekView,
            opts: &PeekOptions,
        ) -> dispatched_core::Result<PeekPage> {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            fake(self.0).page(e, v, opts).await
        }
        async fn sequences(
            &self,
            _e: &EntityPath,
            _v: PeekView,
            _s: &[i64],
            _b: u64,
            _c: &CancellationToken,
        ) -> dispatched_core::Result<SequenceCollection> {
            unreachable!()
        }
    }

    async fn finished(ev: &Recorder) -> JobEvent {
        for _ in 0..400 {
            if let Some(f) = ev
                .jobs
                .lock()
                .unwrap()
                .iter()
                .find(|j| matches!(j, JobEvent::Finished { .. }))
            {
                return f.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("the job never finished");
    }

    #[tokio::test]
    async fn a_started_job_writes_the_picked_file_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Arc::new(Recorder::default());
        let jobs = Arc::new(Jobs::default());
        let target = dir.path().join("out.json");
        let pick = remember_pick(&jobs, target.clone()).pick;
        let started = start(&jobs, Arc::new(Slow(3)), ev.clone(), request(&pick, all())).unwrap();
        assert_eq!(
            finished(&ev).await,
            JobEvent::Finished {
                id: started.id.clone(),
                error: None,
                summary: Some("Exported 3 messages to out.json".into()),
                output: Some(target.display().to_string()),
                written: Some(3),
                missing: Some(0),
                file_name: Some("out.json".into()),
            }
        );
        assert!(
            matches!(&ev.jobs.lock().unwrap()[0], JobEvent::Started { id, title, total: Some(5), file_name }
            if *id == started.id && title == "Export orders (dead-letter queue)"
                && file_name.as_deref() == Some("out.json"))
        );
        assert_eq!(jobs.output(&started.id), Some(target));
        assert!(!jobs.cancel(&started.id));
        let log = ev.activity.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].action, "Exported 3 messages to out.json");
    }

    #[tokio::test]
    async fn a_pick_is_spent_by_one_export_and_kept_when_the_request_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Arc::new(Recorder::default());
        let jobs = Arc::new(Jobs::default());
        let unknown = start(
            &jobs,
            Arc::new(Slow(1)),
            ev.clone(),
            request("pick-unknown", all()),
        )
        .unwrap_err();
        assert_eq!(
            (unknown.kind, unknown.message.as_str()),
            ("invalid", "Choose where to save the export.")
        );
        let pick = remember_pick(&jobs, dir.path().join("out.json")).pick;
        // A filter that does not compile: refused, and the pick can be used again once it is fixed.
        let bad = ExportScope::All {
            filter: dispatched_core::FilterSpec {
                property: Some("= = (".into()),
                ..Default::default()
            },
            from_sequence: 0,
            max: None,
        };
        assert_eq!(
            start(&jobs, Arc::new(Slow(1)), ev.clone(), request(&pick, bad))
                .unwrap_err()
                .kind,
            "invalid"
        );
        assert!(
            ev.jobs.lock().unwrap().is_empty(),
            "no job for a refused request"
        );
        start(&jobs, Arc::new(Slow(1)), ev.clone(), request(&pick, all())).unwrap();
        finished(&ev).await;
        let again = start(&jobs, Arc::new(Slow(1)), ev.clone(), request(&pick, all())).unwrap_err();
        assert_eq!(
            again.message, "Choose where to save the export.",
            "a pick authorises one export"
        );
    }

    #[tokio::test]
    async fn cancelling_a_running_job_finishes_it_as_cancelled_with_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Arc::new(Recorder::default());
        let jobs = Arc::new(Jobs::default());
        let pick = remember_pick(&jobs, dir.path().join("out.json")).pick;
        let started = start(
            &jobs,
            Arc::new(Slow(100_000)),
            ev.clone(),
            request(&pick, all()),
        )
        .unwrap();
        // Cancel once the first page is written, so the job is mid-export.
        let running = || {
            ev.jobs
                .lock()
                .unwrap()
                .iter()
                .any(|j| matches!(j, JobEvent::Progress { .. }))
        };
        for _ in 0..400 {
            if running() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(running(), "the job never wrote a page");
        assert!(jobs.cancel(&started.id));
        let JobEvent::Finished {
            error,
            summary,
            output,
            written,
            missing,
            file_name,
            ..
        } = finished(&ev).await
        else {
            unreachable!()
        };
        assert_eq!(
            error.map(|e| (e.kind, e.message)),
            Some(("cancelled".into(), "Export cancelled.".into()))
        );
        assert_eq!((summary, output), (None, None));
        assert_eq!((written, missing, file_name), (None, None, None));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "temp file removed"
        );
        assert!(jobs.output(&started.id).is_none());
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Export cancelled");
        assert!(log[0].error.is_none());
    }
}
