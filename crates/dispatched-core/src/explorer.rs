//! The facade every surface uses: one namespace, lazily-opened AMQP connection.

use crate::messaging::{
    PeekOptions, PeekPage, PeekSource, PeekedMessage, SequenceCollection, ServiceBusConnection,
    StateCounts, collect_sequences, count_messages, peek_one, peek_tail, scan_messages,
};
use crate::mgmt::{EntityCounts, ManagementClient, QueueInfo, SubscriptionInfo, TopicInfo};
use crate::namespace::{EntityPath, Namespace, PeekView, SubQueue};
use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::safety::CancellationToken;
use tokio::sync::{Mutex, MutexGuard};

type Guard<'a> = MutexGuard<'a, Option<ServiceBusConnection>>;

/// Browse and (from M2) operate one namespace.
pub struct Explorer {
    ns: Namespace,
    mgmt: ManagementClient,
    amqp: Mutex<Option<ServiceBusConnection>>,
}

impl Explorer {
    /// Most messages examined per sub-queue when counting by peek.
    const COUNT_CAP: u32 = 10_000;
    /// Most entities per list call that get counted by peek.
    const MAX_COUNTED_ENTITIES: usize = 50;

    /// Explorer for `ns`. No network I/O until first use.
    pub fn new(ns: Namespace) -> Result<Self> {
        let mgmt = ManagementClient::new(&ns)?;
        Ok(Self {
            ns,
            mgmt,
            amqp: Mutex::new(None),
        })
    }

    /// The namespace.
    pub fn namespace(&self) -> &Namespace {
        &self.ns
    }

    /// The management client, for callers that need plain listings without
    /// the approximate-count work (e.g. connection diagnostics).
    pub fn management_client(&self) -> &ManagementClient {
        &self.mgmt
    }

    /// Queues with counts. Where the service reports none (the emulator),
    /// counts are taken by peeking and marked approximate; if that fails
    /// (for example no Listen right), `counts` stays `None`.
    pub async fn list_queues(&self) -> Result<Vec<QueueInfo>> {
        let mut queues = self.mgmt.list_queues().await?;
        let mut missing: Vec<(String, &mut Option<EntityCounts>)> = queues
            .iter_mut()
            .filter(|q| q.counts.is_none())
            .map(|q| (q.name.clone(), &mut q.counts))
            .collect();
        fill_counts(&mut missing, "queues", Self::MAX_COUNTED_ENTITIES, |name| {
            let path = EntityPath::queue(&name);
            async move { self.count_by_peek(&path, Self::COUNT_CAP).await }
        })
        .await;
        Ok(queues)
    }

    /// Topics.
    pub async fn list_topics(&self) -> Result<Vec<TopicInfo>> {
        self.mgmt.list_topics().await
    }

    /// Subscriptions of `topic`.
    ///
    /// Missing counts are filled by peeking, as in [`Explorer::list_queues`].
    pub async fn list_subscriptions(&self, topic: &str) -> Result<Vec<SubscriptionInfo>> {
        let mut subs = self.mgmt.list_subscriptions(topic).await?;
        let mut missing: Vec<(String, &mut Option<EntityCounts>)> = subs
            .iter_mut()
            .filter(|s| s.counts.is_none())
            .map(|s| (s.name.clone(), &mut s.counts))
            .collect();
        fill_counts(
            &mut missing,
            "subscriptions",
            Self::MAX_COUNTED_ENTITIES,
            |name| {
                let path = EntityPath::subscription(topic, &name);
                async move { self.count_by_peek(&path, Self::COUNT_CAP).await }
            },
        )
        .await;
        Ok(subs)
    }

    /// Count messages by peeking, for namespaces such as the emulator that
    /// report no counts. At most `cap` messages are examined per sub-queue;
    /// the result is `approximate`, and `truncated` when a cap was hit (the
    /// numbers are then lower bounds). `active` includes deferred messages, as
    /// Service Bus counts them.
    pub async fn count_by_peek(&self, entity: &EntityPath, cap: u32) -> Result<EntityCounts> {
        with_one_retry(|_attempt| self.count_once(entity, cap)).await
    }

