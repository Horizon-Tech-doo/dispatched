//! Import and replay: send messages read from an export file to a queue or topic.

use super::failed;
use crate::ImportedMessage;
use crate::jobs::{ItemOutcome, ItemResult, JobKind, JobReport, JobRun, NotReached, Progress};
use crate::messaging::CloneOptions;
use crate::messaging::backend::MessagingBackend;
use crate::messaging::outgoing::OutgoingMessage;
use crate::messaging::sender::{copy_error, send_target};
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;

/// Messages one `send_all` call carries; cancel is checked between slices.
const IMPORT_SLICE: usize = 1_000;

/// How to turn imported messages into messages to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportOptions {
    /// Give each message a fresh random MessageId.
    pub new_message_id: bool,
    /// Keep SessionId (and the PartitionKey that equals it); `false` clears them.
    pub keep_sessions: bool,
    /// Drop `DeadLetterReason` and `DeadLetterErrorDescription`.
    pub drop_dead_letter_properties: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            new_message_id: false,
            keep_sessions: true,
            drop_dead_letter_properties: true,
        }
    }
}

impl ImportOptions {
    /// The clone rule these options map onto (the one dead-letter rule lives there).
    pub fn clone_options(&self) -> CloneOptions {
        CloneOptions {
            new_message_id: self.new_message_id,
            keep_dead_letter_properties: !self.drop_dead_letter_properties,
            from_dead_letter: true,
        }
    }

    /// The message to send for `m`.
    fn prepare(&self, m: &OutgoingMessage) -> OutgoingMessage {
        let rule = self.clone_options();
        let mut out = m.clone();
        out.properties.retain(|name, _| !rule.drops_property(name));
        if rule.new_message_id {
            out.message_id = Some(uuid::Uuid::new_v4().to_string());
        }
        if !self.keep_sessions {
            if out.session_id.is_some() && out.partition_key == out.session_id {
                out.partition_key = None;
            }
            out.session_id = None;
        }
        out
    }
}

fn item(entity: &str, m: &ImportedMessage, sent: &OutgoingMessage, o: ItemOutcome) -> ItemResult {
    ItemResult {
        sequence_number: m.source_sequence,
        message_id: sent.message_id.clone(),
        session_id: sent.session_id.clone(),
        entity: entity.to_string(),
        new_sequence_number: None,
        outcome: o,
    }
}

fn detail(m: &ImportedMessage) -> Option<String> {
    (!m.notes.is_empty()).then(|| m.notes.join("; "))
}

