//! A change to an entity's settings: the settings given, and the rules for which settings a
//! kind has and which are fixed at creation. The CLI's flags and the desktop's form both build
//! one, so "only the given settings change" means the same in both.

use super::EntityKind;
use super::model::{EntityStatus, QueueSettings, SubscriptionSettings, TimeSpan, TopicSettings};
use crate::messaging::parse_duration;
use crate::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::time::Duration;

/// The settings to set. `None` leaves a setting as it is (on create: the service default).
/// `""` for a forwarding target or the metadata clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SettingsChange {
    /// Peek-lock duration (queues, subscriptions).
    #[serde(deserialize_with = "de_duration")]
    pub lock_duration: Option<Duration>,
    /// Maximum size in megabytes (queues, topics).
    pub max_size_in_megabytes: Option<u64>,
    /// Duplicate detection (create only; queues, topics).
    pub requires_duplicate_detection: Option<bool>,
    /// Sessions (create only; queues, subscriptions).
    pub requires_session: Option<bool>,
    /// Default message time to live; `max` / `never` is [`TimeSpan::MAX`].
    #[serde(deserialize_with = "de_duration")]
    pub default_message_time_to_live: Option<Duration>,
    /// Dead-letter messages that expire (queues, subscriptions).
    pub dead_lettering_on_message_expiration: Option<bool>,
    /// Dead-letter messages whose filter evaluation fails (subscriptions).
    pub dead_lettering_on_filter_evaluation_exceptions: Option<bool>,
    /// Duplicate detection window (queues, topics). On create it turns detection on.
    #[serde(deserialize_with = "de_duration")]
    pub duplicate_detection_history_time_window: Option<Duration>,
    /// Deliveries before a message is dead-lettered (queues, subscriptions).
    pub max_delivery_count: Option<u32>,
    /// Server-side batched operations.
    pub enable_batched_operations: Option<bool>,
    /// Status.
    pub status: Option<EntityStatus>,
    /// Auto-forward target (queues, subscriptions); `""` stops forwarding.
    pub forward_to: Option<String>,
    /// Auto-forward target for dead-lettered messages; `""` stops it.
    pub forward_dead_lettered_messages_to: Option<String>,
    /// Idle time after which the entity is deleted; `max` / `never` is [`TimeSpan::MAX`].
    #[serde(deserialize_with = "de_duration")]
    pub auto_delete_on_idle: Option<Duration>,
    /// Partitioning (create only; queues, topics).
    pub enable_partitioning: Option<bool>,
    /// Express entity (queues, topics).
    pub enable_express: Option<bool>,
    /// Ordered delivery (topics).
    pub support_ordering: Option<bool>,
    /// Maximum message size in kilobytes (Premium; queues, topics).
    pub max_message_size_in_kilobytes: Option<u64>,
    /// Free text; `""` clears it.
    pub user_metadata: Option<String>,
}

/// Why a setting was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueReason {
    /// The kind of entity has no such setting.
    NotForKind,
    /// It is set when the entity is created and cannot change.
    FixedAtCreation,
    /// The value is not one the setting takes.
    Value,
}

/// A refused setting, by its [`SettingsChange`] field name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldIssue {
    /// Field name.
    pub field: &'static str,
    /// What is wrong, as a sentence.
    pub message: String,
    /// Why.
    pub reason: IssueReason,
}