    /// One counting attempt; drops the cached connection on connection-level
    /// failures, like [`Explorer::peek_once`].
    async fn count_once(&self, entity: &EntityPath, cap: u32) -> Result<EntityCounts> {
        let mut guard = self.connection().await?;
        let result = count_sub_queues(guard.as_mut().expect("opened"), entity, cap).await;
        Self::settle(&mut guard, result).await
    }

    /// Peek `view` of `entity` according to `opts`.
    pub async fn peek(
        &self,
        entity: &EntityPath,
        view: PeekView,
        opts: &PeekOptions,
    ) -> Result<PeekPage> {
        let address = entity.address(view.sub_queue());
        with_one_retry(|_attempt| self.peek_once(&address, view, opts)).await
    }

    /// One peek attempt on the cached connection (opened on demand). A
    /// connection-level or auth failure drops the cached connection so the
    /// next attempt starts clean.
    async fn peek_once(
        &self,
        address: &str,
        view: PeekView,
        opts: &PeekOptions,
    ) -> Result<PeekPage> {
        let mut guard = self.connection().await?;
        let result = scan_messages(guard.as_mut().expect("opened"), address, view, opts).await;
        Self::settle(&mut guard, result).await
    }

    /// The last `opts.max` matches of `view` ("from the end"); see [`peek_tail`]. Locks the
    /// connection per batch ([`PerBatch`]), not for the whole search.
    pub async fn peek_tail(
        &self,
        entity: &EntityPath,
        view: PeekView,
        opts: &PeekOptions,
    ) -> Result<PeekPage> {
        let address = entity.address(view.sub_queue());
        with_one_retry(|_| async {
            peek_tail(&mut PerBatch { explorer: self }, &address, view, opts).await
        })
        .await
    }

    /// One message by sequence number, if it is still in `view`.
    pub async fn peek_message(
        &self,
        entity: &EntityPath,
        view: PeekView,
        sequence: i64,
    ) -> Result<Option<PeekedMessage>> {
        let address = entity.address(view.sub_queue());
        with_one_retry(|_| async {
            let mut guard = self.connection().await?;
            let result = peek_one(guard.as_mut().expect("opened"), &address, view, sequence).await;
            Self::settle(&mut guard, result).await
        })
        .await
    }

    /// Selected messages again, by sequence number; see [`collect_sequences`]. Not retried
    /// (a retry would repeat a long scan); the caller reports the error. Locks the connection
    /// per batch ([`PerBatch`]), so an export does not block browsing.
    pub async fn collect_sequences(
        &self,
        entity: &EntityPath,
        view: PeekView,
        sequences: &[i64],
        budget: u64,
        cancel: Option<&CancellationToken>,
    ) -> Result<SequenceCollection> {
        let address = entity.address(view.sub_queue());
        collect_sequences(
            &mut PerBatch { explorer: self },
            &address,
            view,
            sequences,
            budget,
            cancel,
        )
        .await
    }

    /// The cached connection, opened on demand. Hold the guard for the whole operation.
    async fn connection(&self) -> Result<Guard<'_>> {
        let mut guard = self.amqp.lock().await;
        if guard.is_none() {
            *guard = Some(ServiceBusConnection::open(&self.ns).await?);
        }
        Ok(guard)
    }

    /// Pass `result` through, dropping the cached connection after a connection-level failure
    /// (see [`retry_after`]) so the next attempt starts clean.
    async fn settle<T>(guard: &mut Guard<'_>, result: Result<T>) -> Result<T> {
        if result.as_ref().is_err_and(|e| retry_after(e, 0))
            && let Some(conn) = guard.take()
        {
            let _ = conn.close().await;
        }
        result
    }

    /// Close the AMQP connection, if open.
    pub async fn close(&self) {
        if let Some(conn) = self.amqp.lock().await.take() {
            let _ = conn.close().await;
        }
    }
}

