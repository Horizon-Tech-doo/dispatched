//! M2a emulator coverage: the link semantics the operate features rest on
//! (`m2a_link_semantics`). Uses the existing queue `m1a-browse` (see
//! testing/emulator/Config.json); the probe empties it first and tags what it
//! sends with a per-run property, so leftovers from earlier runs are harmless.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m2a -- --nocapture`

use dispatched_core::messaging::{ServiceBusConnection, testing};
use dispatched_core::{ConnectionString, Namespace};

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
}
