//! Event channels from Rust to the UI: activity log, jobs, sign-in prompts.
//! Payloads are built from names, counts and error texts only, never from
//! secrets or message bodies.

use crate::error::CommandError;
use serde::Serialize;
use std::future::Future;
use std::time::Instant;

pub const ACTIVITY: &str = "activity";
pub const JOB: &str = "job";
pub const SIGN_IN_PROMPT: &str = "sign-in-prompt";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivityError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivityEntry {
    /// RFC 3339, UTC.
    pub at: String,
    pub action: String,
    pub duration_ms: u64,
    pub error: Option<ActivityError>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum JobEvent {
    Started {
        id: String,
        /// What the job is, without where its file goes: `Export orders (dead-letter queue)`.
        title: String,
        total: Option<u64>,
        /// The file the job writes, by name.
        file_name: Option<String>,
    },
    Progress {
        id: String,
        done: u64,
        total: Option<u64>,
        /// Messages examined so far. Past `done` when a filter skips messages.
        scanned: u64,
    },
    Finished {
        id: String,
        error: Option<ActivityError>,
        /// What was done, e.g. `Exported 1,284 messages to orders-dead-letter-20261016-0941.json`:
        /// the activity log's line. The UI composes its own from the fields below.
        summary: Option<String>,
        /// The file written, in full.
        output: Option<String>,
        /// Messages written; null on failure.
        written: Option<u64>,
        /// Selected messages that were no longer there; null on failure.
        missing: Option<u64>,
        /// The file written, by name; null on failure.
        file_name: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SignInPrompt {
    /// `browser` or `device-code`.
    pub kind: &'static str,
    pub url: String,
    pub user_code: Option<String>,
    pub message: String,
}

pub trait Events: Send + Sync {
    fn activity(&self, e: ActivityEntry);
    fn job(&self, e: JobEvent);
    fn sign_in_prompt(&self, p: SignInPrompt);
}

/// Emits to every window.
pub struct TauriEvents(pub tauri::AppHandle);

impl Events for TauriEvents {
    fn activity(&self, e: ActivityEntry) {
        use tauri::Emitter as _;
        let _ = self.0.emit(ACTIVITY, e);
    }
    fn job(&self, e: JobEvent) {
        use tauri::Emitter as _;
        let _ = self.0.emit(JOB, e);
    }
    fn sign_in_prompt(&self, p: SignInPrompt) {
        use tauri::Emitter as _;
        let _ = self.0.emit(SIGN_IN_PROMPT, p);
    }
}

/// Collects events (tests).
#[cfg(test)]
#[derive(Default)]
pub struct Recorder {
    pub activity: std::sync::Mutex<Vec<ActivityEntry>>,
    pub jobs: std::sync::Mutex<Vec<JobEvent>>,
    pub prompts: std::sync::Mutex<Vec<SignInPrompt>>,
}

#[cfg(test)]
impl Events for Recorder {
    fn activity(&self, e: ActivityEntry) {
        self.activity.lock().unwrap().push(e);
    }
    fn job(&self, e: JobEvent) {
        self.jobs.lock().unwrap().push(e);
    }
    fn sign_in_prompt(&self, p: SignInPrompt) {
        self.prompts.lock().unwrap().push(p);
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// Run `work` and record one activity entry: `describe(&value)` on success,
/// `failed` plus the error on failure. A cancelled operation is the user's choice,
/// not a failure: it is recorded as its message ("Sign-in cancelled") with no error.
/// A cancel by the user (`cancelled`) or a stop because the connection closed (`stopped`): the
/// operation ended as asked, so it is logged as its message, not as a failure.
fn is_stop(e: &CommandError) -> bool {
    matches!(e.kind, "cancelled" | "stopped")
}

pub async fn tracked<T>(
    ev: &dyn Events,
    failed: &str,
    work: impl Future<Output = Result<T, CommandError>>,
    describe: impl FnOnce(&T) -> String,
) -> Result<T, CommandError> {
    let started = Instant::now();
    let result = work.await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let (action, error) = match &result {
        Ok(v) => (describe(v), None),
        // Cancelled by the user, or stopped because the connection closed: not failures.
        Err(e) if is_stop(e) => (e.message.trim_end_matches('.').to_string(), None),
        Err(e) => (
            failed.to_string(),
            Some(ActivityError {
                kind: e.kind.to_string(),
                message: e.message.clone(),
            }),
        ),
    };
    ev.activity(ActivityEntry {
        at: now_rfc3339(),
        action,
        duration_ms,
        error,
    });
    result
}

/// Run `work` and record an activity entry only if it fails (reads that happen on every
/// selection or page would otherwise flood the log). A cancellation is not a failure.
pub async fn tracked_failures<T>(
    ev: &dyn Events,
    failed: &str,
    work: impl Future<Output = Result<T, CommandError>>,
) -> Result<T, CommandError> {
    let started = Instant::now();
    let result = work.await;
    if let Err(e) = &result
        && !is_stop(e)
    {
        ev.activity(ActivityEntry {
            at: now_rfc3339(),
            action: failed.to_string(),
            duration_ms: started.elapsed().as_millis() as u64,
            error: Some(ActivityError {
                kind: e.kind.to_string(),
                message: e.message.clone(),
            }),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CommandError;

    #[tokio::test]
    async fn tracked_records_success_and_failure_with_durations() {
        let ev = Recorder::default();
        let ok: Result<u32, CommandError> = tracked(&ev, "Counting failed", async { Ok(3) }, |n| {
            format!("Counted {n}")
        })
        .await;
        assert_eq!(ok.unwrap(), 3);
        let err: Result<u32, CommandError> = tracked(
            &ev,
            "Counting failed",
            async {
                Err(CommandError::new(
                    "forbidden",
                    "access denied (needs Listen): no",
                ))
            },
            |n| format!("Counted {n}"),
        )
        .await;
        assert!(err.is_err());
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Counted 3");
        assert!(log[0].error.is_none());
        assert_eq!(log[1].action, "Counting failed");
        assert_eq!(log[1].error.as_ref().unwrap().kind, "forbidden");
        assert!(
            time::OffsetDateTime::parse(&log[0].at, &time::format_description::well_known::Rfc3339)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_cancelled_operation_is_recorded_without_an_error() {
        let ev = Recorder::default();
        let r: Result<(), CommandError> = tracked(
            &ev,
            "Sign-in failed",
            async { Err(CommandError::new("cancelled", "Sign-in cancelled.")) },
            |_| String::new(),
        )
        .await;
        assert_eq!(r.unwrap_err().kind, "cancelled");
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Sign-in cancelled");
        assert!(log[0].error.is_none());
    }

    #[tokio::test]
    async fn tracked_failures_logs_only_failures() {
        let ev = Recorder::default();
        let _ = tracked_failures(&ev, "Reading failed", async { Ok::<_, CommandError>(1) }).await;
        assert!(ev.activity.lock().unwrap().is_empty());
        let _ = tracked_failures::<u8>(&ev, "Reading failed", async {
            Err(CommandError::new("network", "reset"))
        })
        .await;
        let _ = tracked_failures::<u8>(&ev, "Reading failed", async {
            Err(CommandError::new("cancelled", "Stopped."))
        })
        .await;
        let log = ev.activity.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].action, "Reading failed");
        assert_eq!(log[0].error.as_ref().unwrap().kind, "network");
    }

    #[test]
    fn events_serialize_in_the_shapes_the_ui_reads() {
        let e = ActivityEntry {
            at: "2026-10-15T09:41:12Z".into(),
            action: "Listed 4 queues and 3 topics".into(),
            duration_ms: 312,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"at":"2026-10-15T09:41:12Z","action":"Listed 4 queues and 3 topics","duration_ms":312,"error":null})
        );
        let s = JobEvent::Started {
            id: "j1".into(),
            title: "Export orders (dead-letter queue)".into(),
            total: None,
            file_name: Some("out.json".into()),
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::json!({"type":"started","id":"j1","title":"Export orders (dead-letter queue)","total":null,"file_name":"out.json"})
        );
        let j = JobEvent::Progress {
            id: "j1".into(),
            done: 5,
            total: Some(10),
            scanned: 1200,
        };
        assert_eq!(
            serde_json::to_value(&j).unwrap(),
            serde_json::json!({"type":"progress","id":"j1","done":5,"total":10,"scanned":1200})
        );
        let f = JobEvent::Finished {
            id: "j1".into(),
            error: None,
            summary: Some("Exported 5 messages to out.json".into()),
            output: Some("/tmp/out.json".into()),
            written: Some(5),
            missing: Some(2),
            file_name: Some("out.json".into()),
        };
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            serde_json::json!({"type":"finished","id":"j1","error":null,"summary":"Exported 5 messages to out.json","output":"/tmp/out.json","written":5,"missing":2,"file_name":"out.json"})
        );
        let p = SignInPrompt {
            kind: "device-code",
            url: "https://microsoft.com/devicelogin".into(),
            user_code: Some("ABCD-EFGH".into()),
            message: "Go to ...".into(),
        };
        assert_eq!(serde_json::to_value(&p).unwrap()["user_code"], "ABCD-EFGH");
    }
}
