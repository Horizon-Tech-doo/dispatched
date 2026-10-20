//! M3b emulator gate: the desktop manage and listener path end to end (the functions behind
//! the Tauri commands, the GUI policy answering typed production confirmations, the persisted
//! activity log) against the local Service Bus emulator. It creates its own `m3b-{run}-…`
//! entities and a guard deletes them on every path, panics included.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-app --lib emulator_m3b -- --nocapture`

use super::manage::{self, EntityRefForm, PolicyChange};
use super::manage_read;
use super::operate;
use super::operate_types::*;
use crate::events::{Events, JobEvent, Recorder};
use crate::ops::test_util::state;
use crate::safety::ConfirmAnswer;
use crate::state::AppState;
use dispatched_core::PeekOptions;
use dispatched_core::activity::{ActivityQuery, Outcome, Phase};
use dispatched_core::mgmt::{AccessRight, EntityRef, EntityStatus, ManagementClient, RuleFilter};
use dispatched_core::namespace::{EntityPath, Namespace, PeekView};
use dispatched_core::{EntityKind, MessageBody};
use horizon_azure_kit::safety::Origin;
use std::sync::{Arc, Mutex};
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

/// Deletes every entity the gate created, on every path.
struct Cleanup {
    ns: Namespace,
    created: Arc<Mutex<Vec<EntityRef>>>,
}

impl Cleanup {
    fn track(&self, e: EntityRef) {
        let mut v = self.created.lock().unwrap_or_else(|p| p.into_inner());
        if !v.contains(&e) {
            v.push(e);
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let ns = self.ns.clone();
        let created = self
            .created
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let done = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let m = ManagementClient::new(&ns).unwrap();
                for e in &created {
                    // Topics take their subscriptions; a missing one is fine (the emulator
                    // answers 400 for it).
                    let _ = match e {
                        EntityRef::Queue { name } => m.delete_queue(name).await,
                        EntityRef::Topic { name } => m.delete_topic(name).await,
                        EntityRef::Subscription { .. } => continue,
                    };
                }
            })
        });
        let _ = done.join();
    }
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

/// A job that changes one entity: it succeeded with one item.
async fn one(ev: &Recorder, id: &str, what: &str) {
    let r = done(ev, id)
        .await
        .unwrap_or_else(|| panic!("{what}: no report"));
    assert_eq!((r.succeeded, r.failed), (1, 0), "{what}: {r:?}");
}

/// Received events of job `id` so far.
fn received(ev: &Recorder, id: &str) -> usize {
    ev.jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| matches!(j, JobEvent::Received { id: i, .. } if i == id))
        .count()
}

