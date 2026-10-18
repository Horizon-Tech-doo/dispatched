//! Seeding helpers for integration tests. NOT a product API: real send,
//! receive and settle (with policy confirmation) arrive in M2.

use super::connection::ServiceBusConnection;
use super::message::{PeekedMessage, PropValue};
use super::receiver::{AmqpReceiver, LockToken, ReceivedMessage, ReceiverSpec, Settlement};
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::types::definitions::{
    Error as AmqpError, ErrorCondition, Fields, ReceiverSettleMode, SenderSettleMode,
};
use fe2o3_amqp::types::messaging::{
    ApplicationProperties, Body, Data, Message, MessageAnnotations, Modified, Properties,
};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Symbol, Timestamp, Value};
use fe2o3_amqp::{Delivery, Receiver, Sender};
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
    session: Option<&str>,
) -> Message<Data> {
    let mut app = OrderedMap::new();
    for (k, v) in props {
        app.insert((*k).to_string(), SimpleValue::String((*v).to_string()));
    }
    let builder = Message::builder()
        .properties({
            let mut p = Properties::builder()
                .message_id(uuid::Uuid::new_v4().to_string())
                .content_type("application/json");
            if let Some(s) = session {
                p = p.group_id(s.to_string());
            }
            p.build()
        })
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
    send_message(conn, address, text_message(body, props, None, None)).await
}

/// Send one text message in session `session_id`.
pub async fn send_session_text(
    conn: &mut ServiceBusConnection,
    address: &str,
    session_id: &str,
    body: &str,
    props: &[(&str, &str)],
) -> Result<()> {
    send_message(
        conn,
        address,
        text_message(body, props, None, Some(session_id)),
    )
    .await
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
    send_message(
        conn,
        address,
        text_message(body, props, Some(enqueue_at), None),
    )
    .await
}

/// Attach a peek-lock receiver holding exactly one credit that is not refilled
/// after settling, so the broker never locks a second message we do not look at.
async fn attach_peek_lock(conn: &mut ServiceBusConnection, address: &str) -> Result<Receiver> {
    authorize(conn, address).await?;
    let mut receiver = step(
        "attach receiver",
        Receiver::builder()
            .name(link_name("rx"))
            .source(address.to_string())
            .receiver_settle_mode(ReceiverSettleMode::Second)
            .credit_mode(CreditMode::Manual)
            .attach(conn.session_mut()),
    )
    .await?;
    step("grant credit", receiver.set_credit(1)).await?;
    Ok(receiver)
}

/// Settlement applied to the one delivery that was picked.
enum Pick {
    DeadLetter { reason: String, description: String },
    Defer,
}

fn dead_letter_error(reason: &str, description: &str) -> AmqpError {
    let mut info = Fields::default();
    info.insert(Symbol::from("DeadLetterReason"), Value::from(reason));
    info.insert(
        Symbol::from("DeadLetterErrorDescription"),
        Value::from(description),
    );
    AmqpError::new(
        ErrorCondition::Custom(Symbol::from("com.microsoft:dead-letter")),
        None,
        Some(info),
    )
}

async fn settle_pick(
    receiver: &mut Receiver,
    delivery: &Delivery<Body<Value>>,
    pick: &Pick,
) -> Result<()> {
    match pick {
        Pick::DeadLetter {
            reason,
            description,
        } => {
            step(
                "dead-letter",
                receiver.reject(delivery, dead_letter_error(reason, description)),
            )
            .await
        }
        Pick::Defer => {
            step(
                "defer",
                receiver.modify(
                    delivery,
                    Modified {
                        delivery_failed: None,
                        undeliverable_here: Some(true),
                        message_annotations: None,
                    },
                ),
            )
            .await
        }
    }
}

/// Whether the delivery's application property `key` is the string `value`.
fn has_prop(delivery: &Delivery<Body<Value>>, key: &str, value: &str) -> bool {
    delivery
        .message()
        .application_properties
        .as_ref()
        .and_then(|a| a.0.get(key))
        .is_some_and(|v| matches!(v, SimpleValue::String(s) if s == value))
}

