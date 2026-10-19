//! M0 gate: list → peek → peek DLQ with reason, against the Service Bus Emulator.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 -- --nocapture`

use dispatched_core::messaging::testing::{self, ServiceBusConnection};
use dispatched_core::{
    ConnectionString, EntityPath, Explorer, Namespace, PeekOptions, PeekView, PeekedMessage,
};
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

fn run_of(m: &PeekedMessage) -> Option<&str> {
    m.application_properties.get("run").and_then(|v| v.as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn m0_gate_flow_against_emulator() {
    if !enabled() {
        // CI (.github/workflows/ci.yml, emulator job) greps the log for
        // "skipped:" to fail a gate that skipped itself; keep this wording.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();

    // Seed: clean slate, 3 messages, dead-letter the first with a reason.
    let mut conn = ServiceBusConnection::open(&ns)
        .await
        .expect("AMQP connect to emulator");
    testing::drain(&mut conn, "orders", Duration::from_secs(2))
        .await
        .unwrap();
    testing::drain(&mut conn, "orders/$DeadLetterQueue", Duration::from_secs(2))
        .await
        .unwrap();
    testing::drain(
        &mut conn,
        "events/Subscriptions/all",
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    for i in 0..3 {
        testing::send_text(
            &mut conn,
            "orders",
            &format!("{{\"run\":\"{run}\",\"i\":{i}}}"),
            &[("run", &run)],
        )
        .await
        .unwrap();
    }
    assert!(
        testing::dead_letter_next(
            &mut conn,
            "orders",
            "m0-gate",
            "dead-lettered by the M0 gate test",
            Duration::from_secs(10)
        )
        .await
        .unwrap(),
        "a message should have been received for dead-lettering"
    );
    conn.close().await.unwrap();

    let explorer = Explorer::new(ns).unwrap();

    // 1. List queues over ATOM (HTTP :5300).
    let queues = explorer
        .list_queues()
        .await
        .expect("ATOM list queues on emulator");
    let orders = queues
        .iter()
        .find(|q| q.name == "orders")
        .expect("orders queue listed");
    eprintln!("GATE-NOTE emulator counts for orders: {:?}", orders.counts);
    assert!(
        queues
            .iter()
            .any(|q| q.name == "sessions-q" && q.requires_session)
    );

    // 2. Peek active messages over AMQP $management.
    let active = explorer
        .peek(
            &EntityPath::queue("orders"),
            PeekView::Active,
            &PeekOptions::new(100),
        )
        .await
        .unwrap()
        .messages;
    let ours: Vec<_> = active
        .iter()
        .filter(|m| run_of(m) == Some(run.as_str()))
        .collect();
    assert_eq!(ours.len(), 2, "two of three messages remain active");
    assert!(ours[0].sequence_number < ours[1].sequence_number);
    assert!(ours[0].enqueued_time.is_some());
    assert!(ours[0].body.as_text().unwrap().contains(&run));

    // 3. Peek the dead-letter queue and see the reason.
    let dead = explorer
        .peek(
            &EntityPath::queue("orders"),
            PeekView::DeadLetter,
            &PeekOptions::new(100),
        )
        .await
        .unwrap()
        .messages;
    let dead = dead
        .iter()
        .find(|m| run_of(m) == Some(run.as_str()))
        .expect("dead-lettered message visible");
    assert_eq!(dead.dead_letter_reason.as_deref(), Some("m0-gate"));
    assert_eq!(
        dead.dead_letter_description.as_deref(),
        Some("dead-lettered by the M0 gate test")
    );

    // 4. Empty entity and past-the-end peeks are empty, not errors.
    let sub = EntityPath::subscription("events", "all");
    assert!(
        explorer
            .peek(&sub, PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap()
            .messages
            .is_empty()
    );
    let last = ours[1].sequence_number;
    assert!(
        explorer
            .peek(
                &EntityPath::queue("orders"),
                PeekView::Active,
                &PeekOptions::new(10).from_sequence(last + 10_000)
            )
            .await
            .unwrap()
            .messages
            .is_empty()
    );

    explorer.close().await;
}
