//! The dead-letter summary: dead-lettered messages grouped by reason and description.

use super::scan::{PeekSource, walk_messages};
use crate::Result;
use serde::Serialize;
use std::collections::HashMap;
use time::OffsetDateTime;

/// The most messages [`crate::Explorer::dlq_summary`] examines.
pub const DLQ_SUMMARY_CAP: u32 = 10_000;

/// Dead-lettered messages that share a reason and a description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasonGroup {
    /// `DeadLetterReason`, when set.
    pub reason: Option<String>,
    /// `DeadLetterErrorDescription`, when set.
    pub description: Option<String>,
    /// Messages in the group.
    pub count: u64,
    /// The lowest sequence number in the group.
    pub first_sequence: i64,
    /// The highest sequence number in the group.
    pub last_sequence: i64,
    /// The earliest enqueued time in the group.
    #[serde(with = "time::serde::rfc3339::option")]
    pub oldest: Option<OffsetDateTime>,
    /// The latest enqueued time in the group.
    #[serde(with = "time::serde::rfc3339::option")]
    pub newest: Option<OffsetDateTime>,
}

/// Dead-lettered messages by reason: the largest group first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DlqSummary {
    /// By count (largest first), then reason, then description.
    pub groups: Vec<ReasonGroup>,
    /// Messages examined.
    pub scanned: u64,
    /// More messages exist beyond the cap: the counts cover only the first `scanned`.
    pub truncated: bool,
}

/// Group the messages of `address` (a dead-letter queue) by reason and description, examining
/// at most `cap`. Read-only: it peeks.
pub(crate) async fn summarize<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    cap: u32,
) -> Result<DlqSummary> {
    type Key = (Option<String>, Option<String>);
    let mut groups: HashMap<Key, ReasonGroup> = HashMap::new();
    let walked = walk_messages(src, address, cap, |m| {
        let key = (
            m.dead_letter_reason.clone(),
            m.dead_letter_description.clone(),
        );
        let seq = m.sequence_number;
        let at = m.enqueued_time;
        let g = groups
            .entry(key)
            .or_insert_with_key(|(reason, description)| ReasonGroup {
                reason: reason.clone(),
                description: description.clone(),
                count: 0,
                first_sequence: seq,
                last_sequence: seq,
                oldest: at,
                newest: at,
            });
        g.count += 1;
        g.first_sequence = g.first_sequence.min(seq);
        g.last_sequence = g.last_sequence.max(seq);
        if let Some(t) = at {
            g.oldest = Some(g.oldest.map_or(t, |o| o.min(t)));
            g.newest = Some(g.newest.map_or(t, |n| n.max(t)));
        }
    })
    .await?;
    let mut groups: Vec<ReasonGroup> = groups.into_values().collect();
    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.reason.cmp(&b.reason))
            .then_with(|| a.description.cmp(&b.description))
    });
    Ok(DlqSummary {
        groups,
        scanned: walked.examined,
        truncated: walked.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::PropValue;
    use crate::messaging::fake::{FakeBroker, FakeMsg};

    fn dead_lettered(reason: &str, description: &str) -> FakeMsg {
        FakeMsg::active("m")
            .prop("DeadLetterReason", PropValue::String(reason.into()))
            .prop(
                "DeadLetterErrorDescription",
                PropValue::String(description.into()),
            )
    }

    #[tokio::test]
    async fn dlq_summary_groups_reasons_and_descriptions() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        for (r, d) in [
            ("MaxDeliveryCountExceeded", "x"),
            ("MaxDeliveryCountExceeded", "x"),
            ("Manual", "bad id"),
            ("MaxDeliveryCountExceeded", "y"),
        ] {
            b.add(dl, dead_lettered(r, d));
        }
        let s = summarize(&mut b.clone(), dl, 10_000).await.unwrap();
        assert_eq!(
            s.groups
                .iter()
                .map(|g| (
                    g.reason.as_deref().unwrap(),
                    g.description.as_deref().unwrap(),
                    g.count
                ))
                .collect::<Vec<_>>(),
            [
                ("MaxDeliveryCountExceeded", "x", 2),
                ("Manual", "bad id", 1),
                ("MaxDeliveryCountExceeded", "y", 1)
            ]
        );
        assert_eq!((s.scanned, s.truncated), (4, false));
    }

    #[tokio::test]
    async fn groups_carry_their_sequence_range_and_times_and_a_cap_truncates() {
        let b = FakeBroker::default();
        let dl = "q/$DeadLetterQueue";
        let first = b.add(dl, dead_lettered("A", "a"));
        let none = b.add(dl, FakeMsg::active("plain"));
        let last = b.add(dl, dead_lettered("A", "a"));
        b.add(dl, dead_lettered("B", "b"));
        let s = summarize(&mut b.clone(), dl, 3).await.unwrap();
        assert_eq!((s.scanned, s.truncated), (3, true));
        assert_eq!(s.groups.len(), 2, "{:?}", s.groups);
        let a = &s.groups[0];
        assert_eq!(
            (a.count, a.first_sequence, a.last_sequence),
            (2, first, last)
        );
        assert!(a.oldest.is_some() && a.oldest <= a.newest);
        let plain = &s.groups[1];
        assert_eq!((plain.reason.as_deref(), plain.count), (None, 1));
        assert_eq!((plain.first_sequence, plain.last_sequence), (none, none));
        let json = serde_json::to_value(&s).unwrap();
        assert!(json["groups"][0]["oldest"].is_string());
        assert_eq!(json["groups"][0]["first_sequence"], first);
    }

    #[tokio::test]
    async fn an_empty_queue_has_no_groups() {
        let b = FakeBroker::default();
        let s = summarize(&mut b.clone(), "q/$DeadLetterQueue", 10_000)
            .await
            .unwrap();
        assert_eq!(s.groups, []);
        assert_eq!((s.scanned, s.truncated), (0, false));
    }
}
