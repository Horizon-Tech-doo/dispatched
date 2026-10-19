//! Per-message job results, progress events and report files for bulk operations.

use crate::activity::{Outcome, redact};
use crate::export::{ExportFormat, ExportedMessage, FileExport};
use crate::messaging::PeekedMessage;
use crate::{Error, Result};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::path::Path;
use std::sync::Mutex;
use time::OffsetDateTime;

/// What a job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobKind {
    Send,
    Import,
    CancelScheduled,
    Receive,
    SettleDeferred,
    Delete,
    Move,
    Resubmit,
    Purge,
    /// Queues, topics, subscriptions or rules created.
    CreateEntities,
    /// Entity settings, status, rules or policies changed.
    UpdateEntities,
    /// Entities deleted.
    DeleteEntities,
    /// An entity duplicated.
    Duplicate,
    /// An entity renamed (copied, then the original deleted).
    Rename,
    /// Entities imported from an entity file.
    ImportEntities,
}

impl JobKind {
    /// Whether the items are entities rather than messages.
    pub fn on_entities(self) -> bool {
        matches!(
            self,
            Self::CreateEntities
                | Self::UpdateEntities
                | Self::DeleteEntities
                | Self::Duplicate
                | Self::Rename
                | Self::ImportEntities
        )
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Send | Self::Import => "Sent",
            Self::CancelScheduled => "Cancelled",
            Self::Receive => "Received",
            Self::SettleDeferred => "Settled",
            Self::Delete => "Deleted",
            Self::Move => "Moved",
            Self::Resubmit => "Resubmitted",
            Self::Purge => "Removed",
            Self::CreateEntities => "Created",
            Self::UpdateEntities => "Updated",
            Self::DeleteEntities => "Deleted",
            Self::Duplicate => "Duplicated",
            Self::Rename => "Renamed",
            Self::ImportEntities => "Imported",
        }
    }
}

/// Why an item was never attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotReached {
    Timeout,
    Budget,
    HeldLimit,
    /// Held locks could not be renewed in time, so the search stopped and released them.
    LockTime,
    Cancelled,
}

/// What happened to one message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ItemOutcome {
    Succeeded {
        detail: Option<String>,
    },
    Failed {
        /// The stable `error_kind` string.
        kind: &'static str,
        reason: String,
    },
    Skipped {
        reason: String,
        /// A skip the command promised, such as scheduled messages a move leaves alone: listed
        /// in the report, but the job is not incomplete because of it. Others (a session locked
        /// by another receiver, a denied entity) leave work undone.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        expected: bool,
    },
    Missing,
    NotReached {
        why: NotReached,
    },
}

/// The result for one message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemResult {
    pub sequence_number: Option<i64>,
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub entity: String,
    pub new_sequence_number: Option<i64>,
    pub outcome: ItemOutcome,
}

/// The finished record of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobReport {
    /// The activity op id (set by `ops::gated`); links the report file to its activity entries.
    pub op_id: Option<String>,
    pub kind: JobKind,
    pub entity: String,
    pub target: Option<String>,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub cancelled: bool,
    pub items: Vec<ItemResult>,
    pub notes: Vec<String>,
    /// Messages a cancelled job never attempted and has no item for (its total minus the items
    /// recorded); 0 unless cancelled.
    pub not_attempted: u64,
}

fn not_reached_text(why: NotReached) -> &'static str {
    match why {
        NotReached::Timeout => "not reached: time limit",
        NotReached::Budget => "not reached: message budget",
        NotReached::HeldLimit => "not reached: held-message limit",
        NotReached::LockTime => "not reached: locks could not be renewed in time",
        NotReached::Cancelled => "not reached: cancelled",
    }
}

impl JobReport {
    fn count(&self, f: impl Fn(&ItemOutcome) -> bool) -> u64 {
        self.items.iter().filter(|i| f(&i.outcome)).count() as u64
    }

    /// Items that worked.
    pub fn succeeded(&self) -> u64 {
        self.count(|o| matches!(o, ItemOutcome::Succeeded { .. }))
    }

    /// Items that failed or were never reached.
    pub fn failed(&self) -> u64 {
        self.count(|o| {
            matches!(
                o,
                ItemOutcome::Failed { .. } | ItemOutcome::NotReached { .. }
            )
        })
    }

