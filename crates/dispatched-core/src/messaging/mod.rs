//! AMQP data plane: connection, CBS, management link, message model, peek.

mod connection;
mod management;
mod message;
pub mod peek;
#[cfg(feature = "test-support")]
pub mod testing;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
pub use message::{
    MessageBody, MessageState, PeekedMessage, PropValue, decode_bytes, decode_message,
};
