//! The facade every surface uses: one namespace, lazily-opened AMQP connection.

use crate::messaging::backend::{Each, LockReceiver, MessageSender, MessagingBackend};
use crate::messaging::receiver::{AmqpReceiver, ReceiverSpec, SessionChoice};
use crate::messaging::sender::AmqpSender;
use crate::messaging::{
    OutgoingMessage, PeekOptions, PeekPage, PeekSource, PeekedMessage, SequenceCollection,
    ServiceBusConnection, StateCounts, collect_sequences, count_messages, peek_one, peek_tail,
    scan_messages, sessions, sessions::SessionListing,
};
use crate::mgmt::{
    EntityCounts, EntityDetails, EntityKind, ManagementClient, QueueInfo, RuleInfo,
    SubscriptionInfo, TopicInfo,
};
use crate::namespace::{EntityPath, Namespace, PeekView, SubQueue};
use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::safety::CancellationToken;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::{Mutex, MutexGuard};

type Guard<'a, C> = MutexGuard<'a, Option<C>>;

/// Opens and closes the connection a [`Lifecycle`] caches. The namespace is the real one; tests
/// use a fake that counts opens.
#[async_trait]
trait Connector: Sync {
    type Conn: PeekSource;
    async fn open(&self) -> Result<Self::Conn>;
    async fn close(&self, conn: Self::Conn);
}

#[async_trait]
impl Connector for Namespace {
    type Conn = ServiceBusConnection;
    async fn open(&self) -> Result<ServiceBusConnection> {
        ServiceBusConnection::open(self).await
    }
    async fn close(&self, conn: ServiceBusConnection) {
        let _ = conn.close().await;
    }
}

/// A lazily-opened, cached connection, open until [`Lifecycle::close`]. Closed means closed: no
/// read opens it again, so a long per-batch read stops at its next batch.
struct Lifecycle<C> {
    slot: Mutex<Option<C>>,
    closed: AtomicBool,
}

impl<C> Default for Lifecycle<C> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
            closed: AtomicBool::new(false),
        }
    }
}

impl<C: PeekSource> Lifecycle<C> {
    /// The cached connection, opened on demand. Hold the guard for the whole operation.
    /// After [`Lifecycle::close`] this is `Cancelled`, and nothing is opened.
    async fn get<K: Connector<Conn = C>>(&self, connector: &K) -> Result<Guard<'_, C>> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        let mut guard = self.slot.lock().await;
        // Checked again under the lock: a close that waited for this lock has taken the
        // connection, and must not see it reopened.
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        if guard.is_none() {
            *guard = Some(connector.open().await?);
        }
        Ok(guard)
    }

    /// Close the connection, if open, for good. Marked closed before waiting for the lock, so
    /// a read that holds the lock for one batch ends at its next one.
    async fn close<K: Connector<Conn = C>>(&self, connector: &K) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(conn) = self.slot.lock().await.take() {
            connector.close(conn).await;
        }
    }
}

/// Browse and (from M2) operate one namespace.
pub struct Explorer {
    ns: Namespace,
    mgmt: ManagementClient,
    amqp: Lifecycle<ServiceBusConnection>,
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
            amqp: Lifecycle::default(),
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

    /// One entity's description; missing queue or subscription counts are counted by peeking
    /// (approximate), as in the listings.
    pub async fn entity_details(&self, kind: EntityKind, path: &str) -> Result<EntityDetails> {
        let mut d = self.mgmt.entity_details(kind, path).await?;
        if d.counts.is_none() && kind != EntityKind::Topic {
            // Counting is best effort: it never fails the details call.
            match peek_target(kind, path) {
                Ok(entity) => match self.count_by_peek(&entity, Self::COUNT_CAP).await {
                    Ok(c) => d.counts = Some(c),
                    Err(e) => {
                        tracing::debug!(entity = %path, error = %e, "approximate count failed")
                    }
                },
                Err(e) => tracing::debug!(entity = %path, error = %e, "no peek target"),
            }
        }
        Ok(d)
    }

