//! Peek through an entity, keeping only messages that match a view and filter.

use super::connection::ServiceBusConnection;
use super::message::{MessageState, PeekedMessage};
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
///
/// Stops at `opts.max` matches, `opts.scan_limit` examined messages, an empty
/// batch (the end of the entity), a batch that makes no progress, or the first
/// message without a sequence number (`< 0`).
///
/// Messages whose sequence number is below the current position (duplicates or
/// stale replays) are skipped: they are neither returned nor counted against
/// `scan_limit`.
///
/// `next_sequence` is where to continue. It is `None` when the end of the
/// entity was reached, when the scan made no progress at all (nothing examined,
/// so retrying from the same place would loop), and when `opts.max == 0`
/// (which returns immediately). If a message without a sequence number is met
/// after real progress, the continuation is the last good sequence plus one; the
/// next call then examines nothing and ends with `None`.
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
            if m.sequence_number < 0 {
                // No sequence number: we cannot position past it.
                break 'outer;
            }
            if m.sequence_number < next {
                // Already seen or stale.
                continue;
            }
            let wanted = view.accepts(m.state) && opts.filter.matches(&m);
            if wanted && out.len() as u32 >= opts.max {
                // Leave this match for the next page.
                next = m.sequence_number;
                break 'outer;
            }
            scanned += 1;
            next = m.sequence_number + 1;
            advanced = true;
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
    let next_sequence = if exhausted || scanned == 0 {
        None
    } else {
        Some(next)
    };
    Ok(PeekPage {
        messages: out,
        scanned,
        next_sequence,
        exhausted,
    })
}

/// Message tallies by state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateCounts {
    /// Active.
    pub active: u64,
    /// Deferred.
    pub deferred: u64,
    /// Scheduled.
    pub scheduled: u64,
    /// All examined.
    pub total: u64,
    /// More messages exist beyond the cap, so the tallies are lower bounds.
    pub truncated: bool,
}