/// Every field, in declaration order, with the kinds that have it.
const FIELDS: &[(&str, &[EntityKind])] = {
    use EntityKind::{Queue as Q, Subscription as S, Topic as T};
    &[
        ("lock_duration", &[Q, S]),
        ("max_size_in_megabytes", &[Q, T]),
        ("requires_duplicate_detection", &[Q, T]),
        ("requires_session", &[Q, S]),
        ("default_message_time_to_live", &[Q, T, S]),
        ("dead_lettering_on_message_expiration", &[Q, S]),
        ("dead_lettering_on_filter_evaluation_exceptions", &[S]),
        ("duplicate_detection_history_time_window", &[Q, T]),
        ("max_delivery_count", &[Q, S]),
        ("enable_batched_operations", &[Q, T, S]),
        ("status", &[Q, T, S]),
        ("forward_to", &[Q, S]),
        ("forward_dead_lettered_messages_to", &[Q, S]),
        ("auto_delete_on_idle", &[Q, T, S]),
        ("enable_partitioning", &[Q, T]),
        ("enable_express", &[Q, T]),
        ("support_ordering", &[T]),
        ("max_message_size_in_kilobytes", &[Q, T]),
        ("user_metadata", &[Q, T, S]),
    ]
};

/// Set at creation only.
pub const FIXED_AT_CREATION: &[&str] = &[
    "requires_duplicate_detection",
    "requires_session",
    "enable_partitioning",
];

/// The fields `kind` has, in declaration order.
pub fn fields_of(kind: EntityKind) -> impl Iterator<Item = &'static str> {
    FIELDS
        .iter()
        .filter(move |(_, kinds)| kinds.contains(&kind))
        .map(|(f, _)| *f)
}

fn kind_name(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Queue => "queue",
        EntityKind::Topic => "topic",
        EntityKind::Subscription => "subscription",
    }
}