/// The explorer's cached connection as a [`PeekSource`] that holds the connection lock for one
/// batch only, so other operations interleave between the batches of a long scan. A
/// connection-level failure drops the cached connection, as `settle` does.
struct PerBatch<'a> {
    explorer: &'a Explorer,
}

#[async_trait]
impl PeekSource for PerBatch<'_> {
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        let mut guard = self.explorer.connection().await?;
        let result = guard
            .as_mut()
            .expect("opened")
            .peek_batch(address, from, max)
            .await;
        Explorer::settle(&mut guard, result).await
    }

    async fn peek_session_batch(
        &mut self,
        address: &str,
        session_id: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        let mut guard = self.explorer.connection().await?;
        let result = guard
            .as_mut()
            .expect("opened")
            .peek_session_batch(address, session_id, from, max)
            .await;
        Explorer::settle(&mut guard, result).await
    }
}

/// Fill the missing counts of up to `max` entities with `count`.
///
/// A failed count leaves that entity's counts `None` and is logged. After a
/// connection-level failure (see [`retry_after`]; `count` has already retried
/// once) the remaining entities are not attempted, since each would fail the
/// same slow way. Answers from a healthy service (`NotFound`, `Forbidden`)
/// only affect the entity they concern.
async fn fill_counts<F, Fut>(
    missing: &mut [(String, &mut Option<EntityCounts>)],
    kind: &str,
    max: usize,
    mut count: F,
) where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<EntityCounts>>,
{
    for (counted, (name, slot)) in missing.iter_mut().enumerate() {
        if counted == max {
            tracing::info!("approximate counts limited to the first {max} {kind}");
            break;
        }
        match count(name.clone()).await {
            Ok(c) => **slot = Some(c),
            Err(e) => {
                tracing::debug!(entity = %name, error = %e, "approximate count failed");
                if retry_after(&e, 0) {
                    tracing::debug!("stopping approximate counts after a connection-level failure");
                    break;
                }
            }
        }
    }
}

async fn count_sub_queues(
    conn: &mut (impl PeekSource + ?Sized),
    entity: &EntityPath,
    cap: u32,
) -> Result<EntityCounts> {
    let main = count_messages(conn, &entity.address(SubQueue::Active), cap).await?;
    let dlq = count_messages(conn, &entity.address(SubQueue::DeadLetter), cap).await?;
    // Entities without transfer support have no transfer dead-letter queue.
    let tdlq = match count_messages(conn, &entity.address(SubQueue::TransferDeadLetter), cap).await
    {
        Ok(c) => c,
        Err(Error::NotFound(_)) => StateCounts::default(),
        Err(e) => return Err(e),
    };
    Ok(EntityCounts {
        active: main.active + main.deferred,
        dead_letter: dlq.total,
        scheduled: main.scheduled,
        transfer: 0,
        transfer_dead_letter: tdlq.total,
        approximate: true,
        truncated: main.truncated || dlq.truncated || tdlq.truncated,
    })
}

/// Whether a failed operation should be retried on a fresh connection.
/// `Auth` usually means a token expired mid-session, and `Network`,
/// `Protocol` and `Timeout` mean the connection or a link died (the broker
/// closes links when their token lapses). A new connection re-puts fresh
/// tokens. Answers from a healthy service (`Forbidden`, `NotFound`, ...) are
/// never retried. These are also exactly the errors that poison the cached
/// connection.
pub fn retry_after(err: &Error, attempt: u32) -> bool {
    attempt == 0
        && matches!(
            err,
            Error::Auth(_) | Error::Network(_) | Error::Protocol(_) | Error::Timeout(_)
        )
}

