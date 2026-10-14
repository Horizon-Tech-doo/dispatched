//! Seeding helpers for integration tests. NOT a product API: real send,
//! receive and settle (with policy confirmation) arrive in M2.

use super::connection::ServiceBusConnection;
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::types::definitions::{
    Error as AmqpError, ErrorCondition, Fields, ReceiverSettleMode, SenderSettleMode,
};
use fe2o3_amqp::types::messaging::{
    ApplicationProperties, Body, Data, Message, MessageAnnotations, Modified, Properties,
};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Symbol, Timestamp, Value};
use fe2o3_amqp::{Receiver, Sender};
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;
use time::OffsetDateTime;

/// Upper bound for every network step, so a stuck broker fails a test instead of hanging it.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// Run one network step with [`STEP_TIMEOUT`]; protocol failures read `{what}: {error}`.
async fn step<T, E: Display>(
    what: &str,
    fut: impl Future<Output = std::result::Result<T, E>>,
) -> Result<T> {
    match tokio::time::timeout(STEP_TIMEOUT, fut).await {
        Err(_) => Err(Error::Timeout(format!(
            "{what} took longer than {}s",
            STEP_TIMEOUT.as_secs()
        ))),
        Ok(r) => r.map_err(|e| Error::Protocol(format!("{what}: {e}"))),
    }
}

/// Close a link, best effort, without ever waiting longer than [`STEP_TIMEOUT`].
async fn close_sender(sender: Sender) {
    let _ = tokio::time::timeout(STEP_TIMEOUT, sender.close()).await;
}

/// Close a receiver, best effort, without ever waiting longer than [`STEP_TIMEOUT`].
async fn close_receiver(receiver: Receiver) {
    let _ = tokio::time::timeout(STEP_TIMEOUT, receiver.close()).await;
}

fn link_name(kind: &str) -> String {
    format!("dispatched-test-{kind}-{}", uuid::Uuid::new_v4().simple())
}

async fn authorize(conn: &mut ServiceBusConnection, address: &str) -> Result<()> {
    match tokio::time::timeout(STEP_TIMEOUT, conn.authorize(address)).await {
        Err(_) => Err(Error::Timeout(format!(
            "authorize {address} took longer than {}s",
            STEP_TIMEOUT.as_secs()
        ))),
        Ok(r) => r,
    }
}

fn text_message(
    body: &str,
    props: &[(&str, &str)],
    scheduled: Option<OffsetDateTime>,
) -> Message<Data> {
    let mut app = OrderedMap::new();
    for (k, v) in props {
        app.insert((*k).to_string(), SimpleValue::String((*v).to_string()));
    }
    let builder = Message::builder()
        .properties(
            Properties::builder()
                .message_id(uuid::Uuid::new_v4().to_string())
                .content_type("application/json")
                .build(),
        )
        .application_properties(ApplicationProperties(app));
    let builder = match scheduled {
        Some(at) => builder.message_annotations(
            MessageAnnotations::builder()
                .insert(
                    "x-opt-scheduled-enqueue-time",
                    Value::Timestamp(Timestamp::from(at)),
                )
                .build(),
        ),
        None => builder,
    };
    builder.data(body.as_bytes().to_vec()).build()
}

async fn send_message(
    conn: &mut ServiceBusConnection,
    address: &str,
    message: Message<Data>,
) -> Result<()> {
    authorize(conn, address).await?;
    let mut sender = step(
        "attach sender",
        Sender::builder()
            .name(link_name("tx"))
            .target(address.to_string())
            .attach(conn.session_mut()),
    )
    .await?;
    let outcome = step("send", sender.send(message)).await?;
    outcome
        .accepted_or_else(|o| o)
        .map_err(|o| Error::Protocol(format!("send not accepted: {o:?}")))?;
    close_sender(sender).await;
    Ok(())
}

/// Send one text message with string application properties.
pub async fn send_text(
    conn: &mut ServiceBusConnection,
    address: &str,
    body: &str,
    props: &[(&str, &str)],
) -> Result<()> {
    send_message(conn, address, text_message(body, props, None)).await
}

