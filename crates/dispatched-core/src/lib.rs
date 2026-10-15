//! Core library for Dispatched, a cross-platform Azure Service Bus explorer.
//! All Service Bus behaviour lives here; the CLI, MCP server and desktop app
//! only translate input and render output.

#![deny(rust_2018_idioms, unsafe_code)]
#![warn(clippy::all)]

pub mod connstr;
pub mod diagnostics;
pub mod discovery;
pub mod error;
pub mod explorer;
pub mod export;
pub mod filter;
pub mod messaging;
pub mod mgmt;
pub mod namespace;
pub mod paths;
pub mod profiles;
pub mod tokens;

pub use connstr::ConnectionString;
pub use error::{Error, Result};
pub use explorer::Explorer;
pub use export::{
    Continuation, EXPORT_PAGE, EXPORT_SCHEMA, ExportFormat, ExportedMessage, FileExport, Streamed,
    continuation, stream_pages, write_messages,
};
pub use filter::{BodyFilter, FilterField, FilterIssue, FilterSpec, MessageFilter, PropertyFilter};
pub use messaging::{
    MessageBody, MessageDetail, MessageState, MessageSummary, PeekOptions, PeekPage, PeekedMessage,
    PropValue, SessionListing, SessionSummary,
};
pub use mgmt::{
    CorrelationProperty, EntityCounts, EntityDetails, EntityKind, EntityProperty, ManagementClient,
    QueueInfo, RuleFilter, RuleInfo, SubscriptionInfo, TopicInfo,
};
pub use namespace::{
    EmulatorPorts, EntityPath, Namespace, PeekView, ProxyConfig, SubQueue, Transport,
};
pub use tokens::{ServiceBusToken, TokenKind, TokenSource};