    /// Items skipped or already gone.
    pub fn skipped(&self) -> u64 {
        self.count(|o| matches!(o, ItemOutcome::Skipped { .. } | ItemOutcome::Missing))
    }

    /// The overall outcome for the activity log.
    pub fn outcome(&self) -> Outcome {
        if self.cancelled {
            Outcome::Cancelled
        } else if self.failed() == 0 {
            Outcome::Succeeded
        } else if self.succeeded() == 0 {
            Outcome::Failed
        } else {
            Outcome::PartlyFailed
        }
    }

    /// Items that leave the job incomplete: failed, not reached, missing, or skipped other than
    /// as the command promised.
    pub fn incomplete(&self) -> u64 {
        self.count(|o| {
            !matches!(
                o,
                ItemOutcome::Succeeded { .. } | ItemOutcome::Skipped { expected: true, .. }
            )
        })
    }

    /// Every item that did not succeed (failed, not reached, skipped, missing), in order.
    pub fn not_succeeded(&self) -> impl Iterator<Item = &ItemResult> {
        self.items
            .iter()
            .filter(|i| !matches!(i.outcome, ItemOutcome::Succeeded { .. }))
    }

    /// Items missing (gone before the job reached them).
    pub fn missing(&self) -> u64 {
        self.count(|o| matches!(o, ItemOutcome::Missing))
    }

    /// The failed and not-reached items, in order.
    pub fn failures(&self) -> impl Iterator<Item = &ItemResult> {
        self.items.iter().filter(|i| {
            matches!(
                i.outcome,
                ItemOutcome::Failed { .. } | ItemOutcome::NotReached { .. }
            )
        })
    }

    /// Sequence numbers of the failed and not-reached items, for a retry.
    pub fn failed_sequences(&self) -> Vec<i64> {
        self.failures().filter_map(|i| i.sequence_number).collect()
    }

    /// Why an item did not succeed (empty for a success).
    pub fn reason(i: &ItemResult) -> String {
        match &i.outcome {
            ItemOutcome::Failed { reason, .. } | ItemOutcome::Skipped { reason, .. } => {
                reason.clone()
            }
            ItemOutcome::NotReached { why } => not_reached_text(*why).into(),
            ItemOutcome::Missing => "not found".into(),
            ItemOutcome::Succeeded { .. } => String::new(),
        }
    }

    /// A copy with secrets scrubbed from every free-text field (reasons, details, notes).
    /// All output (`summary_line`, JSON, report files) goes through this, so hand-built
    /// reports are covered too.
    pub fn scrubbed(&self) -> JobReport {
        let mut r = self.clone();
        for item in &mut r.items {
            match &mut item.outcome {
                ItemOutcome::Succeeded { detail } => {
                    if let Some(d) = detail {
                        *d = redact(d);
                    }
                }
                ItemOutcome::Failed { reason, .. } | ItemOutcome::Skipped { reason, .. } => {
                    *reason = redact(reason);
                }
                ItemOutcome::Missing | ItemOutcome::NotReached { .. } => {}
            }
        }
        for n in &mut r.notes {
            *n = redact(n);
        }
        r
    }

    /// One sentence describing the result.
    pub fn summary_line(&self) -> String {
        self.scrubbed().summary_line_inner()
    }