/// Send one text message that the broker enqueues only at `enqueue_at`
/// (annotation `x-opt-scheduled-enqueue-time`).
pub async fn send_scheduled(
    conn: &mut ServiceBusConnection,
    address: &str,
    body: &str,
    props: &[(&str, &str)],
    enqueue_at: OffsetDateTime,
) -> Result<()> {
    send_message(conn, address, text_message(body, props, Some(enqueue_at))).await
}

/// Attach a peek-lock receiver with one credit.
async fn attach_peek_lock(conn: &mut ServiceBusConnection, address: &str) -> Result<Receiver> {
    authorize(conn, address).await?;
    step(
        "attach receiver",
        Receiver::builder()
            .name(link_name("rx"))
            .source(address.to_string())
            .receiver_settle_mode(ReceiverSettleMode::Second)
            .credit_mode(CreditMode::Auto(1))
            .attach(conn.session_mut()),
    )
    .await
}

/// Receive the next message (peek-lock) and dead-letter it with `reason`.
/// Returns `false` if nothing arrived within `wait`.
pub async fn dead_letter_next(
    conn: &mut ServiceBusConnection,
    address: &str,
    reason: &str,
    description: &str,
    wait: Duration,
) -> Result<bool> {
    let mut receiver = attach_peek_lock(conn, address).await?;
    let delivery = match tokio::time::timeout(wait, receiver.recv::<Body<Value>>()).await {
        Err(_) => {
            close_receiver(receiver).await;
            return Ok(false);
        }
        Ok(r) => r.map_err(|e| Error::Protocol(format!("receive: {e}")))?,
    };
    let mut info = Fields::default();
    info.insert(Symbol::from("DeadLetterReason"), Value::from(reason));
    info.insert(
        Symbol::from("DeadLetterErrorDescription"),
        Value::from(description),
    );
    let error = AmqpError::new(
        ErrorCondition::Custom(Symbol::from("com.microsoft:dead-letter")),
        None,
        Some(info),
    );
    step("dead-letter", receiver.reject(&delivery, error)).await?;
    close_receiver(receiver).await;
    Ok(true)
}

/// Receive the next message (peek-lock) and defer it: settle with `Modified`
/// and `undeliverable_here`. Returns `false` if nothing arrived within `wait`.
pub async fn defer_next(
    conn: &mut ServiceBusConnection,
    address: &str,
    wait: Duration,
) -> Result<bool> {
    let mut receiver = attach_peek_lock(conn, address).await?;
    let delivery = match tokio::time::timeout(wait, receiver.recv::<Body<Value>>()).await {
        Err(_) => {
            close_receiver(receiver).await;
            return Ok(false);
        }
        Ok(r) => r.map_err(|e| Error::Protocol(format!("receive: {e}")))?,
    };
    step(
        "defer",
        receiver.modify(
            &delivery,
            Modified {
                delivery_failed: None,
                undeliverable_here: Some(true),
                message_annotations: None,
            },
        ),
    )
    .await?;
    close_receiver(receiver).await;
    Ok(true)
}

/// Receive-and-delete everything in `address` until it has been idle for `idle`.
pub async fn drain(
    conn: &mut ServiceBusConnection,
    address: &str,
    idle: Duration,
) -> Result<usize> {
    authorize(conn, address).await?;
    let mut receiver = step(
        "attach drain receiver",
        Receiver::builder()
            .name(link_name("drain"))
            .source(address.to_string())
            .sender_settle_mode(SenderSettleMode::Settled)
            .credit_mode(CreditMode::Auto(50))
            .attach(conn.session_mut()),
    )
    .await?;
    let mut drained = 0;
    while let Ok(result) = tokio::time::timeout(idle, receiver.recv::<Body<Value>>()).await {
        let delivery = result.map_err(|e| Error::Protocol(format!("drain receive: {e}")))?;
        let _ = tokio::time::timeout(STEP_TIMEOUT, receiver.accept(&delivery)).await;
        drained += 1;
    }
    close_receiver(receiver).await;
    Ok(drained)
}