/// Receive in peek-lock until a delivery matches `prop` (or `wait` elapses),
/// settle that one with `pick`, and release every non-matching delivery that
/// was received on the way. Non-matching ones are released only at the end so
/// the broker cannot hand them straight back; credit is granted one message at
/// a time so nothing unexamined gets locked.
async fn settle_first_matching(
    conn: &mut ServiceBusConnection,
    address: &str,
    prop: (&str, &str),
    pick: Pick,
    wait: Duration,
) -> Result<bool> {
    authorize(conn, address).await?;
    let mut receiver = step(
        "attach receiver",
        Receiver::builder()
            .name(link_name("rx"))
            .source(address.to_string())
            .receiver_settle_mode(ReceiverSettleMode::Second)
            .credit_mode(CreditMode::Manual)
            .attach(conn.session_mut()),
    )
    .await?;
    let deadline = tokio::time::Instant::now() + wait;
    let mut skipped = Vec::new();
    let mut picked = false;
    let outcome = loop {
        // One credit per receive: the broker never locks messages we did not look at.
        if let Err(e) = step("grant credit", receiver.set_credit(1)).await {
            break Err(e);
        }
        match tokio::time::timeout_at(deadline, receiver.recv::<Body<Value>>()).await {
            Err(_) => break Ok(()),
            Ok(Err(e)) => break Err(Error::Protocol(format!("receive: {e}"))),
            Ok(Ok(delivery)) if has_prop(&delivery, prop.0, prop.1) => {
                match settle_pick(&mut receiver, &delivery, &pick).await {
                    Ok(()) => picked = true,
                    Err(e) => break Err(e),
                }
                break Ok(());
            }
            Ok(Ok(delivery)) => skipped.push(delivery),
        }
    };
    for delivery in &skipped {
        let _ = tokio::time::timeout(STEP_TIMEOUT, receiver.release(delivery)).await;
    }
    close_receiver(receiver).await;
    outcome.map(|()| picked)
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
    let pick = Pick::DeadLetter {
        reason: reason.to_string(),
        description: description.to_string(),
    };
    settle_pick(&mut receiver, &delivery, &pick).await?;
    close_receiver(receiver).await;
    Ok(true)
}

/// Like [`dead_letter_next`], but only the first message whose application
/// property `prop.0` equals `prop.1` is dead-lettered; others are released.
pub async fn dead_letter_next_matching(
    conn: &mut ServiceBusConnection,
    address: &str,
    prop: (&str, &str),
    reason: &str,
    description: &str,
    wait: Duration,
) -> Result<bool> {
    let pick = Pick::DeadLetter {
        reason: reason.to_string(),
        description: description.to_string(),
    };
    settle_first_matching(conn, address, prop, pick, wait).await
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
    settle_pick(&mut receiver, &delivery, &Pick::Defer).await?;
    close_receiver(receiver).await;
    Ok(true)
}

/// Defer the first message whose application property `prop.0` equals
/// `prop.1`; other messages received on the way are released (abandoned).
/// Returns whether a message was deferred within `wait`.
pub async fn defer_next_matching(
    conn: &mut ServiceBusConnection,
    address: &str,
    prop: (&str, &str),
    wait: Duration,
) -> Result<bool> {
    settle_first_matching(conn, address, prop, Pick::Defer, wait).await
}

/// Receive-and-delete everything in `address`; returns how many messages were removed.
///
/// The link is receive-and-delete (settled by the sender), so there is
/// nothing to accept. Passes repeat until two consecutive passes receive
/// nothing, each pass ending after `idle` without a message. Deferred and
/// scheduled messages cannot be received this way and are not touched.
pub async fn drain(
    conn: &mut ServiceBusConnection,
    address: &str,
    idle: Duration,
) -> Result<usize> {
    let mut drained = 0;
    let mut empty_passes = 0;
    while empty_passes < 2 {
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
        let mut pass = 0;
        while let Ok(result) = tokio::time::timeout(idle, receiver.recv::<Body<Value>>()).await {
            result.map_err(|e| Error::Protocol(format!("drain receive: {e}")))?;
            pass += 1;
        }
        close_receiver(receiver).await;
        drained += pass;
        empty_passes = if pass == 0 { empty_passes + 1 } else { 0 };
    }
    Ok(drained)
}

/// What the M2a link-semantics probe observed (see [`probe_m2a_link_semantics`]).
#[derive(Debug, Clone)]
pub struct LinkProbe {
    /// (a) Delivery count after a peek-lock receive and a link `released` (expected 0).
    pub released_delivery_count: u32,
    /// (b) Delivery count after a further receive and `settle(Abandon)` (expected 1).
    pub abandoned_delivery_count: u32,
    /// (c) Whether `update-disposition` completed a link-received message.
    pub management_complete_removed: bool,
    /// (c) The refusal, when `update-disposition` failed.
    pub management_complete_error: Option<String>,
    /// (d) Messages a receive-and-delete `receive(3, 2 s)` returned with 2 available.
    pub drain_returned: usize,
    /// (d) `drain_shortfall()` after that receive.
    pub drain_credit_left: u32,
    /// (d) Tagged messages still in the queue after closing and waiting 2 s.
    pub left_after_drain: usize,
}

fn probe_tagged(m: &PeekedMessage, tag: &str) -> bool {
    matches!(m.application_properties.get("probe"), Some(PropValue::String(s)) if s == tag)
}

/// Every message in `address` (all states), paged by sequence number.
async fn peek_everything(
    conn: &mut ServiceBusConnection,
    address: &str,
) -> Result<Vec<PeekedMessage>> {
    let mut all = Vec::new();
    let mut from = 0;
    loop {
        let page = super::peek::peek(conn, address, from, super::peek::MAX_PER_REQUEST).await?;
        let Some(last) = page.last() else {
            return Ok(all);
        };
        from = last.sequence_number + 1;
        all.extend(page);
    }
}

