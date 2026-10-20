//! M1a emulator coverage: views, filters, continuation, export, DLQ, approximate counts.
//! Uses the dedicated queue `m1a-browse` (see testing/emulator/Config.json) so M0 leftovers
//! cannot interfere. Assertions filter by a per-run property and compare count deltas, so
//! leftovers from earlier runs (deferred and scheduled messages cannot be drained) are harmless.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m1 -- --nocapture`

use dispatched_core::filter::{BodyFilter, MessageFilter, PropertyFilter};
use dispatched_core::messaging::testing::{self, ServiceBusConnection};
use dispatched_core::{
    ConnectionString, EntityCounts, EntityPath, Explorer, ExportFormat, MessageState, Namespace,
    PeekOptions, PeekView, write_messages,
};
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

const QUEUE: &str = "m1a-browse";
const DLQ: &str = "m1a-browse/$DeadLetterQueue";

async fn counts_of(explorer: &Explorer) -> EntityCounts {
    let queues = explorer.list_queues().await.unwrap();
    queues
        .iter()
        .find(|q| q.name == QUEUE)
        .unwrap_or_else(|| {
            panic!("queue {QUEUE} missing; is testing/emulator/Config.json current?")
        })
        .counts
        .clone()
        .expect("approximate counts on the emulator")
}