/// Send every message over one sender, in slices. An invalid message fails alone; a link
/// failure fails the rest of its slice and the next slice opens a new link. A cancelled job
/// stops before its next slice; the rest are `NotReached`. The sender is closed on every path.
pub(crate) async fn run<B: MessagingBackend + ?Sized>(
    b: &mut B,
    target: &str,
    messages: Vec<ImportedMessage>,
    opts: &ImportOptions,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<JobReport> {
    let entity = send_target(target)?.base();
    let mut run = JobRun::new(
        JobKind::Import,
        entity.clone(),
        None,
        progress,
        Some(messages.len() as u64),
    );
    let mut sender: Option<Box<dyn crate::messaging::backend::MessageSender>> = None;
    let mut attempted = false;
    let mut cancelled = false;
    for (n, slice) in messages.chunks(IMPORT_SLICE).enumerate() {
        if cancel.is_cancelled() {
            cancelled = true;
            for m in &messages[n * IMPORT_SLICE..] {
                let sent = opts.prepare(&m.message);
                let why = NotReached::Cancelled;
                run.item(item(&entity, m, &sent, ItemOutcome::NotReached { why }));
            }
            break;
        }
        let prepared: Vec<OutgoingMessage> =
            slice.iter().map(|m| opts.prepare(&m.message)).collect();
        let mut outcomes: Vec<Option<ItemOutcome>> = prepared
            .iter()
            .map(|p| p.validate().err().map(|e| failed(&e)))
            .collect();
        let ready: Vec<usize> = (0..prepared.len())
            .filter(|i| outcomes[*i].is_none())
            .collect();
        if !ready.is_empty() {
            let link = match sender.take() {
                Some(s) => Ok(s),
                None => b.open_sender(&entity).await,
            };
            match link {
                // Nothing reached the broker yet: an error, not a job.
                Err(e) if !attempted => return Err(e),
                Err(e) => {
                    for i in &ready {
                        outcomes[*i] = Some(failed(&e));
                    }
                }
                Ok(mut link) => {
                    attempted = true;
                    let batch: Vec<OutgoingMessage> =
                        ready.iter().map(|i| prepared[*i].clone()).collect();
                    let call = link
                        .send_all(&batch, &mut |j, r| {
                            outcomes[ready[j]] = Some(match r {
                                Ok(()) => ItemOutcome::Succeeded {
                                    detail: detail(&slice[ready[j]]),
                                },
                                Err(e) => failed(&e),
                            });
                        })
                        .await;
                    for i in &ready {
                        if outcomes[*i].is_none() {
                            let e = call.as_ref().err().map_or_else(
                                || {
                                    Error::Protocol(
                                        "the broker reported no outcome for this message".into(),
                                    )
                                },
                                copy_error,
                            );
                            outcomes[*i] = Some(failed(&e));
                        }
                    }
                    if call.is_ok() {
                        sender = Some(link);
                    } else {
                        link.close().await;
                    }
                }
            }
        }
        for ((m, sent), o) in slice.iter().zip(&prepared).zip(outcomes) {
            run.item(item(&entity, m, sent, o.expect("filled")));
        }
    }
    if let Some(s) = sender {
        s.close().await;
    }
    Ok(run.finish(cancelled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::NoProgress;
    use crate::messaging::OutgoingBody;
    use crate::messaging::TypedValue;
    use crate::messaging::fake::FakeBroker;

    fn imported(seq: i64, body: &str) -> ImportedMessage {
        ImportedMessage {
            source_sequence: Some(seq),
            message: OutgoingMessage::text(body),
            notes: vec![],
        }
    }

    #[tokio::test]
    async fn import_sends_each_message_and_reports_by_source_sequence() {
        let b = FakeBroker::default();
        let msgs = vec![imported(3, "a"), imported(9, "b")];
        let r = run(
            &mut b.clone(),
            "q",
            msgs,
            &ImportOptions::default(),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            r.items
                .iter()
                .map(|i| i.sequence_number)
                .collect::<Vec<_>>(),
            [Some(3), Some(9)]
        );
        assert_eq!(b.sent("q").len(), 2);
    }

    #[tokio::test]
    async fn invalid_messages_fail_alone() {
        let b = FakeBroker::default();
        let mut bad = imported(1, "bad");
        bad.message.message_id = Some("x".repeat(200));
        let r = run(
            &mut b.clone(),
            "q",
            vec![bad, imported(2, "good")],
            &ImportOptions::default(),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(
            matches!(&r.items[0].outcome, ItemOutcome::Failed { kind: "invalid", reason } if reason.contains("MessageId"))
        );
        assert!(matches!(r.items[1].outcome, ItemOutcome::Succeeded { .. }));
        assert_eq!(b.sent("q").len(), 1);
        assert_eq!(b.sent("q")[0].body, OutgoingBody::Bytes(b"good".to_vec()));
    }

    #[tokio::test]
    async fn options_strip_sessions_and_dead_letter_properties() {
        let b = FakeBroker::default();
        let mut m = imported(1, "x");
        m.message.session_id = Some("s".into());
        m.message.partition_key = Some("s".into());
        m.message.properties.insert(
            "DeadLetterReason".into(),
            TypedValue::String("Manual".into()),
        );
        m.message.properties.insert(
            "DeadLetterErrorDescription".into(),
            TypedValue::String("bad".into()),
        );
        m.message
            .properties
            .insert("kind".into(), TypedValue::String("order".into()));
        let opts = ImportOptions {
            keep_sessions: false,
            ..Default::default()
        };
        run(
            &mut b.clone(),
            "q",
            vec![m.clone()],
            &opts,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let sent = &b.sent("q")[0];
        assert_eq!(
            (sent.session_id.as_deref(), sent.partition_key.as_deref()),
            (None, None)
        );
        assert!(
            !sent.properties.contains_key("DeadLetterReason")
                && !sent.properties.contains_key("DeadLetterErrorDescription"),
            "dropped by default"
        );
        assert_eq!(sent.properties["kind"], TypedValue::String("order".into()));
        let keep = ImportOptions {
            drop_dead_letter_properties: false,
            ..Default::default()
        };
        run(
            &mut b.clone(),
            "q2",
            vec![m],
            &keep,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let kept = &b.sent("q2")[0];
        assert_eq!(
            kept.session_id.as_deref(),
            Some("s"),
            "sessions kept by default"
        );
        assert!(kept.properties.contains_key("DeadLetterReason"));
    }

    #[tokio::test]
    async fn a_subscription_is_refused_and_cancel_leaves_the_rest_not_reached() {
        let b = FakeBroker::default();
        let e = run(
            &mut b.clone(),
            "t/Subscriptions/s",
            vec![imported(1, "a")],
            &ImportOptions::default(),
            &NoProgress,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(e, Err(Error::Invalid(_))));
        let c = CancellationToken::new();
        c.cancel();
        let r = run(
            &mut b.clone(),
            "q",
            vec![imported(1, "a")],
            &ImportOptions::default(),
            &NoProgress,
            &c,
        )
        .await
        .unwrap();
        assert!(r.cancelled);
        assert!(matches!(
            r.items[0].outcome,
            ItemOutcome::NotReached {
                why: NotReached::Cancelled
            }
        ));
        assert!(b.sent("q").is_empty());
    }

    #[tokio::test]
    async fn new_message_id_and_notes_detail() {
        let b = FakeBroker::default();
        let mut m = imported(1, "a");
        m.message.message_id = Some("old".into());
        m.notes = vec!["body re-encoded".into()];
        let opts = ImportOptions {
            new_message_id: true,
            ..Default::default()
        };
        let r = run(
            &mut b.clone(),
            "q",
            vec![m],
            &opts,
            &NoProgress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_ne!(b.sent("q")[0].message_id.as_deref(), Some("old"));
        assert!(
            matches!(&r.items[0].outcome, ItemOutcome::Succeeded { detail: Some(d) } if d == "body re-encoded")
        );
    }
}
