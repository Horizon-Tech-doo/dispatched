//! M2b emulator gate: the desktop operate path end to end (commands' `ops::operate` functions,
//! the GUI policy answering typed production confirmations, the persisted activity log) against
//! the local Service Bus emulator, on the `m2b-*` entities of testing/emulator/Config.json.
//! Each run empties what it uses first, so leftovers from earlier runs are harmless.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-app --lib emulator_m2b -- --nocapture`

use super::operate;
use super::operate_types::*;
use crate::events::{Events, JobEvent, Recorder};
use crate::ops::test_util::state;
use crate::safety::ConfirmAnswer;
use crate::state::AppState;
use dispatched_core::PeekOptions;
use dispatched_core::activity::{ActivityQuery, Outcome, Phase};
use dispatched_core::namespace::{EntityPath, PeekView, SubQueue};
use dispatched_core::ops::PurgeScope;
use horizon_azure_kit::safety::Origin;
use std::sync::Arc;
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

/// The namespace the gate runs on: the emulator, or `DISPATCHED_GATE_CONNECTION_STRING` (a cloud
/// test namespace; the string comes only from the environment and is never printed).
fn gate_cs() -> String {
    std::env::var("DISPATCHED_GATE_CONNECTION_STRING").unwrap_or_else(|_| EMULATOR.to_string())
}

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

