//! AMQP data plane: connection, CBS, management link, message model, peek.

pub(crate) mod backend;
mod connection;
#[cfg(test)]
pub(crate) mod fake;
pub mod lockops;
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

pub use connection::ServiceBusConnection;
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