    /// The rules of a subscription.
    pub async fn list_rules(&self, topic: &str, subscription: &str) -> Result<Vec<RuleInfo>> {
        self.mgmt.list_rules(topic, subscription).await
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
        drop_if_broken(&self.ns, &mut guard, result).await
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
        drop_if_broken(&self.ns, &mut guard, result).await
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
        with_one_retry(|_| async { peek_tail(&mut self.per_batch(), &address, view, opts).await })
            .await
    }

    /// One message by sequence number, if it is still in `view`; read through `session` when
    /// given (see [`peek_one`]).
    pub async fn peek_message(
        &self,
        entity: &EntityPath,
        view: PeekView,
        sequence: i64,
        session: Option<&str>,
    ) -> Result<Option<PeekedMessage>> {
        let address = entity.address(view.sub_queue());
        with_one_retry(|_| async {
            let mut guard = self.connection().await?;
            let result = peek_one(
                guard.as_mut().expect("opened"),
                &address,
                view,
                sequence,
                session,
            )
            .await;
            drop_if_broken(&self.ns, &mut guard, result).await
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
        session: Option<&str>,
    ) -> Result<SequenceCollection> {
        let address = entity.address(view.sub_queue());
        collect_sequences(
            &mut self.per_batch(),
            &address,
            view,
            sequences,
            budget,
            cancel,
            session,
        )
        .await
    }

    /// Sessions of `entity`: the broker's list merged with a peek tally of up to
    /// `COUNT_CAP` messages. Fails only when both fail, with the broker's error. The tally
    /// runs over [`PerBatch`] and takes the connection lock per batch; the broker list is
    /// short and keeps the lock for its few pages. Read-only: no session is accepted.
    pub async fn list_sessions(&self, entity: &EntityPath) -> Result<SessionListing> {
        let address = entity.address(SubQueue::Active);
        let broker = with_one_retry(|_| async {
            let mut guard = self.connection().await?;
            let result =
                sessions::list_session_ids(guard.as_mut().expect("opened"), &address).await;
            drop_if_broken(&self.ns, &mut guard, result).await
        })
        .await;
        let tally = with_one_retry(|_| async {
            sessions::tally_sessions(&mut self.per_batch(), &address, Self::COUNT_CAP).await
        })
        .await;
        sessions::combine(broker, tally)
    }

    /// The cached connection, opened on demand. Hold the guard for the whole operation.
    async fn connection(&self) -> Result<Guard<'_, ServiceBusConnection>> {
        self.amqp.get(&self.ns).await
    }

    /// The cached connection as a [`PerBatch`] source.
    fn per_batch(&self) -> PerBatch<'_, Namespace> {
        PerBatch {
            life: &self.amqp,
            connector: &self.ns,
        }
    }

    /// The message plane over the cached connection (see [`ExplorerPlane`]).
    #[allow(dead_code)] // callers arrive with the operations (Task 12)
    pub(crate) fn plane(&self) -> ExplorerPlane<'_> {
        ExplorerPlane(self)
    }

    /// Close the AMQP connection, if open, for good: reads started before, including long
    /// per-batch scans, stop at their next batch with [`Error::Cancelled`], and later reads fail
    /// the same way without connecting. Management (HTTP) calls are unaffected.
    pub async fn close(&self) {
        self.amqp.close(&self.ns).await;
    }
}

/// The explorer's message plane. Each call locks the cached connection for that call only; links
/// it opens live on after the lock is released. Opening a link may retry once on a
/// connection-level error ([`retry_open_only`]); schedule and cancel run once, and a broken
/// connection is dropped and the error returned. Peeks lock per batch ([`PerBatch`]).
pub(crate) struct ExplorerPlane<'a>(&'a Explorer);

