//! The shapes operate commands take from and give to the webview, and their mapping onto core
//! requests. Sequence numbers and 64-bit property values cross as strings: a partitioned
//! entity's sequence numbers and a `long` property can exceed what a JavaScript number holds.

use crate::error::CommandError;
use dispatched_core::jobs::{ItemOutcome, ItemResult, JobReport};
use dispatched_core::messaging::{
    MessageIdSource, MessageSummary, OutgoingBody, OutgoingMessage, PropType, Settlement,
    TypedValue, parse_duration,
};
use dispatched_core::namespace::{EntityPath, PeekView, SubQueue};
use dispatched_core::ops::{LocateLimits, MessageSelection, PurgePlan, ReceiveAction, SelectSpec};
use dispatched_core::safety::CountEstimate;
use dispatched_core::{DlqSummary, FilterSpec};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid", message)
}

/// A sequence number from the webview: a decimal string (or, from older callers, a number).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seq(pub i64);

impl<'de> Deserialize<'de> for Seq {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Number(i64),
        }
        match Raw::deserialize(d)? {
            Raw::Number(n) => Ok(Seq(n)),
            Raw::Text(t) => t
                .trim()
                .parse()
                .map(Seq)
                .map_err(|_| serde::de::Error::custom(format!("not a sequence number: {t}"))),
        }
    }
}

pub fn seqs(list: &[Seq]) -> Vec<i64> {
    list.iter().map(|s| s.0).collect()
}

/// One custom property: its name, type name (`string`, `int`, `long`, `double`, `bool`,
/// `datetime`, `timespan`, `guid`, and the other export tags) and value as typed.
#[derive(Debug, Clone, Deserialize)]
pub struct PropertyForm {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

impl PropertyForm {
    pub fn typed(&self) -> Result<(String, TypedValue), CommandError> {
        if self.name.trim().is_empty() {
            return Err(invalid("A property needs a name."));
        }
        let ty = PropType::parse(&self.kind)?;
        let value = TypedValue::parse(ty, &self.value)
            .map_err(|e| invalid(format!("{}: {e}", self.name)))?;
        Ok((self.name.clone(), value))
    }
}

fn properties(list: &[PropertyForm]) -> Result<BTreeMap<String, TypedValue>, CommandError> {
    let mut out = BTreeMap::new();
    for p in list {
        let (name, value) = p.typed()?;
        if out.insert(name.clone(), value).is_some() {
            return Err(invalid(format!("Property {name} is set twice.")));
        }
    }
    Ok(out)
}

fn blank_none(v: &Option<String>) -> Option<String> {
    v.as_ref().filter(|s| !s.is_empty()).cloned()
}

/// The message every copy starts from.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OutgoingForm {
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub correlation_id: Option<String>,
    pub subject: Option<String>,
    pub to: Option<String>,
    pub reply_to: Option<String>,
    pub reply_to_session_id: Option<String>,
    pub content_type: Option<String>,
    pub partition_key: Option<String>,
    /// `90s`, `5m`, `1h`, `7d`, or a .NET `TimeSpan`.
    pub time_to_live: Option<String>,
    pub properties: Vec<PropertyForm>,
}

impl OutgoingForm {
    pub fn message(&self) -> Result<OutgoingMessage, CommandError> {
        let time_to_live = match blank_none(&self.time_to_live) {
            Some(t) => Some(parse_duration(&t).map_err(|e| invalid(format!("Time to live: {e}")))?),
            None => None,
        };
        Ok(OutgoingMessage {
            body: OutgoingBody::default(),
            message_id: blank_none(&self.message_id),
            session_id: blank_none(&self.session_id),
            correlation_id: blank_none(&self.correlation_id),
            subject: blank_none(&self.subject),
            to: blank_none(&self.to),
            reply_to: blank_none(&self.reply_to),
            reply_to_session_id: blank_none(&self.reply_to_session_id),
            content_type: blank_none(&self.content_type),
            partition_key: blank_none(&self.partition_key),
            time_to_live,
            scheduled_enqueue_time: None,
            properties: properties(&self.properties)?,
        })
    }
}

