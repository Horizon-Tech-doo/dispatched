//! AMQP data plane: connection, CBS, management link, message model, peek.

mod connection;
mod management;
mod message;
pub mod peek;
pub mod scan;
#[cfg(feature = "test-support")]
pub mod testing;
mod tokens_ledger;
mod transport;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
#[cfg(test)]
pub(crate) use message::test_message;
pub use message::{
    MessageBody, MessageState, PeekedMessage, PropValue, decode_bytes, decode_message,
};
pub use scan::{
    DEFAULT_SCAN_LIMIT, PeekOptions, PeekPage, PeekSource, StateCounts, count_messages,
    scan_messages,
};