impl ExplorerPlane<'_> {
    async fn open_sender_once(&self, address: &str) -> Result<AmqpSender> {
        let mut guard = self.0.connection().await?;
        let result = guard.as_mut().expect("opened").open_sender(address).await;
        drop_if_broken(&self.0.ns, &mut guard, result).await
    }

    async fn open_receiver_once(
        &self,
        spec: &ReceiverSpec,
        wait: Duration,
    ) -> Result<Option<AmqpReceiver>> {
        let mut guard = self.0.connection().await?;
        let conn = guard.as_mut().expect("opened");
        let result = match spec.session {
            SessionChoice::NextAvailable => conn.try_accept_next(spec, wait).await,
            _ => conn.open_receiver(spec).await.map(Some),
        };
        drop_if_broken(&self.0.ns, &mut guard, result).await
    }

    /// The management description's `requires_session` and counts of `entity`; `None` when
    /// the identity may not read it.
    async fn description(
        &self,
        entity: &EntityPath,
    ) -> Result<Option<(bool, Option<EntityCounts>)>> {
        let found = match &entity.subscription {
            None => self
                .0
                .mgmt
                .get_queue(&entity.entity)
                .await
                .map(|q| (q.requires_session, q.counts)),
            Some(sub) => self
                .0
                .mgmt
                .list_subscriptions(&entity.entity)
                .await
                .and_then(|subs| {
                    subs.into_iter()
                        .find(|s| s.name.eq_ignore_ascii_case(sub))
                        .map(|s| (s.requires_session, s.counts))
                        .ok_or_else(|| Error::NotFound(format!("subscription `{}`", entity.base())))
                }),
        };
        match found {
            Ok(d) => Ok(Some(d)),
            Err(Error::Forbidden { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl PeekSource for ExplorerPlane<'_> {
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        self.0.per_batch().peek_batch(address, from, max).await
    }

    async fn peek_session_batch(
        &mut self,
        address: &str,
        session_id: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        self.0
            .per_batch()
            .peek_session_batch(address, session_id, from, max)
            .await
    }
}

#[async_trait]
impl MessagingBackend for ExplorerPlane<'_> {
    async fn open_sender(&mut self, address: &str) -> Result<Box<dyn MessageSender>> {
        let sender = plane_call(PlaneOp::OpenSender, |_| self.open_sender_once(address)).await?;
        Ok(Box::new(sender))
    }

    async fn schedule(
        &mut self,
        address: &str,
        batch: &[OutgoingMessage],
        at: OffsetDateTime,
        each: Each<'_, i64>,
    ) -> Result<()> {
        debug_assert!(!retry_open_only(PlaneOp::Schedule));
        let mut guard = self.0.connection().await?;
        let result = guard
            .as_mut()
            .expect("opened")
            .schedule(address, batch, at, each)
            .await;
        drop_if_broken(&self.0.ns, &mut guard, result).await
    }

    async fn cancel_scheduled(
        &mut self,
        address: &str,
        seqs: &[i64],
        each: Each<'_, ()>,
    ) -> Result<()> {
        debug_assert!(!retry_open_only(PlaneOp::CancelScheduled));
        let mut guard = self.0.connection().await?;
        let result = guard
            .as_mut()
            .expect("opened")
            .cancel_scheduled(address, seqs, each)
            .await;
        drop_if_broken(&self.0.ns, &mut guard, result).await
    }

    async fn open_receiver(
        &mut self,
        spec: &ReceiverSpec,
        wait: Duration,
    ) -> Result<Option<Box<dyn LockReceiver>>> {
        let receiver = plane_call(PlaneOp::OpenReceiver, |_| {
            self.open_receiver_once(spec, wait)
        })
        .await?;
        Ok(receiver.map(|r| Box::new(r) as Box<dyn LockReceiver>))
    }

    async fn entity_counts(&mut self, entity: &EntityPath) -> Result<Option<EntityCounts>> {
        Ok(self
            .description(entity)
            .await?
            .and_then(|(_, counts)| counts))
    }

    async fn requires_session(&mut self, entity: &EntityPath) -> Result<Option<bool>> {
        Ok(self
            .description(entity)
            .await?
            .map(|(sessions, _)| sessions))
    }

    async fn session_ids(&mut self, address: &str) -> Result<Vec<String>> {
        let broker = with_one_retry(|_| async {
            let mut guard = self.0.connection().await?;
            let result = sessions::list_session_ids(guard.as_mut().expect("opened"), address).await;
            drop_if_broken(&self.0.ns, &mut guard, result).await
        })
        .await;
        let tally = with_one_retry(|_| async {
            sessions::tally_sessions(&mut self.0.per_batch(), address, Explorer::COUNT_CAP).await
        })
        .await;
        let listing = sessions::combine(broker, tally)?;
        Ok(listing.sessions.into_iter().map(|s| s.id).collect())
    }
}

/// Pass `result` through, dropping the cached connection after a connection-level failure
/// (see [`retry_after`]) so the next attempt starts clean.
async fn drop_if_broken<K: Connector, T>(
    connector: &K,
    guard: &mut Guard<'_, K::Conn>,
    result: Result<T>,
) -> Result<T> {
    if let Some(conn) = take_if_broken(guard, &result) {
        connector.close(conn).await;
    }
    result
}

/// A cached connection as a [`PeekSource`] that holds the connection lock for one batch only,
/// so other operations interleave between the batches of a long scan. A connection-level
/// failure drops the cached connection, as `drop_if_broken` does.
struct PerBatch<'a, K: Connector> {
    life: &'a Lifecycle<K::Conn>,
    connector: &'a K,
}

