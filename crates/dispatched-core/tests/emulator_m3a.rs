//! M3a emulator gate: entity management, incumbent XML export and import, and the listener,
//! end to end against the local emulator (`m3a_manage`).
//!
//! Every entity is created by the run itself and named `m3a-{run}-…`; a guard deletes all of
//! them when the test ends, on every path, panics included. Where the emulator cannot do
//! something (it drops SAS policies, and listing rules returns 500), the gate probes it, prints
//! a `GATE-NOTE: …` line and skips that one assertion; it never passes a check it did not make.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m3a -- --nocapture`

use dispatched_core::activity::{ActivityQuery, Outcome, Phase};
use dispatched_core::messaging::testing::{self, ServiceBusConnection};
use dispatched_core::mgmt::incumbent::{
    ExportEntity, ExportSubscription, export_entities, parse_entities,
};
use dispatched_core::mgmt::{
    AccessRight, Description, EntityRef, EntityStatus, QueueSettings, RuleSpec,
    SubscriptionSettings, TopicSettings,
};
use dispatched_core::safety::{Actor, Gate, ProfileRef};
use dispatched_core::{
    ActivityLog, ConnectionString, EntityPath, EntitySpec, Error, Explorer, JobEvent, JobReport,
    ListenMode, ListenRequest, ListenSessions, ManagementClient, Namespace, NoProgress, Operator,
    PeekOptions, PeekView, Progress, RuleFilter, SubQueue,
};
use horizon_azure_kit::safety::{AllowAll, CancellationToken, Origin, Policy};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";
const IDLE: Duration = Duration::from_secs(1);

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

fn note(text: &str) {
    eprintln!("GATE-NOTE: {text}");
}

fn gate(policy: Arc<dyn Policy>, log: Arc<ActivityLog>) -> Gate {
    Gate::new(
        policy,
        log,
        Actor {
            origin: Origin::Cli,
            profile: ProfileRef {
                name: "emulator".into(),
                production: false,
            },
        },
    )
}

/// Deletes every entity the run created when dropped, panics included. It runs on its own
/// thread and runtime, so it works from `Drop` inside the test's runtime.
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
                let mut left = Vec::new();
                // Topics take their subscriptions with them; a missing entity is fine (the
                // emulator answers 400 for it, not 404).
                for e in &created {
                    let r = match e {
                        EntityRef::Queue { name } => m.delete_queue(name).await,
                        EntityRef::Topic { name } => m.delete_topic(name).await,
                        EntityRef::Subscription { .. } => continue,
                    };
                    if r.is_ok() {
                        left.push(e.path());
                    }
                }
                left
            })
        })
        .join();
        match done {
            Ok(deleted) if deleted.is_empty() => {
                eprintln!("cleanup: nothing left to delete")
            }
            Ok(deleted) => eprintln!("cleanup: deleted leftovers {deleted:?}"),
            Err(_) => eprintln!("cleanup: the cleanup thread panicked"),
        }
    }
}

/// The activity log has a started entry and a finished entry with `outcome` for `op_id`.
fn assert_logged(log: &ActivityLog, op_id: &str, outcome: Outcome, step: &str) {
    let entries = log
        .list(&ActivityQuery {
            op_id: Some(op_id.to_string()),
            ..Default::default()
        })
        .unwrap();
    let started: Vec<_> = entries
        .iter()
        .filter(|e| e.phase == Phase::Started)
        .collect();
    let finished: Vec<_> = entries
        .iter()
        .filter(|e| e.phase == Phase::Finished)
        .collect();
    assert!(
        matches!(started.as_slice(), [e] if e.outcome == Outcome::Pending),
        "{step}: not exactly one started entry for {op_id}: {entries:?}"
    );
    assert!(
        matches!(finished.as_slice(), [e] if e.outcome == outcome),
        "{step}: not exactly one finished entry with {outcome:?} for {op_id}: {entries:?}"
    );
}

/// Checks the report succeeded and its started/finished pair is in the log; counts it.
fn logged(log: &ActivityLog, report: &JobReport, step: &str, gated: &AtomicU64) {
    let op_id = report
        .op_id
        .as_deref()
        .unwrap_or_else(|| panic!("{step}: the report has no op id"));
    assert_eq!(report.outcome(), Outcome::Succeeded, "{step}: {report:?}");
    assert_logged(log, op_id, Outcome::Succeeded, step);
    gated.fetch_add(1, Ordering::Relaxed);
    eprintln!("ok {step}: logged as {op_id}");
}

