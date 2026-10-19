//! Listener jobs: a live receive stream of a queue or subscription, until the user stops it.
//! A listener that removes messages (complete, delete) holds every message it removed, so the
//! report and "Send again" have them; at the held cap it stops itself rather than drop one.

use super::operate::{Done, Held, start_op};
use crate::error::CommandError;
use crate::events::Events;
use crate::jobs::Jobs;
use crate::ops::export::JobStarted;
use crate::state::AppState;
use dispatched_core::jobs::JobKind;
use dispatched_core::messaging::{DETAIL_BODY_CAP, MessageDetail};
use dispatched_core::namespace::{EntityPath, SubQueue};
use dispatched_core::ops::{ListenMode, ListenRequest, ListenSessions};
use serde::Deserialize;
use std::sync::Arc;

/// What a listener does with each message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModeForm {
    Watch,
    Complete,
    Delete,
}

/// Which sessions a listener takes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SessionsForm {
    None,
    Id { id: String },
    Each,
}

/// The listener form.
#[derive(Debug, Clone, Deserialize)]
pub struct ListenForm {
    pub entity: String,
    pub sub_queue: SubQueue,
    pub mode: ModeForm,
    pub sessions: SessionsForm,
    pub concurrency: u8,
    #[serde(default)]
    pub prefetch: u32,
}

/// The core request of a form, refused here when the listener could not run it.
pub fn listen_request(form: ListenForm) -> Result<ListenRequest, CommandError> {
    let req = ListenRequest {
        entity: EntityPath::parse(&form.entity)?,
        sub_queue: form.sub_queue,
        sessions: match form.sessions {
            SessionsForm::None => ListenSessions::None,
            SessionsForm::Id { id } if id.trim().is_empty() => {
                return Err(CommandError::new(
                    "invalid",
                    "Name the session to listen to.",
                ));
            }
            SessionsForm::Id { id } => ListenSessions::Id(id),
            SessionsForm::Each => ListenSessions::Each,
        },
        mode: match form.mode {
            ModeForm::Watch => ListenMode::Watch,
            ModeForm::Complete => ListenMode::Complete,
            ModeForm::Delete => ListenMode::Delete,
        },
        concurrency: form.concurrency,
        prefetch: form.prefetch,
    };
    dispatched_core::ops::listen::check(&req)?;
    Ok(req)
}

/// Start a listener. It runs until the user stops it (or, removing messages, until the held
/// cap); every message arrives as a `received` event.
pub async fn listen(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: ListenForm,
) -> Result<JobStarted, CommandError> {
    let entity = form.entity.clone();
    let req = listen_request(form)?;
    let title = match req.mode {
        ListenMode::Watch => format!("Watch {entity}"),
        ListenMode::Complete => format!("Listen to {entity} (receive and complete)"),
        ListenMode::Delete => format!("Listen to {entity} (receive and delete)"),
    };
    let jobs = state.jobs.clone();
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Listen,
        entity,
        title,
        move |op, b, c| async move {
            let held = (req.mode != ListenMode::Watch).then(|| {
                let held = Arc::new(Held::new());
                jobs.set_held(b.id(), held.clone());
                let (jobs, id) = (jobs.clone(), b.id().to_string());
                b.hold(
                    held.clone(),
                    Box::new(move |note| jobs.stop_with(&id, &note)),
                );
                held
            });
            let outcome = op.listen(req, &*b, &c).await?;
            let messages = held.map(|h| h.take()).unwrap_or_default();
            Ok(Done::Report(outcome.report, messages))
        },
    )
    .await
}

