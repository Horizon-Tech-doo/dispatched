//! Seeding helpers for integration tests. NOT a product API: real send,
//! receive and settle (with policy confirmation) arrive in M2.

use super::connection::ServiceBusConnection;
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::types::definitions::{
    Error as AmqpError, ErrorCondition, Fields, ReceiverSettleMode, SenderSettleMode,
};
use fe2o3_amqp::types::messaging::{ApplicationProperties, Body, Message, Properties};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Symbol, Value};
use fe2o3_amqp::{Receiver, Sender};
use std::time::Duration;

fn link_name(kind: &str) -> String {
    format!("dispatched-test-{kind}-{}", uuid::Uuid::new_v4().simple())
}

/// Send one text message with string application properties.
pub async fn send_text(
    conn: &mut ServiceBusConnection,
    address: &str,
    body: &str,
    props: &[(&str, &str)],
) -> Result<()> {
    conn.authorize(address).await?;
    let mut sender = Sender::builder()
        .name(link_name("tx"))
        .target(address.to_string())
        .attach(conn.session_mut())
        .await
        .map_err(|e| Error::Protocol(format!("attach sender: {e}")))?;
    let mut app = OrderedMap::new();
    for (k, v) in props {
        app.insert((*k).to_string(), SimpleValue::String((*v).to_string()));
    }
    let message = Message::builder()
        .properties(
            Properties::builder()
                .message_id(uuid::Uuid::new_v4().to_string())
                .content_type("application/json")
                .build(),
        )
        .application_properties(ApplicationProperties(app))
        .data(body.as_bytes().to_vec())
        .build();
    let outcome = sender
        .send(message)
        .await
        .map_err(|e| Error::Protocol(format!("send: {e}")))?;
    outcome
        .accepted_or_else(|o| o)
        .map_err(|o| Error::Protocol(format!("send not accepted: {o:?}")))?;
    let _ = sender.close().await;
    Ok(())
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
    conn.authorize(address).await?;
    let mut receiver = Receiver::builder()
        .name(link_name("rx"))
        .source(address.to_string())
        .receiver_settle_mode(ReceiverSettleMode::Second)
        .credit_mode(CreditMode::Auto(1))
        .attach(conn.session_mut())
        .await
        .map_err(|e| Error::Protocol(format!("attach receiver: {e}")))?;
    let delivery = match tokio::time::timeout(wait, receiver.recv::<Body<Value>>()).await {
        Err(_) => {
            let _ = receiver.close().await;
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
    receiver
        .reject(&delivery, error)
        .await
        .map_err(|e| Error::Protocol(format!("dead-letter: {e}")))?;
    let _ = receiver.close().await;
    Ok(true)
}

/// Receive-and-delete everything in `address` until it has been idle for `idle`.
pub async fn drain(
    conn: &mut ServiceBusConnection,
    address: &str,
    idle: Duration,
) -> Result<usize> {
    conn.authorize(address).await?;
    let mut receiver = Receiver::builder()
        .name(link_name("drain"))
        .source(address.to_string())
        .sender_settle_mode(SenderSettleMode::Settled)
        .credit_mode(CreditMode::Auto(50))
        .attach(conn.session_mut())
        .await
        .map_err(|e| Error::Protocol(format!("attach drain receiver: {e}")))?;
    let mut drained = 0;
    while let Ok(result) = tokio::time::timeout(idle, receiver.recv::<Body<Value>>()).await {
        let delivery = result.map_err(|e| Error::Protocol(format!("drain receive: {e}")))?;
        let _ = receiver.accept(&delivery).await;
        drained += 1;
    }
    let _ = receiver.close().await;
    Ok(drained)
}