impl SettingsChange {
    /// Names of the fields that are given, in declaration order.
    pub fn given(&self) -> Vec<&'static str> {
        let set = [
            self.lock_duration.is_some(),
            self.max_size_in_megabytes.is_some(),
            self.requires_duplicate_detection.is_some(),
            self.requires_session.is_some(),
            self.default_message_time_to_live.is_some(),
            self.dead_lettering_on_message_expiration.is_some(),
            self.dead_lettering_on_filter_evaluation_exceptions
                .is_some(),
            self.duplicate_detection_history_time_window.is_some(),
            self.max_delivery_count.is_some(),
            self.enable_batched_operations.is_some(),
            self.status.is_some(),
            self.forward_to.is_some(),
            self.forward_dead_lettered_messages_to.is_some(),
            self.auto_delete_on_idle.is_some(),
            self.enable_partitioning.is_some(),
            self.enable_express.is_some(),
            self.support_ordering.is_some(),
            self.max_message_size_in_kilobytes.is_some(),
            self.user_metadata.is_some(),
        ];
        FIELDS
            .iter()
            .zip(set)
            .filter(|(_, s)| *s)
            .map(|((f, _), _)| *f)
            .collect()
    }

    /// No setting is given.
    pub fn is_empty(&self) -> bool {
        self.given().is_empty()
    }

    /// Every given setting `kind` does not have, and on update every one fixed at creation,
    /// one issue per field in declaration order.
    pub fn issues(&self, kind: EntityKind, create: bool) -> Vec<FieldIssue> {
        let mut out = Vec::new();
        for field in self.given() {
            let has = FIELDS
                .iter()
                .any(|(f, kinds)| *f == field && kinds.contains(&kind));
            if !has {
                out.push(FieldIssue {
                    field,
                    message: format!("A {} has no such setting.", kind_name(kind)),
                    reason: IssueReason::NotForKind,
                });
            } else if !create && FIXED_AT_CREATION.contains(&field) {
                out.push(FieldIssue {
                    field,
                    message: "Set when the entity is created; it cannot change.".into(),
                    reason: IssueReason::FixedAtCreation,
                });
            }
        }
        if create
            && self.requires_duplicate_detection == Some(false)
            && self.duplicate_detection_history_time_window.is_some()
        {
            out.push(FieldIssue {
                field: "duplicate_detection_history_time_window",
                message: "A duplicate detection window needs duplicate detection.".into(),
                reason: IssueReason::Value,
            });
        }
        out
    }

    fn check(&self, kind: EntityKind, create: bool) -> Result<()> {
        let issues = self.issues(kind, create);
        let names = |r: IssueReason| -> Vec<&str> {
            issues
                .iter()
                .filter(|i| i.reason == r)
                .map(|i| i.field)
                .collect()
        };
        let wrong = names(IssueReason::NotForKind);
        if !wrong.is_empty() {
            return Err(Error::Invalid(format!(
                "{} does not apply to a {}",
                wrong.join(", "),
                kind_name(kind)
            )));
        }
        let fixed = names(IssueReason::FixedAtCreation);
        if !fixed.is_empty() {
            return Err(Error::Invalid(format!(
                "{} cannot be changed after the entity is created",
                fixed.join(", ")
            )));
        }
        if let Some(i) = issues.first() {
            return Err(Error::Invalid(format!("{}: {}", i.field, i.message)));
        }
        Ok(())
    }

    /// Put the given settings on a queue's (`create`: a new queue).
    pub fn apply_queue(&self, s: &mut QueueSettings, create: bool) -> Result<()> {
        self.check(EntityKind::Queue, create)?;
        set(&mut s.lock_duration, self.lock_duration);
        set(&mut s.max_size_in_megabytes, self.max_size_in_megabytes);
        set(&mut s.requires_session, self.requires_session);
        set(
            &mut s.default_message_time_to_live,
            self.default_message_time_to_live,
        );
        set(
            &mut s.dead_lettering_on_message_expiration,
            self.dead_lettering_on_message_expiration,
        );
        self.duplicates(
            &mut s.requires_duplicate_detection,
            &mut s.duplicate_detection_history_time_window,
            create,
        )?;
        set(&mut s.max_delivery_count, self.max_delivery_count);
        set(
            &mut s.enable_batched_operations,
            self.enable_batched_operations,
        );
        set(&mut s.status, self.status);
        clearable(&mut s.forward_to, &self.forward_to);
        clearable(
            &mut s.forward_dead_lettered_messages_to,
            &self.forward_dead_lettered_messages_to,
        );
        set(&mut s.auto_delete_on_idle, self.auto_delete_on_idle);
        set(&mut s.enable_partitioning, self.enable_partitioning);
        set(&mut s.enable_express, self.enable_express);
        set(
            &mut s.max_message_size_in_kilobytes,
            self.max_message_size_in_kilobytes,
        );
        text(&mut s.user_metadata, &self.user_metadata);
        Ok(())
    }

    /// Put the given settings on a topic's.
    pub fn apply_topic(&self, s: &mut TopicSettings, create: bool) -> Result<()> {
        self.check(EntityKind::Topic, create)?;
        set(&mut s.max_size_in_megabytes, self.max_size_in_megabytes);
        self.duplicates(
            &mut s.requires_duplicate_detection,
            &mut s.duplicate_detection_history_time_window,
            create,
        )?;
        set(
            &mut s.default_message_time_to_live,
            self.default_message_time_to_live,
        );
        set(
            &mut s.enable_batched_operations,
            self.enable_batched_operations,
        );
        set(&mut s.status, self.status);
        set(&mut s.auto_delete_on_idle, self.auto_delete_on_idle);
        set(&mut s.enable_partitioning, self.enable_partitioning);
        set(&mut s.enable_express, self.enable_express);
        set(&mut s.support_ordering, self.support_ordering);
        set(
            &mut s.max_message_size_in_kilobytes,
            self.max_message_size_in_kilobytes,
        );
        text(&mut s.user_metadata, &self.user_metadata);
        Ok(())
    }

    /// Put the given settings on a subscription's.
    pub fn apply_subscription(&self, s: &mut SubscriptionSettings, create: bool) -> Result<()> {
        self.check(EntityKind::Subscription, create)?;
        set(&mut s.lock_duration, self.lock_duration);
        set(&mut s.requires_session, self.requires_session);
        set(
            &mut s.default_message_time_to_live,
            self.default_message_time_to_live,
        );
        set(
            &mut s.dead_lettering_on_message_expiration,
            self.dead_lettering_on_message_expiration,
        );
        set(
            &mut s.dead_lettering_on_filter_evaluation_exceptions,
            self.dead_lettering_on_filter_evaluation_exceptions,
        );
        set(&mut s.max_delivery_count, self.max_delivery_count);
        set(
            &mut s.enable_batched_operations,
            self.enable_batched_operations,
        );
        set(&mut s.status, self.status);
        clearable(&mut s.forward_to, &self.forward_to);
        clearable(
            &mut s.forward_dead_lettered_messages_to,
            &self.forward_dead_lettered_messages_to,
        );
        set(&mut s.auto_delete_on_idle, self.auto_delete_on_idle);
        text(&mut s.user_metadata, &self.user_metadata);
        Ok(())
    }

    /// Duplicate detection and its window: on create a window turns detection on; on update a
    /// window changes only an entity that has detection (it cannot be turned on later).
    fn duplicates(
        &self,
        requires: &mut Option<bool>,
        window: &mut Option<Duration>,
        create: bool,
    ) -> Result<()> {
        if create {
            set(requires, self.requires_duplicate_detection);
        }
        if let Some(w) = self.duplicate_detection_history_time_window {
            if create {
                *requires = Some(true);
            } else if *requires != Some(true) {
                return Err(Error::Invalid(
                    "duplicate detection is off on this entity and can only be turned on when \
                     it is created"
                        .into(),
                ));
            }
            *window = Some(w);
        }
        Ok(())
    }
}