/// Count messages in `address` by peeking, examining at most `cap`.
///
/// `truncated` is set only when a message exists beyond the cap (an entity of
/// exactly `cap` messages is not truncated). Like [`scan_messages`], a message
/// without a sequence number ends the count and stale messages (below the
/// current position) are skipped and not counted; a batch that makes no
/// progress ends the count, so it always terminates.
pub async fn count_messages(
    src: &mut (impl PeekSource + ?Sized),
    address: &str,
    cap: u32,
) -> Result<StateCounts> {
    let mut counts = StateCounts::default();
    let mut next = 0i64;
    'outer: loop {
        // When the cap is reached, ask for one more message to see whether
        // anything lies beyond it.
        let want = (u64::from(cap).saturating_sub(counts.total))
            .min(u64::from(MAX_PER_REQUEST))
            .max(1) as u32;
        let batch = src.peek_batch(address, next, want).await?;
        if batch.is_empty() {
            break;
        }
        let mut advanced = false;
        for m in &batch {
            if m.sequence_number < 0 {
                break 'outer;
            }
            if m.sequence_number < next {
                continue;
            }
            if counts.total >= u64::from(cap) {
                counts.truncated = true;
                break 'outer;
            }
            counts.total += 1;
            match m.state {
                MessageState::Active => counts.active += 1,
                MessageState::Deferred => counts.deferred += 1,
                MessageState::Scheduled => counts.scheduled += 1,
            }
            next = m.sequence_number + 1;
            advanced = true;
        }
        if !advanced {
            break;
        }
    }
    Ok(counts)
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

    /// A misbehaving source that always returns messages without sequence numbers.
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
        assert!(
            page.messages.is_empty(),
            "unsequenced messages are not emitted"
        );
        assert_eq!(page.scanned, 0);
        assert_eq!(
            page.next_sequence, None,
            "no progress means no continuation"
        );
        assert!(!page.exhausted);
    }

    /// Returns scripted batches in order, then empty batches.
    struct Scripted {
        batches: std::collections::VecDeque<Vec<PeekedMessage>>,
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Scripted {
        async fn peek_batch(
            &mut self,
            _address: &str,
            _from: i64,
            _max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok(self.batches.pop_front().unwrap_or_default())
        }
    }

    fn seqs(list: &[i64]) -> Vec<PeekedMessage> {
        list.iter().map(|&i| test_message(i)).collect()
    }

    fn seq_numbers(page: &PeekPage) -> Vec<i64> {
        page.messages.iter().map(|m| m.sequence_number).collect()
    }

    #[tokio::test]
    async fn duplicate_and_stale_messages_are_skipped() {
        let mut src = Scripted {
            batches: [seqs(&[5, 5, 6])].into(),
            calls: 0,
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [5, 6]);
        assert_eq!(
            page.scanned, 2,
            "stale messages do not count toward the scan limit"
        );
        assert!(page.exhausted);
    }

    #[tokio::test]
    async fn stale_only_batch_makes_no_progress() {
        let mut src = Scripted {
            batches: [seqs(&[3, 4]), seqs(&[1, 2])].into(),
            calls: 0,
        };
        let opts = PeekOptions::new(10).from_sequence(10);
        let page = scan_messages(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(src.calls, 1);
        assert!(page.messages.is_empty());
        assert_eq!(page.next_sequence, None);
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn gapped_sequences_continue_correctly() {
        let mut src = Fake {
            messages: seqs(&[1, 2, 10, 11]),
            calls: 0,
        };
        let first = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(3))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&first), [1, 2, 10]);
        assert_eq!(first.next_sequence, Some(11));
        let opts = PeekOptions::new(3).from_sequence(11);
        let second = scan_messages(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&second), [11]);
        assert!(second.exhausted);
    }

    #[tokio::test]
    async fn max_mid_batch_points_at_the_first_unexamined_message() {
        // Even sequence numbers are scheduled, so Active matches 1, 3, 5, ...
        let mut src = entity(10, 2);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(2))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [1, 3]);
        assert_eq!(page.scanned, 4, "1, 2, 3 and the non-match 4 were examined");
        assert_eq!(page.next_sequence, Some(5));
        let opts = PeekOptions::new(2).from_sequence(5);
        let second = scan_messages(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&second), [5, 7]);
    }

    #[tokio::test]
    async fn deferred_view_keeps_only_deferred_messages() {
        let mut src = entity(9, 0);
        for m in src
            .messages
            .iter_mut()
            .filter(|m| m.sequence_number % 3 == 0)
        {
            m.state = MessageState::Deferred;
        }
        let page = scan_messages(&mut src, "q", PeekView::Deferred, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [3, 6, 9]);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(page.messages.len(), 6);
    }

    #[tokio::test]
    async fn unsequenced_message_after_progress_keeps_the_continuation() {
        let mut batch = seqs(&[1, 2]);
        batch.push(test_message(-1));
        let mut src = Scripted {
            batches: [batch].into(),
            calls: 0,
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [1, 2]);
        assert_eq!(page.next_sequence, Some(3));
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn zero_max_returns_immediately_without_a_continuation() {
        let mut src = entity(10, 0);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(0))
            .await
            .unwrap();
        assert_eq!(src.calls, 0);
        assert!(page.messages.is_empty());
        assert_eq!(page.next_sequence, None);
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

    #[tokio::test]
    async fn count_messages_tallies_states_and_respects_cap() {
        let mut src = entity(10, 5);
        src.messages[0].state = MessageState::Deferred;
        let c = count_messages(&mut src, "q", 1_000).await.unwrap();
        assert_eq!(
            (c.active, c.deferred, c.scheduled, c.total, c.truncated),
            (7, 1, 2, 10, false)
        );
        let capped = count_messages(&mut src, "q", 4).await.unwrap();
        assert_eq!(capped.total, 4);
        assert!(capped.truncated);
    }

    #[tokio::test]
    async fn count_at_exactly_the_cap_is_not_truncated() {
        let mut src = entity(10, 0);
        let c = count_messages(&mut src, "q", 10).await.unwrap();
        assert_eq!((c.total, c.truncated), (10, false));
        let c = count_messages(&mut src, "q", 9).await.unwrap();
        assert_eq!((c.total, c.truncated), (9, true));
        let mut empty = entity(0, 0);
        let c = count_messages(&mut empty, "q", 0).await.unwrap();
        assert_eq!((c.total, c.truncated), (0, false));
    }

    #[tokio::test]
    async fn count_pages_past_one_request() {
        let mut src = entity(600, 0);
        let c = count_messages(&mut src, "q", 10_000).await.unwrap();
        assert_eq!((c.total, c.truncated), (600, false));
        assert!(src.calls <= 4, "got {}", src.calls);
    }

    #[tokio::test]
    async fn count_terminates_on_a_stuck_source() {
        let mut src = Stuck { calls: 0 };
        let c = count_messages(&mut src, "q", 100).await.unwrap();
        assert_eq!(src.calls, 1);
        assert_eq!((c.total, c.truncated), (0, false));
        // Stale replays only: no progress, must not loop.
        let mut src = Scripted {
            batches: [seqs(&[1, 2]), seqs(&[1, 2]), seqs(&[1, 2])].into(),
            calls: 0,
        };
        let c = count_messages(&mut src, "q", 100).await.unwrap();
        assert_eq!(c.total, 2);
        assert_eq!(src.calls, 2, "second batch is all stale and ends the count");
    }

    #[tokio::test]
    async fn count_skips_duplicates_and_stops_at_unsequenced() {
        let mut batch = seqs(&[5, 5, 6]);
        batch.push(test_message(-1));
        batch.push(test_message(7));
        let mut src = Scripted {
            batches: [batch].into(),
            calls: 0,
        };
        let c = count_messages(&mut src, "q", 100).await.unwrap();
        assert_eq!(c.total, 2);
    }
}
