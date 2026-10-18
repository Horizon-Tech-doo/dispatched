//! Pipelined AMQP sender, and the encoding for scheduled messages.
//!
//! `send_all` uses `send_batchable`, which hands back a delivery future that
//! resolves with the broker's disposition, so up to [`MAX_IN_FLIGHT`] sends are
//! unsettled at once and each message's outcome is the broker's real answer.

use super::lockops::ScheduleEntry;
use super::management::{SETUP_TIMEOUT, map_remote_error};
use super::outgoing::OutgoingMessage;
use crate::namespace::EntityPath;
use crate::{Error, Result};
use fe2o3_amqp::Sender;
use fe2o3_amqp::link::delivery::DeliveryFut;
use fe2o3_amqp::link::{DetachError, SendError};
use fe2o3_amqp::types::messaging::message::__private::Serializable;
use fe2o3_amqp::types::messaging::{Body, Message, Outcome};
use fe2o3_amqp::types::primitives::Value;
use std::collections::VecDeque;
use std::future::Future;
use std::time::Duration;
use time::OffsetDateTime;

/// Unsettled sends kept in flight on one link.
pub(crate) const MAX_IN_FLIGHT: usize = 64;

/// Messages (or sequence numbers) per schedule / cancel request.
pub(crate) const REQUEST_CHUNK: usize = 100;

/// Bound for closing a sender; an unresponsive peer must not stall the caller.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

type AmqpMessage = Message<Body<Value>>;

/// The one place that refuses "send to a subscription": messages are sent to a
/// queue or a topic.
pub fn send_target(name: &str) -> Result<EntityPath> {
    let path = EntityPath::parse(name)?;
    if path.subscription.is_some() {
        return Err(Error::Invalid(format!(
            "send to the topic `{}`, not to its subscription",
            path.entity
        )));
    }
    Ok(path)
}

/// Split `items` into request-sized chunks.
pub(crate) fn chunks<T>(items: &[T]) -> std::slice::Chunks<'_, T> {
    items.chunks(REQUEST_CHUNK)
}

/// Encode `m` for `schedule-message`: the AMQP bytes with the enqueue time set,
/// the message id (generated when missing; the broker requires one), and the
/// session and partition keys the request carries beside the message.
pub(crate) fn encode_for_schedule(
    m: &OutgoingMessage,
    at: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<ScheduleEntry> {
    if at < now {
        return Err(Error::Invalid(
            "the schedule time is in the past; pick a time after now".into(),
        ));
    }
    let mut m = m.clone();
    m.scheduled_enqueue_time = Some(at);
    let id = match &m.message_id {
        Some(id) => id.clone(),
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            m.message_id = Some(id.clone());
            id
        }
    };
    let amqp = m.to_amqp(now)?;
    let bytes = serde_amqp::to_vec(&Serializable(amqp))
        .map_err(|e| Error::Protocol(format!("encode the scheduled message: {e}")))?;
    Ok((bytes, id, m.session_id.clone(), m.partition_key.clone()))
}

/// Run `items` through `call` in chunks of [`REQUEST_CHUNK`], reporting a result per
/// original index. See the tests for the contract; [`Chunked`] is the same loop for callers
/// that cannot pass an async closure (one inside an `async_trait` future must be `Send`
/// for every lifetime, which rustc cannot prove for a closure borrowing its captures).
#[cfg(test)]
pub(crate) async fn run_chunks<T, R>(
    items: Vec<Result<T>>,
    mut call: impl AsyncFnMut(&[T]) -> Result<Vec<R>>,
    mut each: impl FnMut(usize, Result<R>),
) -> Result<()> {
    let mut chunks = Chunked::new(items, &mut each);
    while let Some(chunk) = chunks.next_chunk() {
        let outcome = call(chunk).await;
        chunks.finish(outcome, &mut each)?;
    }
    Ok(())
}

