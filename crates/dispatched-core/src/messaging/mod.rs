//! AMQP data plane: connection, CBS, management link, message model, peek.

mod connection;
mod management;
mod message;
pub mod outgoing;
pub mod peek;
pub mod props;
pub mod scan;
pub mod sessions;
pub mod summary;
#[cfg(feature = "test-support")]
pub mod testing;
mod tokens_ledger;
mod transport;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
#[cfg(test)]
pub(crate) use message::test_message;
pub use message::{
    MessageBody, MessageState, PeekedMessage, PropValue, decode_bytes, decode_message, ttl_ms,
};
pub use outgoing::{
    BodySource, CloneOptions, Cloned, Edits, MessageIdSource, OutgoingBody, OutgoingMessage,
};
pub use props::{PropType, TypedValue, parse_duration, parse_prop_arg, parse_when};
pub use scan::{
    DEFAULT_SCAN_LIMIT, PeekOptions, PeekPage, PeekSource, SequenceCollection, StateCounts,
    collect_sequences, count_messages, last_sequence, peek_one, peek_tail, scan_messages,
};
pub use sessions::{BrokerList, SessionListing, SessionSummary};
pub use summary::{DETAIL_BODY_CAP, MessageDetail, MessageSummary};