/// Cancels the listener once it has emitted `want` received messages.
struct StopAfter {
    want: u64,
    seen: AtomicU64,
    cancel: CancellationToken,
}

impl Progress for StopAfter {
    fn event(&self, e: &JobEvent) {
        if let JobEvent::Received(_) = e
            && self.seen.fetch_add(1, Ordering::SeqCst) + 1 >= self.want
        {
            self.cancel.cancel();
        }
    }
}

async fn send_n(conn: &mut ServiceBusConnection, queue: &str, run: &str, tag: &str, n: usize) {
    for i in 0..n {
        testing::send_text(
            conn,
            queue,
            &format!("{{\"run\":\"{run}\",\"tag\":\"{tag}\",\"i\":{i}}}"),
            &[("run", run), ("tag", tag), ("i", &i.to_string())],
        )
        .await
        .unwrap();
    }
}

async fn peek_all(explorer: &Explorer, queue: &str) -> Vec<dispatched_core::PeekedMessage> {
    explorer
        .peek(
            &EntityPath::queue(queue),
            PeekView::Active,
            &PeekOptions::new(1_000).scan_limit(10_000),
        )
        .await
        .unwrap()
        .messages
}

/// Peek `queue` until it shows `want` messages or 10 s pass.
async fn peek_until(explorer: &Explorer, queue: &str, want: usize) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let n = peek_all(explorer, queue).await.len();
        if n == want || tokio::time::Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The non-default queue settings the gate creates with.
fn queue_settings(run: &str) -> QueueSettings {
    QueueSettings {
        lock_duration: Some(Duration::from_secs(45)),
        max_delivery_count: Some(7),
        // The emulator caps the TTL at one hour.
        default_message_time_to_live: Some(Duration::from_secs(30 * 60)),
        dead_lettering_on_message_expiration: Some(true),
        user_metadata: Some(format!("m3a gate {run}")),
        ..Default::default()
    }
}

/// Every setting the gate set is read back as set.
fn assert_queue_has(got: &QueueSettings, want: &QueueSettings, step: &str) {
    assert_eq!(got.lock_duration, want.lock_duration, "{step}: lock");
    assert_eq!(
        got.max_delivery_count, want.max_delivery_count,
        "{step}: max delivery count"
    );
    assert_eq!(
        got.default_message_time_to_live, want.default_message_time_to_live,
        "{step}: ttl"
    );
    assert_eq!(
        got.dead_lettering_on_message_expiration, want.dead_lettering_on_message_expiration,
        "{step}: dead-lettering on expiration"
    );
    check_metadata(&got.user_metadata, &want.user_metadata, step);
}