#[tokio::test(flavor = "multi_thread")]
async fn m1a_views_filters_export_counts() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(&gate_cs()).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let explorer = Explorer::new(ns.clone()).unwrap();

    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();
    for addr in [QUEUE, DLQ] {
        testing::drain(&mut conn, addr, Duration::from_secs(2))
            .await
            .unwrap();
    }
    let before = counts_of(&explorer).await;

    // Seed 5 plain messages i=0..4 and 1 scheduled (in 30 min), then defer i=0 and
    // dead-letter i=1. Left: i=2,3,4 active, i=0 deferred, i=1 dead-lettered, 1 scheduled.
    for i in 0..5 {
        testing::send_text(
            &mut conn,
            QUEUE,
            &format!("{{\"run\":\"{run}\",\"i\":{i},\"kind\":\"plain\"}}"),
            &[("run", &run), ("i", &i.to_string())],
        )
        .await
        .unwrap();
    }
    let later = time::OffsetDateTime::now_utc() + time::Duration::minutes(30);
    testing::send_scheduled(
        &mut conn,
        QUEUE,
        &format!("{{\"run\":\"{run}\",\"kind\":\"scheduled\"}}"),
        &[("run", &run)],
        later,
    )
    .await
    .unwrap();
    assert!(
        testing::defer_next_matching(&mut conn, QUEUE, ("i", "0"), Duration::from_secs(10))
            .await
            .unwrap(),
        "message i=0 of this run was not found to defer"
    );
    assert!(
        testing::dead_letter_next_matching(
            &mut conn,
            QUEUE,
            ("i", "1"),
            "m1a-test-reason",
            "dead-lettered by emulator_m1",
            Duration::from_secs(10),
        )
        .await
        .unwrap(),
        "message i=1 of this run was not found to dead-letter"
    );
    conn.close().await.unwrap();

    // Counts: exact deltas. Deferred messages count as active (Service Bus rule).
    let after = counts_of(&explorer).await;
    // The emulator has no management counts, so they are counted by peeking; a cloud namespace
    // reports them exactly.
    assert_eq!(after.approximate, gate_cs() == EMULATOR, "{after:?}");
    assert_eq!(
        after.active - before.active,
        4,
        "3 active + 1 deferred: {before:?} -> {after:?}"
    );
    assert_eq!(
        after.scheduled - before.scheduled,
        1,
        "{before:?} -> {after:?}"
    );
    assert_eq!(
        after.dead_letter - before.dead_letter,
        1,
        "{before:?} -> {after:?}"
    );
    eprintln!("GATE-NOTE m1a counts for {QUEUE}: before {before:?} after {after:?}");

    let e = EntityPath::queue(QUEUE);
    let ours = |f: &str| MessageFilter {
        property: Some(PropertyFilter::parse(&format!("run = '{run}'{f}")).unwrap()),
        body: vec![],
        ..Default::default()
    };
    let with_body = |f: BodyFilter| {
        let mut m = ours("");
        m.body.push(f);
        m
    };
    let seqs = |p: &dispatched_core::PeekPage| -> Vec<i64> {
        p.messages.iter().map(|m| m.sequence_number).collect()
    };
    let is = |i: i64, m: &dispatched_core::PeekedMessage| {
        m.body.as_text().unwrap().contains(&format!("\"i\":{i},"))
    };

    // Views.
    let scheduled = explorer
        .peek(
            &e,
            PeekView::Scheduled,
            &PeekOptions::new(10).filter(ours("")),
        )
        .await
        .unwrap();
    assert_eq!(scheduled.messages.len(), 1);
    assert_eq!(scheduled.messages[0].state, MessageState::Scheduled);

    let deferred = explorer
        .peek(
            &e,
            PeekView::Deferred,
            &PeekOptions::new(10).filter(ours("")),
        )
        .await
        .unwrap();
    assert_eq!(deferred.messages.len(), 1);
    assert_eq!(deferred.messages[0].state, MessageState::Deferred);
    assert!(is(0, &deferred.messages[0]), "i=0 was deferred");

    let active = explorer
        .peek(&e, PeekView::Active, &PeekOptions::new(10).filter(ours("")))
        .await
        .unwrap();
    assert_eq!(active.messages.len(), 3, "i=2..4 stay active");
    assert!(
        active
            .messages
            .iter()
            .all(|m| m.state == MessageState::Active)
    );
    assert!((2..=4).all(|i| active.messages.iter().any(|m| is(i, m))));

    // DLQ view.
    let dlq = explorer
        .peek(
            &e,
            PeekView::DeadLetter,
            &PeekOptions::new(10).filter(ours("")),
        )
        .await
        .unwrap();
    assert_eq!(dlq.messages.len(), 1);
    assert!(is(1, &dlq.messages[0]), "i=1 was dead-lettered");
    assert_eq!(
        dlq.messages[0].dead_letter_reason.as_deref(),
        Some("m1a-test-reason")
    );
    assert_eq!(
        dlq.messages[0].dead_letter_description.as_deref(),
        Some("dead-lettered by emulator_m1")
    );

    // Property filter.
    let by_prop = explorer
        .peek(
            &e,
            PeekView::Active,
            &PeekOptions::new(10).filter(ours(" AND i = 3")),
        )
        .await
        .unwrap();
    assert_eq!(by_prop.messages.len(), 1);
    assert!(is(3, &by_prop.messages[0]));

    // Body filters that must exclude something.
    let peek_body = |view: PeekView, f: BodyFilter| {
        let (explorer, e) = (&explorer, &e);
        let filter = with_body(f);
        async move {
            explorer
                .peek(e, view, &PeekOptions::new(10).filter(filter))
                .await
                .unwrap()
        }
    };
    let json_i2 = peek_body(
        PeekView::Active,
        BodyFilter::json("$.i", Some("2")).unwrap(),
    )
    .await;
    assert_eq!(json_i2.messages.len(), 1);
    assert!(is(2, &json_i2.messages[0]));
    let plain = peek_body(
        PeekView::Active,
        BodyFilter::json("$.kind", Some("plain")).unwrap(),
    )
    .await;
    assert_eq!(plain.messages.len(), 3);
    let sched_in_active = peek_body(
        PeekView::Active,
        BodyFilter::json("$.kind", Some("scheduled")).unwrap(),
    )
    .await;
    assert_eq!(sched_in_active.messages.len(), 0);
    let sched_in_sched = peek_body(
        PeekView::Scheduled,
        BodyFilter::json("$.kind", Some("scheduled")).unwrap(),
    )
    .await;
    assert_eq!(sched_in_sched.messages.len(), 1);
    let contains = peek_body(PeekView::Active, BodyFilter::contains("\"i\":4,")).await;
    assert_eq!(contains.messages.len(), 1);
    assert!(is(4, &contains.messages[0]));
    let regex = peek_body(PeekView::Active, BodyFilter::regex(r#""i":[34],"#).unwrap()).await;
    assert_eq!(regex.messages.len(), 2);

    // Continuation: two at a time, resume, no overlap, full coverage.
    let first = explorer
        .peek(&e, PeekView::Active, &PeekOptions::new(2).filter(ours("")))
        .await
        .unwrap();
    assert_eq!(first.messages.len(), 2);
    assert!(!first.exhausted);
    let next = first
        .next_sequence
        .expect("more to read after the first page");
    let rest = explorer
        .peek(
            &e,
            PeekView::Active,
            &PeekOptions::new(10).from_sequence(next).filter(ours("")),
        )
        .await
        .unwrap();
    assert_eq!(rest.messages.len(), 1, "the remaining run message");
    let mut joined = seqs(&first);
    joined.extend(seqs(&rest));
    let mut all = seqs(&active);
    all.sort_unstable();
    let mut unique = joined.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        joined.len(),
        "no overlap between pages: {joined:?}"
    );
    assert_eq!(unique, all, "pages cover exactly the active run messages");
    assert!(seqs(&rest).iter().all(|s| *s >= next));

    // Export: every NDJSON line is the stable schema.
    let mut out = Vec::new();
    let n = write_messages(&mut out, &active.messages, ExportFormat::Ndjson).unwrap();
    assert_eq!(n, 3);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().count(), 3);
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).expect("each line is JSON");
        assert_eq!(v["schema"], "dispatched.message/v1", "{v}");
        assert_eq!(v["state"], "active", "{v}");
        assert_eq!(
            v["application_properties"]["run"]["value"],
            run.as_str(),
            "{v}"
        );
    }

    // Best-effort cleanup; assertions above never depend on it. Deferred and
    // scheduled messages cannot be drained and stay (isolated in this queue).
    explorer.close().await;
    if let Ok(mut conn) = ServiceBusConnection::open(&ns).await {
        for addr in [QUEUE, DLQ] {
            let _ = testing::drain(&mut conn, addr, Duration::from_secs(1)).await;
        }
        let _ = conn.close().await;
    }
}
