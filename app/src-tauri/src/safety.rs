//! The desktop side of the core safety gate: confirmations asked in the webview, and the shared
//! activity log.
//!
//! A mutating operation runs behind a core [`Gate`]. Its policy, [`GuiPolicy`], emits a
//! `confirm` event carrying the core's own sentence for the request and waits until the sheet
//! that started the job answers through `confirm_answer`, the job is cancelled, or the
//! connection closes (both answer "no"). The gate's audit, [`EmittingAudit`], writes the shared
//! `activity.db` and emits each entry, so the Activity log shows it at once.

use crate::error::CommandError;
use crate::events::Events;
use crate::state::AppState;
use async_trait::async_trait;
use dispatched_core::activity::{ActivityEntry, ActivityLog, Audit};
use dispatched_core::ops::Operator;
use dispatched_core::safety::{Actor, Gate, ProfileRef};
use horizon_azure_kit::safety::{CancellationToken, ConfirmRequest, Decision, Origin, Policy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// What the webview is asked to confirm. The sentence is the core's ([`ConfirmRequest`]);
/// the webview adds nothing to it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfirmEvent {
    /// Answer with this id.
    pub id: String,
    /// The job that asks.
    pub job_id: String,
    pub verb: String,
    /// The entity; on production the typed text must equal it.
    pub target: String,
    pub summary: String,
    /// As a string: counts are shown, not computed with.
    pub item_count: Option<String>,
    pub reversible: bool,
    pub production: bool,
    /// 1 for the job's first request, 2 for its second (a production move's target), and so on.
    pub step: u32,
}

/// The webview's answer.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ConfirmAnswer {
    /// Go ahead (not enough on production).
    Allow,
    /// Go ahead; the person typed `text`.
    Typed { text: String },
    /// Do not.
    Deny,
}

struct Pending {
    job_id: String,
    production: bool,
    tx: oneshot::Sender<Decision>,
}

/// Confirmations waiting for an answer, by id.
#[derive(Default)]
pub struct Confirms {
    pending: Mutex<HashMap<String, Pending>>,
}

const CANCELLED: &str = "cancelled before it was confirmed";

impl Confirms {
    /// Register a question for `job_id`: its id and where the answer arrives.
    pub fn ask(&self, job_id: &str, req: &ConfirmRequest) -> (String, oneshot::Receiver<Decision>) {
        let id = format!("confirm-{}", uuid::Uuid::new_v4().simple());
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                job_id: job_id.to_string(),
                production: req.production,
                tx,
            },
        );
        (id, rx)
    }

    /// Answer question `id`. False when it is not waiting (answered, cancelled, unknown).
    /// On production only a typed answer goes through; the kit then checks the text.
    pub fn answer(&self, id: &str, answer: ConfirmAnswer) -> bool {
        let Some(p) = self.pending.lock().unwrap().remove(id) else {
            return false;
        };
        let decision = match answer {
            ConfirmAnswer::Allow if p.production => {
                Decision::Deny("a production target needs its name typed".into())
            }
            ConfirmAnswer::Allow => Decision::Allow,
            ConfirmAnswer::Typed { text } => Decision::ConfirmedTyped(text),
            ConfirmAnswer::Deny => Decision::Deny("not confirmed".into()),
        };
        p.tx.send(decision).is_ok()
    }

    /// Answer "no" to every question of `job_id` (the job was cancelled).
    pub fn cancel_job(&self, job_id: &str) {
        let mut pending = self.pending.lock().unwrap();
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, p)| p.job_id == job_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(p) = pending.remove(&id) {
                let _ = p.tx.send(Decision::Deny(CANCELLED.into()));
            }
        }
    }

    /// Answer "no" to every waiting question (the connection is closing).
    pub fn cancel_all(&self) {
        for (_, p) in self.pending.lock().unwrap().drain() {
            let _ = p.tx.send(Decision::Deny(CANCELLED.into()));
        }
    }

    /// How many questions wait.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