/// Items split into request-sized chunks, with the result of each chunk reported per
/// original index. Drive it with `next_chunk` / `finish` until `next_chunk` is `None` or
/// `finish` fails.
pub(crate) struct Chunked<T> {
    indexes: Vec<usize>,
    values: Vec<T>,
    first: usize,
}

impl<T> Chunked<T> {
    /// Items that failed to encode are reported through `each` at once and left out of
    /// the requests.
    pub(crate) fn new<R>(items: Vec<Result<T>>, each: &mut impl FnMut(usize, Result<R>)) -> Self {
        let mut indexes = Vec::with_capacity(items.len());
        let mut values = Vec::with_capacity(items.len());
        for (i, item) in items.into_iter().enumerate() {
            match item {
                Ok(v) => {
                    indexes.push(i);
                    values.push(v);
                }
                Err(e) => each(i, Err(e)),
            }
        }
        Self {
            indexes,
            values,
            first: 0,
        }
    }

    /// The next chunk to send, `None` when all were sent.
    pub(crate) fn next_chunk(&self) -> Option<&[T]> {
        chunks(&self.values[self.first..]).next()
    }

    /// Report the outcome of the chunk `next_chunk` gave. A failure (or a result count that
    /// does not match the chunk) fails that chunk's items and every later one, and is
    /// returned.
    pub(crate) fn finish<R>(
        &mut self,
        outcome: Result<Vec<R>>,
        each: &mut impl FnMut(usize, Result<R>),
    ) -> Result<()> {
        let len = (self.values.len() - self.first).min(REQUEST_CHUNK);
        let outcome = match outcome {
            Ok(got) if got.len() == len => Ok(got),
            Ok(got) => Err(Error::Protocol(format!(
                "the broker handled {} messages but returned {} results; they may have taken effect, check the entity before retrying",
                len,
                got.len()
            ))),
            Err(e) => Err(e),
        };
        match outcome {
            Ok(got) => {
                for (i, r) in self.indexes[self.first..self.first + len].iter().zip(got) {
                    each(*i, Ok(r));
                }
                self.first += len;
                Ok(())
            }
            Err(e) => {
                for i in &self.indexes[self.first..] {
                    each(*i, Err(copy_error(&e)));
                }
                self.first = self.values.len();
                Err(e)
            }
        }
    }
}

/// What the pipeline needs from a link; the real one wraps [`Sender`].
trait Submit {
    /// The future that resolves with the broker's disposition.
    type Settle: Future<Output = std::result::Result<Outcome, SendError>>;
    /// Put one message on the wire; returns once it is transferred.
    async fn submit(
        &mut self,
        message: AmqpMessage,
    ) -> std::result::Result<Self::Settle, SendError>;
}

impl Submit for Sender {
    type Settle = DeliveryFut<std::result::Result<Outcome, SendError>>;

    async fn submit(
        &mut self,
        message: AmqpMessage,
    ) -> std::result::Result<Self::Settle, SendError> {
        self.send_batchable(message).await
    }
}

/// A sender link to a queue or topic.
pub(crate) struct AmqpSender {
    sender: Sender,
}

impl AmqpSender {
    pub(super) fn new(sender: Sender) -> Self {
        Self { sender }
    }

    /// Send `batch` in order, pipelining up to [`MAX_IN_FLIGHT`] unsettled
    /// sends, and report each message's outcome by index through `each`.
    /// A message the broker does not accept is that message's
    /// `Error::Protocol("the broker refused the message: ...")`. A failure of
    /// the link fails every message not yet reported with that error and
    /// returns it.
    pub(crate) async fn send_all(
        &mut self,
        batch: &[OutgoingMessage],
        each: impl FnMut(usize, Result<()>),
    ) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        pipeline(
            &mut self.sender,
            batch.len(),
            |i| batch[i].to_amqp(now),
            each,
        )
        .await
    }

    /// Detach the link (best effort, bounded).
    pub(crate) async fn close(self) {
        if tokio::time::timeout(CLOSE_TIMEOUT, self.sender.close())
            .await
            .is_err()
        {
            tracing::warn!("closing a sender link timed out; dropped it");
        }
    }
}

