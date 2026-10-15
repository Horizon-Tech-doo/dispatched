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
    /// The path the native save dialog returned last; the user answered its "replace?" there.
    picked: Mutex<Option<PathBuf>>,
}

impl Jobs {
    /// Remember what the save dialog returned (`None` when it was cancelled).
    pub fn set_picked(&self, path: Option<PathBuf>) {
        *self.picked.lock().unwrap() = path;
    }

    /// The last picked path, once: a pick authorises one export.
    pub fn take_picked(&self) -> Option<PathBuf> {
        self.picked.lock().unwrap().take()
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
    fn a_picked_path_is_taken_once() {
        let jobs = Jobs::default();
        assert!(jobs.take_picked().is_none());
        jobs.set_picked(Some("/tmp/a.json".into()));
        assert_eq!(jobs.take_picked().unwrap().to_string_lossy(), "/tmp/a.json");
        assert!(jobs.take_picked().is_none());
    }
}
