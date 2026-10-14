//! Peek through an entity, keeping only messages that match a view and filter.

use super::connection::ServiceBusConnection;
use super::message::PeekedMessage;
use super::peek::{self, MAX_PER_REQUEST};
use crate::Result;
use crate::filter::MessageFilter;
use crate::namespace::PeekView;
use async_trait::async_trait;
use serde::Serialize;

/// Messages examined per peek call unless the caller says otherwise.
pub const DEFAULT_SCAN_LIMIT: u32 = 10_000;

/// Anything that can return a batch of peeked messages.
#[async_trait]
pub trait PeekSource: Send {
    /// Peek up to `max` messages from `address` starting at sequence `from`.
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>>;
}

#[async_trait]
impl PeekSource for ServiceBusConnection {
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        peek::peek(self, address, from, max).await
    }
}

/// What to peek.
#[derive(Debug, Clone)]
pub struct PeekOptions {
    /// First sequence number to look at.
    pub from_sequence: i64,
    /// Maximum matching messages to return.
    pub max: u32,
    /// Maximum messages to examine (matching or not).
    pub scan_limit: u32,
    /// Property/body filter.
    pub filter: MessageFilter,
}

impl PeekOptions {
    /// Return up to `max` matches from the start, scanning at most [`DEFAULT_SCAN_LIMIT`].
    pub fn new(max: u32) -> Self {
        Self {
            from_sequence: 0,
            max,
            scan_limit: DEFAULT_SCAN_LIMIT,
            filter: MessageFilter::default(),
        }
    }
    /// Start at `seq`.
    pub fn from_sequence(mut self, seq: i64) -> Self {
        self.from_sequence = seq;
        self
    }
    /// Examine at most `limit` messages.
    pub fn scan_limit(mut self, limit: u32) -> Self {
        self.scan_limit = limit.max(1);
        self
    }
    /// Apply `filter`.
    pub fn filter(mut self, filter: MessageFilter) -> Self {
        self.filter = filter;
        self
    }
}

/// One page of results.
#[derive(Debug, Clone, Serialize)]
pub struct PeekPage {
    /// Matching messages, in sequence order.
    pub messages: Vec<PeekedMessage>,
    /// Messages examined.
    pub scanned: u32,
    /// Where to continue, or `None` when the end of the entity was reached.
    pub next_sequence: Option<i64>,
    /// The end of the entity was reached.
    pub exhausted: bool,
}

/// Peek through `address`, keeping messages accepted by `view` and `opts.filter`.
/// Stops at `opts.max` matches, `opts.scan_limit` examined messages, an empty
/// batch, or a batch whose sequence numbers do not advance.
pub async fn scan_messages(
    src: &mut (impl PeekSource + ?Sized),
    address: &str,
    view: PeekView,
    opts: &PeekOptions,
) -> Result<PeekPage> {
    let mut out: Vec<PeekedMessage> = Vec::new();
    let mut scanned: u32 = 0;
    let mut next = opts.from_sequence;
    let mut exhausted = false;
    'outer: while (out.len() as u32) < opts.max && scanned < opts.scan_limit {
        let want = (opts.scan_limit - scanned).min(MAX_PER_REQUEST);
        let batch = src.peek_batch(address, next, want).await?;
        if batch.is_empty() {
            exhausted = true;
            break;
        }
        let mut advanced = false;
        for m in batch {
            let wanted = view.accepts(m.state) && opts.filter.matches(&m);
            if wanted && out.len() as u32 >= opts.max {
                // Leave this match for the next page.
                next = m.sequence_number;
                break 'outer;
            }
            scanned += 1;
            if m.sequence_number >= next {
                next = m.sequence_number + 1;
                advanced = true;
            }
            if wanted {
                out.push(m);
            }
            if scanned >= opts.scan_limit {
                break 'outer;
            }
        }
        if !advanced {
            break;
        }
    }
    Ok(PeekPage {
        messages: out,
        scanned,
        next_sequence: if exhausted { None } else { Some(next) },
        exhausted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::PropertyFilter;
    use crate::messaging::{MessageState, test_message};

    struct Fake {
        messages: Vec<PeekedMessage>,
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Fake {
        async fn peek_batch(
            &mut self,
            _address: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok(self
                .messages
                .iter()
                .filter(|m| m.sequence_number >= from)
                .take(max as usize)
                .cloned()
                .collect())
        }
    }

    fn entity(n: i64, scheduled_every: i64) -> Fake {
        let messages = (1..=n)
            .map(|i| {
                let mut m = test_message(i);
                if scheduled_every > 0 && i % scheduled_every == 0 {
                    m.state = MessageState::Scheduled;
                }
                m
            })
            .collect();
        Fake { messages, calls: 0 }
    }

    #[tokio::test]
    async fn views_keep_only_their_state() {
        let mut src = entity(10, 5);
        let page = scan_messages(&mut src, "q", PeekView::Scheduled, &PeekOptions::new(50))
            .await
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [5, 10]
        );
        assert!(page.exhausted);
        assert_eq!(page.next_sequence, None);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(50))
            .await
            .unwrap();
        assert_eq!(page.messages.len(), 8);
    }

    #[tokio::test]
    async fn max_leaves_the_next_match_for_the_continuation() {
        let mut src = entity(10, 0);
        let first = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(3))
            .await
            .unwrap();
        assert_eq!(
            first
                .messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        let next = first.next_sequence.unwrap();
        let second = scan_messages(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(3).from_sequence(next),
        )
        .await
        .unwrap();
        assert_eq!(
            second
                .messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );
    }

    #[tokio::test]
    async fn scan_stops_at_limit_with_continuation() {
        let mut src = entity(100_000, 0);
        let opts = PeekOptions::new(10).scan_limit(1_000);
        let page = scan_messages(&mut src, "q", PeekView::Scheduled, &opts)
            .await
            .unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(page.scanned, 1_000);
        assert_eq!(page.next_sequence, Some(1_001));
        assert!(!page.exhausted);
        assert!(src.calls <= 4, "1000 / 250 batches, got {}", src.calls);
    }

    /// A misbehaving source that ignores `from` and always returns the same
    /// messages without sequence numbers.
    struct Stuck {
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Stuck {
        async fn peek_batch(
            &mut self,
            _address: &str,
            _from: i64,
            _max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok((0..5).map(|_| test_message(-1)).collect())
        }
    }

    #[tokio::test]
    async fn scan_stops_when_sequence_does_not_advance() {
        let mut src = Stuck { calls: 0 };
        let page = scan_messages(&mut src, "q", PeekView::DeadLetter, &PeekOptions::new(100))
            .await
            .unwrap();
        assert_eq!(src.calls, 1, "must not re-read the same range");
        assert_eq!(page.messages.len(), 5);
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn filters_apply_during_the_scan() {
        let mut src = entity(20, 0);
        for m in src.messages.iter_mut() {
            m.delivery_count = (m.sequence_number % 3) as u32;
        }
        let filter = MessageFilter {
            property: Some(PropertyFilter::parse("sys.DeliveryCount = 2").unwrap()),
            body: vec![],
        };
        let page = scan_messages(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(100).filter(filter),
        )
        .await
        .unwrap();
        assert!(page.messages.iter().all(|m| m.delivery_count == 2));
        assert_eq!(page.messages.len(), 7);
    }

    #[tokio::test]
    async fn empty_entity_is_exhausted_immediately() {
        let mut src = entity(0, 0);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert!(page.exhausted && page.messages.is_empty() && page.scanned == 0);
    }
}