/// Answers every confirmation with the typed entity name, as a person would on production.
fn answer_typed(s: &Arc<AppState>, ev: &Arc<Recorder>) -> tokio::task::JoinHandle<()> {
    let (s, ev) = (s.clone(), ev.clone());
    tokio::spawn(async move {
        let mut seen = 0;
        loop {
            let pending: Vec<_> = ev.confirms.lock().unwrap()[seen..].to_vec();
            for c in pending {
                seen += 1;
                assert!(c.production, "the emulator profile is marked production");
                s.jobs
                    .confirms()
                    .answer(&c.id, ConfirmAnswer::Typed { text: c.target });
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
}

/// Wait for job `id` to finish; its report summary, or a panic with its error.
async fn done(ev: &Recorder, id: &str) -> Option<ReportSummary> {
    for _ in 0..3_000 {
        let found = ev.jobs.lock().unwrap().iter().find_map(|j| match j {
            JobEvent::Finished {
                id: i,
                error,
                report,
                ..
            } if i == id => Some((error.clone(), report.clone())),
            _ => None,
        });
        if let Some((error, report)) = found {
            assert!(error.is_none(), "job {id} failed: {error:?}");
            return report.map(|r| *r);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job {id} did not finish in 60 s");
}

async fn sequences(s: &AppState, entity: &str, view: PeekView) -> Vec<String> {
    let active = s.active().await.unwrap();
    active
        .explorer
        .peek(
            &EntityPath::parse(entity).unwrap(),
            view,
            &PeekOptions::new(100),
        )
        .await
        .unwrap()
        .messages
        .iter()
        .map(|m| m.sequence_number.to_string())
        .collect()
}

async fn purge_all(s: &AppState, ev: &Arc<Recorder>, profile: &str, entities: &[&str]) {
    let ev2: Arc<dyn Events> = ev.clone();
    let plans = operate::purge_plan(
        s,
        ev2.clone(),
        profile,
        entities.iter().map(|e| e.to_string()).collect(),
        PurgeScope::All,
    )
    .await
    .unwrap();
    assert_eq!(plans.plans.len(), entities.len());
    let started = operate::purge(s, ev2, profile, &plans.plan_id)
        .await
        .unwrap();
    done(ev, &started.id).await;
}

fn select(entity: &str, view: PeekView, seqs: &[String]) -> SelectForm {
    serde_json::from_value(serde_json::json!({
        "entity": entity, "view": view,
        "selection": {"type": "sequences", "sequences": seqs}
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn emulator_m2b() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let s = Arc::new(state());
    let ev = Arc::new(Recorder::default());
    let dyn_ev: Arc<dyn Events> = ev.clone();
    let p = s
        .store
        .add_connection_string("emulator-prod", &gate_cs(), s.secrets.as_ref(), true)
        .unwrap();
    crate::ops::connection::connect(&s, &*ev, &p.id)
        .await
        .unwrap();
    let answers = answer_typed(&s, &ev);

    // 0. Start empty (two entities: one confirmation each on production).
    let before = ev.confirms.lock().unwrap().len();
    purge_all(&s, &ev, &p.id, &["m2b-a", "m2b-b"]).await;
    assert_eq!(ev.confirms.lock().unwrap().len() - before, 2);
    assert!(sequences(&s, "m2b-a", PeekView::Active).await.is_empty());

    // 1. Send 5, with a long property above 2^53.
    let form: SendForm = serde_json::from_value(serde_json::json!({
        "target": "m2b-a",
        "template": {"subject": "gate", "properties": [{"name": "n", "type": "long", "value": "9007199254740993"}]},
        "body": {"type": "text", "text": "{\"gate\":\"m2b\"}"},
        "ids": {"type": "increment", "base": "m2b-0001"},
        "count": 5
    }))
    .unwrap();
    let started = operate::send(&s, dyn_ev.clone(), &p.id, form)
        .await
        .unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 5);
    let seqs = sequences(&s, "m2b-a", PeekView::Active).await;
    assert_eq!(seqs.len(), 5);
    let active = s.active().await.unwrap();
    let first = active
        .explorer
        .peek(
            &EntityPath::queue("m2b-a"),
            PeekView::Active,
            &PeekOptions::new(1),
        )
        .await
        .unwrap();
    assert_eq!(
        first.messages[0].application_properties.get("n"),
        Some(&dispatched_core::PropValue::Long(9_007_199_254_740_993))
    );

    // 2. Receive 2 and complete them.
    let form: ReceiveForm = serde_json::from_value(serde_json::json!({
        "entity": "m2b-a", "sub_queue": "active", "max": 2, "wait_seconds": 3,
        "action": {"type": "settle", "how": {"type": "complete"}}
    }))
    .unwrap();
    let started = operate::receive(&s, dyn_ev.clone(), &p.id, form)
        .await
        .unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 2);
    let received = ev
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| matches!(j, JobEvent::Received { id, .. } if *id == started.id))
        .count();
    assert_eq!(received, 2, "received messages stream to the webview");
    let seqs = sequences(&s, "m2b-a", PeekView::Active).await;
    assert_eq!(seqs.len(), 3);

    // 3. Delete one by sequence number, plus one that is not there.
    let missing = (seqs[2].parse::<i64>().unwrap() + 1_000).to_string();
    let started = operate::delete(
        &s,
        dyn_ev.clone(),
        &p.id,
        select("m2b-a", PeekView::Active, &[seqs[0].clone(), missing]),
    )
    .await
    .unwrap();
    let r = done(&ev, &started.id).await.unwrap();
    assert_eq!((r.succeeded, r.missing), (1, 1));
    let items: Vec<_> = ev
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter_map(|j| match j {
            JobEvent::Item { id, item } if *id == started.id => Some(item.outcome),
            _ => None,
        })
        .collect();
    assert_eq!(
        items.len(),
        2,
        "every item reaches the webview once: {items:?}"
    );

    // 4. Move one to m2b-b: two typed confirmations (source, then target).
    let before = ev.confirms.lock().unwrap().len();
    let started = operate::move_messages(
        &s,
        dyn_ev.clone(),
        &p.id,
        select("m2b-a", PeekView::Active, &[seqs[1].clone()]),
        "m2b-b".into(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 1);
    let asked: Vec<_> = ev.confirms.lock().unwrap()[before..]
        .iter()
        .map(|c| (c.target.clone(), c.step))
        .collect();
    assert_eq!(asked, [("m2b-a".into(), 1), ("m2b-b".into(), 2)]);
    assert_eq!(sequences(&s, "m2b-b", PeekView::Active).await.len(), 1);

    // 5. Dead-letter the last one, then resubmit it from the DLQ with an edit, removing the
    //    original.
    let form: ReceiveForm = serde_json::from_value(serde_json::json!({
        "entity": "m2b-a", "sub_queue": "active", "max": 1, "wait_seconds": 3,
        "action": {"type": "settle", "how": {"type": "dead-letter", "reason": "gate"}}
    }))
    .unwrap();
    let started = operate::receive(&s, dyn_ev.clone(), &p.id, form)
        .await
        .unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 1);
    let mut dlq = Vec::new();
    for _ in 0..50 {
        dlq = sequences(&s, "m2b-a", PeekView::DeadLetter).await;
        if !dlq.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(dlq.len(), 1);
    let form = ResubmitForm {
        source: select("m2b-a", PeekView::DeadLetter, &dlq),
        target: None,
        edits: serde_json::from_value(serde_json::json!({"subject": "resubmitted"})).unwrap(),
        new_message_id: false,
        remove_original: true,
        keep_dead_letter_properties: false,
        drop_uncopyable_properties: false,
    };
    let started = operate::resubmit(&s, dyn_ev.clone(), &p.id, form)
        .await
        .unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 1);
    assert!(
        sequences(&s, "m2b-a", PeekView::DeadLetter)
            .await
            .is_empty()
    );
    let back = active
        .explorer
        .peek(
            &EntityPath::queue("m2b-a"),
            PeekView::Active,
            &PeekOptions::new(10),
        )
        .await
        .unwrap();
    assert_eq!(back.messages.len(), 1);
    assert_eq!(back.messages[0].subject.as_deref(), Some("resubmitted"));

    // 6. Purge both queues (two confirmations), then both are empty.
    let plans = operate::purge_plan(
        &s,
        dyn_ev.clone(),
        &p.id,
        vec!["m2b-a".into(), "m2b-b".into()],
        PurgeScope::Active,
    )
    .await
    .unwrap();
    assert!(plans.plans.iter().all(|p| !p.unknown));
    let started = operate::purge(&s, dyn_ev.clone(), &p.id, &plans.plan_id)
        .await
        .unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 2);
    assert!(sequences(&s, "m2b-a", PeekView::Active).await.is_empty());
    assert!(sequences(&s, "m2b-b", PeekView::Active).await.is_empty());
    assert!(
        operate::purge(&s, dyn_ev.clone(), &p.id, &plans.plan_id)
            .await
            .is_err(),
        "a plan runs once"
    );

    // 7. Session state: set, read back, clear.
    let session = format!("m2b-{}", uuid::Uuid::new_v4().simple());
    let form: SendForm = serde_json::from_value(serde_json::json!({
        "target": "m2b-sessions", "template": {"session_id": session},
        "body": {"type": "text", "text": "s"}, "ids": {"type": "random"}, "count": 1
    }))
    .unwrap();
    let started = operate::send(&s, dyn_ev.clone(), &p.id, form)
        .await
        .unwrap();
    done(&ev, &started.id).await;
    use base64::Engine as _;
    let value = base64::engine::general_purpose::STANDARD.encode("hello");
    let started = operate::session_state_set(
        &s,
        dyn_ev.clone(),
        &p.id,
        "m2b-sessions",
        &session,
        Some(value),
        false,
    )
    .await
    .unwrap();
    assert!(
        done(&ev, &started.id).await.is_none(),
        "a plain job has no report"
    );
    let got = operate::session_state_get(&s, &p.id, "m2b-sessions", &session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.text.as_deref(), Some("hello"));
    let started = operate::session_state_set(
        &s,
        dyn_ev.clone(),
        &p.id,
        "m2b-sessions",
        &session,
        None,
        false,
    )
    .await
    .unwrap();
    done(&ev, &started.id).await;
    assert!(
        operate::session_state_get(&s, &p.id, "m2b-sessions", &session)
            .await
            .unwrap()
            .is_none()
    );

    // 8. The DLQ summary of an empty dead-letter queue reads empty.
    let summary = operate::dlq_summary(&s, &p.id, "m2b-a", SubQueue::DeadLetter)
        .await
        .unwrap();
    assert!(summary.groups.is_empty());

    // 9. Every gated step left a started and a finished `gui` entry, none pending.
    let log = s.activity.as_ref().unwrap();
    let entries = log
        .list(&ActivityQuery {
            limit: 1_000,
            ..ActivityQuery::default()
        })
        .unwrap();
    assert!(entries.iter().all(|e| e.origin == Origin::Gui));
    let finished = entries
        .iter()
        .filter(|e| e.phase == Phase::Finished)
        .count();
    let started_n = entries.iter().filter(|e| e.phase == Phase::Started).count();
    // A purge runs the gate once per entity: purge (2) + send + receive + delete + move +
    // receive + resubmit + purge (2) + send + set + clear = 13 gate runs.
    assert_eq!((started_n, finished), (13, 13), "{entries:#?}");
    // Every started entry has its finished one, and no finished entry is pending.
    for e in entries.iter().filter(|e| e.phase == Phase::Started) {
        assert!(
            entries
                .iter()
                .any(|f| f.phase == Phase::Finished && f.op_id == e.op_id),
            "{} was never finished",
            e.op_id
        );
    }
    assert!(
        entries
            .iter()
            .filter(|e| e.phase == Phase::Finished)
            .all(|e| e.outcome == Outcome::Succeeded),
        "{entries:#?}"
    );
    assert_eq!(
        ev.records.lock().unwrap().len(),
        entries.len(),
        "every entry was emitted"
    );
    answers.abort();
    eprintln!("emulator_m2b: all steps passed");
}
