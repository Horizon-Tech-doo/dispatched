//! Running and finished jobs: their cancel tokens and output files.

use horizon_azure_kit::safety::CancellationToken;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long an unspent pick stays valid.
const PICK_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
struct Entry {
    cancel: Option<CancellationToken>,
    output: Option<PathBuf>,
    /// Stopped because its connection closed (disconnect, another connect, sign-out), not by the user.
    stopped_by_close: bool,
    /// An operate job's report, once it finished.
    report: Option<std::sync::Arc<crate::ops::operate::StoredReport>>,
}

#[derive(Default)]
pub struct Jobs {
    inner: Mutex<HashMap<String, Entry>>,
    /// Paths the native save dialog returned, by one-time pick id, with when they were picked.
    /// Exports write only to these: the webview names a pick, never a path, and the dialog has
    /// asked "replace?" for each. At most one is unspent, and only for [`PICK_TTL`].
    picks: Mutex<HashMap<String, (PathBuf, Instant)>>,
    /// Confirmations the jobs wait for: cancelling a job, or closing its connection, answers
    /// them "no".
    confirms: std::sync::Arc<crate::safety::Confirms>,
    /// Purge plans counted for a confirmation, by plan id, with the profile they were counted
    /// on and when. Each runs once.
    plans: Mutex<HashMap<String, PlannedPurge>>,
    /// Files the native open dialog returned, by one-time pick id: bodies, templates and import
    /// files are read only from these, never from a path the webview names.
    opened: Mutex<HashMap<String, (Vec<PathBuf>, Instant)>>,
}

/// Purge plans waiting to run.
pub struct PlannedPurge {
    pub profile_id: String,
    pub plans: Vec<dispatched_core::ops::PurgePlan>,
    pub at: Instant,
}

/// How long counted purge plans stay valid: counts go stale.
const PLAN_TTL: Duration = Duration::from_secs(10 * 60);

impl Jobs {
    /// Remember a path the save dialog returned. Returns its pick id. It replaces any unspent
    /// pick: one export sheet is open at a time, so an older pick is from a sheet that was closed
    /// and must not stay a valid write authorisation.
    pub fn pick(&self, path: PathBuf) -> String {
        let id = format!("pick-{}", uuid::Uuid::new_v4().simple());
        let mut picks = self.picks.lock().unwrap();
        picks.clear();
        picks.insert(id.clone(), (path, Instant::now()));
        id
    }

    /// The picked path, once: a pick authorises one export. `None` once it is older than
    /// [`PICK_TTL`].
    pub fn take_pick(&self, id: &str) -> Option<PathBuf> {
        self.take_pick_at(id, Instant::now())
    }

    /// [`Jobs::take_pick`] as of `now`.
    pub fn take_pick_at(&self, id: &str, now: Instant) -> Option<PathBuf> {
        let (path, at) = self.picks.lock().unwrap().remove(id)?;
        (now.saturating_duration_since(at) <= PICK_TTL).then_some(path)
    }

    /// Put back a pick that was taken for an export that did not start.
    pub fn restore_pick(&self, id: &str, path: PathBuf) {
        self.picks
            .lock()
            .unwrap()
            .insert(id.to_string(), (path, Instant::now()));
    }

    /// Remember files the open dialog returned. Returns their pick id.
    pub fn open_pick(&self, paths: Vec<PathBuf>) -> String {
        let id = format!("open-{}", uuid::Uuid::new_v4().simple());
        let mut all = self.opened.lock().unwrap();
        all.retain(|_, (_, at)| at.elapsed() <= PICK_TTL);
        all.insert(id.clone(), (paths, Instant::now()));
        id
    }

    /// The files of open pick `id`, once, if still fresh.
    pub fn take_open(&self, id: &str) -> Option<Vec<PathBuf>> {
        let (paths, at) = self.opened.lock().unwrap().remove(id)?;
        (at.elapsed() <= PICK_TTL).then_some(paths)
    }

    /// The confirmations the jobs wait for.
    pub fn confirms(&self) -> std::sync::Arc<crate::safety::Confirms> {
        self.confirms.clone()
    }

    /// A new job id and its cancel token.
    pub fn start(&self) -> (String, CancellationToken) {
        let id = format!("job-{}", uuid::Uuid::new_v4().simple());
        let token = CancellationToken::new();
        self.inner.lock().unwrap().insert(
            id.clone(),
            Entry {
                cancel: Some(token.clone()),
                ..Entry::default()
            },
        );
        (id, token)
    }

    /// Ask a running job to stop. False when there is no such running job.
    pub fn cancel(&self, id: &str) -> bool {
        // The token first, so a question asked after this sees it; then the waiting ones.
        let found = match self
            .inner
            .lock()
            .unwrap()
            .get(id)
            .and_then(|e| e.cancel.as_ref())
        {
            Some(t) => {
                t.cancel();
                true
            }
            None => false,
        };
        self.confirms.cancel_job(id);
        found
    }

    /// Ask every running job to stop. Returns how many.
    pub fn cancel_running(&self) -> usize {
        self.stop_running(false)
    }

    /// Ask every running job to stop because its connection is closing (disconnect, another
    /// connect, sign-out): they finish as `stopped`, not `cancelled`. Returns how many.
    pub fn close_running(&self) -> usize {
        self.stop_running(true)
    }