    fn summary_line_inner(&self) -> String {
        let verb = self.kind.verb();
        if self.kind.on_entities() {
            return self.entity_summary_line(verb);
        }
        let place = match (&self.target, self.kind) {
            (Some(t), _) => format!("from {} to {t}", self.entity),
            // Sends put messages on the entity.
            (None, JobKind::Send | JobKind::Import) => format!("to {}", self.entity),
            (None, _) => format!("from {}", self.entity),
        };
        let unreached = self.not_attempted.saturating_add(self.count(|o| {
            matches!(
                o,
                ItemOutcome::NotReached {
                    why: NotReached::Cancelled
                }
            )
        }));
        let cancelled = match (self.cancelled, unreached) {
            (false, _) => String::new(),
            (true, 0) => " (cancelled)".into(),
            (true, n) => format!(
                " (cancelled; {} not attempted)",
                crate::safety::count_text(n)
            ),
        };
        let failed = self.failed();
        // What else was not done: incomplete work must never read as complete.
        let skipped = self.count(|o| matches!(o, ItemOutcome::Skipped { .. }));
        let missing = self.missing();
        let others: Vec<String> = [(skipped, "skipped"), (missing, "missing")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{} {what}", crate::safety::count_text(n)))
            .collect();
        let others = if others.is_empty() {
            String::new()
        } else {
            format!("; {}", others.join(", "))
        };
        if failed == 0 {
            let n = self.succeeded();
            let noun = if n == 1 { "message" } else { "messages" };
            return format!("{verb} {n} {noun} {place}{cancelled}{others}");
        }
        let list: Vec<String> = self
            .failures()
            .take(5)
            .map(|i| match i.sequence_number {
                Some(s) => format!("sequence {s}: {}", Self::reason(i)),
                None => match &i.message_id {
                    Some(m) => format!("message {m}: {}", Self::reason(i)),
                    None => Self::reason(i),
                },
            })
            .collect();
        let mut list = list.join(", ");
        if failed > 5 {
            list.push_str(&format!(", and {} more", failed - 5));
        }
        format!(
            "{verb} {} of {} messages {place}{cancelled}; {failed} failed ({list}){others}",
            self.succeeded(),
            self.items.len()
        )
    }

    /// The summary of a job whose items are entities: `Created 1 of 2 entities; 1 failed
    /// (queue b: already exists)`.
    fn entity_summary_line(&self, verb: &str) -> String {
        let n = self.succeeded();
        let total = self.items.len() as u64;
        let noun = |n: u64| if n == 1 { "entity" } else { "entities" };
        let mut s = if n == total {
            format!("{verb} {n} {}", noun(n))
        } else {
            format!("{verb} {n} of {total} {}", noun(total))
        };
        if self.cancelled {
            s.push_str(" (cancelled)");
        }
        let skipped = self.count(|o| matches!(o, ItemOutcome::Skipped { .. }));
        if skipped > 0 {
            s.push_str(&format!("; {} skipped", crate::safety::count_text(skipped)));
        }
        let failed = self.failed();
        if failed > 0 {
            let list: Vec<String> = self
                .failures()
                .take(5)
                .map(|i| format!("{}: {}", i.entity, Self::reason(i)))
                .collect();
            let mut list = list.join(", ");
            if failed > 5 {
                list.push_str(&format!(", and {} more", failed - 5));
            }
            s.push_str(&format!("; {failed} failed ({list})"));
        }
        s
    }

    fn failed_json(&self) -> Vec<serde_json::Value> {
        self.failures()
            .map(|i| serde_json::json!({"seq": i.sequence_number, "reason": Self::reason(i)}))
            .collect()
    }

    fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "op_id": self.op_id,
            "kind": self.kind,
            "entity": self.entity,
            "target": self.target,
            "started_at": rfc3339(self.started_at),
            "finished_at": rfc3339(self.finished_at),
            "cancelled": self.cancelled,
            "not_attempted": self.not_attempted,
            "succeeded": self.succeeded(),
            "failed": self.failed_json(),
            "skipped": self.skipped(),
            "notes": self.notes,
        })
    }

    /// Write the report to `path`, atomically (never a partial file; no silent overwrite).
    /// JSON is one document with the summary and every item; NDJSON is one line per item
    /// followed by a summary line with `"type":"summary"`.
    pub fn write(&self, path: &Path, format: ExportFormat, overwrite: bool) -> Result<()> {
        let report = self.scrubbed();
        report.write_scrubbed(path, format, overwrite, None)
    }

    /// [`JobReport::write`] with the messages the job received, in full, in the export shape
    /// (`dispatched.message/v1`, as `peek --format json` prints them): JSON adds a `messages`
    /// array; NDJSON writes one `"type":"message"` line per message before the items. The
    /// report is scrubbed; message content is the user's data and is written as received.
    pub fn write_with_messages(
        &self,
        path: &Path,
        format: ExportFormat,
        overwrite: bool,
        messages: &[PeekedMessage],
    ) -> Result<()> {
        let report = self.scrubbed();
        report.write_scrubbed(path, format, overwrite, Some(messages))
    }

    fn write_scrubbed(
        &self,
        path: &Path,
        format: ExportFormat,
        overwrite: bool,
        messages: Option<&[PeekedMessage]>,
    ) -> Result<()> {
        let mut out = FileExport::create(path, format, overwrite)?;
        let exported = |m| ExportedMessage::new(m);
        let bytes = match format {
            ExportFormat::Json => {
                let mut doc = serde_json::to_value(self.serializable()).map_err(json_err)?;
                if let (Some(messages), Some(fields)) = (messages, doc.as_object_mut()) {
                    let list: Vec<_> = messages.iter().map(exported).collect();
                    fields.insert(
                        "messages".into(),
                        serde_json::to_value(list).map_err(json_err)?,
                    );
                }
                let mut v = serde_json::to_vec_pretty(&doc).map_err(json_err)?;
                v.push(b'\n');
                v
            }
            ExportFormat::Ndjson => {
                let mut v = Vec::new();
                for m in messages.unwrap_or_default() {
                    let mut line = serde_json::to_value(exported(m)).map_err(json_err)?;
                    if let Some(fields) = line.as_object_mut() {
                        fields.insert("type".into(), "message".into());
                    }
                    serde_json::to_writer(&mut v, &line).map_err(json_err)?;
                    v.push(b'\n');
                }
                for item in &self.items {
                    serde_json::to_writer(&mut v, item).map_err(json_err)?;
                    v.push(b'\n');
                }
                let mut summary = self.summary_json();
                summary["type"] = "summary".into();
                serde_json::to_writer(&mut v, &summary).map_err(json_err)?;
                v.push(b'\n');
                v
            }
        };
        out.write_raw(&bytes)?;
        out.finish()?;
        Ok(())
    }
}

