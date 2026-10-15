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
use std::path::PathBuf;
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

#[derive(Debug, Clone, Deserialize)]
pub struct ExportRequest {
    pub entity: String,
    pub view: PeekView,
    pub scope: ExportScope,
    pub format: String,
    pub path: String,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub total_hint: Option<u64>,
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
    pub title: String,
    pub total: Option<u64>,
}

#[derive(Debug)]
pub struct ExportSummary {
    /// Messages written. The UI gets the count through `summary`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub written: u64,
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

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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

/// Validate a request and open the export file (fails early when it exists and may not be replaced).
pub fn plan(req: ExportRequest) -> Result<ExportPlan, CommandError> {
    let entity = EntityPath::parse(&req.entity)?;
    let format = ExportFormat::parse(&req.format)?;
    let path = PathBuf::from(&req.path);
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
    let file = FileExport::create(&path, format, req.overwrite)?;
    let title = format!(
        "Export {entity} ({}) to {}",
        req.view.as_str(),
        file_name(&path)
    );
    Ok(ExportPlan {
        entity,
        view: req.view,
        scope,
        file,
        path,
        title,
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
            let streamed = stream_pages(max, from, Some(cancel), fetch, |page, _| {
                file.write(&page)?;
                ev.job(JobEvent::Progress {
                    id: id.to_string(),
                    done: file.written(),
                    total,
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
            });
            match found.missing.len() {
                0 => {}
                1 => note = "; 1 selected message was no longer there".into(),
                n => note = format!("; {n} selected messages were no longer there"),
            }
        }
    }
    let written = file.finish()?;
    Ok(ExportSummary {
        written,
        summary: format!(
            "Exported {} to {}{note}",
            messages(written),
            file_name(&path)
        ),
        path,
    })
}

/// `overwrite` comes from the webview, which must not be able to replace any file it names
/// (injected script could). It is honoured only for the exact path the native save dialog
/// returned last, where the user answered "replace?"; every other path is written only when
/// nothing is there. Taking the pick also spends it.
pub fn authorize_overwrite(jobs: &Jobs, req: &mut ExportRequest) {
    let picked = jobs.take_picked();
    req.overwrite = req.overwrite && picked.is_some_and(|p| p == std::path::Path::new(&req.path));
}

/// Validate, then run the export in the background. Returns at once with the job id.
pub fn start(
    jobs: &Arc<Jobs>,
    source: Arc<dyn ExportSource>,
    ev: Arc<dyn Events>,
    mut req: ExportRequest,
) -> Result<JobStarted, CommandError> {
    authorize_overwrite(jobs, &mut req);
    let plan = plan(req)?;
    let (id, cancel) = jobs.start();
    ev.job(JobEvent::Started {
        id: id.clone(),
        title: plan.title.clone(),
        total: plan.total,
    });
    let (jobs, job_id) = (jobs.clone(), id.clone());
    tokio::spawn(async move {
        let failed = format!("{} failed", plan.title);
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
                });
            }
        }
    });
    Ok(JobStarted { id })
}