async fn peek_sequence(
    conn: &mut ServiceBusConnection,
    address: &str,
    seq: i64,
) -> Result<Option<PeekedMessage>> {
    Ok(super::peek::peek(conn, address, seq, 1)
        .await?
        .into_iter()
        .find(|m| m.sequence_number == seq))
}

/// Receive (peek-lock) until the message tagged `tag` arrives; others met on
/// the way are released afterwards so the broker cannot hand them straight back.
async fn receive_probe_message(rx: &mut AmqpReceiver, tag: &str) -> Result<ReceivedMessage> {
    let mut others = Vec::new();
    let mut found = None;
    for _ in 0..50 {
        let Some(m) = rx.receive(1, Duration::from_secs(10)).await?.pop() else {
            break;
        };
        if probe_tagged(&m.message, tag) {
            found = Some(m);
            break;
        }
        others.extend(m.lock_token);
    }
    for token in others {
        let _ = rx.release(token).await;
    }
    found.ok_or_else(|| Error::Timeout(format!("the probe message {tag} did not arrive")))
}

fn token_of(m: &ReceivedMessage) -> Result<LockToken> {
    m.lock_token
        .ok_or_else(|| Error::Protocol("a peek-lock delivery came without a lock token".into()))
}

/// Check the link semantics the M2a operations rest on, on `address` (the
/// emulator's `m1a-browse`): (a) `released` leaves the delivery count alone,
/// (b) abandon raises it, (c) `update-disposition` completes a link-received
/// message, (d) a receive-and-delete drain returns what was available and
/// removes nothing after it. Empties the queue first.
pub async fn probe_m2a_link_semantics(
    conn: &mut ServiceBusConnection,
    address: &str,
) -> Result<LinkProbe> {
    drain(conn, address, Duration::from_secs(1)).await?;
    let run = uuid::Uuid::new_v4().simple().to_string();
    let tag_a = format!("{run}-a");
    let tag_d = format!("{run}-d");
    let settle_pause = Duration::from_secs(1);

    send_text(conn, address, "m2a probe a", &[("probe", &tag_a)]).await?;
    let mut rx = conn
        .open_receiver(&ReceiverSpec::peek_lock(address))
        .await?;

    // (a) release over the link.
    let m = receive_probe_message(&mut rx, &tag_a).await?;
    let seq = m.message.sequence_number;
    rx.release(token_of(&m)?).await?;
    tokio::time::sleep(settle_pause).await;
    let released_delivery_count = peek_sequence(conn, address, seq)
        .await?
        .ok_or_else(|| Error::Protocol(format!("(a) message {seq} vanished after release")))?
        .delivery_count;

    // (b) abandon through update-disposition (control case).
    let m = receive_probe_message(&mut rx, &tag_a).await?;
    rx.settle(token_of(&m)?, &Settlement::Abandon).await?;
    tokio::time::sleep(settle_pause).await;
    let abandoned_delivery_count = peek_sequence(conn, address, seq)
        .await?
        .ok_or_else(|| Error::Protocol(format!("(b) message {seq} vanished after abandon")))?
        .delivery_count;

    // (c) complete a link-received message through update-disposition.
    let m = receive_probe_message(&mut rx, &tag_a).await?;
    let token = token_of(&m)?;
    let management_complete_error = rx
        .settle(token, &Settlement::Complete)
        .await
        .err()
        .map(|e| e.to_string());
    tokio::time::sleep(settle_pause).await;
    let management_complete_removed =
        management_complete_error.is_none() && peek_sequence(conn, address, seq).await?.is_none();
    if management_complete_error.is_some() {
        // Clean up over the link; the probe reports the refusal above.
        let _ = rx.settle_on_link(token, &Settlement::Complete).await;
    }
    rx.close().await;

    // (d) receive-and-delete with more credit than messages.
    send_text(conn, address, "m2a probe d1", &[("probe", &tag_d)]).await?;
    send_text(conn, address, "m2a probe d2", &[("probe", &tag_d)]).await?;
    let mut rd = conn
        .open_receiver(&ReceiverSpec::receive_and_delete(address))
        .await?;
    let got = rd.receive(3, Duration::from_secs(2)).await?;
    let drain_returned = got.len();
    let drain_credit_left = rd.drain_shortfall();
    rd.close().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let left_after_drain = peek_everything(conn, address)
        .await?
        .iter()
        .filter(|m| probe_tagged(m, &tag_d))
        .count();

    Ok(LinkProbe {
        released_delivery_count,
        abandoned_delivery_count,
        management_complete_removed,
        management_complete_error,
        drain_returned,
        drain_credit_left,
        left_after_drain,
    })
}