/// Run `f(attempt)` and, if [`retry_after`] says so, run it once more.
/// At most two calls; the second result is returned as is.
async fn with_one_retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match f(0).await {
        Err(e) if retry_after(&e, 0) => f(1).await,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>(_: &T) {}

    /// Compile-time check: the browse futures can run on a multi-threaded runtime (Tauri
    /// commands require it). Never called; nothing touches the network.
    #[allow(dead_code)]
    fn browse_futures_are_send(e: &Explorer, p: &EntityPath, o: &PeekOptions) {
        assert_send(&e.peek_tail(p, PeekView::Active, o));
        assert_send(&e.peek_message(p, PeekView::Active, 1));
        assert_send(&e.collect_sequences(p, PeekView::Active, &[1], 10, None));
        assert_send(&e.peek(p, PeekView::Active, o));
    }

    #[test]
    fn retry_set_is_auth_and_connection_level_only() {
        assert!(!retry_after(
            &Error::Forbidden {
                needed: None,
                detail: "x".into()
            },
            0
        ));
        for e in [
            Error::Auth("x".into()),
            Error::Network("x".into()),
            Error::Protocol("x".into()),
            Error::Timeout("x".into()),
        ] {
            assert!(retry_after(&e, 0), "{e:?}");
            assert!(!retry_after(&e, 1), "{e:?}");
        }
        for e in [
            Error::NotFound("q".into()),
            Error::Invalid("x".into()),
            Error::Io("x".into()),
            Error::Cancelled,
        ] {
            assert!(!retry_after(&e, 0), "{e:?}");
        }
    }

    #[tokio::test]
    async fn one_retry_succeeds_second_time_after_protocol_error() {
        let calls = std::cell::Cell::new(0u32);
        let out = with_one_retry(|attempt| {
            calls.set(calls.get() + 1);
            async move {
                if attempt == 0 {
                    Err(Error::Protocol("link closed".into()))
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(out.unwrap(), 7);
        assert_eq!(calls.get(), 2);
    }

    #[tokio::test]
    async fn one_retry_skips_forbidden() {
        let calls = std::cell::Cell::new(0u32);
        let out: Result<()> = with_one_retry(|_| {
            calls.set(calls.get() + 1);
            async {
                Err(Error::Forbidden {
                    needed: None,
                    detail: "x".into(),
                })
            }
        })
        .await;
        assert!(matches!(out, Err(Error::Forbidden { .. })));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn one_retry_returns_second_error_and_calls_at_most_twice() {
        let calls = std::cell::Cell::new(0u32);
        let out: Result<()> = with_one_retry(|attempt| {
            calls.set(calls.get() + 1);
            async move {
                if attempt == 0 {
                    Err(Error::Auth("first".into()))
                } else {
                    Err(Error::Network("second".into()))
                }
            }
        })
        .await;
        assert!(matches!(out, Err(Error::Network(m)) if m == "second"));
        assert_eq!(calls.get(), 2);
    }

    /// Serves fixed message lists by address; unknown addresses are `NotFound`.
    struct ByAddress(std::collections::HashMap<String, Vec<crate::messaging::PeekedMessage>>);

    #[async_trait::async_trait]
    impl PeekSource for ByAddress {
        async fn peek_batch(
            &mut self,
            address: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<crate::messaging::PeekedMessage>> {
            let list = self
                .0
                .get(address)
                .ok_or_else(|| Error::NotFound(address.into()))?;
            Ok(list
                .iter()
                .filter(|m| m.sequence_number >= from)
                .take(max as usize)
                .cloned()
                .collect())
        }
    }

    fn msgs(n: i64) -> Vec<crate::messaging::PeekedMessage> {
        (1..=n).map(crate::messaging::test_message).collect()
    }

    #[tokio::test]
    async fn sub_queue_counts_combine_and_flag_truncation() {
        let q = EntityPath::queue("orders");
        let mut deferred = msgs(3);
        deferred[0].state = crate::messaging::MessageState::Deferred;
        let mut src = ByAddress(
            [
                ("orders".to_string(), deferred),
                ("orders/$DeadLetterQueue".to_string(), msgs(5)),
                ("orders/$Transfer/$DeadLetterQueue".to_string(), msgs(1)),
            ]
            .into(),
        );
        let c = count_sub_queues(&mut src, &q, 100).await.unwrap();
        assert_eq!(
            (
                c.active,
                c.dead_letter,
                c.scheduled,
                c.transfer,
                c.transfer_dead_letter
            ),
            (3, 5, 0, 0, 1),
            "deferred counts as active"
        );
        assert!(c.approximate && !c.truncated);
        let c = count_sub_queues(&mut src, &q, 4).await.unwrap();
        assert!(c.approximate && c.truncated, "the DLQ has more than 4");
        assert_eq!(c.dead_letter, 4);
    }

    #[tokio::test]
    async fn missing_transfer_dlq_counts_as_zero_but_other_errors_propagate() {
        let q = EntityPath::queue("orders");
        let mut src = ByAddress(
            [
                ("orders".to_string(), msgs(2)),
                ("orders/$DeadLetterQueue".to_string(), msgs(0)),
            ]
            .into(),
        );
        let c = count_sub_queues(&mut src, &q, 100).await.unwrap();
        assert_eq!((c.active, c.transfer_dead_letter), (2, 0));
        // A missing main queue is an error.
        let mut src = ByAddress(Default::default());
        assert!(matches!(
            count_sub_queues(&mut src, &q, 100).await,
            Err(Error::NotFound(_))
        ));
    }

    fn fake_counts() -> EntityCounts {
        EntityCounts {
            approximate: true,
            ..EntityCounts::default()
        }
    }

    async fn run_fill(
        n: usize,
        max: usize,
        fail_first_with: impl Fn() -> Error,
    ) -> (Vec<Option<EntityCounts>>, Vec<String>) {
        let mut slots: Vec<Option<EntityCounts>> = vec![None; n];
        let mut missing: Vec<(String, &mut Option<EntityCounts>)> = slots
            .iter_mut()
            .enumerate()
            .map(|(i, s)| (format!("e{}", i + 1), s))
            .collect();
        let attempted = std::cell::RefCell::new(Vec::new());
        fill_counts(&mut missing, "queues", max, |name| {
            attempted.borrow_mut().push(name.clone());
            let first = name == "e1";
            let err = fail_first_with();
            async move { if first { Err(err) } else { Ok(fake_counts()) } }
        })
        .await;
        (slots, attempted.into_inner())
    }

    #[tokio::test]
    async fn connection_level_failure_stops_counting_the_rest() {
        for e in [
            || Error::Network("x".into()),
            || Error::Auth("x".into()),
            || Error::Protocol("x".into()),
            || Error::Timeout("x".into()),
        ] {
            let (slots, attempted) = run_fill(4, 50, e).await;
            assert_eq!(attempted, ["e1"]);
            assert!(slots.iter().all(Option::is_none));
        }
    }

    #[tokio::test]
    async fn healthy_service_errors_only_skip_that_entity() {
        for e in [
            || Error::NotFound("x".into()),
            || Error::Forbidden {
                needed: None,
                detail: "x".into(),
            },
        ] {
            let (slots, attempted) = run_fill(3, 50, e).await;
            assert_eq!(attempted, ["e1", "e2", "e3"]);
            assert!(slots[0].is_none());
            assert!(slots[1].is_some() && slots[2].is_some());
        }
    }

    #[tokio::test]
    async fn counting_is_capped_per_call() {
        let (slots, attempted) = run_fill(5, 2, || Error::NotFound("x".into())).await;
        assert_eq!(attempted, ["e1", "e2"]);
        assert!(slots[2..].iter().all(Option::is_none));
    }
}
