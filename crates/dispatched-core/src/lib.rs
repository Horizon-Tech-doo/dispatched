//! Core library for Dispatched, a cross-platform Azure Service Bus explorer.
//! All Service Bus behaviour lives here; the CLI, MCP server and desktop app
//! only translate input and render output.

#![deny(rust_2018_idioms, unsafe_code)]
#![warn(clippy::all)]

pub mod activity;
pub mod connstr;
pub mod diagnostics;
pub mod discovery;
pub mod error;
pub mod explorer;
pub mod export;
pub mod filter;
pub mod import;
pub mod jobs;
pub mod messaging;
pub mod mgmt;
pub mod namespace;
pub mod ops;
pub mod paths;
pub mod profiles;
pub mod safety;
pub mod tokens;

pub use activity::{
    ActivityEntry, ActivityLog, ActivityQuery, Audit, Limits, MemoryAudit, Outcome, Phase,
};
pub use connstr::ConnectionString;
pub use error::{Error, Result};
pub use explorer::Explorer;
pub use export::{
    Continuation, EXPORT_PAGE, EXPORT_SCHEMA, ExportFormat, ExportedMessage, FileExport, Streamed,
    continuation, stream_pages, write_messages,
};
pub use filter::{BodyFilter, FilterField, FilterIssue, FilterSpec, MessageFilter, PropertyFilter};
pub use import::{ImportedMessage, read_messages};
pub use jobs::{
    ItemOutcome, ItemResult, JobEvent, JobKind, JobReport, NoProgress, NotReached, Progress,
    RecordingProgress,
};
pub use messaging::{
    BodySource, BrokerList, CloneOptions, Cloned, Edits, MessageBody, MessageDetail,
    MessageIdSource, MessageState, MessageSummary, OutgoingBody, OutgoingMessage, PeekOptions,
    PeekPage, PeekedMessage, PropValue, SessionListing, SessionSummary,
};
pub use mgmt::{
    CorrelationProperty, EntityCounts, EntityDetails, EntityKind, EntityProperty, ManagementClient,
    QueueInfo, RuleFilter, RuleInfo, SubscriptionInfo, TopicInfo,
};
pub use namespace::{
    EmulatorPorts, EntityPath, Namespace, PeekView, ProxyConfig, SubQueue, Transport,
};
pub use ops::{Operator, ReceiveAction, ReceiveOutcome, ReceiveRequest, SendRequest};
pub use safety::{
    Actor, CountEstimate, Mutation, OpKind, Operation, ProfileRef, SettleVerb, Target, count_text,
};
pub use tokens::{ServiceBusToken, TokenKind, TokenSource};
