//! M1a emulator coverage: views, filters, export, approximate counts.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m1 -- --nocapture`

use dispatched_core::filter::{BodyFilter, MessageFilter, PropertyFilter};
use dispatched_core::messaging::{ServiceBusConnection, testing};
use dispatched_core::{
    ConnectionString, EntityPath, Explorer, ExportFormat, MessageState, Namespace, PeekOptions,
    PeekView, write_messages,
};
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

#[tokio::test(flavor = "multi_thread")]
async fn m1a_views_filters_export_counts() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns =
        Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();
    for addr in ["orders", "orders/$DeadLetterQueue"] {
        testing::drain(&mut conn, addr, Duration::from_secs(2))
            .await
            .unwrap();
    }
    // Seed: 4 plain messages i=0..3 and 1 scheduled (in 30 min). The queue was
    // drained first, so `defer_next` defers the oldest receivable one: i=0.
    for i in 0..4 {
        testing::send_text(
            &mut conn,
            "orders",
            &format!("{{\"run\":\"{run}\",\"i\":{i},\"kind\":\"plain\"}}"),
            &[("run", &run), ("i", &i.to_string())],
        )
        .await
        .unwrap();
    }
    let later = time::OffsetDateTime::now_utc() + time::Duration::minutes(30);
    testing::send_scheduled(
        &mut conn,
        "orders",
        &format!("{{\"run\":\"{run}\",\"kind\":\"scheduled\"}}"),
        &[("run", &run)],
        later,
    )
    .await
    .unwrap();
    assert!(
        testing::defer_next(&mut conn, "orders", Duration::from_secs(10))
            .await
            .unwrap()
    );
    conn.close().await.unwrap();

    let explorer = Explorer::new(ns).unwrap();
    let e = EntityPath::queue("orders");
    let ours = |f: &str| MessageFilter {
        property: Some(PropertyFilter::parse(&format!("run = '{run}'{f}")).unwrap()),
        body: vec![],
    };

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
    assert!(
        deferred.messages[0]
            .body
            .as_text()
            .unwrap()
            .contains("\"i\":0"),
        "the oldest message was deferred"
    );

    let active = explorer
        .peek(&e, PeekView::Active, &PeekOptions::new(10).filter(ours("")))
        .await
        .unwrap();
    assert_eq!(active.messages.len(), 3, "i=1..3 stay active");
    assert!(
        active
            .messages
            .iter()
            .all(|m| m.state == MessageState::Active)
    );

    let by_prop = explorer
        .peek(
            &e,
            PeekView::Active,
            &PeekOptions::new(10).filter(ours(" AND i = 2")),
        )
        .await
        .unwrap();
    assert_eq!(by_prop.messages.len(), 1);

    let mut by_body = ours("");
    by_body
        .body
        .push(BodyFilter::json("$.kind", Some("plain")).unwrap());
    let plain = explorer
        .peek(&e, PeekView::Active, &PeekOptions::new(10).filter(by_body))
        .await
        .unwrap();
    assert_eq!(plain.messages.len(), 3);

    let mut out = Vec::new();
    let n = write_messages(&mut out, &active.messages, ExportFormat::Ndjson).unwrap();
    assert_eq!(n, 3);
    assert_eq!(String::from_utf8(out).unwrap().lines().count(), 3);

    let queues = explorer.list_queues().await.unwrap();
    let orders = queues.iter().find(|q| q.name == "orders").unwrap();
    let counts = orders
        .counts
        .as_ref()
        .expect("approximate counts on the emulator");
    assert!(counts.approximate);
    assert!(counts.active >= 4, "active includes deferred: {counts:?}");
    assert!(counts.scheduled >= 1);
    eprintln!("GATE-NOTE m1a counts for orders: {counts:?}");
    explorer.close().await;
}
