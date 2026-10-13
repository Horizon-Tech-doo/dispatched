//! AMQP data plane: connection, CBS, management link, message model.

mod connection;
mod management;
mod message;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
pub use message::{MessageBody, PeekedMessage, PropValue, decode_bytes, decode_message};