#[async_trait]
impl<K: Connector> PeekSource for PerBatch<'_, K> {
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        let mut guard = self.life.get(self.connector).await?;
        let result = guard
            .as_mut()
            .expect("opened")
            .peek_batch(address, from, max)
            .await;
        drop_if_broken(self.connector, &mut guard, result).await
    }

    async fn peek_session_batch(
        &mut self,
        address: &str,
        session_id: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        let mut guard = self.life.get(self.connector).await?;
        let result = guard
            .as_mut()
            .expect("opened")
            .peek_session_batch(address, session_id, from, max)
            .await;
        drop_if_broken(self.connector, &mut guard, result).await
    }
}

/// Take the connection out of `slot` when `result` is a failure that means it is broken
/// (see [`retry_after`]); answers from a healthy service, such as `Unsupported`, leave it.
fn take_if_broken<C, T>(slot: &mut Option<C>, result: &Result<T>) -> Option<C> {
    if result.as_ref().is_err_and(|e| retry_after(e, 0)) {
        slot.take()
    } else {
        None
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

/// A message-plane call, for the retry policy.
#[allow(dead_code)] // callers arrive with the operations (Task 12)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaneOp {
    /// Attach a sender link.
    OpenSender,
    /// Attach a receiver link.
    OpenReceiver,
    /// `schedule-message`.
    Schedule,
    /// `cancel-scheduled-message`.
    CancelScheduled,
}

/// Whether `op` may run once more on a fresh connection.
/// Only opening a link (no broker side effect) is retried; a mutation that failed on a broken
/// connection is reported, never repeated.
#[allow(dead_code)] // callers arrive with the operations (Task 12)
pub(crate) fn retry_open_only(op: PlaneOp) -> bool {
    match op {
        PlaneOp::OpenSender | PlaneOp::OpenReceiver => true,
        PlaneOp::Schedule | PlaneOp::CancelScheduled => false,
    }
}

/// Run `f` once, or with one retry when [`retry_open_only`] allows it for `op`.
async fn plane_call<T, F, Fut>(op: PlaneOp, mut f: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    if retry_open_only(op) {
        with_one_retry(f).await
    } else {
        f(0).await
    }
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

/// The entity to count by peeking for `kind` at `path`. The known kind decides, so a queue named
/// like a subscription path stays a queue. Topics have nothing to peek.
fn peek_target(kind: EntityKind, path: &str) -> Result<EntityPath> {
    match kind {
        EntityKind::Queue => Ok(EntityPath::queue(path)),
        EntityKind::Subscription => {
            let e = EntityPath::parse(path)?;
            if e.subscription.is_some() {
                Ok(e)
            } else {
                Err(Error::Invalid(format!("`{path}` is not a subscription")))
            }
        }
        EntityKind::Topic => Err(Error::Invalid("a topic has no messages to count".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_target_follows_the_known_kind() {
        let q = peek_target(EntityKind::Queue, "orders").unwrap();
        assert_eq!(q, EntityPath::queue("orders"));
        // A queue is a queue even when its name looks like a subscription path.
        let odd = peek_target(EntityKind::Queue, "x/subscriptions/y").unwrap();
        assert_eq!(odd, EntityPath::queue("x/subscriptions/y"));
        let s = peek_target(EntityKind::Subscription, "events/Subscriptions/audit").unwrap();
        assert_eq!(s, EntityPath::subscription("events", "audit"));
        assert!(peek_target(EntityKind::Subscription, "events").is_err());
        assert!(peek_target(EntityKind::Topic, "events").is_err());
    }

    fn assert_send<T: Send>(_: &T) {}

    /// Compile-time check: the browse futures can run on a multi-threaded runtime (Tauri
    /// commands require it). Never called; nothing touches the network.
    #[allow(dead_code)]
    fn browse_futures_are_send(e: &Explorer, p: &EntityPath, o: &PeekOptions) {
        assert_send(&e.peek_tail(p, PeekView::Active, o));
        assert_send(&e.peek_message(p, PeekView::Active, 1, Some("s")));
        assert_send(&e.collect_sequences(p, PeekView::Active, &[1], 10, None, Some("s")));
        assert_send(&e.peek(p, PeekView::Active, o));
        assert_send(&e.list_sessions(p));
    }

    #[test]
    fn only_connection_level_failures_drop_the_cached_connection() {
        let refused: Result<()> = Err(Error::Unsupported {
            capability: "peek within a session".into(),
        });
        let mut slot = Some(1u8);
        assert!(take_if_broken(&mut slot, &refused).is_none());
        assert_eq!(
            slot,
            Some(1),
            "an Unsupported session peek leaves the connection"
        );
        assert!(take_if_broken(&mut slot, &Ok(())).is_none());
        let broken: Result<()> = Err(Error::Network("reset".into()));
        assert_eq!(take_if_broken(&mut slot, &broken), Some(1));
        assert_eq!(slot, None);
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

    /// A connection that serves `1..=n` and, on its first batch, announces itself and waits for
    /// `gate`, so a test can close the lifecycle while a read is between batches.
    struct Gated {
        n: i64,
        calls: Arc<AtomicUsize>,
        started: Arc<tokio::sync::Notify>,
        gate: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl PeekSource for Gated {
        async fn peek_batch(
            &mut self,
            _address: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<crate::messaging::PeekedMessage>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.started.notify_one();
                self.gate.notified().await;
            }
            Ok((from.max(1)..=self.n)
                .take(max as usize)
                .map(crate::messaging::test_message)
                .collect())
        }
    }

    /// Opens [`Gated`] connections and counts the opens.
    struct CountingConnector {
        opens: AtomicUsize,
        calls: Arc<AtomicUsize>,
        started: Arc<tokio::sync::Notify>,
        gate: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl Connector for CountingConnector {
        type Conn = Gated;
        async fn open(&self) -> Result<Gated> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            Ok(Gated {
                n: 50_000,
                calls: self.calls.clone(),
                started: self.started.clone(),
                gate: self.gate.clone(),
            })
        }
        async fn close(&self, _conn: Gated) {}
    }

    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn close_stops_a_running_per_batch_read_and_opens_nothing_new() {
        let k = CountingConnector {
            opens: AtomicUsize::new(0),
            calls: Arc::default(),
            started: Arc::default(),
            gate: Arc::default(),
        };
        let life = Lifecycle::<Gated>::default();
        let opts = PeekOptions::new(10).scan_limit(100_000);
        let read = async {
            peek_tail(
                &mut PerBatch {
                    life: &life,
                    connector: &k,
                },
                "q",
                PeekView::Active,
                &opts,
            )
            .await
        };
        let close = async {
            k.started.notified().await;
            // Close while the first batch holds the lock; then let that batch finish.
            tokio::join!(life.close(&k), async { k.gate.notify_one() });
        };
        let (out, ()) = tokio::join!(read, close);
        assert!(
            matches!(out, Err(Error::Cancelled)),
            "the read stops at its next batch: {out:?}"
        );
        assert_eq!(
            k.opens.load(Ordering::SeqCst),
            1,
            "no connection is reopened"
        );
        assert_eq!(k.calls.load(Ordering::SeqCst), 1, "no batch after close");
    }

    #[tokio::test]
    async fn a_closed_explorer_refuses_reads_without_connecting() {
        // `.invalid` never resolves, so nothing could connect even if the check were missing.
        let cs = crate::connstr::ConnectionString::parse(
            "Endpoint=sb://closed.invalid/;SharedAccessKeyName=k;SharedAccessKey=a2V5",
        )
        .unwrap();
        let ns = Namespace::from_connection_string(&cs).unwrap();
        let e = Explorer::new(ns).unwrap();
        e.close().await;
        let q = EntityPath::queue("orders");
        let opts = PeekOptions::new(10);
        assert!(matches!(
            e.peek_tail(&q, PeekView::Active, &opts).await,
            Err(Error::Cancelled)
        ));
        assert!(matches!(
            e.peek(&q, PeekView::Active, &opts).await,
            Err(Error::Cancelled)
        ));
        assert!(matches!(
            e.collect_sequences(&q, PeekView::Active, &[1], 10, None, None)
                .await,
            Err(Error::Cancelled)
        ));
        assert!(matches!(e.list_sessions(&q).await, Err(Error::Cancelled)));
    }

    /// Compile-time check: the plane is a `MessagingBackend` the jobs can take as a trait
    /// object. Never called.
    #[allow(dead_code)]
    fn the_plane_is_an_object_safe_backend(e: &Explorer) {
        let mut plane = e.plane();
        let _: &mut dyn MessagingBackend = &mut plane;
    }

    #[test]
    fn only_opening_a_link_may_retry() {
        assert!(retry_open_only(PlaneOp::OpenSender) && retry_open_only(PlaneOp::OpenReceiver));
        assert!(!retry_open_only(PlaneOp::Schedule) && !retry_open_only(PlaneOp::CancelScheduled));
    }

    #[test]
    fn a_failed_mutation_drops_a_broken_connection() {
        let mut slot = Some("conn");
        assert_eq!(
            take_if_broken(&mut slot, &Err::<(), _>(Error::Network("reset".into()))),
            Some("conn")
        );
        assert!(
            slot.is_none(),
            "the next call opens a fresh connection; this one is not repeated"
        );
        let mut slot = Some("conn");
        assert_eq!(
            take_if_broken(&mut slot, &Err::<(), _>(Error::LockLost("x".into()))),
            None,
            "a healthy service answered"
        );
        assert!(slot.is_some());
    }

    #[tokio::test]
    async fn counting_is_capped_per_call() {
        let (slots, attempted) = run_fill(5, 2, || Error::NotFound("x".into())).await;
        assert_eq!(attempted, ["e1", "e2"]);
        assert!(slots[2..].iter().all(Option::is_none));
    }
}
