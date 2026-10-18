//! M2a emulator coverage: the link semantics the operate features rest on
//! (`m2a_link_semantics`), and every operate op end to end (`m2a_operate`).
//! `m2a_link_semantics` uses the existing queue `m1a-browse`; `m2a_operate` uses the `m2a-*`
//! entities (see testing/emulator/Config.json). Both tag what they send with a per-run
//! property and empty what they rely on first, so leftovers from earlier runs are harmless.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m2a -- --nocapture`

use dispatched_core::activity::{ActivityQuery, Outcome, Phase};
use dispatched_core::filter::{FilterSpec, MessageFilter};
use dispatched_core::messaging::{
    MessageIdSource, OutgoingBody, OutgoingMessage, PeekedMessage, PropValue, ServiceBusConnection,
    Settlement, TypedValue, testing,
};
use dispatched_core::ops::{
    ImportOptions, LocateLimits, MessageSelection, ResubmitSpec, SelectSpec,
};
use dispatched_core::safety::{Actor, Gate, ProfileRef};
use dispatched_core::{
    ActivityLog, ConnectionString, EntityKind, EntityPath, Error, Explorer, ExportFormat,
    ItemOutcome, JobReport, Namespace, NoProgress, Operator, PeekOptions, PeekView, PurgeScope,
    ReceiveAction, ReceiveRequest, SendRequest, SubQueue, read_messages, write_messages,
};
use horizon_azure_kit::safety::{AllowAll, CancellationToken, DenyAll, Origin, Policy};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

#[tokio::test(flavor = "multi_thread")]
async fn m2a_link_semantics() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();
    let p = testing::probe_m2a_link_semantics(&mut conn, "m1a-browse")
        .await
        .unwrap();
    eprintln!("link probe: {p:?}");
    assert_eq!(
        p.released_delivery_count, 0,
        "(a) release raised the delivery count: stop before Task 13 and ask for a ruling"
    );
    assert_eq!(
        p.abandoned_delivery_count, 1,
        "(b) control: abandon raises the delivery count"
    );
    assert!(
        p.management_complete_removed,
        "(c) update-disposition did not complete a link-received message: {:?}",
        p.management_complete_error
    );
    assert_eq!(
        p.drain_returned, 2,
        "(d) receive-and-delete with credit 3 on 2 messages returns 2"
    );
    assert!(
        p.drain_credit_left <= 1,
        "(d) the shortfall is never above the outstanding credit"
    );
    assert_eq!(
        p.left_after_drain, 0,
        "(d) nothing arrived or was left after the drain"
    );
    assert!(
        p.late_arrival_kept,
        "(d) a message sent after the drain was removed while the link stayed open: the drain did not stop the link"
    );
    assert_eq!(
        p.removed_unreturned, 0,
        "(d) close found no removed-but-unreturned messages"
    );
    assert_eq!(
        p.close_uncertain, 0,
        "(d) the close (drain, sweep, detach) completed, settling the unconfirmed drain"
    );
}

const OPS: &str = "m2a-ops";
const TARGET: &str = "m2a-target";
const SESSIONS: &str = "m2a-sessions";
const PURGE_A: &str = "m2a-purge-a";
const PURGE_B: &str = "m2a-purge-b";
const TOPIC: &str = "m2a-topic";
const IDLE: Duration = Duration::from_secs(1);
const WAIT: Duration = Duration::from_secs(10);

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

/// The messages of this run (property `run`), and with `tag` only those of one step.
fn ours(run: &str, tag: Option<&str>) -> MessageFilter {
    let property = match tag {
        Some(t) => format!("run = '{run}' AND tag = '{t}'"),
        None => format!("run = '{run}'"),
    };
    FilterSpec {
        property: Some(property),
        ..Default::default()
    }
    .build()
    .unwrap()
}

async fn peek_ours(
    explorer: &Explorer,
    entity: &EntityPath,
    view: PeekView,
    filter: MessageFilter,
) -> Vec<PeekedMessage> {
    explorer
        .peek(
            entity,
            view,
            &PeekOptions::new(1_000).scan_limit(100_000).filter(filter),
        )
        .await
        .unwrap()
        .messages
}