fn json_err(e: serde_json::Error) -> Error {
    Error::Invalid(format!("could not write the report: {e}"))
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "invalid-time".into())
}

impl Serialize for JobReport {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.scrubbed().serializable().serialize(s)
    }
}

/// Wrapper that serializes an already scrubbed report.
struct Scrubbed<'a>(&'a JobReport);

impl JobReport {
    fn serializable(&self) -> Scrubbed<'_> {
        Scrubbed(self)
    }
}

impl Serialize for Scrubbed<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let this = self.0;
        let mut st = s.serialize_struct("JobReport", 13)?;
        st.serialize_field("op_id", &this.op_id)?;
        st.serialize_field("kind", &this.kind)?;
        st.serialize_field("entity", &this.entity)?;
        st.serialize_field("target", &this.target)?;
        st.serialize_field("started_at", &rfc3339(this.started_at))?;
        st.serialize_field("finished_at", &rfc3339(this.finished_at))?;
        st.serialize_field("cancelled", &this.cancelled)?;
        st.serialize_field("not_attempted", &this.not_attempted)?;
        st.serialize_field("succeeded", &this.succeeded())?;
        st.serialize_field("failed", &this.failed_json())?;
        st.serialize_field("skipped", &this.skipped())?;
        st.serialize_field("items", &this.items)?;
        st.serialize_field("notes", &this.notes)?;
        st.end()
    }
}

/// A progress event from a running job.
#[derive(Debug, Clone)]
pub enum JobEvent {
    Started {
        kind: JobKind,
        total: Option<u64>,
    },
    Item(ItemResult),
    /// A message a receive took, in full, as its item is recorded (for receive-and-delete it
    /// is already removed), so a surface can keep a copy even if the job never returns.
    Received(Box<PeekedMessage>),
    Progress {
        done: u64,
        total: Option<u64>,
    },
    Note(String),
}

/// Receives job events. Implementations must be cheap and non-blocking.
pub trait Progress: Send + Sync {
    fn event(&self, e: &JobEvent);
}

/// Ignores every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoProgress;

impl Progress for NoProgress {
    fn event(&self, _: &JobEvent) {}
}

/// Records every event (test helper).
#[derive(Debug, Default)]
pub struct RecordingProgress {
    events: Mutex<Vec<JobEvent>>,
}