/// Asks the webview, through a `confirm` event, and waits for its answer.
/// Asks nothing once its job is cancelled, and withdraws a question when the job is cancelled
/// while it waits.
pub struct GuiPolicy {
    job_id: String,
    confirms: Arc<Confirms>,
    events: Arc<dyn Events>,
    cancel: CancellationToken,
    step: AtomicU32,
}

impl GuiPolicy {
    pub fn new(
        job_id: &str,
        confirms: Arc<Confirms>,
        events: Arc<dyn Events>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            job_id: job_id.to_string(),
            confirms,
            events,
            cancel,
            step: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl Policy for GuiPolicy {
    async fn confirm(&self, req: &ConfirmRequest) -> Decision {
        if self.cancel.is_cancelled() {
            return Decision::Deny(CANCELLED.into());
        }
        let (id, rx) = self.confirms.ask(&self.job_id, req);
        self.events.confirm(ConfirmEvent {
            id,
            job_id: self.job_id.clone(),
            verb: req.verb.to_string(),
            target: req.target.clone(),
            summary: req.summary.clone(),
            item_count: req.item_count.map(|n| n.to_string()),
            reversible: req.reversible,
            production: req.production,
            step: self.step.fetch_add(1, Ordering::SeqCst) + 1,
        });
        tokio::select! {
            answer = rx => answer.unwrap_or_else(|_| Decision::Deny(CANCELLED.into())),
            () = self.cancel.cancelled() => {
                self.confirms.cancel_job(&self.job_id);
                Decision::Deny(CANCELLED.into())
            }
        }
    }
}

/// Writes the shared activity log, then emits the entry (with its row id).
pub struct EmittingAudit {
    log: Arc<ActivityLog>,
    events: Arc<dyn Events>,
}

impl EmittingAudit {
    pub fn new(log: Arc<ActivityLog>, events: Arc<dyn Events>) -> Self {
        Self { log, events }
    }
}

impl Audit for EmittingAudit {
    fn record(&self, entry: &ActivityEntry) -> dispatched_core::Result<i64> {
        let id = self.log.record(entry)?;
        let mut shown = entry.clone();
        shown.id = id;
        self.events.activity_record(shown);
        Ok(id)
    }
}

/// The gate for one job on the connected profile `profile_id`: asks through the webview,
/// logs to the shared activity log as `gui`.
pub async fn gate_for(
    state: &AppState,
    profile_id: &str,
    job_id: &str,
    events: Arc<dyn Events>,
    cancel: CancellationToken,
) -> Result<(Gate, Arc<dispatched_core::Explorer>), CommandError> {
    let active = state.active_for(profile_id).await?;
    let log = state.activity.clone().ok_or_else(|| {
        CommandError::new(
            "activity_unavailable",
            "The activity log could not be opened, so nothing that changes messages can run. Restart Dispatched.",
        )
    })?;
    let gate = Gate::new(
        Arc::new(GuiPolicy::new(
            job_id,
            state.jobs.confirms(),
            events.clone(),
            cancel,
        )),
        Arc::new(EmittingAudit::new(log, events)),
        Actor {
            origin: Origin::Gui,
            profile: ProfileRef {
                name: active.profile_name.clone(),
                production: active.production,
            },
        },
    );
    Ok((gate, active.explorer))
}

/// The operator for one job on the connected profile `profile_id`.
pub async fn operator_for(
    state: &AppState,
    profile_id: &str,
    job_id: &str,
    events: Arc<dyn Events>,
    cancel: CancellationToken,
) -> Result<Operator, CommandError> {
    let (gate, explorer) = gate_for(state, profile_id, job_id, events, cancel).await?;
    Ok(Operator::new(explorer, gate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use dispatched_core::activity::{ActivityQuery, Outcome, Phase};
    use dispatched_core::namespace::{EntityPath, SubQueue};
    use dispatched_core::safety::{Operation, Target};
    use horizon_azure_kit::safety::CancellationToken;
    use std::time::Duration;

    struct Setup {
        events: Arc<Recorder>,
        confirms: Arc<Confirms>,
        log: Arc<ActivityLog>,
    }

    impl Setup {
        fn new() -> Self {
            Self {
                events: Arc::new(Recorder::default()),
                confirms: Arc::new(Confirms::default()),
                log: Arc::new(ActivityLog::open_in_memory().unwrap()),
            }
        }

        fn gate(&self, production: bool) -> Gate {
            self.gate_with(production, CancellationToken::new())
        }

        fn gate_with(&self, production: bool, cancel: CancellationToken) -> Gate {
            let ev: Arc<dyn Events> = self.events.clone();
            Gate::new(
                Arc::new(GuiPolicy::new(
                    "job-1",
                    self.confirms.clone(),
                    ev.clone(),
                    cancel,
                )),
                Arc::new(EmittingAudit::new(self.log.clone(), ev)),
                Actor {
                    origin: Origin::Gui,
                    profile: ProfileRef {
                        name: "prod-weu".into(),
                        production,
                    },
                },
            )
        }

        /// The `n`th confirm event, once it has arrived.
        async fn asked(&self, n: usize) -> ConfirmEvent {
            for _ in 0..200 {
                if let Some(e) = self.events.confirms.lock().unwrap().get(n - 1) {
                    return e.clone();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("no confirm request {n}");
        }

        fn entries(&self) -> Vec<ActivityEntry> {
            self.log.list(&ActivityQuery::default()).unwrap()
        }
    }

    fn delete_op() -> Operation {
        Operation::delete(
            Target::new(EntityPath::queue("orders"), SubQueue::Active),
            &[7, 9],
        )
    }

    fn move_op() -> Operation {
        Operation::move_to(
            Target::new(EntityPath::queue("orders"), SubQueue::DeadLetter),
            "retry",
            2,
        )
    }

    #[tokio::test]
    async fn allow_on_non_production_proceeds() {
        let s = Setup::new();
        let gate = s.gate(false);
        let run = tokio::spawn(async move { gate.run(&delete_op(), |_| async { Ok(()) }).await });
        let asked = s.asked(1).await;
        assert_eq!(
            (asked.target.as_str(), asked.step, asked.production),
            ("orders", 1, false)
        );
        assert!(asked.summary.contains('7'), "{}", asked.summary);
        assert!(s.confirms.answer(&asked.id, ConfirmAnswer::Allow));
        run.await.unwrap().unwrap();
        let phases: Vec<_> = s.entries().iter().map(|e| (e.phase, e.outcome)).collect();
        assert!(
            phases.contains(&(Phase::Finished, Outcome::Succeeded)),
            "{phases:?}"
        );
        assert!(s.entries().iter().all(|e| e.origin == Origin::Gui));
    }

    #[tokio::test]
    async fn typed_wrong_name_is_denied_on_production() {
        let s = Setup::new();
        let gate = s.gate(true);
        let run = tokio::spawn(async move { gate.run(&delete_op(), |_| async { Ok(()) }).await });
        let asked = s.asked(1).await;
        assert!(asked.production);
        s.confirms.answer(
            &asked.id,
            ConfirmAnswer::Typed {
                text: "order".into(),
            },
        );
        assert!(run.await.unwrap().is_err());
        let e = s.entries();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].outcome, Outcome::Denied);
    }

    #[tokio::test]
    async fn allow_alone_is_not_enough_on_production() {
        let s = Setup::new();
        let gate = s.gate(true);
        let run = tokio::spawn(async move { gate.run(&delete_op(), |_| async { Ok(()) }).await });
        let asked = s.asked(1).await;
        s.confirms.answer(&asked.id, ConfirmAnswer::Allow);
        assert!(run.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn production_move_asks_source_then_target() {
        let s = Setup::new();
        let gate = s.gate(true);
        let ran = Arc::new(AtomicU32::new(0));
        let r2 = ran.clone();
        let run = tokio::spawn(async move {
            gate.run(&move_op(), |_| async move {
                r2.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
        });
        let first = s.asked(1).await;
        assert_eq!((first.target.as_str(), first.step), ("orders", 1));
        s.confirms.answer(
            &first.id,
            ConfirmAnswer::Typed {
                text: "orders".into(),
            },
        );
        let second = s.asked(2).await;
        assert_eq!((second.target.as_str(), second.step), ("retry", 2));
        s.confirms.answer(&second.id, ConfirmAnswer::Deny);
        assert!(run.await.unwrap().is_err());
        assert_eq!(ran.load(Ordering::SeqCst), 0, "nothing ran");
        assert_eq!(s.entries()[0].outcome, Outcome::Denied);
    }

    #[tokio::test]
    async fn unanswered_confirm_cancels_on_job_cancel() {
        let s = Setup::new();
        let gate = s.gate(false);
        let run = tokio::spawn(async move { gate.run(&delete_op(), |_| async { Ok(()) }).await });
        s.asked(1).await;
        s.confirms.cancel_job("job-1");
        assert!(run.await.unwrap().is_err());
        assert_eq!(s.confirms.waiting(), 0);
        let e = s.entries();
        assert!(e.iter().all(|e| e.outcome != Outcome::Pending), "{e:?}");
        assert_eq!(e[0].outcome, Outcome::Denied);
    }

    #[tokio::test]
    async fn unanswered_confirm_cancels_on_close() {
        let s = Setup::new();
        let gate = s.gate(true);
        let run = tokio::spawn(async move { gate.run(&move_op(), |_| async { Ok(()) }).await });
        let asked = s.asked(1).await;
        s.confirms.cancel_all();
        assert!(run.await.unwrap().is_err());
        assert!(
            !s.confirms.answer(&asked.id, ConfirmAnswer::Deny),
            "no longer waiting"
        );
        assert!(s.entries().iter().all(|e| e.outcome == Outcome::Denied));
    }

    #[tokio::test]
    async fn a_question_asked_after_the_cancel_is_denied_at_once() {
        let s = Setup::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let gate = s.gate_with(false, cancel);
        let run = tokio::time::timeout(
            Duration::from_secs(2),
            gate.run(&delete_op(), |_| async { Ok(()) }),
        )
        .await
        .expect("never waits");
        assert!(run.is_err());
        assert_eq!(s.confirms.waiting(), 0);
        assert!(
            s.events.confirms.lock().unwrap().is_empty(),
            "nothing is asked"
        );
    }

    #[tokio::test]
    async fn a_cancel_while_waiting_denies_by_the_token_alone() {
        let s = Setup::new();
        let cancel = CancellationToken::new();
        let gate = s.gate_with(false, cancel.clone());
        let run = tokio::spawn(async move { gate.run(&delete_op(), |_| async { Ok(()) }).await });
        s.asked(1).await;
        cancel.cancel();
        let r = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .expect("ends");
        assert!(r.unwrap().is_err());
        assert_eq!(s.confirms.waiting(), 0, "the question is withdrawn");
    }

    #[tokio::test]
    async fn audit_entry_is_emitted_and_persisted() {
        let s = Setup::new();
        let gate = s.gate(false);
        let run = tokio::spawn(async move { gate.run(&delete_op(), |_| async { Ok(()) }).await });
        let asked = s.asked(1).await;
        s.confirms.answer(&asked.id, ConfirmAnswer::Allow);
        run.await.unwrap().unwrap();
        let emitted = s.events.records.lock().unwrap().clone();
        let stored = s.entries();
        assert_eq!(emitted.len(), 2);
        assert_eq!(stored.len(), 2);
        assert!(emitted.iter().all(|e| e.id > 0));
        let mut ids: Vec<i64> = stored.iter().map(|e| e.id).collect();
        ids.sort();
        assert_eq!(ids, emitted.iter().map(|e| e.id).collect::<Vec<_>>());
    }

    #[test]
    fn answers_deserialize_from_the_webview_shape() {
        let a: ConfirmAnswer =
            serde_json::from_value(serde_json::json!({"type":"typed","text":"orders"})).unwrap();
        assert_eq!(
            a,
            ConfirmAnswer::Typed {
                text: "orders".into()
            }
        );
        let d: ConfirmAnswer = serde_json::from_value(serde_json::json!({"type":"deny"})).unwrap();
        assert_eq!(d, ConfirmAnswer::Deny);
    }
}