async fn pipeline<S: Submit>(
    link: &mut S,
    count: usize,
    encode: impl Fn(usize) -> Result<AmqpMessage>,
    mut each: impl FnMut(usize, Result<()>),
) -> Result<()> {
    let mut inflight: VecDeque<(usize, S::Settle)> = VecDeque::new();
    let mut next = 0;
    let mut failure: Option<Error> = None;
    let mut unsettled: Option<usize> = None;
    loop {
        let can_submit = failure.is_none() && next < count && inflight.len() < MAX_IN_FLIGHT;
        if can_submit {
            let i = next;
            next += 1;
            let message = match encode(i) {
                Ok(m) => m,
                Err(e) => {
                    each(i, Err(e));
                    continue;
                }
            };
            match link.submit(message).await {
                Ok(settle) => inflight.push_back((i, settle)),
                Err(e) if is_per_message(&e) => each(i, Err(per_message_error(&e))),
                Err(e) => {
                    // Not on the wire: it fails with the rest below. What is
                    // already in flight still gets its real outcome.
                    next = i;
                    failure = Some(link_error(&e));
                }
            }
        } else if let Some((i, settle)) = inflight.pop_front() {
            match tokio::time::timeout(SETUP_TIMEOUT, settle).await {
                Ok(Ok(outcome)) => each(i, outcome_result(outcome)),
                Ok(Err(e)) => {
                    failure.get_or_insert_with(|| link_error(&e));
                    unsettled = Some(i);
                    break;
                }
                Err(_) => {
                    failure.get_or_insert_with(|| {
                        Error::Timeout(format!(
                            "the broker did not settle a send within {}s",
                            SETUP_TIMEOUT.as_secs()
                        ))
                    });
                    unsettled = Some(i);
                    break;
                }
            }
        } else {
            break;
        }
    }
    let Some(err) = failure else {
        return Ok(());
    };
    for i in unsettled
        .into_iter()
        .chain(inflight.iter().map(|(i, _)| *i))
    {
        each(i, Err(copy_error(&err)));
    }
    for i in next..count {
        each(i, Err(copy_error(&err)));
    }
    Err(err)
}

fn outcome_result(outcome: Outcome) -> Result<()> {
    match outcome {
        Outcome::Accepted(_) => Ok(()),
        Outcome::Rejected(r) => {
            let detail = r
                .error
                .map(|e| {
                    e.description
                        .unwrap_or_else(|| format!("{:?}", e.condition))
                })
                .unwrap_or_else(|| "rejected".into());
            Err(Error::Protocol(format!(
                "the broker refused the message: {detail}"
            )))
        }
        Outcome::Released(_) => Err(Error::Protocol(
            "the broker refused the message: released".into(),
        )),
        Outcome::Modified(_) => Err(Error::Protocol(
            "the broker refused the message: modified".into(),
        )),
    }
}

/// Failures that concern one message and leave the link usable.
fn is_per_message(e: &SendError) -> bool {
    matches!(
        e,
        SendError::MessageSizeExceeded(_) | SendError::MessageEncodeError
    )
}

fn per_message_error(e: &SendError) -> Error {
    match e {
        SendError::MessageSizeExceeded(_) => Error::Invalid(
            "the message is larger than the broker accepts on this link; shrink the body".into(),
        ),
        _ => Error::Invalid("the message could not be encoded".into()),
    }
}

fn link_error(e: &SendError) -> Error {
    match e {
        SendError::Detached(DetachError::RemoteDetachedWithError(remote)) => {
            map_remote_error("send", "the entity", remote)
        }
        other => Error::Protocol(format!("send failed: {other}")),
    }
}