    fn stop_running(&self, by_close: bool) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let mut n = 0;
        for e in inner.values_mut() {
            if let Some(t) = &e.cancel {
                // Marked before the cancel, so the job sees the reason when it notices.
                e.stopped_by_close |= by_close;
                t.cancel();
                n += 1;
            }
        }
        drop(inner);
        self.confirms.cancel_all();
        n
    }

    /// Whether job `id` was stopped by its connection closing.
    pub fn stopped_by_close(&self, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|e| e.stopped_by_close)
    }

    /// Whether any job is still running.
    pub fn any_running(&self) -> bool {
        self.inner
            .lock()
            .unwrap()
            .values()
            .any(|e| e.cancel.is_some())
    }

    /// Cancel every running job and wait, at most `limit`, until all have ended (an export's
    /// temp file is removed before its job ends). For app exit, which skips destructors. True
    /// when none is left running.
    pub async fn stop_all_and_wait(&self, limit: Duration) -> bool {
        self.cancel_running();
        let deadline = Instant::now() + limit;
        while self.any_running() {
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    /// Mark a job finished, keeping the file it wrote.
    pub fn finish(&self, id: &str, output: Option<PathBuf>) {
        if let Some(e) = self.inner.lock().unwrap().get_mut(id) {
            e.cancel = None;
            e.output = output;
        }
    }

    /// Keep a finished operate job's report.
    pub fn set_report(&self, id: &str, report: std::sync::Arc<crate::ops::operate::StoredReport>) {
        if let Some(e) = self.inner.lock().unwrap().get_mut(id) {
            e.report = Some(report);
        }
    }

    /// A finished operate job's report.
    pub fn report(&self, id: &str) -> Option<std::sync::Arc<crate::ops::operate::StoredReport>> {
        self.inner
            .lock()
            .unwrap()
            .get(id)
            .and_then(|e| e.report.clone())
    }

    /// Remember purge plans until they run. Returns their plan id.
    pub fn plan(&self, profile_id: &str, plans: Vec<dispatched_core::ops::PurgePlan>) -> String {
        let id = format!("plan-{}", uuid::Uuid::new_v4().simple());
        let mut all = self.plans.lock().unwrap();
        all.retain(|_, p| p.at.elapsed() <= PLAN_TTL);
        all.insert(
            id.clone(),
            PlannedPurge {
                profile_id: profile_id.to_string(),
                plans,
                at: Instant::now(),
            },
        );
        id
    }

    /// The plans of `id`, once, if still fresh.
    pub fn take_plan(&self, id: &str) -> Option<PlannedPurge> {
        let p = self.plans.lock().unwrap().remove(id)?;
        (p.at.elapsed() <= PLAN_TTL).then_some(p)
    }

    /// The file a finished job wrote.
    pub fn output(&self, id: &str) -> Option<PathBuf> {
        self.inner
            .lock()
            .unwrap()
            .get(id)
            .and_then(|e| e.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_reaches_the_token_and_outputs_are_kept() {
        let jobs = Jobs::default();
        let (id, token) = jobs.start();
        assert!(id.starts_with("job-"));
        assert!(jobs.cancel(&id));
        assert!(token.is_cancelled());
        assert!(!jobs.cancel("nope"));
        jobs.finish(&id, Some("/tmp/x.json".into()));
        assert_eq!(jobs.output(&id).unwrap().to_string_lossy(), "/tmp/x.json");
        assert!(!jobs.cancel(&id), "a finished job can't be cancelled");
    }

    #[test]
    fn a_pick_is_taken_once_and_can_be_put_back() {
        let jobs = Jobs::default();
        assert!(jobs.take_pick("pick-none").is_none());
        let a = jobs.pick("/tmp/a.json".into());
        assert!(a.starts_with("pick-"));
        assert_eq!(jobs.take_pick(&a).unwrap().to_string_lossy(), "/tmp/a.json");
        assert!(jobs.take_pick(&a).is_none(), "spent");
        jobs.restore_pick(&a, "/tmp/a.json".into());
        assert_eq!(jobs.take_pick(&a).unwrap().to_string_lossy(), "/tmp/a.json");
    }

    #[test]
    fn a_new_pick_replaces_an_unspent_one() {
        let jobs = Jobs::default();
        let a = jobs.pick("/tmp/a.json".into());
        let b = jobs.pick("/tmp/b.json".into());
        assert_ne!(a, b);
        assert!(jobs.take_pick(&a).is_none(), "replaced by the newer pick");
        assert_eq!(jobs.take_pick(&b).unwrap().to_string_lossy(), "/tmp/b.json");
    }

    #[test]
    fn an_unspent_pick_expires_after_ten_minutes() {
        let jobs = Jobs::default();
        let a = jobs.pick("/tmp/a.json".into());
        let later = Instant::now() + Duration::from_secs(10 * 60 + 1);
        assert!(jobs.take_pick_at(&a, later).is_none(), "expired");
        assert!(jobs.take_pick(&a).is_none(), "and gone");
        let b = jobs.pick("/tmp/b.json".into());
        let soon = Instant::now() + Duration::from_secs(9 * 60);
        assert!(jobs.take_pick_at(&b, soon).is_some());
    }

    #[test]
    fn close_running_marks_the_jobs_stopped_by_the_connection() {
        let jobs = Jobs::default();
        let (a, ta) = jobs.start();
        let (b, _tb) = jobs.start();
        jobs.cancel(&b);
        assert_eq!(jobs.close_running(), 2);
        assert!(ta.is_cancelled());
        assert!(jobs.stopped_by_close(&a));
        assert!(!jobs.stopped_by_close("nope"));
    }

    #[test]
    fn cancel_running_stops_every_running_job_only() {
        let jobs = Jobs::default();
        let (a, ta) = jobs.start();
        let (b, tb) = jobs.start();
        let (done, td) = jobs.start();
        jobs.finish(&done, Some("/tmp/x.json".into()));
        assert_eq!(jobs.cancel_running(), 2);
        assert!(ta.is_cancelled() && tb.is_cancelled() && !td.is_cancelled());
        assert_eq!(jobs.output(&done).unwrap().to_string_lossy(), "/tmp/x.json");
        let _ = (a, b);
    }
}
