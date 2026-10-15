//! Running and finished jobs: their cancel tokens and output files.

use horizon_azure_kit::safety::CancellationToken;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct Entry {
    cancel: Option<CancellationToken>,
    output: Option<PathBuf>,
}

#[derive(Default)]
pub struct Jobs {
    inner: Mutex<HashMap<String, Entry>>,
    /// Paths the native save dialog returned, by one-time pick id. Exports write only to these:
    /// the webview names a pick, never a path, and the dialog has asked "replace?" for each.
    picks: Mutex<HashMap<String, PathBuf>>,
}

impl Jobs {
    /// Remember a path the save dialog returned. Returns its pick id.
    pub fn pick(&self, path: PathBuf) -> String {
        let id = format!("pick-{}", uuid::Uuid::new_v4().simple());
        self.picks.lock().unwrap().insert(id.clone(), path);
        id
    }

    /// The picked path, once: a pick authorises one export.
    pub fn take_pick(&self, id: &str) -> Option<PathBuf> {
        self.picks.lock().unwrap().remove(id)
    }

    /// Put back a pick that was taken for an export that did not start.
    pub fn restore_pick(&self, id: &str, path: PathBuf) {
        self.picks.lock().unwrap().insert(id.to_string(), path);
    }

    /// A new job id and its cancel token.
    pub fn start(&self) -> (String, CancellationToken) {
        let id = format!("job-{}", uuid::Uuid::new_v4().simple());
        let token = CancellationToken::new();
        self.inner.lock().unwrap().insert(
            id.clone(),
            Entry {
                cancel: Some(token.clone()),
                output: None,
            },
        );
        (id, token)
    }

    /// Ask a running job to stop. False when there is no such running job.
    pub fn cancel(&self, id: &str) -> bool {
        match self
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
        }
    }

    /// Ask every running job to stop (their connection is closing). Returns how many.
    pub fn cancel_running(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        let running: Vec<&CancellationToken> =
            inner.values().filter_map(|e| e.cancel.as_ref()).collect();
        running.iter().for_each(|t| t.cancel());
        running.len()
    }

    /// Mark a job finished, keeping the file it wrote.
    pub fn finish(&self, id: &str, output: Option<PathBuf>) {
        if let Some(e) = self.inner.lock().unwrap().get_mut(id) {
            e.cancel = None;
            e.output = output;
        }
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
        let b = jobs.pick("/tmp/b.json".into());
        assert!(a.starts_with("pick-") && a != b);
        assert_eq!(jobs.take_pick(&a).unwrap().to_string_lossy(), "/tmp/a.json");
        assert!(jobs.take_pick(&a).is_none(), "spent");
        jobs.restore_pick(&a, "/tmp/a.json".into());
        assert_eq!(jobs.take_pick(&a).unwrap().to_string_lossy(), "/tmp/a.json");
        assert_eq!(jobs.take_pick(&b).unwrap().to_string_lossy(), "/tmp/b.json");
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