/// Set `field` when the setting is given.
fn set<T>(field: &mut Option<T>, v: Option<T>) {
    if v.is_some() {
        *field = v;
    }
}

/// A name where `""` (after trimming) clears.
fn clearable(field: &mut Option<String>, v: &Option<String>) {
    if let Some(s) = v {
        let s = s.trim();
        *field = (!s.is_empty()).then(|| s.to_string());
    }
}

/// Free text where `""` clears; other text is kept as given.
fn text(field: &mut Option<String>, v: &Option<String>) {
    if let Some(s) = v {
        *field = (!s.is_empty()).then(|| s.clone());
    }
}

/// A duration as text: the `parse_duration` syntax, or `max` / `never` for the longest.
pub fn parse_setting_duration(text: &str) -> Result<Duration> {
    if matches!(text.trim().to_ascii_lowercase().as_str(), "max" | "never") {
        return Ok(TimeSpan::MAX);
    }
    parse_duration(text)
}

fn de_duration<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<Duration>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|t| parse_setting_duration(&t).map_err(serde::de::Error::custom))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn refuses_a_setting_the_kind_lacks() {
        let c = SettingsChange {
            support_ordering: Some(true),
            ..Default::default()
        };
        let e = c
            .apply_queue(&mut QueueSettings::default(), true)
            .unwrap_err();
        assert!(e.to_string().contains("support_ordering"), "{e}");
        let c = SettingsChange {
            lock_duration: Some(secs(30)),
            ..Default::default()
        };
        assert!(c.apply_topic(&mut TopicSettings::default(), true).is_err());
    }

    #[test]
    fn refuses_creation_only_settings_on_update() {
        for c in [
            SettingsChange {
                requires_session: Some(true),
                ..Default::default()
            },
            SettingsChange {
                enable_partitioning: Some(true),
                ..Default::default()
            },
            SettingsChange {
                requires_duplicate_detection: Some(true),
                ..Default::default()
            },
        ] {
            let e = c
                .apply_queue(&mut QueueSettings::default(), false)
                .unwrap_err();
            assert!(e.to_string().contains("cannot be changed after"), "{e}");
            assert!(c.apply_queue(&mut QueueSettings::default(), true).is_ok());
        }
    }

    #[test]
    fn duplicate_window_on_create_turns_detection_on() {
        let c = SettingsChange {
            duplicate_detection_history_time_window: Some(secs(600)),
            ..Default::default()
        };
        let mut s = QueueSettings::default();
        c.apply_queue(&mut s, true).unwrap();
        assert_eq!(s.requires_duplicate_detection, Some(true));
        assert_eq!(s.duplicate_detection_history_time_window, Some(secs(600)));
    }

    #[test]
    fn duplicate_window_on_update_needs_detection() {
        let c = SettingsChange {
            duplicate_detection_history_time_window: Some(secs(600)),
            ..Default::default()
        };
        let mut s = TopicSettings {
            requires_duplicate_detection: Some(false),
            ..Default::default()
        };
        assert!(c.apply_topic(&mut s, false).is_err());
        s.requires_duplicate_detection = Some(true);
        c.apply_topic(&mut s, false).unwrap();
        assert_eq!(s.duplicate_detection_history_time_window, Some(secs(600)));
    }

    #[test]
    fn empty_string_clears_forwarding_and_metadata() {
        let c = SettingsChange {
            forward_to: Some(String::new()),
            forward_dead_lettered_messages_to: Some(" ".into()),
            user_metadata: Some(String::new()),
            ..Default::default()
        };
        let mut s = SubscriptionSettings {
            forward_to: Some("x".into()),
            forward_dead_lettered_messages_to: Some("y".into()),
            user_metadata: Some("z".into()),
            ..Default::default()
        };
        c.apply_subscription(&mut s, false).unwrap();
        assert_eq!(s.forward_to, None);
        assert_eq!(s.forward_dead_lettered_messages_to, None);
        assert_eq!(s.user_metadata, None);
    }

    #[test]
    fn only_given_fields_change() {
        let full = QueueSettings {
            lock_duration: Some(secs(45)),
            max_size_in_megabytes: Some(2048),
            requires_duplicate_detection: Some(true),
            requires_session: Some(false),
            default_message_time_to_live: Some(TimeSpan::MAX),
            dead_lettering_on_message_expiration: Some(true),
            duplicate_detection_history_time_window: Some(secs(600)),
            max_delivery_count: Some(7),
            enable_batched_operations: Some(true),
            status: Some(EntityStatus::Active),
            forward_to: Some("a".into()),
            forward_dead_lettered_messages_to: Some("b".into()),
            auto_delete_on_idle: Some(secs(86_400)),
            enable_partitioning: Some(false),
            enable_express: Some(false),
            max_message_size_in_kilobytes: Some(1024),
            user_metadata: Some("m".into()),
        };
        let mut s = full.clone();
        SettingsChange {
            max_delivery_count: Some(3),
            ..Default::default()
        }
        .apply_queue(&mut s, false)
        .unwrap();
        assert_eq!(
            s,
            QueueSettings {
                max_delivery_count: Some(3),
                ..full
            }
        );
    }

    #[test]
    fn deserializes_duration_text() {
        let c: SettingsChange = serde_json::from_str(
            r#"{"lock_duration":"45s","default_message_time_to_live":"never","auto_delete_on_idle":"P7D"}"#,
        )
        .unwrap();
        assert_eq!(c.lock_duration, Some(secs(45)));
        assert_eq!(c.default_message_time_to_live, Some(TimeSpan::MAX));
        assert_eq!(c.auto_delete_on_idle, Some(secs(7 * 86_400)));
        assert!(serde_json::from_str::<SettingsChange>(r#"{"lock_duration":"0s"}"#).is_err());
        assert!(serde_json::from_str::<SettingsChange>(r#"{"bogus":1}"#).is_err());
    }

    #[test]
    fn issues_lists_each_bad_field_once() {
        let c = SettingsChange {
            lock_duration: Some(secs(30)),
            requires_session: Some(true),
            support_ordering: Some(true),
            ..Default::default()
        };
        let issues = c.issues(EntityKind::Topic, true);
        let fields: Vec<_> = issues.iter().map(|i| i.field).collect();
        assert_eq!(fields, ["lock_duration", "requires_session"]);
        assert!(
            c.issues(EntityKind::Queue, true)
                .iter()
                .any(|i| i.field == "support_ordering")
        );
    }
}
