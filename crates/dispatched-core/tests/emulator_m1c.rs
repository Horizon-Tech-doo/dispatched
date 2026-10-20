//! M1c emulator gate: tail peeks, single-message reads, selected sequences, sessions
//! (listing and per-session peeks), entity details. Per-run ids keep leftovers harmless.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m1c -- --nocapture`

use dispatched_core::filter::{FilterSpec, MessageFilter};
use dispatched_core::messaging::testing::{self, ServiceBusConnection};
use dispatched_core::{
    ConnectionString, EntityKind, EntityPath, Explorer, Namespace, PeekOptions, PeekView,
};

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

/// The namespace the gate runs on: the emulator, or `DISPATCHED_GATE_CONNECTION_STRING` (a cloud
/// test namespace; the string comes only from the environment and is never printed).
fn gate_cs() -> String {
    std::env::var("DISPATCHED_GATE_CONNECTION_STRING").unwrap_or_else(|_| EMULATOR.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn m1c_tail_sequences_sessions_details() {
    if std::env::var("DISPATCHED_EMULATOR").as_deref() != Ok("1") {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(&gate_cs()).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let explorer = Explorer::new(ns.clone()).unwrap();
    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();

    // Plain queue: five messages of this run.
    for i in 0..5 {
        testing::send_text(
            &mut conn,
            "m1a-browse",
            &format!("{{\"run\":\"{run}\",\"i\":{i}}}"),
            &[("run", &run), ("i", &i.to_string())],
        )
        .await
        .unwrap();
    }
    // Session queue: two in session A, one in session B.
    let (a, b) = (format!("{run}-a"), format!("{run}-b"));
    for (s, i) in [(&a, 0), (&a, 1), (&b, 2)] {
        testing::send_session_text(
            &mut conn,
            "sessions-q",
            s,
            &format!("{{\"run\":\"{run}\",\"i\":{i}}}"),
            &[("run", &run)],
        )
        .await
        .unwrap();
    }
    conn.close().await.unwrap();

    let q = EntityPath::queue("m1a-browse");
    let ours = FilterSpec {
        property: Some(format!("run = '{run}'")),
        ..Default::default()
    }
    .build()
    .unwrap();

    // From the end: the last three of this run, oldest first.
    let tail = explorer
        .peek_tail(
            &q,
            PeekView::Active,
            &PeekOptions::new(3).scan_limit(100_000).filter(ours.clone()),
        )
        .await
        .unwrap();
    let is: Vec<String> = tail
        .messages
        .iter()
        .map(|m| m.application_properties["i"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(is, ["2", "3", "4"], "tail of this run");

    // One message by sequence number, and a gone one.
    let seq = tail.messages[0].sequence_number;
    assert_eq!(
        explorer
            .peek_message(&q, PeekView::Active, seq, None)
            .await
            .unwrap()
            .unwrap()
            .sequence_number,
        seq
    );
    assert!(
        explorer
            .peek_message(&q, PeekView::DeadLetter, seq, None)
            .await
            .unwrap()
            .is_none(),
        "not in the DLQ"
    );

    // Selected sequences.
    let picked: Vec<i64> = tail.messages.iter().map(|m| m.sequence_number).collect();
    let c = explorer
        .collect_sequences(&q, PeekView::Active, &picked, 100_000, None, None)
        .await
        .unwrap();
    assert_eq!(c.messages.len(), 3);
    assert!(c.missing.is_empty());

    // Sessions: listing (broker, peek tally, or both) and a per-session peek.
    let sq = EntityPath::queue("sessions-q");
    let listing = explorer.list_sessions(&sq).await.unwrap();
    eprintln!(
        "GATE-NOTE m1c sessions: broker={:?} ids={} scanned={} count={}",
        listing.broker,
        listing.sessions.iter().filter(|s| s.in_broker).count(),
        listing.scanned,
        listing.sessions.len()
    );
    for id in [&a, &b] {
        assert!(
            listing.sessions.iter().any(|s| &s.id == id),
            "session {id} listed"
        );
    }
    let in_a = MessageFilter {
        session_id: Some(a.clone()),
        ..Default::default()
    };
    let page = explorer
        .peek(&sq, PeekView::Active, &PeekOptions::new(10).filter(in_a))
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 2, "session A's messages");
    assert!(
        page.messages
            .iter()
            .all(|m| m.session_id.as_deref() == Some(a.as_str()))
    );

    // Details.
    let d = explorer
        .entity_details(EntityKind::Queue, "m1a-browse")
        .await
        .unwrap();
    assert!(
        d.properties
            .iter()
            .any(|p| p.name == "MaxDeliveryCount" && p.value == "10"),
        "{:?}",
        d.properties
    );
    assert!(d.counts.is_some(), "reported or counted by peeking");
    match explorer.list_rules("events", "all").await {
        Ok(rules) => eprintln!(
            "GATE-NOTE m1c rules on events/all: {:?}",
            rules.iter().map(|r| &r.name).collect::<Vec<_>>()
        ),
        Err(e) => eprintln!("GATE-NOTE m1c rules unsupported on the emulator: {e}"),
    }
    explorer.close().await;
}