impl RecordingProgress {
    /// Everything received so far.
    pub fn events(&self) -> Vec<JobEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Progress for RecordingProgress {
    fn event(&self, e: &JobEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(e.clone());
    }
}

/// Builds a [`JobReport`] while emitting progress events.
/// Copies a job sent without some custom properties (their type cannot be copied), for the
/// per-job note "k copies lost properties: …".
#[derive(Debug, Default)]
pub(crate) struct LostProperties {
    copies: u64,
    names: std::collections::BTreeSet<String>,
}

impl LostProperties {
    /// The most property names the note lists.
    const SHOWN: usize = 10;

    /// A copy was sent without `names` (nothing when it lost none).
    pub(crate) fn add(&mut self, names: &[String]) {
        if names.is_empty() {
            return;
        }
        self.copies += 1;
        self.names.extend(names.iter().cloned());
    }

    /// The note, when any copy lost properties.
    pub(crate) fn note(&self) -> Option<String> {
        if self.copies == 0 {
            return None;
        }
        let mut list: Vec<&str> = self
            .names
            .iter()
            .take(Self::SHOWN)
            .map(String::as_str)
            .collect();
        let more = self.names.len().saturating_sub(Self::SHOWN);
        let more = (more > 0).then(|| format!("and {more} more"));
        list.extend(more.as_deref());
        let copies = if self.copies == 1 {
            "1 copy".to_string()
        } else {
            format!("{} copies", crate::safety::count_text(self.copies))
        };
        Some(format!("{copies} lost properties: {}", list.join(", ")))
    }
}

pub(crate) struct JobRun<'a> {
    kind: JobKind,
    entity: String,
    target: Option<String>,
    progress: &'a dyn Progress,
    total: Option<u64>,
    started_at: OffsetDateTime,
    items: Vec<ItemResult>,
    notes: Vec<String>,
}