/// [`peek_ours`] again until it shows `want` messages or 10 s pass, returning the last peek.
/// The emulator applies some writes (topic fan-out, dead-lettering) a moment after it
/// accepted them, so a count read at once can be short; the caller still asserts it exactly.
/// Only for messages that should appear: once the count is reached, one more peek after
/// 500 ms is returned, so a late extra copy (a duplicate send) still fails the caller.
async fn peek_want(
    explorer: &Explorer,
    entity: &EntityPath,
    view: PeekView,
    filter: MessageFilter,
    want: usize,
) -> Vec<PeekedMessage> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let got = peek_ours(explorer, entity, view, filter.clone()).await;
        if got.len() == want {
            tokio::time::sleep(Duration::from_millis(500)).await;
            return peek_ours(explorer, entity, view, filter).await;
        }
        if tokio::time::Instant::now() >= deadline {
            eprintln!(
                "GATE-NOTE peek of {} {view:?} still shows {} of {want} after 10 s",
                entity.base(),
                got.len()
            );
            return got;
        }
        eprintln!(
            "GATE-NOTE peek of {} {view:?} shows {} of {want}; waiting",
            entity.base(),
            got.len()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// `n` text messages to `address`, tagged with the run and `tag`.
async fn seed(conn: &mut ServiceBusConnection, address: &str, run: &str, tag: &str, n: usize) {
    for i in 0..n {
        testing::send_text(
            conn,
            address,
            &format!("{{\"run\":\"{run}\",\"tag\":\"{tag}\",\"i\":{i}}}"),
            &[("run", run), ("tag", tag), ("i", &i.to_string())],
        )
        .await
        .unwrap();
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

/// The op id of the newest activity entry (for operations that return no report).
fn last_op_id(log: &ActivityLog) -> String {
    log.list(&ActivityQuery {
        limit: 1,
        ..Default::default()
    })
    .unwrap()
    .remove(0)
    .op_id
}

/// The report's op id, after checking its pair in the log.
fn logged_report(log: &ActivityLog, report: &JobReport, step: &str) {
    let op_id = report
        .op_id
        .as_deref()
        .unwrap_or_else(|| panic!("{step}: the report has no op id"));
    assert_logged(log, op_id, report.outcome(), step);
    assert_eq!(report.outcome(), Outcome::Succeeded, "{step}: {report:?}");
}

/// The activity op ids a multi-entity purge report names in its notes.
fn purge_op_ids(report: &JobReport) -> Vec<String> {
    report
        .notes
        .iter()
        .filter_map(|n| n.split("activity op ").nth(1))
        .map(|rest| rest.split_whitespace().next().unwrap().to_string())
        .collect()
}

fn select(entity: &EntityPath, view: PeekView, seqs: Vec<i64>) -> SelectSpec {
    SelectSpec {
        entity: entity.clone(),
        view,
        selection: MessageSelection::Sequences(seqs),
        limits: LocateLimits {
            idle: Duration::from_secs(3),
            ..Default::default()
        },
    }
}

fn receive(entity: &EntityPath, max: u32, action: ReceiveAction) -> ReceiveRequest {
    ReceiveRequest {
        entity: entity.clone(),
        sub_queue: SubQueue::Active,
        session: None,
        max,
        wait: Duration::from_secs(5),
        action,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn m2a_operate() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    eprintln!("m2a_operate run {run}");
    let explorer = Arc::new(Explorer::new(ns.clone()).unwrap());
    let log = Arc::new(ActivityLog::open_in_memory().unwrap());
    let op = Operator::new(explorer.clone(), gate(Arc::new(AllowAll), log.clone()));
    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();
    let (np, cancel) = (NoProgress, CancellationToken::new());
    let q = EntityPath::queue(OPS);
    let target = EntityPath::queue(TARGET);

    // Receives below take whatever is next: start from empty queues.
    for address in [
        OPS.to_string(),
        format!("{OPS}/$DeadLetterQueue"),
        TARGET.to_string(),
        format!("{TOPIC}/Subscriptions/m2a-sub"),
    ] {
        testing::drain(&mut conn, &address, IDLE).await.unwrap();
    }

    // 1. Send: three copies, incrementing ids, typed properties, TTL, ContentType.
    let guid = uuid::Uuid::new_v4();
    let at = time::OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap();
    let span = time::Duration::seconds(90);
    let mut properties = BTreeMap::new();
    properties.insert("run".to_string(), TypedValue::String(run.clone()));
    properties.insert("tag".to_string(), TypedValue::String("s1".into()));
    properties.insert("n".to_string(), TypedValue::Int(42));
    properties.insert("span".to_string(), TypedValue::TimeSpan(span));
    properties.insert("g".to_string(), TypedValue::Guid(guid));
    properties.insert("at".to_string(), TypedValue::DateTime(at));
    let template = OutgoingMessage {
        body: OutgoingBody::Bytes(format!("{{\"run\":\"{run}\"}}").into_bytes()),
        content_type: Some("application/json".into()),
        time_to_live: Some(Duration::from_secs(600)),
        properties,
        ..Default::default()
    };
    let report = op
        .send(
            SendRequest {
                target: OPS.into(),
                template: template.clone(),
                body: None,
                ids: MessageIdSource::Increment {
                    base: format!("{run}-0001"),
                },
                count: 3,
                schedule: None,
                batch: false,
            },
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "1 send");
    assert_eq!(report.succeeded(), 3);
    let sent = peek_want(&explorer, &q, PeekView::Active, ours(&run, Some("s1")), 3).await;
    let mut ids: Vec<String> = sent.iter().filter_map(|m| m.message_id.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        [1, 2, 3].map(|i| format!("{run}-000{i}")),
        "1: incrementing ids"
    );
    for m in &sent {
        let p = &m.application_properties;
        assert_eq!(p["n"], PropValue::Int(42), "1: int");
        assert_eq!(p["span"], PropValue::TimeSpan(span), "1: timespan");
        assert_eq!(p["g"], PropValue::Uuid(guid), "1: guid");
        assert_eq!(p["at"], PropValue::Timestamp(at), "1: datetime");
        assert_eq!(m.time_to_live, Some(Duration::from_secs(600)), "1: TTL");
        assert_eq!(m.content_type.as_deref(), Some("application/json"));
    }

    // 2. Batch to a topic: five, all in the subscription.
    let report = op
        .send(
            SendRequest {
                target: TOPIC.into(),
                template: OutgoingMessage {
                    properties: BTreeMap::from([
                        ("run".to_string(), TypedValue::String(run.clone())),
                        ("tag".to_string(), TypedValue::String("s2".into())),
                    ]),
                    ..OutgoingMessage::text("batch")
                },
                body: None,
                ids: MessageIdSource::Random,
                count: 5,
                schedule: None,
                batch: true,
            },
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "2 batch send");
    let sub = EntityPath::subscription(TOPIC, "m2a-sub");
    assert_eq!(
        peek_want(&explorer, &sub, PeekView::Active, ours(&run, Some("s2")), 5)
            .await
            .len(),
        5,
        "2: five in the subscription"
    );

    // 3. Schedule two for +30 min, then cancel them and one unknown number.
    let report = op
        .send(
            SendRequest {
                target: OPS.into(),
                template: OutgoingMessage {
                    properties: BTreeMap::from([
                        ("run".to_string(), TypedValue::String(run.clone())),
                        ("tag".to_string(), TypedValue::String("s3".into())),
                    ]),
                    ..OutgoingMessage::text("scheduled")
                },
                body: None,
                ids: MessageIdSource::Random,
                count: 2,
                schedule: Some(time::OffsetDateTime::now_utc() + time::Duration::minutes(30)),
                batch: false,
            },
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "3 schedule");
    let mut scheduled: Vec<i64> = report
        .items
        .iter()
        .map(|i| {
            i.new_sequence_number
                .expect("3: a sequence number per index")
        })
        .collect();
    scheduled.sort_unstable();
    assert_eq!(scheduled.len(), 2);
    let mut in_view: Vec<i64> = peek_want(
        &explorer,
        &q,
        PeekView::Scheduled,
        ours(&run, Some("s3")),
        2,
    )
    .await
    .iter()
    .map(|m| m.sequence_number)
    .collect();
    in_view.sort_unstable();
    assert_eq!(in_view, scheduled, "3: the scheduled view shows them");
    let unknown = scheduled[1] + 10_000_000;
    let report = op
        .cancel_scheduled(&q, &[scheduled[0], scheduled[1], unknown], &np, &cancel)
        .await
        .unwrap();
    logged_report(&log, &report, "3 cancel");
    assert_eq!(report.succeeded(), 2, "3: {report:?}");
    assert_eq!(
        report
            .items
            .iter()
            .filter(|i| i.outcome == ItemOutcome::Missing)
            .map(|i| i.sequence_number)
            .collect::<Vec<_>>(),
        [Some(unknown)],
        "3: the unknown number is missing"
    );
    assert!(
        peek_ours(&explorer, &q, PeekView::Scheduled, ours(&run, Some("s3")))
            .await
            .is_empty(),
        "3: cancelled"
    );

    // 4. Peek-lock receive, one message per settlement.
    testing::drain(&mut conn, OPS, IDLE).await.unwrap();
    seed(&mut conn, OPS, &run, "s4", 4).await;
    let mut settled = Vec::new();
    let mut manual_dlq_seq = None;
    for how in [
        Settlement::Abandon,
        Settlement::Defer,
        Settlement::DeadLetter {
            reason: Some("Manual".into()),
            description: Some("gate".into()),
        },
        Settlement::Complete,
    ] {
        let out = op
            .receive(
                receive(&q, 1, ReceiveAction::Settle(how.clone())),
                &np,
                &cancel,
            )
            .await
            .unwrap();
        logged_report(&log, &out.report, &format!("4 receive {how:?}"));
        assert_eq!(out.messages.len(), 1, "4 {how:?}: one message");
        let m = &out.messages[0];
        assert_eq!(
            m.application_properties.get("run"),
            Some(&PropValue::String(run.clone()))
        );
        let seq = m.sequence_number;
        match &how {
            Settlement::Abandon => {
                let after = explorer
                    .peek_message(&q, PeekView::Active, seq, None)
                    .await
                    .unwrap()
                    .expect("4: the abandoned message is back");
                assert_eq!(after.delivery_count, 1, "4: abandon raises the count");
            }
            Settlement::Defer => {
                let after = explorer
                    .peek_message(&q, PeekView::Deferred, seq, None)
                    .await
                    .unwrap()
                    .expect("4: the deferred message");
                assert_eq!(after.state, dispatched_core::MessageState::Deferred);
            }
            Settlement::DeadLetter { .. } => {
                let id = m.message_id.clone();
                let dl = peek_want(
                    &explorer,
                    &q,
                    PeekView::DeadLetter,
                    ours(&run, Some("s4")),
                    1,
                )
                .await
                .into_iter()
                .find(|d| d.message_id == id)
                .expect("4: in the DLQ");
                assert_eq!(dl.dead_letter_reason.as_deref(), Some("Manual"));
                assert_eq!(dl.dead_letter_description.as_deref(), Some("gate"));
                manual_dlq_seq = Some(dl.sequence_number);
            }
            Settlement::Complete => {
                for view in [PeekView::Active, PeekView::Deferred] {
                    assert!(
                        explorer
                            .peek_message(&q, view, seq, None)
                            .await
                            .unwrap()
                            .is_none(),
                        "4: completed message gone"
                    );
                }
            }
        }
        settled.push((how, seq));
    }
    let deferred_seq = settled[1].1;

    // 5. Receive-and-delete takes exactly what was asked for.
    testing::drain(&mut conn, OPS, IDLE).await.unwrap();
    seed(&mut conn, OPS, &run, "s5", 5).await;
    let out = op
        .receive(receive(&q, 3, ReceiveAction::Delete), &np, &cancel)
        .await
        .unwrap();
    eprintln!("5 receive-and-delete notes: {:?}", out.report.notes);
    logged_report(&log, &out.report, "5 receive-and-delete");
    assert_eq!(out.messages.len(), 3, "5: exactly three returned");
    for m in &out.messages {
        assert_eq!(
            (
                m.application_properties.get("run"),
                m.application_properties.get("tag")
            ),
            (
                Some(&PropValue::String(run.clone())),
                Some(&PropValue::String("s5".into()))
            ),
            "5: a message of this step"
        );
    }
    assert_eq!(
        peek_ours(&explorer, &q, PeekView::Active, ours(&run, Some("s5")))
            .await
            .len(),
        2,
        "5: exactly two remain"
    );

    // 6. Settle the deferred message.
    let report = op
        .settle_deferred(
            &q,
            SubQueue::Active,
            &[deferred_seq],
            None,
            Settlement::Complete,
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "6 settle deferred");
    assert_eq!(report.succeeded(), 1);
    assert!(
        explorer
            .peek_message(&q, PeekView::Deferred, deferred_seq, None)
            .await
            .unwrap()
            .is_none(),
        "6: gone"
    );

    // 7. Renew a lock (proves the lock-token byte order).
    testing::drain(&mut conn, OPS, IDLE).await.unwrap();
    seed(&mut conn, OPS, &run, "s7", 1).await;
    let (before, after) = testing::renew_lock_once(&mut conn, OPS).await.unwrap();
    eprintln!("7 renew-lock: {before} -> {after}");

    // 8. Delete by sequence number among active messages (#872).
    testing::drain(&mut conn, OPS, IDLE).await.unwrap();
    seed(&mut conn, OPS, &run, "s8", 4).await;
    let mut s8: Vec<i64> = peek_ours(&explorer, &q, PeekView::Active, ours(&run, Some("s8")))
        .await
        .iter()
        .map(|m| m.sequence_number)
        .collect();
    s8.sort_unstable();
    assert_eq!(s8.len(), 4);
    let report = op
        .delete(
            select(&q, PeekView::Active, vec![s8[1], s8[3]]),
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "8 delete");
    assert_eq!(report.succeeded(), 2, "8: {report:?}");
    let left = peek_ours(&explorer, &q, PeekView::Active, ours(&run, Some("s8"))).await;
    let mut left_seqs: Vec<(i64, u32)> = left
        .iter()
        .map(|m| (m.sequence_number, m.delivery_count))
        .collect();
    left_seqs.sort_unstable();
    assert_eq!(
        left_seqs,
        [(s8[0], 0), (s8[2], 0)],
        "8: the 1st and 3rd remain with delivery count 0 (release must not raise it: if the \
         count is 1, STOP and report, Decision 4 depends on it)"
    );

    // 9. Move from the DLQ to another queue.
    seed(&mut conn, OPS, &run, "s9", 2).await;
    for _ in 0..2 {
        assert!(
            testing::dead_letter_next_matching(&mut conn, OPS, ("tag", "s9"), "Seed", "move", WAIT)
                .await
                .unwrap()
        );
    }
    let dl = peek_want(
        &explorer,
        &q,
        PeekView::DeadLetter,
        ours(&run, Some("s9")),
        2,
    )
    .await;
    assert_eq!(dl.len(), 2, "9: two dead-lettered");
    let mut moved_ids: Vec<String> = dl.iter().filter_map(|m| m.message_id.clone()).collect();
    moved_ids.sort();
    let report = op
        .move_messages(
            select(
                &q,
                PeekView::DeadLetter,
                dl.iter().map(|m| m.sequence_number).collect(),
            ),
            TARGET.into(),
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "9 move");
    assert_eq!(report.succeeded(), 2, "9: {report:?}");
    assert!(
        peek_ours(&explorer, &q, PeekView::DeadLetter, ours(&run, Some("s9")))
            .await
            .is_empty(),
        "9: none of this run left in the DLQ"
    );
    let mut at_target: Vec<String> = peek_want(
        &explorer,
        &target,
        PeekView::Active,
        ours(&run, Some("s9")),
        2,
    )
    .await
    .iter()
    .filter_map(|m| m.message_id.clone())
    .collect();
    at_target.sort();
    assert_eq!(at_target, moved_ids, "9: moved with their MessageIds");

    // 10. Resubmit with edits, keeping the original.
    seed(&mut conn, OPS, &run, "s10", 1).await;
    assert!(
        testing::dead_letter_next_matching(
            &mut conn,
            OPS,
            ("tag", "s10"),
            "Seed",
            "resubmit",
            WAIT
        )
        .await
        .unwrap()
    );
    let original = peek_want(
        &explorer,
        &q,
        PeekView::DeadLetter,
        ours(&run, Some("s10")),
        1,
    )
    .await
    .pop()
    .expect("10: dead-lettered");
    let mut edits = dispatched_core::Edits::default();
    edits
        .set
        .insert("retried".to_string(), TypedValue::Bool(true));
    let report = op
        .resubmit(
            ResubmitSpec {
                source: select(&q, PeekView::DeadLetter, vec![original.sequence_number]),
                target: OPS.into(),
                edits,
                new_message_id: true,
                remove_original: false,
                keep_dead_letter_properties: false,
            },
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "10 resubmit");
    assert_eq!(report.succeeded(), 1, "10: {report:?}");
    assert_eq!(
        peek_ours(&explorer, &q, PeekView::DeadLetter, ours(&run, Some("s10")))
            .await
            .len(),
        1,
        "10: the original is kept"
    );
    let copies = peek_want(&explorer, &q, PeekView::Active, ours(&run, Some("s10")), 1).await;
    assert_eq!(copies.len(), 1, "10: one copy");
    assert_ne!(copies[0].message_id, original.message_id, "10: a new id");
    assert_eq!(
        copies[0].application_properties.get("retried"),
        Some(&PropValue::Bool(true)),
        "10: edited"
    );

    // 11. Sessions: the session-required refusal, session state, delete inside a session.
    let sq = EntityPath::queue(SESSIONS);
    let (a, b) = (format!("{run}-A"), format!("{run}-B"));
    for s in [&a, &a, &b] {
        testing::send_session_text(
            &mut conn,
            SESSIONS,
            s,
            "in a session",
            &[("run", &run), ("tag", "s11")],
        )
        .await
        .unwrap();
    }
    let refused = op
        .receive(
            receive(&sq, 1, ReceiveAction::Settle(Settlement::Abandon)),
            &np,
            &cancel,
        )
        .await;
    eprintln!("11 plain receive on a session queue: {refused:?}");
    match &refused {
        Err(Error::Invalid(m)) if m.contains("requires sessions") => {}
        other => panic!(
            "11: a plain receive on a session queue must be the session-required error, got {other:?}"
        ),
    }
    assert_logged(
        &log,
        &last_op_id(&log),
        Outcome::Failed,
        "11 refused receive",
    );
    op.set_session_state(&sq, &a, Some(b"step-2".to_vec()))
        .await
        .unwrap();
    assert_logged(&log, &last_op_id(&log), Outcome::Succeeded, "11 set state");
    assert_eq!(
        op.session_state(&sq, &a).await.unwrap(),
        Some(b"step-2".to_vec()),
        "11: state set"
    );
    assert_eq!(
        explorer.session_state(&sq, &a).await.unwrap(),
        Some(b"step-2".to_vec()),
        "11: the Explorer reads it too"
    );
    op.set_session_state(&sq, &a, None).await.unwrap();
    assert_logged(
        &log,
        &last_op_id(&log),
        Outcome::Succeeded,
        "11 clear state",
    );
    assert_eq!(
        op.session_state(&sq, &a).await.unwrap(),
        None,
        "11: cleared"
    );
    let in_b = MessageFilter {
        session_id: Some(b.clone()),
        ..ours(&run, Some("s11"))
    };
    let b_msg = peek_want(&explorer, &sq, PeekView::Active, in_b.clone(), 1)
        .await
        .pop()
        .expect("11: session B's message");
    let report = op
        .delete(
            select(&sq, PeekView::Active, vec![b_msg.sequence_number]),
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "11 delete in session");
    assert_eq!(report.succeeded(), 1, "11: {report:?}");
    assert!(
        peek_ours(&explorer, &sq, PeekView::Active, in_b)
            .await
            .is_empty(),
        "11: session B's message gone"
    );
    let in_a = MessageFilter {
        session_id: Some(a.clone()),
        ..ours(&run, Some("s11"))
    };
    assert_eq!(
        peek_ours(&explorer, &sq, PeekView::Active, in_a)
            .await
            .len(),
        2,
        "11: session A untouched by the delete in session B"
    );

    // 12. DLQ summary.
    let summary = explorer
        .dlq_summary(&q, SubQueue::DeadLetter)
        .await
        .unwrap();
    // The DLQ was drained at the start, so the Manual/gate group is exactly this run's one
    // step 4 message.
    let manual_dlq_seq = manual_dlq_seq.expect("4: dead-lettered");
    let groups: Vec<_> = summary
        .groups
        .iter()
        .filter(|g| {
            g.reason.as_deref() == Some("Manual") && g.description.as_deref() == Some("gate")
        })
        .collect();
    assert!(
        matches!(groups.as_slice(), [g] if g.count == 1
            && g.first_sequence == manual_dlq_seq
            && g.last_sequence == manual_dlq_seq),
        "12: one Manual/gate group holding this run's message {manual_dlq_seq}: {summary:?}"
    );

    // 13. Purge across entities, bounded by the plan.
    for address in [
        PURGE_A.to_string(),
        format!("{PURGE_A}/$DeadLetterQueue"),
        PURGE_B.to_string(),
        format!("{PURGE_B}/$DeadLetterQueue"),
    ] {
        testing::drain(&mut conn, &address, IDLE).await.unwrap();
    }
    let (pa, pb) = (EntityPath::queue(PURGE_A), EntityPath::queue(PURGE_B));
    seed(&mut conn, PURGE_A, &run, "s13", 3).await;
    seed(&mut conn, PURGE_B, &run, "s13", 2).await;
    assert!(
        testing::dead_letter_next(&mut conn, PURGE_B, "Seed", "purge", WAIT)
            .await
            .unwrap()
    );
    let plan_a = op.plan_purge(&pa, PurgeScope::All).await.unwrap();
    let plan_b = op.plan_purge(&pb, PurgeScope::All).await.unwrap();
    eprintln!("13 plans: {plan_a:?} {plan_b:?}");
    assert_eq!(plan_a.total.bound(), Some(3), "13: {plan_a:?}");
    assert_eq!(plan_b.total.bound(), Some(2), "13: {plan_b:?}");
    assert!(!plan_a.deferred_truncated && !plan_b.deferred_truncated);
    seed(&mut conn, PURGE_A, &run, "s13-late", 1).await;
    let report = op.purge(vec![plan_a, plan_b], &np, &cancel).await.unwrap();
    eprintln!("13 purge notes: {:?}", report.notes);
    let ids = purge_op_ids(&report);
    assert_eq!(
        ids.len(),
        2,
        "13: one activity op per entity: {:?}",
        report.notes
    );
    for id in &ids {
        assert_logged(&log, id, Outcome::Succeeded, "13 purge");
    }
    assert_eq!(report.outcome(), Outcome::Succeeded, "13: {report:?}");
    let everything = || MessageFilter::default();
    for view in [PeekView::Active, PeekView::DeadLetter, PeekView::Deferred] {
        assert!(
            peek_ours(&explorer, &pb, view, everything())
                .await
                .is_empty(),
            "13: {PURGE_B} {view:?} empty"
        );
    }
    assert_eq!(
        peek_ours(&explorer, &pa, PeekView::Active, everything())
            .await
            .len(),
        1,
        "13: the bound held: one message left in {PURGE_A}"
    );
    assert!(
        report.notes.iter().any(|n| n.contains("more remain")),
        "13: the 'more remain' note: {:?}",
        report.notes
    );

    // 14. Export two messages, import them elsewhere.
    let exported = peek_ours(&explorer, &q, PeekView::Active, ours(&run, Some("s8"))).await;
    assert_eq!(exported.len(), 2);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.ndjson");
    write_messages(
        std::fs::File::create(&path).unwrap(),
        &exported,
        ExportFormat::Ndjson,
    )
    .unwrap();
    let imported = read_messages(&path).unwrap();
    assert_eq!(imported.len(), 2);
    let report = op
        .import(
            TARGET.into(),
            imported,
            ImportOptions::default(),
            &np,
            &cancel,
        )
        .await
        .unwrap();
    logged_report(&log, &report, "14 import");
    assert_eq!(report.succeeded(), 2, "14: {report:?}");
    let landed = peek_want(
        &explorer,
        &target,
        PeekView::Active,
        ours(&run, Some("s8")),
        2,
    )
    .await;
    assert_eq!(landed.len(), 2, "14: two imported");
    for m in &exported {
        let copy = landed
            .iter()
            .find(|l| l.message_id == m.message_id)
            .expect("14: imported with its MessageId");
        assert_eq!(copy.body, m.body, "14: bodies equal");
    }

    // 15. R23: entity kinds.
    assert_eq!(
        explorer.resolve_kind(&q).await.unwrap(),
        Some(EntityKind::Queue)
    );
    assert_eq!(
        explorer
            .resolve_kind(&EntityPath::queue(TOPIC))
            .await
            .unwrap(),
        Some(EntityKind::Topic)
    );

    // 16. Denied: nothing is touched and the log says so.
    let denying = Operator::new(explorer.clone(), gate(Arc::new(DenyAll), log.clone()));
    let plan = denying.plan_purge(&pa, PurgeScope::All).await.unwrap();
    assert_eq!(plan.total.bound(), Some(1));
    let report = denying.purge(vec![plan], &np, &cancel).await.unwrap();
    eprintln!("16 denied purge: {report:?}");
    // The shape the CLI's purge exit code reads (Task 20): one Skipped item without a
    // sequence number whose reason starts "denied: ", and one "(denied)" op note.
    assert!(
        matches!(
            report.items.as_slice(),
            [i] if i.sequence_number.is_none()
                && i.entity == PURGE_A
                && matches!(&i.outcome, ItemOutcome::Skipped { reason } if reason.starts_with("denied: "))
        ),
        "16: one skipped item for the denial: {report:?}"
    );
    assert!(
        matches!(report.notes.as_slice(), [n] if n.starts_with(&format!("{PURGE_A}: activity op op-")) && n.ends_with(" (denied)")),
        "16: {:?}",
        report.notes
    );
    let denied_id = purge_op_ids(&report).pop().expect("16: the denial's op id");
    let entries = log
        .list(&ActivityQuery {
            op_id: Some(denied_id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        matches!(entries.as_slice(), [e] if e.phase == Phase::Finished
            && e.outcome == Outcome::Denied
            && e.error.as_deref().is_some_and(|m| m.starts_with("denied: "))),
        "16: exactly one denied entry and no started entry: {entries:?}"
    );
    assert_eq!(
        peek_ours(&explorer, &pa, PeekView::Active, everything())
            .await
            .len(),
        1,
        "16: untouched"
    );

    // Leave the purge queue empty for the next run.
    testing::drain(&mut conn, PURGE_A, IDLE).await.unwrap();
    conn.close().await.unwrap();
    explorer.close().await;
}