/// A message a listener or receive job took, by sequence number (decimal text): from the
/// running listener's held messages, or the finished job's report.
pub fn job_message(
    jobs: &Jobs,
    id: &str,
    sequence: &str,
) -> Result<Option<MessageDetail>, CommandError> {
    let seq: i64 = sequence.trim().parse().map_err(|_| {
        CommandError::new("invalid", format!("{sequence} is not a sequence number."))
    })?;
    let found = jobs.held(id).and_then(|h| h.find(seq)).or_else(|| {
        jobs.report(id).and_then(|r| {
            r.messages
                .iter()
                .find(|m| m.sequence_number == seq)
                .cloned()
        })
    });
    Ok(found.map(|m| MessageDetail::new(m, DETAIL_BODY_CAP)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{JobEvent as Ev, Recorder};
    use crate::ops::operate::{Held, ProgressBridge};
    use dispatched_core::jobs::{JobEvent as CoreEvent, Progress};
    use dispatched_core::messaging::{MessageBody, MessageState, PeekedMessage};
    use std::sync::{Arc, Mutex};

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
            time_to_live: None,
            dead_letter_reason: None,
            dead_letter_description: None,
            dead_letter_source: None,
            application_properties: Default::default(),
            body: MessageBody::Text(format!("body-{seq}")),
        }
    }

    fn receive(b: &ProgressBridge, seqs: &[i64]) {
        for s in seqs {
            b.event(&CoreEvent::Received(Box::new(msg(*s))));
        }
    }

    fn received_events(ev: &Recorder) -> usize {
        ev.jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Ev::Received { .. }))
            .count()
    }

    #[test]
    fn watch_holds_nothing() {
        let ev = Arc::new(Recorder::default());
        let b = ProgressBridge::new("j", ev.clone());
        receive(&b, &[1, 2, 3]);
        assert_eq!(received_events(&ev), 3);
        assert!(b.held().is_none());
    }

    #[test]
    fn destructive_listener_holds_every_message() {
        let ev = Arc::new(Recorder::default());
        let b = ProgressBridge::new("j", ev.clone());
        let held = Arc::new(Held::new());
        b.hold(held.clone(), Box::new(|_| panic!("no cap reached")));
        receive(&b, &[1, 2, 3]);
        let seqs: Vec<_> = held.take().iter().map(|m| m.sequence_number).collect();
        assert_eq!(seqs, [1, 2, 3]);
    }

    #[test]
    fn destructive_listener_stops_at_the_held_cap() {
        let ev = Arc::new(Recorder::default());
        let b = ProgressBridge::new("j", ev.clone());
        let held = Arc::new(Held::with_caps(2, u64::MAX));
        let stops = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = stops.clone();
        b.hold(
            held.clone(),
            Box::new(move |note| s.lock().unwrap().push(note)),
        );
        receive(&b, &[1, 2, 3]);
        assert_eq!(
            *stops.lock().unwrap(),
            ["Stopped at 2 held messages: save the report, then start again."]
        );
        assert_eq!(
            held.take().len(),
            3,
            "the cap stops the listener; it never drops one"
        );
    }

    #[test]
    fn user_stop_of_a_listener_is_not_an_error() {
        let s = crate::ops::test_util::state();
        let ev = Recorder::default();
        let (id, cancel) = s.jobs.start();
        cancel.cancel();
        let b = ProgressBridge::new(&id, Arc::new(Recorder::default()));
        finish_listen(
            &s.jobs,
            &ev,
            &id,
            &cancel,
            &b,
            Ok(outcome(false)),
            Vec::new(),
        );
        let jobs = ev.jobs.lock().unwrap();
        let Some(Ev::Finished { error, .. }) = jobs.last() else {
            panic!("finished")
        };
        assert!(error.is_none(), "{error:?}");
    }

    #[test]
    fn a_cap_stop_says_why() {
        let s = crate::ops::test_util::state();
        let ev = Recorder::default();
        let (id, cancel) = s.jobs.start();
        s.jobs.stop_with(
            &id,
            "Stopped at 2 held messages: save the report, then start again.",
        );
        let b = ProgressBridge::new(&id, Arc::new(Recorder::default()));
        finish_listen(
            &s.jobs,
            &ev,
            &id,
            &cancel,
            &b,
            Ok(outcome(false)),
            vec![msg(1), msg(2)],
        );
        let jobs = ev.jobs.lock().unwrap();
        let Some(Ev::Finished { error: Some(e), .. }) = jobs.last() else {
            panic!("an error")
        };
        assert_eq!(e.kind, "stopped");
        assert!(e.message.starts_with("Stopped at 2 held"));
        assert_eq!(s.jobs.report(&id).unwrap().messages.len(), 2);
    }

    #[test]
    fn listener_stopped_by_close_keeps_messages() {
        let s = crate::ops::test_util::state();
        let ev = Recorder::default();
        let (id, cancel) = s.jobs.start();
        s.jobs.close_running();
        let b = ProgressBridge::new(&id, Arc::new(Recorder::default()));
        finish_listen(
            &s.jobs,
            &ev,
            &id,
            &cancel,
            &b,
            Ok(outcome(false)),
            vec![msg(7)],
        );
        let jobs = ev.jobs.lock().unwrap();
        let Some(Ev::Finished { error: Some(e), .. }) = jobs.last() else {
            panic!("an error")
        };
        assert_eq!(e.kind, "stopped");
        assert_eq!(s.jobs.report(&id).unwrap().messages.len(), 1);
    }

    #[test]
    fn job_message_finds_a_held_message() {
        let s = crate::ops::test_util::state();
        let (id, _c) = s.jobs.start();
        let held = Arc::new(Held::new());
        s.jobs.set_held(&id, held.clone());
        let b = ProgressBridge::new(&id, Arc::new(Recorder::default()));
        b.hold(held, Box::new(|_| {}));
        receive(&b, &[9_007_199_254_740_993]);
        let d = job_message(&s.jobs, &id, "9007199254740993")
            .unwrap()
            .expect("held");
        assert_eq!(d.message.sequence_number, 9_007_199_254_740_993);
        assert!(job_message(&s.jobs, &id, "5").unwrap().is_none());
        assert_eq!(job_message(&s.jobs, &id, "x").unwrap_err().kind, "invalid");
    }

    #[test]
    fn listen_form_refusals() {
        let form = |mode: &str,
                    concurrency: u8,
                    prefetch: u32,
                    sessions: serde_json::Value|
         -> ListenForm {
            serde_json::from_value(serde_json::json!({
                "entity": "orders", "sub_queue": "active", "mode": mode,
                "sessions": sessions, "concurrency": concurrency, "prefetch": prefetch
            }))
            .unwrap()
        };
        let none = serde_json::json!({"type": "none"});
        assert!(listen_request(form("watch", 1, 0, none.clone())).is_ok());
        for f in [
            form("complete", 1, 10, none.clone()),
            form("delete", 0, 0, none.clone()),
            form("delete", 9, 0, none),
            form(
                "complete",
                2,
                0,
                serde_json::json!({"type": "id", "id": "s1"}),
            ),
        ] {
            assert_eq!(listen_request(f).unwrap_err().kind, "invalid");
        }
    }

    /// [`finish`] for a listener's outcome (the seam the tests drive).
    fn finish_listen(
        jobs: &Jobs,
        ev: &dyn Events,
        id: &str,
        cancel: &horizon_azure_kit::safety::CancellationToken,
        bridge: &ProgressBridge,
        result: dispatched_core::Result<dispatched_core::ops::ListenOutcome>,
        held: Vec<PeekedMessage>,
    ) {
        let entity = result
            .as_ref()
            .map(|o| o.report.entity.clone())
            .unwrap_or_default();
        crate::ops::operate::finish(
            jobs,
            ev,
            id,
            crate::ops::operate::JobTag::Core(JobKind::Listen),
            &entity,
            time::OffsetDateTime::now_utc(),
            cancel,
            bridge,
            result.map(|o| Done::Report(o.report, held)),
        );
    }

    fn outcome(cancelled: bool) -> dispatched_core::ops::ListenOutcome {
        let now = time::OffsetDateTime::now_utc();
        dispatched_core::ops::ListenOutcome {
            report: dispatched_core::jobs::JobReport {
                op_id: None,
                kind: dispatched_core::jobs::JobKind::Listen,
                entity: "orders".into(),
                target: None,
                started_at: now,
                finished_at: now,
                cancelled,
                items: Vec::new(),
                notes: vec!["2 received, 2 removed".into()],
                not_attempted: 0,
            },
            received: 2,
            removed: 2,
        }
    }
}