/// Open the folder holding a finished export's file.
pub fn reveal(jobs: &Jobs, id: &str) -> Result<(), CommandError> {
    let path = jobs
        .output(id)
        .ok_or_else(|| CommandError::new("not_found", "That export has no file to show."))?;
    let folder = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or(path);
    open::that(&folder)
        .map_err(|e| CommandError::new("io", format!("Couldn't open the folder: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use dispatched_core::{MessageBody, MessageState, PeekPage, PeekedMessage};
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
            _c: &CancellationToken,
        ) -> dispatched_core::Result<SequenceCollection> {
            let (found, missing): (Vec<i64>, Vec<i64>) = seqs.iter().partition(|s| **s <= self.n);
            Ok(SequenceCollection {
                messages: found.into_iter().map(msg).collect(),
                missing,
                scanned: 0,
            })
        }
    }

    fn request(dir: &std::path::Path, scope: ExportScope) -> ExportRequest {
        ExportRequest {
            entity: "orders".into(),
            view: PeekView::DeadLetter,
            scope,
            format: "json".into(),
            path: dir.join("out.json").to_string_lossy().into_owned(),
            overwrite: false,
            total_hint: Some(5),
        }
    }

    #[tokio::test]
    async fn exports_every_page_with_progress_and_a_summary() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let plan = plan(request(
            dir.path(),
            ExportScope::All {
                filter: Default::default(),
                from_sequence: 0,
                max: None,
            },
        ))
        .unwrap();
        let fake = Fake {
            n: 5,
            calls: Mutex::new(0),
            cancel_after: None,
        };
        let s = run(plan, &fake, &CancellationToken::new(), &ev, "j1")
            .await
            .unwrap();
        assert_eq!(s.written, 5);
        assert_eq!(s.summary, "Exported 5 messages to out.json");
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("out.json")).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 5);
        let jobs = ev.jobs.lock().unwrap();
        let done: Vec<u64> = jobs
            .iter()
            .filter_map(|j| match j {
                JobEvent::Progress { done, .. } => Some(*done),
                _ => None,
            })
            .collect();
        assert_eq!(done, [2, 4, 5]);
    }

    #[tokio::test]
    async fn cancel_stops_the_export_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let token = CancellationToken::new();
        let plan = plan(request(
            dir.path(),
            ExportScope::All {
                filter: Default::default(),
                from_sequence: 0,
                max: None,
            },
        ))
        .unwrap();
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
    async fn selected_export_summary_counts_missing_messages() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let plan = plan(request(
            dir.path(),
            ExportScope::Selected {
                sequences: vec![1, 2, 9],
            },
        ))
        .unwrap();
        let fake = Fake {
            n: 5,
            calls: Mutex::new(0),
            cancel_after: None,
        };
        let s = run(plan, &fake, &CancellationToken::new(), &ev, "j1")
            .await
            .unwrap();
        assert_eq!(s.written, 2);
        assert_eq!(
            s.summary,
            "Exported 2 messages to out.json; 1 selected message was no longer there"
        );
    }

    #[test]
    fn plan_refuses_bad_input_before_any_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = request(
            dir.path(),
            ExportScope::All {
                filter: Default::default(),
                from_sequence: 0,
                max: None,
            },
        );
        r.format = "csv".into();
        assert_eq!(plan(r).unwrap_err().kind, "invalid");
        let mut r = request(
            dir.path(),
            ExportScope::All {
                filter: Default::default(),
                from_sequence: 0,
                max: None,
            },
        );
        r.path = "relative.json".into();
        assert_eq!(plan(r).unwrap_err().kind, "invalid");
        let r = request(dir.path(), ExportScope::Selected { sequences: vec![] });
        assert_eq!(plan(r).unwrap_err().kind, "invalid");
        std::fs::write(dir.path().join("out.json"), "x").unwrap();
        let r = request(
            dir.path(),
            ExportScope::All {
                filter: Default::default(),
                from_sequence: 0,
                max: None,
            },
        );
        assert!(plan(r).unwrap_err().message.contains("already exists"));
    }

    #[test]
    fn overwrite_is_allowed_only_for_the_path_the_dialog_returned() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        std::fs::write(&target, "keep").unwrap();
        let all = || ExportScope::All {
            filter: Default::default(),
            from_sequence: 0,
            max: None,
        };
        let asking = || {
            let mut r = request(dir.path(), all());
            r.overwrite = true;
            r
        };
        let jobs = Jobs::default();
        // The webview asks to replace a path the dialog never returned: refused.
        let mut r = asking();
        authorize_overwrite(&jobs, &mut r);
        assert!(!r.overwrite);
        assert!(plan(r).unwrap_err().message.contains("already exists"));
        // Another path than the picked one: refused.
        jobs.set_picked(Some(dir.path().join("other.json")));
        let mut r = asking();
        authorize_overwrite(&jobs, &mut r);
        assert!(!r.overwrite);
        // The picked path itself: allowed, once.
        jobs.set_picked(Some(target.clone()));
        let mut r = asking();
        authorize_overwrite(&jobs, &mut r);
        assert!(r.overwrite);
        let mut again = asking();
        authorize_overwrite(&jobs, &mut again);
        assert!(!again.overwrite, "a pick authorises one export");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    #[tokio::test]
    async fn export_events_name_the_file_not_the_messages() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Recorder::default();
        let plan = plan(request(
            dir.path(),
            ExportScope::All {
                filter: Default::default(),
                from_sequence: 0,
                max: None,
            },
        ))
        .unwrap();
        let fake = Fake {
            n: 3,
            calls: Mutex::new(0),
            cancel_after: None,
        };
        run(plan, &fake, &CancellationToken::new(), &ev, "j1")
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
        let p = plan(request(
            dir.path(),
            ExportScope::Selected {
                sequences: vec![3, 1, 3, 2, 1],
            },
        ))
        .unwrap();
        assert_eq!(p.total, Some(3));
        assert!(matches!(&p.scope, Scope::Selected(s) if s == &[1, 2, 3]));
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
            Fake {
                n: self.0,
                calls: Mutex::new(0),
                cancel_after: None,
            }
            .page(e, v, opts)
            .await
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
    async fn a_started_job_reports_its_file_and_can_be_revealed() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Arc::new(Recorder::default());
        let jobs = Arc::new(Jobs::default());
        let all = ExportScope::All {
            filter: Default::default(),
            from_sequence: 0,
            max: None,
        };
        let started = start(
            &jobs,
            Arc::new(Slow(3)),
            ev.clone(),
            request(dir.path(), all),
        )
        .unwrap();
        let target = dir.path().join("out.json");
        assert_eq!(
            finished(&ev).await,
            JobEvent::Finished {
                id: started.id.clone(),
                error: None,
                summary: Some("Exported 3 messages to out.json".into()),
                output: Some(target.display().to_string()),
            }
        );
        assert!(
            matches!(&ev.jobs.lock().unwrap()[0], JobEvent::Started { id, title, total: Some(5) }
            if *id == started.id && title == "Export orders (dead-letter) to out.json")
        );
        assert_eq!(jobs.output(&started.id), Some(target));
        assert!(!jobs.cancel(&started.id));
        let log = ev.activity.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].action, "Exported 3 messages to out.json");
    }

    #[tokio::test]
    async fn cancelling_a_running_job_finishes_it_as_cancelled_with_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let ev = Arc::new(Recorder::default());
        let jobs = Arc::new(Jobs::default());
        let all = ExportScope::All {
            filter: Default::default(),
            from_sequence: 0,
            max: None,
        };
        let started = start(
            &jobs,
            Arc::new(Slow(100_000)),
            ev.clone(),
            request(dir.path(), all),
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

    #[tokio::test]
    async fn a_job_that_cannot_start_emits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("out.json"), "keep").unwrap();
        let ev = Arc::new(Recorder::default());
        let jobs = Arc::new(Jobs::default());
        let mut r = request(dir.path(), ExportScope::Selected { sequences: vec![1] });
        r.overwrite = true;
        let e = start(&jobs, Arc::new(Slow(3)), ev.clone(), r).unwrap_err();
        assert!(e.message.contains("already exists"), "{}", e.message);
        assert!(ev.jobs.lock().unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.json")).unwrap(),
            "keep"
        );
    }
}