impl<'a> JobRun<'a> {
    pub(crate) fn new(
        kind: JobKind,
        entity: String,
        target: Option<String>,
        progress: &'a dyn Progress,
        total: Option<u64>,
    ) -> Self {
        progress.event(&JobEvent::Started { kind, total });
        Self {
            kind,
            entity,
            target,
            progress,
            total,
            started_at: OffsetDateTime::now_utc(),
            items: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Record one result and emit `Item` then `Progress`.
    pub(crate) fn item(&mut self, item: ItemResult) {
        self.progress.event(&JobEvent::Item(item.clone()));
        self.items.push(item);
        self.progress.event(&JobEvent::Progress {
            done: self.items.len() as u64,
            total: self.total,
        });
    }

    /// Record `item` unless an item for its sequence number was recorded already; that one is
    /// replaced, and emitted again only if it changed.
    pub(crate) fn settle_item(&mut self, item: ItemResult) {
        let seen = item.sequence_number.and_then(|seq| {
            self.items
                .iter()
                .position(|i| i.sequence_number == Some(seq))
        });
        match seen {
            Some(at) if self.items[at] == item => {}
            Some(at) => {
                self.progress.event(&JobEvent::Item(item.clone()));
                self.items[at] = item;
            }
            None => self.item(item),
        }
    }

    /// Order the items as `order` lists their sequence numbers; others keep their place at the end.
    pub(crate) fn reorder(&mut self, order: &[Option<i64>]) {
        let rank = |i: &ItemResult| {
            order
                .iter()
                .position(|s| *s == i.sequence_number)
                .unwrap_or(usize::MAX)
        };
        self.items.sort_by_key(rank);
    }

    /// Emit a message a receive took ([`JobEvent::Received`]).
    pub(crate) fn received(&mut self, m: &PeekedMessage) {
        self.progress
            .event(&JobEvent::Received(Box::new(m.clone())));
    }

    /// Record a note for the report and emit it.
    pub(crate) fn note(&mut self, note: String) {
        self.progress.event(&JobEvent::Note(note.clone()));
        self.notes.push(note);
    }

    /// Close the run.
    pub(crate) fn finish(self, cancelled: bool) -> JobReport {
        // Only where the total is a list of messages: a receive's or a purge's is a bound.
        let listed = !matches!(self.kind, JobKind::Receive | JobKind::Purge);
        let not_attempted = match (cancelled && listed, self.total) {
            (true, Some(total)) => total.saturating_sub(self.items.len() as u64),
            _ => 0,
        };
        JobReport {
            op_id: None,
            kind: self.kind,
            entity: self.entity,
            target: self.target,
            started_at: self.started_at,
            finished_at: OffsetDateTime::now_utc(),
            cancelled,
            items: self.items,
            notes: self.notes,
            not_attempted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(seq: i64, outcome: ItemOutcome) -> ItemResult {
        ItemResult {
            sequence_number: Some(seq),
            message_id: Some(format!("m{seq}")),
            session_id: None,
            entity: "orders/$DeadLetterQueue".into(),
            new_sequence_number: None,
            outcome,
        }
    }
    fn ok(seq: i64) -> ItemResult {
        item(seq, ItemOutcome::Succeeded { detail: None })
    }
    fn failed(seq: i64, kind: &'static str, reason: &str) -> ItemResult {
        item(
            seq,
            ItemOutcome::Failed {
                kind,
                reason: reason.into(),
            },
        )
    }
    fn skipped(seq: i64) -> ItemResult {
        item(
            seq,
            ItemOutcome::Skipped {
                reason: "already there".into(),
                expected: false,
            },
        )
    }
    fn missing(seq: i64) -> ItemResult {
        item(seq, ItemOutcome::Missing)
    }
    fn not_reached(seq: i64, why: NotReached) -> ItemResult {
        item(seq, ItemOutcome::NotReached { why })
    }
    fn report_kind(kind: JobKind, items: &[ItemResult]) -> JobReport {
        JobReport {
            op_id: None,
            kind,
            entity: "orders/$DeadLetterQueue".into(),
            target: Some("orders".into()),
            started_at: OffsetDateTime::now_utc(),
            finished_at: OffsetDateTime::now_utc(),
            cancelled: false,
            items: items.to_vec(),
            notes: vec![],
            not_attempted: 0,
        }
    }
    fn report(items: &[ItemResult]) -> JobReport {
        report_kind(JobKind::Move, items)
    }

    #[test]
    fn report_counts_and_outcome() {
        let mut r = report(&[
            ok(1),
            failed(2, "lock_lost", "the lock expired"),
            skipped(3),
            missing(4),
            not_reached(5, NotReached::Timeout),
        ]);
        assert_eq!((r.succeeded(), r.failed(), r.skipped()), (1, 2, 2));
        assert_eq!(r.outcome(), Outcome::PartlyFailed);
        assert_eq!(r.failed_sequences(), [2, 5]);
        r.cancelled = true;
        assert_eq!(r.outcome(), Outcome::Cancelled);
        assert_eq!(
            report(&[]).outcome(),
            Outcome::Succeeded,
            "nothing to do is success"
        );
        assert_eq!(
            report(&[failed(1, "timeout", "x")]).outcome(),
            Outcome::Failed
        );
    }

    #[test]
    fn json_has_the_spec_summary_shape() {
        let r = report(&[ok(1), failed(2, "lock_lost", "the lock expired")]);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["succeeded"], 1);
        assert_eq!(
            v["failed"],
            serde_json::json!([{"seq": 2, "reason": "the lock expired"}])
        );
        assert_eq!(v["skipped"], 0);
        assert_eq!(v["items"][1]["outcome"]["status"], "failed");
        assert_eq!(v["items"][1]["outcome"]["kind"], "lock_lost");
        assert_eq!(v["op_id"], serde_json::Value::Null);
        let mut r = r;
        r.op_id = Some("op-1".into());
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["op_id"], "op-1");
        assert!(
            time::OffsetDateTime::parse(
                v["started_at"].as_str().unwrap(),
                &time::format_description::well_known::Rfc3339
            )
            .is_ok()
        );
        assert_eq!(
            serde_json::to_value(NotReached::LockTime).unwrap(),
            "lock-time"
        );
    }

    #[test]
    fn summary_line_reads_like_a_sentence() {
        assert_eq!(
            report_kind(
                JobKind::Move,
                &[ok(1), ok(2), failed(3, "lock_lost", "the lock expired")]
            )
            .summary_line(),
            "Moved 2 of 3 messages from orders/$DeadLetterQueue to orders; 1 failed (sequence 3: the lock expired)"
        );
        let mut purge = report_kind(JobKind::Purge, &[ok(1)]);
        purge.target = None;
        assert_eq!(
            purge.summary_line(),
            "Removed 1 message from orders/$DeadLetterQueue"
        );
        assert_eq!(
            report_kind(JobKind::Move, &[ok(1), ok(2)]).summary_line(),
            "Moved 2 messages from orders/$DeadLetterQueue to orders",
            "all succeeded: no \"of N\""
        );
    }

    #[test]
    fn summary_line_says_what_was_skipped_or_missing() {
        assert_eq!(
            report(&[ok(1), skipped(2), missing(3), missing(4)]).summary_line(),
            "Moved 1 message from orders/$DeadLetterQueue to orders; 1 skipped, 2 missing"
        );
        assert_eq!(
            report(&[
                ok(1),
                failed(2, "lock_lost", "the lock expired"),
                missing(3)
            ])
            .summary_line(),
            "Moved 1 of 3 messages from orders/$DeadLetterQueue to orders; 1 failed (sequence 2: the lock expired); 1 missing"
        );
        assert_eq!(
            report(&[skipped(2)]).summary_line(),
            "Moved 0 messages from orders/$DeadLetterQueue to orders; 1 skipped"
        );
    }

    #[test]
    fn incomplete_counts_everything_but_successes_and_skips_asked_for() {
        let asked = item(
            9,
            ItemOutcome::Skipped {
                reason: "scheduled".into(),
                expected: true,
            },
        );
        let r = report(&[
            ok(1),
            failed(2, "timeout", "slow"),
            skipped(3),
            missing(4),
            not_reached(5, NotReached::Budget),
            asked.clone(),
        ]);
        assert_eq!(r.incomplete(), 4);
        assert_eq!(report(&[ok(1), asked]).incomplete(), 0);
        assert_eq!(
            r.not_succeeded()
                .map(|i| i.sequence_number)
                .collect::<Vec<_>>(),
            [Some(2), Some(3), Some(4), Some(5), Some(9)]
        );
        assert_eq!(JobReport::reason(&r.items[2]), "already there");
        assert_eq!(JobReport::reason(&r.items[3]), "not found");
    }

    #[test]
    fn sends_say_to_and_a_cancelled_job_says_what_it_never_attempted() {
        for kind in [JobKind::Send, JobKind::Import] {
            let mut r = report_kind(kind, &[ok(1), ok(2)]);
            r.entity = "orders".into();
            r.target = None;
            assert_eq!(
                r.summary_line(),
                format!("{} 2 messages to orders", kind.verb())
            );
        }
        let mut r = report_kind(JobKind::Send, &[ok(1), ok(2), ok(3)]);
        r.entity = "orders".into();
        r.target = None;
        r.cancelled = true;
        r.not_attempted = 97;
        assert_eq!(
            r.summary_line(),
            "Sent 3 messages to orders (cancelled; 97 not attempted)"
        );
        // Items recorded as not reached because of the cancel count too.
        let mut r = report(&[ok(1), not_reached(2, NotReached::Cancelled)]);
        r.cancelled = true;
        assert_eq!(
            r.summary_line(),
            "Moved 1 of 2 messages from orders/$DeadLetterQueue to orders (cancelled; 1 not attempted); 1 failed (sequence 2: not reached: cancelled)"
        );
        let mut r = report(&[ok(1)]);
        r.cancelled = true;
        assert!(
            r.summary_line().ends_with(" (cancelled)"),
            "{}",
            r.summary_line()
        );
    }

    #[tokio::test]
    async fn a_cancelled_run_counts_what_it_never_reached() {
        let mut run = JobRun::new(JobKind::Send, "orders".into(), None, &NoProgress, Some(5));
        run.item(ok(1));
        let r = run.finish(true);
        assert_eq!(r.not_attempted, 4);
        let run = JobRun::new(JobKind::Send, "orders".into(), None, &NoProgress, Some(5));
        assert_eq!(
            run.finish(false).not_attempted,
            0,
            "only a cancel leaves them"
        );
    }

    #[test]
    fn summary_line_caps_the_failure_list() {
        let items: Vec<_> = (1..=7).map(|n| failed(n, "timeout", "slow")).collect();
        let line = report(&items).summary_line();
        assert!(line.contains("7 failed ("), "{line}");
        assert!(line.contains("sequence 5: slow"), "{line}");
        assert!(!line.contains("sequence 6"), "{line}");
        assert!(line.ends_with(", and 2 more)"), "{line}");
    }

    #[test]
    fn report_writes_json_and_ndjson_atomically() {
        use crate::export::ExportFormat;
        let dir = tempfile::tempdir().unwrap();
        let r = report(&[ok(1), ok(2)]);
        r.write(&dir.path().join("r.json"), ExportFormat::Json, false)
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("r.json")).unwrap()).unwrap();
        assert_eq!(v["succeeded"], 2);
        assert!(
            r.write(&dir.path().join("r.json"), ExportFormat::Json, false)
                .is_err(),
            "no silent overwrite"
        );
        r.write(&dir.path().join("r.ndjson"), ExportFormat::Ndjson, false)
            .unwrap();
        // NDJSON: one line per item, then one summary line with "type": "summary".
        let text = std::fs::read_to_string(dir.path().join("r.ndjson")).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(
            text.lines()
                .last()
                .unwrap()
                .contains("\"type\":\"summary\"")
        );
    }