/// The emulator accepts `UserMetadata` and drops it (probed: the PUT succeeds, the read-back
/// has no element). Asserted where kept; noted where dropped.
fn check_metadata(got: &Option<String>, want: &Option<String>, step: &str) {
    match got {
        None if want.is_some() => note(&format!(
            "{step}: the emulator dropped UserMetadata (set {want:?}, read back none); \
             metadata not checked here"
        )),
        _ => assert_eq!(got, want, "{step}: metadata"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn m3a_manage() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    eprintln!("m3a_manage run {run}");
    let cleanup = Cleanup {
        ns: ns.clone(),
        created: Arc::default(),
    };
    let explorer = Arc::new(Explorer::new(ns.clone()).unwrap());
    let m = explorer.management_client().clone();
    let log = Arc::new(ActivityLog::open_in_memory().unwrap());
    let op = Operator::new(explorer.clone(), gate(Arc::new(AllowAll), log.clone()));
    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();
    let gated = AtomicU64::new(0);

    let q = format!("m3a-{run}-q");
    let copy = format!("m3a-{run}-copy");
    let moved = format!("m3a-{run}-moved");
    let topic = format!("m3a-{run}-t");
    let sub = format!("m3a-{run}-s");
    for name in [&q, &copy, &moved] {
        cleanup.track(EntityRef::queue(name));
    }
    cleanup.track(EntityRef::topic(&topic));

    // 1. Create a queue with non-default settings; it reads back exactly.
    let want = queue_settings(&run);
    let r = op
        .create_entity(EntitySpec::Queue(q.clone(), Description::new(want.clone())))
        .await
        .unwrap();
    logged(&log, &r, "1 create queue", &gated);
    let d = m.get_queue_description(&q).await.unwrap();
    assert_queue_has(&d.settings, &want, "1 read back");
    assert_eq!(d.settings.status, Some(EntityStatus::Active), "1 status");
    let before = d.settings.clone();

    // 2. Update two settings (read-modify-write); every other one is unchanged.
    let mut d = m.get_queue_description(&q).await.unwrap();
    d.settings.max_delivery_count = Some(3);
    d.settings.lock_duration = Some(Duration::from_secs(60));
    let r = op
        .update_entity(EntitySpec::Queue(q.clone(), d))
        .await
        .unwrap();
    logged(&log, &r, "2 update two settings", &gated);
    let after = m.get_queue_description(&q).await.unwrap().settings;
    let expected = QueueSettings {
        max_delivery_count: Some(3),
        lock_duration: Some(Duration::from_secs(60)),
        ..before.clone()
    };
    assert_eq!(after, expected, "2: only the two settings changed");

    // 3. Status to SendDisabled, then back to Active.
    let r = op
        .set_status(EntityRef::queue(&q), EntityStatus::SendDisabled)
        .await
        .unwrap();
    logged(&log, &r, "3 status SendDisabled", &gated);
    let s = m.get_queue_description(&q).await.unwrap().settings;
    assert_eq!(s.status, Some(EntityStatus::SendDisabled), "3 status set");
    assert_eq!(
        QueueSettings {
            status: Some(EntityStatus::Active),
            ..s
        },
        expected,
        "3: the status change kept the other settings"
    );
    let r = op
        .set_status(EntityRef::queue(&q), EntityStatus::Active)
        .await
        .unwrap();
    logged(&log, &r, "3 status Active", &gated);
    assert_eq!(
        m.get_queue_description(&q).await.unwrap().settings.status,
        Some(EntityStatus::Active),
        "3 status back"
    );

    // 4. Duplicate: the copy has the same settings.
    let r = op
        .duplicate(EntityRef::queue(&q), copy.clone())
        .await
        .unwrap();
    logged(&log, &r, "4 duplicate", &gated);
    let c = m.get_queue_description(&copy).await.unwrap().settings;
    assert_eq!(c, expected, "4: the copy has the source's settings");

    // 5. Rename the (empty) copy: the new name exists with its settings, the old is gone.
    let r = op
        .rename(EntityRef::queue(&copy), moved.clone())
        .await
        .unwrap();
    logged(&log, &r, "5 rename", &gated);
    let mv = m.get_queue_description(&moved).await.unwrap().settings;
    assert_eq!(mv, expected, "5: the renamed queue kept its settings");
    let gone = m.get_queue_description(&copy).await;
    assert!(
        matches!(gone, Err(Error::NotFound(_))),
        "5: the original is gone: {gone:?}"
    );

    // 6. A rename of a queue holding a message is refused before the gate; nothing changes.
    send_n(&mut conn, &q, &run, "hold", 1).await;
    assert_eq!(peek_until(&explorer, &q, 1).await, 1, "6 seeded");
    let entries_before = log.list(&ActivityQuery::default()).unwrap().len();
    let refused = op
        .rename(EntityRef::queue(&q), format!("m3a-{run}-never"))
        .await;
    cleanup.track(EntityRef::queue(&format!("m3a-{run}-never")));
    match &refused {
        Err(Error::Conflict(msg)) => {
            assert!(msg.contains("move them first"), "6 message: {msg}");
            eprintln!("ok 6 rename refused: {msg}");
        }
        other => panic!("6: a rename with a message must be refused: {other:?}"),
    }
    assert_eq!(
        log.list(&ActivityQuery::default()).unwrap().len(),
        entries_before,
        "6: a refused rename logs nothing"
    );
    assert!(
        m.get_queue_description(&q).await.is_ok(),
        "6: the source is still there"
    );
    assert!(
        matches!(
            m.get_queue_description(&format!("m3a-{run}-never")).await,
            Err(Error::NotFound(_))
        ),
        "6: no target was created"
    );
    testing::drain(&mut conn, &q, IDLE).await.unwrap();
    assert_eq!(peek_until(&explorer, &q, 0).await, 0, "6 drained");

    // 7. A topic, and a subscription with an initial SQL rule.
    let r = op
        .create_entity(EntitySpec::Topic(
            topic.clone(),
            Description::new(TopicSettings {
                default_message_time_to_live: Some(Duration::from_secs(45 * 60)),
                user_metadata: Some(format!("m3a topic {run}")),
                ..Default::default()
            }),
        ))
        .await
        .unwrap();
    logged(&log, &r, "7 create topic", &gated);
    let sql = format!("region = 'eu' AND run = '{run}'");
    let rule = RuleSpec {
        name: "eu".into(),
        filter: RuleFilter::Sql {
            expression: sql.clone(),
        },
        action: None,
    };
    let sub_settings = SubscriptionSettings {
        lock_duration: Some(Duration::from_secs(30)),
        max_delivery_count: Some(5),
        user_metadata: Some(format!("m3a sub {run}")),
        ..Default::default()
    };
    let r = op
        .create_entity(EntitySpec::Subscription {
            topic: topic.clone(),
            name: sub.clone(),
            d: Description::new(sub_settings.clone()),
            rule: Some(rule.clone()),
        })
        .await
        .unwrap();
    logged(&log, &r, "7 create subscription with rule", &gated);
    let t_before = m.get_topic_description(&topic).await.unwrap().settings;
    assert_eq!(
        t_before.default_message_time_to_live,
        Some(Duration::from_secs(45 * 60)),
        "7 topic read back"
    );
    check_metadata(
        &t_before.user_metadata,
        &Some(format!("m3a topic {run}")),
        "7 topic",
    );
    let s_before = m.get_subscription(&topic, &sub).await.unwrap().settings;
    assert_eq!(
        s_before.lock_duration, sub_settings.lock_duration,
        "7 sub lock"
    );
    assert_eq!(
        s_before.max_delivery_count, sub_settings.max_delivery_count,
        "7 sub max delivery"
    );
    check_metadata(
        &s_before.user_metadata,
        &sub_settings.user_metadata,
        "7 subscription",
    );
    let got_rule = m.get_rule(&topic, &sub, "eu").await.unwrap().settings;
    assert_eq!(
        got_rule.filter, rule.filter,
        "7: the initial rule reads back"
    );
    match m.list_rules(&topic, &sub).await {
        Ok(rules) => {
            let names: Vec<_> = rules.iter().map(|r| r.name.as_str()).collect();
            assert_eq!(names, ["eu"], "7: the initial rule replaced $Default");
            eprintln!("ok 7 rules listed: {names:?} (no $Default)");
        }
        Err(e) => note(&format!(
            "7 listing the rules of a subscription fails on the emulator ({e}); the initial \
             rule was read back by name instead, and $Default's absence was not checked"
        )),
    }
    // Add and remove a second rule through the gate (single-rule reads only).
    let extra = RuleSpec {
        name: "billing".into(),
        filter: RuleFilter::Sql {
            expression: "kind = 'billing'".into(),
        },
        action: None,
    };
    match op
        .add_rule(EntityRef::subscription(&topic, &sub), extra.clone())
        .await
    {
        Ok(r) if r.outcome() == Outcome::Succeeded => {
            logged(&log, &r, "7 add rule", &gated);
            assert_eq!(
                m.get_rule(&topic, &sub, "billing")
                    .await
                    .unwrap()
                    .settings
                    .filter,
                extra.filter,
                "7 added rule reads back"
            );
            let r = op
                .remove_rule(EntityRef::subscription(&topic, &sub), "billing".into())
                .await
                .unwrap();
            logged(&log, &r, "7 remove rule", &gated);
            assert!(
                m.get_rule(&topic, &sub, "billing").await.is_err(),
                "7 removed rule is gone"
            );
        }
        other => note(&format!(
            "7 adding a rule to an existing subscription failed on the emulator: {other:?}; \
             rule add/remove not checked here (mockito covers it)"
        )),
    }

    // 8. Add and remove a SAS policy.
    let r = op
        .add_policy(
            EntityRef::queue(&q),
            "m3a-sender".into(),
            vec![AccessRight::Send],
        )
        .await
        .unwrap();
    logged(&log, &r, "8 add policy", &gated);
    let d = m.get_queue_description(&q).await.unwrap();
    if d.policies.iter().any(|p| p.key_name == "m3a-sender") {
        let p = d
            .policies
            .iter()
            .find(|p| p.key_name == "m3a-sender")
            .unwrap();
        assert_eq!(p.rights, vec![AccessRight::Send], "8 rights");
        let r = op
            .remove_policy(EntityRef::queue(&q), "m3a-sender".into())
            .await
            .unwrap();
        logged(&log, &r, "8 remove policy", &gated);
        let d = m.get_queue_description(&q).await.unwrap();
        assert!(
            !d.policies.iter().any(|p| p.key_name == "m3a-sender"),
            "8 policy removed"
        );
    } else {
        note(
            "8 the emulator accepted the policy PUT but reads back no policy (it drops SAS \
             policies); the read-back and the removal were not checked here (mockito covers \
             them; the cloud checklist has them)",
        );
    }
    let d = m.get_queue_description(&q).await.unwrap();
    assert_eq!(d.settings, expected, "8: a policy change kept the settings");

    // 9. Export to incumbent XML, delete, import, read back equal.
    let probe = explorer
        .collect_for_export(&[EntityRef::topic(&topic)])
        .await;
    let mut entities = explorer
        .collect_for_export(&[EntityRef::queue(&q), EntityRef::queue(&moved)])
        .await
        .unwrap();
    match probe {
        Ok(t) => {
            eprintln!("ok 9 the topic was collected for export with its rules");
            entities.extend(t)
        }
        Err(e) => {
            note(&format!(
                "9 exporting a topic fails on the emulator ({e}: it lists rules); the topic \
                 entry was built from single reads (topic, subscription, rule by name) instead"
            ));
            entities.push(ExportEntity::Topic {
                name: topic.clone(),
                d: m.get_topic_description(&topic).await.unwrap(),
                subscriptions: vec![ExportSubscription {
                    name: sub.clone(),
                    d: m.get_subscription(&topic, &sub).await.unwrap(),
                    rules: vec![m.get_rule(&topic, &sub, "eu").await.unwrap().settings],
                }],
            });
        }
    }
    let xml = export_entities("sbemulatorns", &entities, false).unwrap();
    assert!(
        xml.starts_with("<Entities xmlns=\"http://schemas.microsoft.com/servicebusexplorer\""),
        "9 root: {xml}"
    );
    assert!(
        !xml.contains("PrimaryKey"),
        "9: no keys without include_keys"
    );
    let parsed = parse_entities(&xml).unwrap();
    assert_eq!(parsed.len(), 4, "9: two queues, a topic, a subscription");
    let r = op
        .delete_entities(vec![
            EntityRef::queue(&q),
            EntityRef::queue(&moved),
            EntityRef::topic(&topic),
        ])
        .await
        .unwrap();
    logged(&log, &r, "9 delete before import", &gated);
    for name in [&q, &moved] {
        assert!(
            matches!(m.get_queue_description(name).await, Err(Error::NotFound(_))),
            "9 {name} deleted"
        );
    }
    let cancel = CancellationToken::new();
    let r = op
        .import_entities(parsed, false, &NoProgress, &cancel)
        .await
        .unwrap();
    logged(&log, &r, "9 import", &gated);
    assert_eq!(r.items.len(), 4, "9: four entities imported: {r:?}");
    assert_eq!(
        m.get_queue_description(&q).await.unwrap().settings,
        expected,
        "9: the queue reads back equal"
    );
    assert_eq!(
        m.get_queue_description(&moved).await.unwrap().settings,
        expected,
        "9: the renamed queue reads back equal"
    );
    assert_eq!(
        m.get_topic_description(&topic).await.unwrap().settings,
        t_before,
        "9: the topic reads back equal"
    );
    assert_eq!(
        m.get_subscription(&topic, &sub).await.unwrap().settings,
        s_before,
        "9: the subscription reads back equal"
    );
    assert_eq!(
        m.get_rule(&topic, &sub, "eu")
            .await
            .unwrap()
            .settings
            .filter,
        rule.filter,
        "9: the rule reads back equal"
    );
    // Importing the same file again skips every entity: they exist.
    let again = parse_entities(&xml).unwrap();
    let r = op
        .import_entities(again, false, &NoProgress, &cancel)
        .await
        .unwrap();
    assert_eq!(
        r.succeeded(),
        0,
        "9: a second import creates nothing: {r:?}"
    );
    assert!(
        r.items
            .iter()
            .all(|i| format!("{:?}", i.outcome).contains("already exists")),
        "9: every entity is skipped as existing: {r:?}"
    );
    assert_logged(
        &log,
        r.op_id.as_deref().unwrap(),
        r.outcome(),
        "9 re-import",
    );

    // 11. The listener (before the bulk delete, on the imported queue).
    let lq = EntityPath::queue(&q);
    send_n(&mut conn, &q, &run, "listen", 5).await;
    assert_eq!(peek_until(&explorer, &q, 5).await, 5, "11 seeded");
    // Watch: shows the five and leaves them, delivery count 0. Not gated.
    let watch = StopAfter {
        want: 5,
        seen: AtomicU64::new(0),
        cancel: CancellationToken::new(),
    };
    let entries_before = log.list(&ActivityQuery::default()).unwrap().len();
    let w = tokio::time::timeout(
        Duration::from_secs(30),
        op.listen(
            ListenRequest {
                entity: lq.clone(),
                sub_queue: SubQueue::Active,
                sessions: ListenSessions::None,
                mode: ListenMode::Watch,
                concurrency: 1,
                prefetch: 0,
            },
            &watch,
            &watch.cancel,
        ),
    )
    .await
    .expect("11 watch stops within 30 s")
    .unwrap();
    assert_eq!(w.received, 5, "11 watch received: {:?}", w.report);
    assert_eq!(w.removed, 0, "11 watch removed nothing");
    eprintln!(
        "ok 11 watch: {} received, {} removed",
        w.received, w.removed
    );
    assert_eq!(
        log.list(&ActivityQuery::default()).unwrap().len(),
        entries_before,
        "11: watch is not gated"
    );
    let left = peek_all(&explorer, &q).await;
    assert_eq!(left.len(), 5, "11: watch left the messages");
    assert!(
        left.iter().all(|m| m.delivery_count == 0),
        "11: watch left the delivery count at 0: {:?}",
        left.iter().map(|m| m.delivery_count).collect::<Vec<_>>()
    );
    // Complete with two receivers until five are received, then cancel.
    let complete = StopAfter {
        want: 5,
        seen: AtomicU64::new(0),
        cancel: CancellationToken::new(),
    };
    let started = tokio::time::Instant::now();
    let c = tokio::time::timeout(
        Duration::from_secs(60),
        op.listen(
            ListenRequest {
                entity: lq.clone(),
                sub_queue: SubQueue::Active,
                sessions: ListenSessions::None,
                mode: ListenMode::Complete,
                concurrency: 2,
                prefetch: 0,
            },
            &complete,
            &complete.cancel,
        ),
    )
    .await
    .expect("11 complete listener stops within 60 s")
    .unwrap();
    eprintln!(
        "11 complete listener: {} received, {} removed in {:?}",
        c.received,
        c.removed,
        started.elapsed()
    );
    assert_eq!(c.received, 5, "11 complete received: {:?}", c.report);
    assert_eq!(c.removed, 5, "11 complete removed");
    assert!(c.report.items.is_empty(), "11 no failures: {:?}", c.report);
    let op_id = c.report.op_id.as_deref().expect("11 listener op id");
    assert_logged(&log, op_id, c.report.outcome(), "11 listen complete");
    assert_eq!(c.report.outcome(), Outcome::Succeeded, "11 outcome");
    gated.fetch_add(1, Ordering::Relaxed);
    assert_eq!(
        peek_until(&explorer, &q, 0).await,
        0,
        "11: the queue is empty"
    );

    // 10. Bulk delete of everything the run created.
    let r = op
        .delete_entities(vec![
            EntityRef::queue(&q),
            EntityRef::queue(&moved),
            EntityRef::topic(&topic),
        ])
        .await
        .unwrap();
    logged(&log, &r, "10 bulk delete", &gated);
    assert_eq!(r.items.len(), 3, "10 three items: {r:?}");
    for name in [&q, &moved] {
        assert!(
            matches!(m.get_queue_description(name).await, Err(Error::NotFound(_))),
            "10 {name} deleted"
        );
    }
    assert!(
        matches!(
            m.get_topic_description(&topic).await,
            Err(Error::NotFound(_))
        ),
        "10 topic deleted"
    );

    // 12. Every gated step left a started and a finished entry (checked per step above).
    let all = log.list(&ActivityQuery::default()).unwrap();
    let started = all.iter().filter(|e| e.phase == Phase::Started).count() as u64;
    let finished = all.iter().filter(|e| e.phase == Phase::Finished).count() as u64;
    let steps = gated.load(Ordering::Relaxed) + 1; // + the re-import
    eprintln!("12 activity: {steps} gated steps, {started} started, {finished} finished");
    assert_eq!(started, steps, "12: one started entry per gated step");
    assert_eq!(finished, steps, "12: one finished entry per gated step");

    // A delete of a missing queue: the emulator answers 400, the service 404.
    match m.delete_queue(&q).await {
        Err(Error::NotFound(_)) => {}
        other => note(&format!(
            "a DELETE of a missing queue is not NotFound on the emulator ({other:?}); the \
             service answers 404 (mockito covers the mapping)"
        )),
    }
    drop(cleanup);
    eprintln!("m3a_manage: all steps passed");
}
