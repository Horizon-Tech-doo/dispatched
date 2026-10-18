//! AMQP data plane: connection, CBS, management link, message model, peek.

pub(crate) mod backend;
mod connection;
pub mod dlq;
#[cfg(test)]
pub(crate) mod fake;
// Request builders for mutating management calls: crate-private, so only the gated
// operations can send them.
pub(crate) mod lockops;
mod management;
mod message;
pub mod outgoing;
pub mod peek;
pub mod props;
pub(crate) mod receiver;
pub mod scan;
pub(crate) mod sender;
pub mod sessions;
pub mod summary;
pub mod template;
#[cfg(feature = "test-support")]
pub mod testing;
mod tokens_ledger;
mod transport;

// Crate-private: a connection can mutate without the gate. The emulator tests reach it
// through `testing` (feature `test-support`).
pub(crate) use connection::ServiceBusConnection;
pub use dlq::{DLQ_SUMMARY_CAP, DlqSummary, ReasonGroup};
pub(crate) use management::is_session_required;
pub use management::{ManagementLink, MgmtResponse};
pub use message::{
    MessageBody, MessageState, PeekedMessage, PropValue, decode_bytes, decode_message, ttl_ms,
};
#[cfg(test)]
pub(crate) use message::{simple_to_prop, test_message};
pub use outgoing::{
    BodySource, CloneOptions, Cloned, Edits, MessageIdSource, OutgoingBody, OutgoingMessage,
};
pub use props::{PropType, TypedValue, parse_duration, parse_prop_arg, parse_when};
pub use receiver::{
    LockToken, ReceiveMode, ReceivedMessage, ReceiverSpec, SessionChoice, Settlement,
    lock_token_from_tag,
};
pub use scan::{
    DEFAULT_SCAN_LIMIT, PeekOptions, PeekPage, PeekSource, SequenceCollection, StateCounts,
    collect_sequences, count_messages, last_sequence, peek_one, peek_tail, scan_messages,
};
pub use sender::send_target;
pub use sessions::{BrokerList, SessionListing, SessionSummary};
pub use summary::{DETAIL_BODY_CAP, MessageDetail, MessageSummary};
pub use template::{parse_template_json, parse_template_xml, read_template};

/// The wall clock, for lock-expiry arithmetic against broker times.
#[cfg(not(test))]
pub(crate) fn wall_now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc()
}

/// The wall clock in tests: one process-wide anchor advanced on tokio time, so the fake
/// broker's clock and `ops::locate`'s read exactly the same time for the same instant (two
/// separate `now_utc()` anchors would differ by however long the test took in between).
/// Only this crate's unit tests get it (`FakeBroker` is `cfg(test)` too); the `test-support`
/// feature and other crates see `now_utc()`, so exact lock-time assertions belong here.
#[cfg(test)]
pub(crate) fn wall_now() -> time::OffsetDateTime {
    use std::sync::OnceLock;
    static ANCHOR: OnceLock<(std::time::Instant, time::OffsetDateTime)> = OnceLock::new();
    let (at, wall) =
        *ANCHOR.get_or_init(|| (std::time::Instant::now(), time::OffsetDateTime::now_utc()));
    let now = tokio::time::Instant::now().into_std();
    if now >= at {
        wall + (now - at)
    } else {
        wall - (at - now)
    }
}