    #[test]
    fn a_report_can_carry_the_received_messages_in_full() {
        let long = "x".repeat(500);
        let mut m = crate::messaging::test_message(9);
        m.body = crate::messaging::MessageBody::Text(long.clone());
        let r = report(&[ok(9)]);
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("r.json");
        r.write_with_messages(&json, ExportFormat::Json, false, std::slice::from_ref(&m))
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(v["messages"][0]["schema"], crate::export::EXPORT_SCHEMA);
        assert_eq!(v["messages"][0]["sequence_number"], 9);
        assert!(v.to_string().contains(&long), "the full body");
        assert!(v["items"].is_array());
        let nd = dir.path().join("r.ndjson");
        r.write_with_messages(&nd, ExportFormat::Ndjson, false, std::slice::from_ref(&m))
            .unwrap();
        let text = std::fs::read_to_string(&nd).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3, "message, item, summary");
        assert_eq!(lines[0]["type"], "message");
        assert!(lines[0].to_string().contains(&long));
        assert_eq!(lines[2]["type"], "summary");
        // An empty receive still says so.
        let empty = dir.path().join("e.json");
        r.write_with_messages(&empty, ExportFormat::Json, false, &[])
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&empty).unwrap()).unwrap();
        assert_eq!(v["messages"], serde_json::json!([]));
    }

    #[test]
    fn secrets_are_scrubbed_from_summary_files_and_json() {
        use crate::export::ExportFormat;
        let secret = "Endpoint=sb://x/;SharedAccessKey=abc123secret and ?sig=deadbeef&se=1";
        let mut r = report(&[
            failed(1, "network", secret),
            item(
                2,
                ItemOutcome::Succeeded {
                    detail: Some(secret.into()),
                },
            ),
        ]);
        r.notes.push(secret.into());
        let dir = tempfile::tempdir().unwrap();
        for (name, fmt) in [
            ("r.json", ExportFormat::Json),
            ("r.ndjson", ExportFormat::Ndjson),
        ] {
            r.write(&dir.path().join(name), fmt, false).unwrap();
            let text = std::fs::read_to_string(dir.path().join(name)).unwrap();
            assert!(text.contains("[redacted]"), "{text}");
            assert!(
                !text.contains("abc123secret") && !text.contains("deadbeef"),
                "{text}"
            );
        }
        let line = r.summary_line();
        assert!(line.contains("[redacted]"), "{line}");
        assert!(
            !line.contains("abc123secret") && !line.contains("deadbeef"),
            "{line}"
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("abc123secret") && !json.contains("deadbeef"),
            "{json}"
        );
    }

    #[test]
    fn job_run_emits_item_and_progress_events() {
        let p = RecordingProgress::default();
        let mut run = JobRun::new(JobKind::Delete, "orders".into(), None, &p, Some(2));
        run.item(ok(1));
        let r = run.finish(false);
        assert_eq!(r.items.len(), 1);
        assert!(matches!(
            p.events()[0],
            JobEvent::Started { total: Some(2), .. }
        ));
        assert!(matches!(
            p.events().last().unwrap(),
            JobEvent::Progress {
                done: 1,
                total: Some(2)
            }
        ));
    }
}