/// Where the body comes from. Files and templates are named by the pick id the open dialog
/// returned, never by a path.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BodyForm {
    Text { text: String },
    Files { pick: String },
    Template { pick: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum MessageIdForm {
    Keep,
    Fixed { id: String },
    Random,
    Increment { base: String },
}

impl From<&MessageIdForm> for MessageIdSource {
    fn from(f: &MessageIdForm) -> Self {
        match f {
            MessageIdForm::Keep => MessageIdSource::Keep,
            MessageIdForm::Fixed { id } => MessageIdSource::Fixed(id.clone()),
            MessageIdForm::Random => MessageIdSource::Random,
            MessageIdForm::Increment { base } => MessageIdSource::Increment { base: base.clone() },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendForm {
    pub target: String,
    #[serde(default)]
    pub template: OutgoingForm,
    pub body: BodyForm,
    pub ids: MessageIdForm,
    pub count: u64,
    /// RFC 3339.
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub batch: bool,
}

pub fn parse_time(text: &str) -> Result<OffsetDateTime, CommandError> {
    OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
        .map_err(|_| invalid(format!("Not a date and time: {text}")))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SettlementForm {
    Complete,
    Abandon,
    Defer,
    DeadLetter {
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
}

impl From<&SettlementForm> for Settlement {
    fn from(f: &SettlementForm) -> Self {
        match f {
            SettlementForm::Complete => Settlement::Complete,
            SettlementForm::Abandon => Settlement::Abandon,
            SettlementForm::Defer => Settlement::Defer,
            SettlementForm::DeadLetter {
                reason,
                description,
            } => Settlement::DeadLetter {
                reason: blank_none(reason),
                description: blank_none(description),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ReceiveActionForm {
    Settle { how: SettlementForm },
    Delete,
}

impl From<&ReceiveActionForm> for ReceiveAction {
    fn from(f: &ReceiveActionForm) -> Self {
        match f {
            ReceiveActionForm::Settle { how } => ReceiveAction::Settle(how.into()),
            ReceiveActionForm::Delete => ReceiveAction::Delete,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiveForm {
    pub entity: String,
    pub sub_queue: SubQueue,
    #[serde(default)]
    pub session: Option<String>,
    pub max: u32,
    pub wait_seconds: u64,
    pub action: ReceiveActionForm,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SelectionForm {
    Sequences { sequences: Vec<Seq> },
    Matching { filter: FilterSpec, limit: u64 },
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelectForm {
    pub entity: String,
    pub view: PeekView,
    pub selection: SelectionForm,
}

impl SelectForm {
    pub fn spec(&self) -> Result<SelectSpec, CommandError> {
        let selection = match &self.selection {
            SelectionForm::Sequences { sequences } => MessageSelection::Sequences(seqs(sequences)),
            SelectionForm::Matching { filter, limit } => MessageSelection::Matching {
                filter: filter.build()?,
                limit: *limit,
            },
        };
        Ok(SelectSpec {
            entity: EntityPath::parse(&self.entity)?,
            view: self.view,
            selection,
            limits: LocateLimits::default(),
        })
    }
}

/// Changes a resubmit makes to each copy. A `None` leaves the field alone; for the ids an empty
/// string clears it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EditsForm {
    pub body: Option<String>,
    pub set: Vec<PropertyForm>,
    pub remove: Vec<String>,
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub correlation_id: Option<String>,
    pub subject: Option<String>,
    pub content_type: Option<String>,
    pub to: Option<String>,
    pub reply_to: Option<String>,
}

fn clearable(v: &Option<String>) -> Option<Option<String>> {
    v.as_ref().map(|s| (!s.is_empty()).then(|| s.clone()))
}

impl EditsForm {
    pub fn edits(&self) -> Result<dispatched_core::messaging::Edits, CommandError> {
        Ok(dispatched_core::messaging::Edits {
            body: self
                .body
                .as_ref()
                .map(|b| OutgoingBody::Bytes(b.clone().into_bytes())),
            set: properties(&self.set)?,
            remove: self.remove.clone(),
            message_id: blank_none(&self.message_id),
            session_id: clearable(&self.session_id),
            correlation_id: clearable(&self.correlation_id),
            subject: clearable(&self.subject),
            content_type: clearable(&self.content_type),
            to: clearable(&self.to),
            reply_to: clearable(&self.reply_to),
            ..Default::default()
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResubmitForm {
    pub source: SelectForm,
    /// Queue or topic; the source entity (its topic, for a subscription) when absent.
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub edits: EditsForm,
    #[serde(default)]
    pub new_message_id: bool,
    #[serde(default)]
    pub remove_original: bool,
    #[serde(default)]
    pub keep_dead_letter_properties: bool,
    #[serde(default)]
    pub drop_uncopyable_properties: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportForm {
    pub target: String,
    pub pick: String,
    #[serde(default)]
    pub new_message_id: bool,
    #[serde(default = "yes")]
    pub keep_sessions: bool,
    #[serde(default)]
    pub keep_dead_letter_properties: bool,
}

fn yes() -> bool {
    true
}

// ---- views ----

fn text(n: Option<i64>) -> Option<String> {
    n.map(|n| n.to_string())
}

/// One item of a job, as the report and the drawer show it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemView {
    pub sequence_number: Option<String>,
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub entity: String,
    pub new_sequence_number: Option<String>,
    /// `succeeded`, `failed`, `skipped`, `missing` or `not-reached`.
    pub outcome: &'static str,
    /// Why, for anything but a plain success; the detail of a success that has one.
    pub reason: Option<String>,
}

impl From<&ItemResult> for ItemView {
    fn from(i: &ItemResult) -> Self {
        let (outcome, reason) = match &i.outcome {
            ItemOutcome::Succeeded { detail } => ("succeeded", detail.clone()),
            ItemOutcome::Failed { .. } => ("failed", Some(JobReport::reason(i))),
            ItemOutcome::Skipped { .. } => ("skipped", Some(JobReport::reason(i))),
            ItemOutcome::Missing => ("missing", Some(JobReport::reason(i))),
            ItemOutcome::NotReached { .. } => ("not-reached", Some(JobReport::reason(i))),
        };
        Self {
            sequence_number: text(i.sequence_number),
            message_id: i.message_id.clone(),
            session_id: i.session_id.clone(),
            entity: i.entity.clone(),
            new_sequence_number: text(i.new_sequence_number),
            outcome,
            reason,
        }
    }
}

/// A received message for the job's live list: the grid's summary, with its sequence number as
/// text (serialised through [`with_text_sequence`], so the field appears once).
#[derive(Debug, Clone)]
pub struct ReceivedView(pub MessageSummary);

/// `summary` with `sequence_number` replaced by its decimal text.
pub fn with_text_sequence(mut summary: serde_json::Value) -> serde_json::Value {
    if let Some(seq) = summary.get("sequence_number").and_then(|v| v.as_i64()) {
        summary["sequence_number"] = serde_json::Value::String(seq.to_string());
    }
    summary
}

impl Serialize for ReceivedView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = serde_json::to_value(&self.0).map_err(serde::ser::Error::custom)?;
        with_text_sequence(v).serialize(s)
    }
}

// `MessageSummary` is not `PartialEq`; compare what is sent.
impl PartialEq for ReceivedView {
    fn eq(&self, other: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl From<MessageSummary> for ReceivedView {
    fn from(summary: MessageSummary) -> Self {
        Self(summary)
    }
}

/// How a finished job went, for the drawer row and the report header.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReportSummary {
    pub op_id: Option<String>,
    pub kind: String,
    pub entity: String,
    pub target: Option<String>,
    /// `succeeded`, `partly-failed`, `failed`, `cancelled`.
    pub outcome: String,
    pub cancelled: bool,
    pub items: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub skipped: u64,
    pub missing: u64,
    /// Items that leave the job incomplete: failed, missing, not reached, or skipped other than
    /// as the command promised (core `JobReport::incomplete`).
    pub incomplete: u64,
    pub notes: Vec<String>,
    pub summary_line: String,
}

fn kebab<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl From<&JobReport> for ReportSummary {
    fn from(r: &JobReport) -> Self {
        Self {
            op_id: r.op_id.clone(),
            kind: kebab(&r.kind),
            entity: r.entity.clone(),
            target: r.target.clone(),
            outcome: kebab(&r.outcome()).replace('_', "-"),
            cancelled: r.cancelled,
            items: r.items.len() as u64,
            succeeded: r.succeeded(),
            failed: r.failed(),
            skipped: r.skipped(),
            missing: r.missing(),
            incomplete: r.incomplete(),
            notes: r.notes.clone(),
            summary_line: r.summary_line(),
        }
    }
}

/// One page of a job's items.
#[derive(Debug, Clone, Serialize)]
pub struct ReportPage {
    pub summary: ReportSummary,
    pub offset: u64,
    pub total: u64,
    pub items: Vec<ItemView>,
}

pub fn report_page(r: &JobReport, offset: u64, limit: u64, only_problems: bool) -> ReportPage {
    let wanted: Vec<&ItemResult> = r
        .items
        .iter()
        .filter(|i| !only_problems || !matches!(i.outcome, ItemOutcome::Succeeded { .. }))
        .collect();
    ReportPage {
        summary: r.into(),
        offset,
        total: wanted.len() as u64,
        items: wanted
            .into_iter()
            .skip(offset as usize)
            .take(limit.min(1_000) as usize)
            .map(ItemView::from)
            .collect(),
    }
}

/// A count as the plan shows it: "1,284", "about 1,284", "at least 10,000", "unknown".
pub fn count_words(c: &CountEstimate) -> String {
    c.text("message")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PurgeLine {
    pub sub_queue: SubQueue,
    pub count: String,
}

/// One entity's purge plan, in words.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PurgePlanView {
    pub entity: String,
    pub scope: dispatched_core::ops::PurgeScope,
    pub lines: Vec<PurgeLine>,
    /// Deferred messages (scope `all`), in words; `None` when the scope leaves them alone.
    pub deferred: Option<String>,
    pub total: String,
    /// The total is not known: the purge is refused before it asks.
    pub unknown: bool,
    pub requires_session: Option<bool>,
}

impl From<&PurgePlan> for PurgePlanView {
    fn from(p: &PurgePlan) -> Self {
        let deferred = (p.scope == dispatched_core::ops::PurgeScope::All).then(|| {
            match (p.deferred, p.deferred_truncated) {
                (0, true) => "an unknown number of deferred messages".to_string(),
                (n, true) => format!(
                    "at least {} deferred messages",
                    dispatched_core::safety::count_text(n)
                ),
                (n, false) => format!(
                    "{} deferred messages",
                    dispatched_core::safety::count_text(n)
                ),
            }
        });
        Self {
            entity: p.entity.to_string(),
            scope: p.scope,
            lines: p
                .per_queue
                .iter()
                .map(|(q, c)| PurgeLine {
                    sub_queue: *q,
                    count: count_words(c),
                })
                .collect(),
            deferred,
            total: count_words(&p.total),
            unknown: matches!(p.total, CountEstimate::Unknown),
            requires_session: p.requires_session,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PurgePlans {
    /// Run these with `op_purge`; valid once.
    pub plan_id: String,
    pub plans: Vec<PurgePlanView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReasonGroupView {
    pub reason: Option<String>,
    pub description: Option<String>,
    pub count: u64,
    pub first_sequence: String,
    pub last_sequence: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DlqSummaryView {
    pub groups: Vec<ReasonGroupView>,
    pub scanned: u64,
    pub truncated: bool,
}

impl From<&DlqSummary> for DlqSummaryView {
    fn from(s: &DlqSummary) -> Self {
        Self {
            groups: s
                .groups
                .iter()
                .map(|g| ReasonGroupView {
                    reason: g.reason.clone(),
                    description: g.description.clone(),
                    count: g.count,
                    first_sequence: g.first_sequence.to_string(),
                    last_sequence: g.last_sequence.to_string(),
                })
                .collect(),
            scanned: s.scanned,
            truncated: s.truncated,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionStateView {
    pub base64: String,
    /// The state as text, when it is valid UTF-8.
    pub text: Option<String>,
    pub len: u64,
}

impl SessionStateView {
    pub fn of(bytes: &[u8]) -> Self {
        use base64::Engine as _;
        Self {
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            text: std::str::from_utf8(bytes).ok().map(str::to_string),
            len: bytes.len() as u64,
        }
    }
}

pub fn decode_state(b64: &str) -> Result<Vec<u8>, CommandError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| invalid("The state is not valid base64."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_view_sends_the_sequence_number_once_as_text() {
        let summary = serde_json::json!({"sequence_number": 9_007_199_254_740_993_i64, "subject": "x"});
        let v = with_text_sequence(summary);
        assert_eq!(v["sequence_number"], "9007199254740993");
        assert_eq!(v["subject"], "x");
        let text = v.to_string();
        assert_eq!(text.matches("sequence_number").count(), 1, "{text}");
    }

    #[test]
    fn sequences_accept_strings_and_numbers() {
        let s: Vec<Seq> =
            serde_json::from_value(serde_json::json!(["9007199254740993", 7])).unwrap();
        assert_eq!(seqs(&s), [9_007_199_254_740_993, 7]);
        assert!(serde_json::from_value::<Seq>(serde_json::json!("x")).is_err());
    }

    #[test]
    fn long_property_round_trips_as_string() {
        let p = PropertyForm {
            name: "n".into(),
            kind: "long".into(),
            value: "9007199254740993".into(),
        };
        assert_eq!(
            p.typed().unwrap().1,
            TypedValue::Long(9_007_199_254_740_993)
        );
        let bad = PropertyForm {
            kind: "int".into(),
            value: "3000000000".into(),
            ..p.clone()
        };
        assert_eq!(bad.typed().unwrap_err().kind, "invalid");
    }

    #[test]
    fn edits_from_the_resubmit_sheet() {
        let f: EditsForm = serde_json::from_value(serde_json::json!({
            "body": "{}", "session_id": "", "subject": "retry",
            "set": [{"name": "n", "type": "long", "value": "9007199254740993"}],
            "remove": ["old"]
        }))
        .unwrap();
        let e = f.edits().unwrap();
        assert_eq!(e.session_id, Some(None), "an empty id clears it");
        assert_eq!(e.subject, Some(Some("retry".into())));
        assert_eq!(e.correlation_id, None, "absent fields are left alone");
        assert_eq!(e.set["n"], TypedValue::Long(9_007_199_254_740_993));
        assert_eq!(e.remove, ["old"]);
    }

    #[test]
    fn duplicate_property_names_are_refused() {
        let f = OutgoingForm {
            properties: vec![
                PropertyForm {
                    name: "a".into(),
                    kind: "string".into(),
                    value: "1".into(),
                },
                PropertyForm {
                    name: "a".into(),
                    kind: "int".into(),
                    value: "2".into(),
                },
            ],
            ..Default::default()
        };
        assert!(f.message().unwrap_err().message.contains("twice"));
    }

    #[test]
    fn select_form_parses_entity_and_sequences() {
        let f: SelectForm = serde_json::from_value(serde_json::json!({
            "entity": "events/subscriptions/audit", "view": "dead-letter",
            "selection": {"type": "sequences", "sequences": ["12", "13"]}
        }))
        .unwrap();
        let spec = f.spec().unwrap();
        assert_eq!(spec.entity, EntityPath::subscription("events", "audit"));
        assert_eq!(
            spec.address(),
            "events/Subscriptions/audit/$DeadLetterQueue"
        );
        assert!(matches!(spec.selection, MessageSelection::Sequences(ref s) if s == &[12, 13]));
    }

    #[test]
    fn deferred_at_least_zero_reads_unknown() {
        let plan = PurgePlan {
            entity: EntityPath::queue("orders"),
            scope: dispatched_core::ops::PurgeScope::All,
            per_queue: vec![(SubQueue::Active, CountEstimate::Exact(1284))],
            deferred: 0,
            deferred_truncated: true,
            total: CountEstimate::AtLeast(1284),
            requires_session: Some(false),
        };
        let v = PurgePlanView::from(&plan);
        assert_eq!(
            v.deferred.as_deref(),
            Some("an unknown number of deferred messages")
        );
        assert_eq!(v.lines[0].count, "1,284 messages");
        assert!(!v.unknown);
    }

    #[test]
    fn report_page_has_string_sequences_and_filters_problems() {
        let item = |seq: i64, outcome: ItemOutcome| ItemResult {
            sequence_number: Some(seq),
            message_id: None,
            session_id: None,
            entity: "orders".into(),
            new_sequence_number: None,
            outcome,
        };
        let now = OffsetDateTime::now_utc();
        let r = JobReport {
            op_id: Some("op-1".into()),
            kind: dispatched_core::jobs::JobKind::Delete,
            entity: "orders".into(),
            target: None,
            started_at: now,
            finished_at: now,
            cancelled: false,
            items: vec![
                item(
                    9_007_199_254_740_993,
                    ItemOutcome::Succeeded { detail: None },
                ),
                item(2, ItemOutcome::Missing),
            ],
            notes: vec![],
            not_attempted: 0,
        };
        let all = report_page(&r, 0, 100, false);
        assert_eq!(
            all.items[0].sequence_number.as_deref(),
            Some("9007199254740993")
        );
        // A missing message is not a failure, but it leaves the job incomplete.
        assert_eq!(all.summary.outcome, "succeeded");
        assert_eq!((all.summary.missing, all.summary.incomplete), (1, 1));
        let bad = report_page(&r, 0, 100, true);
        assert_eq!(bad.total, 1);
        assert_eq!(bad.items[0].outcome, "missing");
    }
}