async fn wait_received(ev: &Recorder, id: &str, n: usize) {
    for _ in 0..1_500 {
        if received(ev, id) >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job {id} received {} of {n} in 30 s", received(ev, id));
}

fn r(kind: EntityKind, entity: &str) -> EntityRefForm {
    EntityRefForm {
        kind,
        entity: entity.to_string(),
    }
}

async fn active_count(s: &AppState, entity: &str) -> usize {
    let active = s.active().await.unwrap();
    active
        .explorer
        .peek(
            &EntityPath::parse(entity).unwrap(),
            PeekView::Active,
            &PeekOptions::new(100),
        )
        .await
        .unwrap()
        .messages
        .len()
}

#[tokio::test(flavor = "multi_thread")]
async fn emulator_m3b() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let run = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let (q, q2, q3, t) = (
        format!("m3b-{run}-q"),
        format!("m3b-{run}-q2"),
        format!("m3b-{run}-q3"),
        format!("m3b-{run}-t"),
    );
    let sub = format!("{t}/subscriptions/s");
    let s = Arc::new(state());
    let ev = Arc::new(Recorder::default());
    let dev: Arc<dyn Events> = ev.clone();
    let p = s
        .store
        .add_connection_string("emulator-prod", &gate_cs(), s.secrets.as_ref(), true)
        .unwrap();
    crate::ops::connection::connect(&s, &*ev, &p.id)
        .await
        .unwrap();
    let guard = Cleanup {
        ns: s.active().await.unwrap().explorer.namespace().clone(),
        created: Arc::default(),
    };
    for e in [
        EntityRef::queue(&q),
        EntityRef::queue(&q2),
        EntityRef::queue(&q3),
        EntityRef::topic(&t),
    ] {
        guard.track(e);
    }
    let answers = answer_typed(&s, &ev);
    let mut gated = 0;

    // 1. Create a queue with non-default settings; it reads back as set.
    let started = manage::entity_create(
        &s,
        dev.clone(),
        &p.id,
        EntityKind::Queue,
        q.clone(),
        serde_json::json!({"lock_duration": "45s", "max_delivery_count": 7, "default_message_time_to_live": "30m"}),
        None,
    )
    .await
    .unwrap();
    one(&ev, &started.id, "create queue").await;
    gated += 1;
    let v = manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Queue, &q)
        .await
        .unwrap();
    assert_eq!(v.settings["lock_duration"], "45s");
    assert_eq!(v.settings["max_delivery_count"], 7);
    assert_eq!(v.settings["default_message_time_to_live"], "30m");

    // 2. Change one setting; the others stay.
    let started = manage::entity_update(
        &s,
        dev.clone(),
        &p.id,
        EntityKind::Queue,
        q.clone(),
        serde_json::json!({"max_delivery_count": 3}),
    )
    .await
    .unwrap();
    one(&ev, &started.id, "update").await;
    gated += 1;
    let v = manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Queue, &q)
        .await
        .unwrap();
    assert_eq!(v.settings["max_delivery_count"], 3);
    assert_eq!(v.settings["lock_duration"], "45s");

    // 3. Status to SendDisabled and back.
    for status in [EntityStatus::SendDisabled, EntityStatus::Active] {
        let started =
            manage::entity_status(&s, dev.clone(), &p.id, r(EntityKind::Queue, &q), status)
                .await
                .unwrap();
        one(&ev, &started.id, "status").await;
        gated += 1;
        let v = manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Queue, &q)
            .await
            .unwrap();
        assert_eq!(v.settings["status"], status.as_str());
    }

    // 4. Duplicate, then rename the empty copy; each confirms both names, in order.
    let before = ev.confirms.lock().unwrap().len();
    let started = manage::entity_copy(
        &s,
        dev.clone(),
        &p.id,
        r(EntityKind::Queue, &q),
        q2.clone(),
        false,
    )
    .await
    .unwrap();
    one(&ev, &started.id, "duplicate").await;
    gated += 1;
    let started = manage::entity_copy(
        &s,
        dev.clone(),
        &p.id,
        r(EntityKind::Queue, &q2),
        q3.clone(),
        true,
    )
    .await
    .unwrap();
    done(&ev, &started.id).await;
    gated += 1;
    let asked: Vec<String> = ev.confirms.lock().unwrap()[before..]
        .iter()
        .map(|c| c.target.clone())
        .collect();
    assert_eq!(
        asked,
        [q2.clone(), q2.clone(), q3.clone()],
        "duplicate asks for the copy; rename for both"
    );
    let v = manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Queue, &q3)
        .await
        .unwrap();
    assert_eq!(v.settings["max_delivery_count"], 3);
    assert!(
        manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Queue, &q2)
            .await
            .is_err(),
        "the renamed queue is gone"
    );

    // 5. A topic, a subscription with an initial SQL rule, then add, update and remove a rule.
    let started = manage::entity_create(
        &s,
        dev.clone(),
        &p.id,
        EntityKind::Topic,
        t.clone(),
        serde_json::json!({"default_message_time_to_live": "45m"}),
        None,
    )
    .await
    .unwrap();
    one(&ev, &started.id, "create topic").await;
    gated += 1;
    let rule = |name: &str, filter: serde_json::Value| -> super::manage_types::RuleForm {
        serde_json::from_value(serde_json::json!({"name": name, "filter": filter})).unwrap()
    };
    let started = manage::entity_create(
        &s,
        dev.clone(),
        &p.id,
        EntityKind::Subscription,
        sub.clone(),
        serde_json::json!({"max_delivery_count": 4}),
        Some(rule(
            "eu",
            serde_json::json!({"kind": "sql", "expression": "region = 'eu'"}),
        )),
    )
    .await
    .unwrap();
    one(&ev, &started.id, "create subscription").await;
    gated += 1;
    let names = |rules: Vec<dispatched_core::RuleInfo>| -> Vec<String> {
        rules.into_iter().map(|r| r.name).collect()
    };
    let rules = || crate::ops::entity::list_rules(&s, &*ev, &p.id, &sub);
    assert_eq!(names(rules().await.unwrap()), ["eu"]);
    let started = manage::rule_save(
        &s,
        dev.clone(),
        &p.id,
        sub.clone(),
        rule("vip", serde_json::json!({"kind": "correlation", "subject": "vip", "properties": [{"name": "tier", "value": "gold"}]})),
        false,
    )
    .await
    .unwrap();
    one(&ev, &started.id, "rule add").await;
    gated += 1;
    let started = manage::rule_save(
        &s,
        dev.clone(),
        &p.id,
        sub.clone(),
        rule(
            "vip",
            serde_json::json!({"kind": "sql", "expression": "tier = 'gold'"}),
        ),
        true,
    )
    .await
    .unwrap();
    one(&ev, &started.id, "rule update").await;
    gated += 1;
    let all = rules().await.unwrap();
    let vip = all.iter().find(|r| r.name == "vip").expect("vip");
    assert_eq!(
        vip.filter,
        RuleFilter::Sql {
            expression: "tier = 'gold'".into()
        }
    );
    let started = manage::rule_remove(&s, dev.clone(), &p.id, sub.clone(), "vip".into())
        .await
        .unwrap();
    one(&ev, &started.id, "rule remove").await;
    gated += 1;
    assert_eq!(names(rules().await.unwrap()), ["eu"]);

    // A policy add: the emulator accepts and drops policies, so only the job is checked.
    let started = manage::policy_change(
        &s,
        dev.clone(),
        &p.id,
        r(EntityKind::Queue, &q),
        "app".into(),
        PolicyChange::Add {
            rights: vec![AccessRight::Listen, AccessRight::Send],
        },
    )
    .await
    .unwrap();
    one(&ev, &started.id, "policy add").await;
    gated += 1;
    let policies = manage_read::entity_policies(&s, &*ev, &p.id, EntityKind::Queue, &q)
        .await
        .unwrap();
    if policies.is_empty() {
        eprintln!(
            "GATE-NOTE: the emulator drops SAS policies; the policy read-back and removal are not checked here"
        );
    } else {
        // A cloud namespace keeps it: it reads back with its rights, and is removed again.
        let app = policies
            .iter()
            .find(|x| x.key_name == "app")
            .expect("policy app");
        assert_eq!(app.rights, [AccessRight::Listen, AccessRight::Send]);
        let started = manage::policy_change(
            &s,
            dev.clone(),
            &p.id,
            r(EntityKind::Queue, &q),
            "app".into(),
            PolicyChange::Remove,
        )
        .await
        .unwrap();
        one(&ev, &started.id, "policy remove").await;
        gated += 1;
        let after = manage_read::entity_policies(&s, &*ev, &p.id, EntityKind::Queue, &q)
            .await
            .unwrap();
        assert!(after.iter().all(|x| x.key_name != "app"));
    }

    // 6. Export, delete in one job, import, read back.
    let file = std::env::temp_dir().join(format!("dispatched-m3b-{run}.xml"));
    let pick = s.jobs.pick(file.clone());
    let exported = manage::entities_export(
        &s,
        &p.id,
        vec![r(EntityKind::Queue, &q), r(EntityKind::Topic, &t)],
        false,
        &pick,
    )
    .await
    .unwrap();
    assert_eq!(exported.count, 3, "queue, topic and subscription");
    let before = ev.confirms.lock().unwrap().len();
    let started = manage::entity_delete(
        &s,
        dev.clone(),
        &p.id,
        vec![
            r(EntityKind::Queue, &q),
            r(EntityKind::Queue, &q3),
            r(EntityKind::Topic, &t),
        ],
    )
    .await
    .unwrap();
    let report = done(&ev, &started.id).await.unwrap();
    gated += 1;
    assert_eq!(report.succeeded, 3, "{report:?}");
    assert_eq!(
        ev.confirms.lock().unwrap().len() - before,
        3,
        "production confirms each entity of a bulk delete"
    );
    let open = s.jobs.open_pick(vec![file.clone()]);
    let preview = manage::entities_import_preview(&s, &p.id, &open)
        .await
        .unwrap();
    assert_eq!(preview.entities.len(), 3);
    let started = manage::entities_import(&s, dev.clone(), &p.id, &preview.preview_id, false)
        .await
        .unwrap();
    let report = done(&ev, &started.id).await.unwrap();
    gated += 1;
    assert_eq!(report.failed, 0, "{report:?}");
    std::fs::remove_file(&file).unwrap();
    let v = manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Queue, &q)
        .await
        .unwrap();
    assert_eq!(
        (
            v.settings["max_delivery_count"].clone(),
            v.settings["lock_duration"].clone()
        ),
        (3.into(), "45s".into())
    );
    let v = manage_read::entity_settings(&s, &*ev, &p.id, EntityKind::Subscription, &sub)
        .await
        .unwrap();
    assert_eq!(v.settings["max_delivery_count"], 4);
    assert_eq!(names(rules().await.unwrap()), ["eu"]);

    // 7. Listener: watch 5 (nothing removed, not gated), then complete them with 2 receivers.
    let form: SendForm = serde_json::from_value(serde_json::json!({
        "target": q, "template": {"subject": "gate"},
        "body": {"type": "text", "text": "m3b"},
        "ids": {"type": "increment", "base": "m3b-0001"}, "count": 5
    }))
    .unwrap();
    let started = operate::send(&s, dev.clone(), &p.id, form).await.unwrap();
    assert_eq!(done(&ev, &started.id).await.unwrap().succeeded, 5);
    gated += 1;
    let listen = |mode: &str, concurrency: u8| -> super::listen::ListenForm {
        serde_json::from_value(serde_json::json!({
            "entity": q, "sub_queue": "active", "mode": mode,
            "sessions": {"type": "none"}, "concurrency": concurrency, "prefetch": 0
        }))
        .unwrap()
    };
    let started = super::listen::listen(&s, dev.clone(), &p.id, listen("watch", 1))
        .await
        .unwrap();
    wait_received(&ev, &started.id, 5).await;
    assert!(s.jobs.cancel(&started.id));
    done(&ev, &started.id).await;
    assert_eq!(
        active_count(&s, &q).await,
        5,
        "watching leaves the messages"
    );
    let started = super::listen::listen(&s, dev.clone(), &p.id, listen("complete", 2))
        .await
        .unwrap();
    wait_received(&ev, &started.id, 5).await;
    assert!(s.jobs.cancel(&started.id));
    done(&ev, &started.id).await;
    gated += 1;
    let seqs: Vec<i64> = ev
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter_map(|j| match j {
            JobEvent::Received { id, message } if *id == started.id => {
                serde_json::to_value(&**message).ok().and_then(|v| {
                    v["sequence_number"]
                        .as_str()
                        .and_then(|t| t.parse().ok())
                        .or_else(|| v["sequence_number"].as_i64())
                })
            }
            _ => None,
        })
        .collect();
    assert_eq!(seqs.len(), 5);
    for seq in &seqs {
        let d = super::listen::job_message(&s.jobs, &started.id, &seq.to_string())
            .unwrap()
            .expect("the completed message is held");
        assert!(
            matches!(&d.message.body, MessageBody::Binary(b) if b == b"m3b"),
            "{:?}",
            d.message.body
        );
    }
    assert_eq!(active_count(&s, &q).await, 0, "completing removes them");

    // 8. One started and one finished `gui` entry per gated step, each succeeded.
    let log = s.activity.as_ref().unwrap();
    let entries = log
        .list(&ActivityQuery {
            limit: 1_000,
            ..ActivityQuery::default()
        })
        .unwrap();
    assert!(entries.iter().all(|e| e.origin == Origin::Gui));
    let started_n = entries.iter().filter(|e| e.phase == Phase::Started).count();
    let finished: Vec<_> = entries
        .iter()
        .filter(|e| e.phase == Phase::Finished)
        .collect();
    assert_eq!((started_n, finished.len()), (gated, gated), "{entries:#?}");
    assert!(
        finished.iter().all(|e| e.outcome == Outcome::Succeeded),
        "{entries:#?}"
    );
    answers.abort();
    drop(guard);
    eprintln!("emulator_m3b: all steps passed");
}