/// `Error` is not `Clone`; every message after a link failure gets a copy.
pub(crate) fn copy_error(e: &Error) -> Error {
    match e {
        Error::Timeout(m) => Error::Timeout(m.clone()),
        Error::NotFound(m) => Error::NotFound(m.clone()),
        Error::Forbidden { needed, detail } => Error::Forbidden {
            needed: needed.clone(),
            detail: detail.clone(),
        },
        Error::Conflict(m) => Error::Conflict(m.clone()),
        Error::Network(m) => Error::Network(m.clone()),
        Error::Protocol(m) => Error::Protocol(m.clone()),
        Error::Invalid(m) => Error::Invalid(m.clone()),
        Error::Auth(m) => Error::Auth(m.clone()),
        Error::NotSignedIn(m) => Error::NotSignedIn(m.clone()),
        Error::LockLost(m) => Error::LockLost(m.clone()),
        Error::Denied(m) => Error::Denied(m.clone()),
        Error::Io(m) => Error::Io(m.clone()),
        Error::Cancelled => Error::Cancelled,
        Error::Throttled { retry_after } => Error::Throttled {
            retry_after: *retry_after,
        },
        Error::Unsupported { capability } => Error::Unsupported {
            capability: capability.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::decode_bytes;
    use fe2o3_amqp::types::definitions::{AmqpError, Error as RemoteError};
    use fe2o3_amqp::types::messaging::{Accepted, Rejected};
    use std::cell::Cell;
    use std::future::{Ready, ready};
    use std::rc::Rc;
    use time::macros::datetime;

    #[test]
    fn schedule_encoding_carries_the_time_and_a_message_id() {
        let now = datetime!(2026-10-17 12:00 UTC);
        let at = now + time::Duration::hours(1);
        let (bytes, id, session, _) = encode_for_schedule(
            &OutgoingMessage {
                session_id: Some("s".into()),
                ..OutgoingMessage::text("x")
            },
            at,
            now,
        )
        .unwrap();
        assert!(
            uuid::Uuid::parse_str(&id).is_ok(),
            "a message id is generated"
        );
        assert_eq!(session.as_deref(), Some("s"));
        let decoded = decode_bytes(&bytes).unwrap();
        assert_eq!(decoded.scheduled_enqueue_time, Some(at));
    }

    #[test]
    fn schedule_keeps_a_given_message_id_and_partition_key() {
        let now = datetime!(2026-10-17 12:00 UTC);
        let m = OutgoingMessage {
            message_id: Some("mine".into()),
            partition_key: Some("p".into()),
            ..OutgoingMessage::text("x")
        };
        let (_, id, session, partition) = encode_for_schedule(&m, now, now).unwrap();
        assert_eq!(
            (id.as_str(), session, partition.as_deref()),
            ("mine", None, Some("p"))
        );
    }

    #[test]
    fn schedule_in_the_past_is_refused() {
        let now = datetime!(2026-10-17 12:00 UTC);
        assert!(matches!(
            encode_for_schedule(&OutgoingMessage::text("x"), now - time::Duration::minutes(1), now),
            Err(Error::Invalid(m)) if m.contains("in the past")
        ));
    }

    #[test]
    fn subscription_targets_are_refused_by_one_function() {
        assert_eq!(send_target("orders").unwrap(), EntityPath::queue("orders"));
        assert!(matches!(
            send_target("events/subscriptions/all"),
            Err(Error::Invalid(m)) if m.contains("send to the topic `events`")
        ));
    }

    #[test]
    fn requests_are_chunked_by_100() {
        assert_eq!(
            chunks(&(0..250).collect::<Vec<i64>>())
                .map(|c| c.len())
                .collect::<Vec<_>>(),
            [100, 100, 50]
        );
    }

    /// A link whose sends settle immediately with a scripted outcome.
    struct Fake {
        submitted: Rc<Cell<usize>>,
        refuse_at: Option<usize>,
        fail_link_at: Option<usize>,
        fail_settle_at: Option<usize>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                submitted: Rc::new(Cell::new(0)),
                refuse_at: None,
                fail_link_at: None,
                fail_settle_at: None,
            }
        }
    }

    impl Submit for Fake {
        type Settle = Ready<std::result::Result<Outcome, SendError>>;

        async fn submit(
            &mut self,
            _message: AmqpMessage,
        ) -> std::result::Result<Self::Settle, SendError> {
            let n = self.submitted.get();
            if self.fail_link_at == Some(n) {
                return Err(SendError::Detached(DetachError::ClosedByRemote));
            }
            self.submitted.set(n + 1);
            Ok(ready(if self.fail_settle_at == Some(n) {
                Err(SendError::Detached(DetachError::RemoteDetachedWithError(
                    RemoteError::new(
                        AmqpError::UnauthorizedAccess,
                        Some("no send claim".into()),
                        None,
                    ),
                )))
            } else if self.refuse_at == Some(n) {
                Ok(Outcome::Rejected(Rejected {
                    error: Some(RemoteError::new(
                        AmqpError::InvalidField,
                        Some("bad property".into()),
                        None,
                    )),
                }))
            } else {
                Ok(Outcome::Accepted(Accepted {}))
            }))
        }
    }

    fn msg(_: usize) -> Result<AmqpMessage> {
        OutgoingMessage::text("x").to_amqp(datetime!(2026-10-17 12:00 UTC))
    }

    async fn run(link: &mut Fake, n: usize) -> (Vec<Result<()>>, Result<()>) {
        let mut seen: Vec<Option<Result<()>>> = (0..n).map(|_| None).collect();
        let r = pipeline(link, n, msg, |i, r| {
            assert!(seen[i].is_none(), "index {i} reported twice");
            seen[i] = Some(r);
        })
        .await;
        (
            seen.into_iter()
                .map(|r| r.expect("every index is reported"))
                .collect(),
            r,
        )
    }

    #[tokio::test]
    async fn every_accepted_message_is_reported_by_index() {
        let mut link = Fake::new();
        let (seen, r) = run(&mut link, 200).await;
        assert!(r.is_ok() && seen.iter().all(|r| r.is_ok()));
    }

    #[tokio::test]
    async fn no_more_than_the_window_is_in_flight() {
        let mut link = Fake::new();
        let submitted = link.submitted.clone();
        let at_first_report = Cell::new(None);
        pipeline(&mut link, 200, msg, |_, _| {
            if at_first_report.get().is_none() {
                at_first_report.set(Some(submitted.get()));
            }
        })
        .await
        .unwrap();
        assert_eq!(at_first_report.get(), Some(MAX_IN_FLIGHT));
    }

    #[tokio::test]
    async fn a_refusal_fails_only_that_message() {
        let mut link = Fake::new();
        link.refuse_at = Some(3);
        let (seen, r) = run(&mut link, 6).await;
        assert!(r.is_ok());
        for (i, o) in seen.iter().enumerate() {
            match (i, o) {
                (3, Err(Error::Protocol(m))) => {
                    assert!(m.starts_with("the broker refused the message: "), "{m}");
                    assert!(m.contains("bad property"));
                }
                (3, other) => panic!("{other:?}"),
                (_, o) => assert!(o.is_ok(), "{i}: {o:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_link_failure_on_send_keeps_real_outcomes_and_fails_the_rest() {
        let mut link = Fake::new();
        link.fail_link_at = Some(4);
        let (seen, r) = run(&mut link, 8).await;
        assert!(matches!(r, Err(Error::Protocol(_))));
        assert!(seen[..4].iter().all(|r| r.is_ok()));
        assert!(seen[4..].iter().all(|r| r.is_err()));
    }

    #[tokio::test]
    async fn a_link_failure_on_settle_fails_that_message_and_the_rest() {
        let mut link = Fake::new();
        link.fail_settle_at = Some(2);
        let (seen, r) = run(&mut link, 5).await;
        assert!(matches!(r, Err(Error::Forbidden { .. })));
        assert!(seen[..2].iter().all(|r| r.is_ok()));
        assert!(seen[2..].iter().all(|r| r.is_err()));
    }

    /// Run `run_chunks` over `items`, echoing each value + 1000, failing the
    /// chunk numbered `fail_chunk`; returns (calls' sizes, per-index results, return).
    async fn chunked(
        items: Vec<Result<i64>>,
        fail_chunk: Option<usize>,
    ) -> (Vec<usize>, Vec<Option<Result<i64>>>, Result<()>) {
        let mut sizes = Vec::new();
        let mut seen: Vec<Option<Result<i64>>> = (0..items.len()).map(|_| None).collect();
        let r = run_chunks(
            items,
            async |chunk: &[i64]| {
                let n = sizes.len();
                sizes.push(chunk.len());
                if fail_chunk == Some(n) {
                    return Err(Error::Timeout("broker stalled".into()));
                }
                Ok(chunk.iter().map(|v| v + 1000).collect())
            },
            |i, r| {
                assert!(seen[i].is_none(), "index {i} reported twice");
                seen[i] = Some(r);
            },
        )
        .await;
        (sizes, seen, r)
    }

    #[tokio::test]
    async fn chunks_that_all_succeed_report_every_index() {
        let (sizes, seen, r) = chunked((0..250).map(Ok).collect(), None).await;
        assert!(r.is_ok());
        assert_eq!(sizes, [100, 100, 50]);
        for (i, s) in seen.into_iter().enumerate() {
            assert_eq!(s.unwrap().unwrap(), i as i64 + 1000);
        }
    }

    #[tokio::test]
    async fn a_failed_middle_chunk_fails_its_indexes_and_the_rest() {
        let (sizes, seen, r) = chunked((0..250).map(Ok).collect(), Some(1)).await;
        assert!(matches!(r, Err(Error::Timeout(_))));
        assert_eq!(sizes, [100, 100], "no request after the failure");
        for (i, s) in seen.into_iter().enumerate() {
            let s = s.expect("every index is reported");
            if i < 100 {
                assert_eq!(s.unwrap(), i as i64 + 1000);
            } else {
                assert!(matches!(s, Err(Error::Timeout(_))), "{i}");
            }
        }
    }

    #[tokio::test]
    async fn an_item_that_fails_to_encode_is_left_out_and_the_rest_go() {
        let mut items: Vec<Result<i64>> = (0..3).map(Ok).collect();
        items[1] = Err(Error::Invalid("bad".into()));
        let (sizes, seen, r) = chunked(items, None).await;
        assert!(r.is_ok());
        assert_eq!(sizes, [2]);
        let seen: Vec<_> = seen.into_iter().map(Option::unwrap).collect();
        assert_eq!(*seen[0].as_ref().unwrap(), 1000);
        assert!(matches!(&seen[1], Err(Error::Invalid(m)) if m == "bad"));
        assert_eq!(*seen[2].as_ref().unwrap(), 1002);
    }

    #[tokio::test]
    async fn a_wrong_result_count_fails_the_chunk() {
        let mut seen = Vec::new();
        let r = run_chunks(
            vec![Ok(1_i64), Ok(2)],
            async |_: &[i64]| Ok(vec![1_i64]),
            |i, r: Result<i64>| seen.push((i, r.is_err())),
        )
        .await;
        assert!(matches!(r, Err(Error::Protocol(m)) if m.contains("returned 1")));
        assert_eq!(seen, [(0, true), (1, true)]);
    }

    #[test]
    fn scheduling_at_exactly_now_is_allowed() {
        let now = datetime!(2026-10-17 12:00 UTC);
        assert!(encode_for_schedule(&OutgoingMessage::text("x"), now, now).is_ok());
    }
}
