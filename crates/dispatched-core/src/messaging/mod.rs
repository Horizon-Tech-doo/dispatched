//! AMQP data plane: connection, CBS, management link, message model.

mod connection;
mod management;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
