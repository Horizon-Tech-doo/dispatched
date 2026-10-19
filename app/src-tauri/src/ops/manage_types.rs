//! What the manage screens send and receive: an entity's settings as a form, rule forms, and
//! the field checks the forms run as they are edited.

use dispatched_core::EntityKind;
use dispatched_core::messaging::duration_text;
use dispatched_core::mgmt::change::{FIXED_AT_CREATION, fields_of};
use dispatched_core::mgmt::{
    CorrelationProperty, RuleFilter, RuleSpec, SettingsChange, validate_filter,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Settings that are durations: shown and edited as `duration_text`.
const DURATIONS: &[&str] = &[
    "lock_duration",
    "default_message_time_to_live",
    "duplicate_detection_history_time_window",
    "auto_delete_on_idle",
];

/// An entity's settings for the edit form, by `SettingsChange` field name: every setting the
/// kind has (`null` when not set), durations as text.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub kind: EntityKind,
    pub entity: String,
    pub settings: Map<String, Value>,
    /// The kind's settings that are fixed at creation.
    pub fixed: Vec<&'static str>,
}

/// The view of `settings` (a queue's, topic's or subscription's).
pub fn settings_view<T: Serialize>(kind: EntityKind, entity: &str, settings: &T) -> SettingsView {
    let all = match serde_json::to_value(settings) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let mut out = Map::new();
    for field in fields_of(kind) {
        let mut v = all.get(field).cloned().unwrap_or(Value::Null);
        if DURATIONS.contains(&field) && !v.is_null() {
            let d: std::time::Duration = serde_json::from_value(v).unwrap_or_default();
            v = Value::String(duration_text(d));
        }
        out.insert(field.to_string(), v);
    }
    SettingsView {
        kind,
        entity: entity.to_string(),
        settings: out,
        fixed: fields_of(kind)
            .filter(|f| FIXED_AT_CREATION.contains(f))
            .collect(),
    }
}

/// A problem with one field of a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormIssue {
    pub field: String,
    pub message: String,
}

/// serde's message without its trailing position, as a sentence.
fn serde_message(e: &serde_json::Error) -> String {
    let m = e.to_string();
    let m = m.split(" at line ").next().unwrap_or(&m).to_string();
    m
}

/// Every problem with a settings form: values that do not parse, unknown fields, settings
/// the kind does not have, and on update the ones fixed at creation. One per field.
pub fn check_settings(kind: EntityKind, form: &Value, create: bool) -> Vec<FormIssue> {
    let Some(map) = form.as_object() else {
        return vec![FormIssue {
            field: String::new(),
            message: "The settings are not a form.".into(),
        }];
    };
    let mut out = Vec::new();
    let mut parsed = Map::new();
    for (k, v) in map {
        let one = Value::Object(Map::from_iter([(k.clone(), v.clone())]));
        match serde_json::from_value::<SettingsChange>(one) {
            Ok(_) => {
                parsed.insert(k.clone(), v.clone());
            }
            Err(e) => out.push(FormIssue {
                field: k.clone(),
                message: serde_message(&e),
            }),
        }
    }
    // The fields that parsed, checked against the kind.
    if let Ok(change) = serde_json::from_value::<SettingsChange>(Value::Object(parsed)) {
        out.extend(change.issues(kind, create).into_iter().map(|i| FormIssue {
            field: i.field.to_string(),
            message: i.message,
        }));
    }
    out.sort_by(|a, b| a.field.cmp(&b.field));
    out.dedup_by(|a, b| a.field == b.field);
    out
}

/// A settings form as a change; the first problem as an `invalid` error.
pub fn settings_change(
    kind: EntityKind,
    form: &Value,
    create: bool,
) -> Result<SettingsChange, crate::error::CommandError> {
    if let Some(i) = check_settings(kind, form, create).into_iter().next() {
        return Err(crate::error::CommandError::new(
            "invalid",
            format!("{}: {}", i.field, i.message),
        ));
    }
    serde_json::from_value(form.clone())
        .map_err(|e| crate::error::CommandError::new("invalid", serde_message(&e)))
}

/// A rule as the rule editor sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct RuleForm {
    pub name: String,
    pub filter: RuleFilterForm,
    #[serde(default)]
    pub action: Option<String>,
}

/// A correlation filter's user property.
#[derive(Debug, Clone, Deserialize)]
pub struct PropertyForm {
    pub name: String,
    pub value: String,
}

/// A rule filter as the editor sends it: the read-side `RuleFilter` shapes.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuleFilterForm {
    True,
    Sql {
        expression: String,
    },
    Correlation {
        #[serde(default)]
        correlation_id: Option<String>,
        #[serde(default)]
        message_id: Option<String>,
        #[serde(default)]
        to: Option<String>,
        #[serde(default)]
        reply_to: Option<String>,
        #[serde(default)]
        subject: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        reply_to_session_id: Option<String>,
        #[serde(default)]
        content_type: Option<String>,
        #[serde(default)]
        properties: Vec<PropertyForm>,
    },
}

/// `""` is not set.
fn some(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

impl From<RuleFilterForm> for RuleFilter {
    fn from(f: RuleFilterForm) -> Self {
        match f {
            RuleFilterForm::True => RuleFilter::True,
            RuleFilterForm::Sql { expression } => RuleFilter::Sql { expression },
            RuleFilterForm::Correlation {
                correlation_id,
                message_id,
                to,
                reply_to,
                subject,
                session_id,
                reply_to_session_id,
                content_type,
                properties,
            } => RuleFilter::Correlation {
                correlation_id: some(correlation_id),
                message_id: some(message_id),
                to: some(to),
                reply_to: some(reply_to),
                subject: some(subject),
                session_id: some(session_id),
                reply_to_session_id: some(reply_to_session_id),
                content_type: some(content_type),
                properties: properties
                    .into_iter()
                    .map(|p| CorrelationProperty {
                        name: p.name,
                        value: p.value,
                    })
                    .collect(),
            },
        }
    }
}

/// The rule's problems: an empty name, and a filter the service would refuse or that matches
/// nothing (#794).
pub fn rule_issues(form: &RuleForm) -> Vec<FormIssue> {
    let mut out = Vec::new();
    if form.name.trim().is_empty() {
        out.push(FormIssue {
            field: "name".into(),
            message: "A rule needs a name.".into(),
        });
    }
    if let Err(e) = validate_filter(&form.filter.clone().into()) {
        out.push(FormIssue {
            field: "filter".into(),
            message: e.to_string(),
        });
    }
    out
}

impl TryFrom<RuleForm> for RuleSpec {
    type Error = crate::error::CommandError;

    fn try_from(form: RuleForm) -> Result<Self, Self::Error> {
        if let Some(i) = rule_issues(&form).into_iter().next() {
            return Err(crate::error::CommandError::new("invalid", i.message));
        }
        Ok(RuleSpec {
            name: form.name.trim().to_string(),
            filter: form.filter.into(),
            action: some(form.action),
        })
    }
}

/// The connection string of an entity's own SAS policy.
pub fn policy_connection_string(
    host: &str,
    emulator: bool,
    entity: &str,
    policy: &str,
    key: &str,
) -> SecretString {
    let tail = if emulator {
        ";UseDevelopmentEmulator=true"
    } else {
        ""
    };
    SecretString::from(format!(
        "Endpoint=sb://{host}/;SharedAccessKeyName={policy};SharedAccessKey={key};EntityPath={entity}{tail}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::mgmt::model::TimeSpan;
    use dispatched_core::mgmt::{AccessPolicy, AccessRight, QueueSettings, RuleFilter};
    use secrecy::SecretString;
    use std::time::Duration;

    #[test]
    fn settings_view_shows_durations_as_text() {
        let s = QueueSettings {
            lock_duration: Some(Duration::from_secs(45)),
            default_message_time_to_live: Some(TimeSpan::MAX),
            max_delivery_count: Some(7),
            ..Default::default()
        };
        let v = settings_view(EntityKind::Queue, "orders", &s);
        assert_eq!(v.settings["lock_duration"], "45s");
        assert_eq!(v.settings["default_message_time_to_live"], "max");
        assert_eq!(v.settings["max_delivery_count"], 7);
        assert!(v.settings["forward_to"].is_null());
        assert!(v.fixed.contains(&"requires_session"));
        assert!(
            !v.settings.contains_key("support_ordering"),
            "a queue has no ordering"
        );
    }

    #[test]
    fn policy_views_have_no_keys() {
        let p = AccessPolicy {
            key_name: "app".into(),
            rights: vec![AccessRight::Listen],
            primary_key: SecretString::from("cHJpbWFyeS1rZXk=".to_string()),
            secondary_key: SecretString::from("c2Vjb25kYXJ5LWtleQ==".to_string()),
        };
        let json = serde_json::to_string(&dispatched_core::mgmt::policies::views(&[p])).unwrap();
        assert!(
            !json.contains("cHJpbWFyeS1rZXk") && !json.contains("c2Vjb25kYXJ5"),
            "{json}"
        );
        assert!(!json.contains("primary"), "{json}");
    }

    #[test]
    fn check_settings_reports_each_field() {
        let form = serde_json::json!({"lock_duration": "0s", "support_ordering": true});
        let issues = check_settings(EntityKind::Queue, &form, true);
        let fields: Vec<_> = issues.iter().map(|i| i.field.as_str()).collect();
        assert_eq!(fields, ["lock_duration", "support_ordering"]);
        assert!(issues[0].message.contains("0s"), "{}", issues[0].message);
        let ok = serde_json::json!({"lock_duration": "45s"});
        assert!(check_settings(EntityKind::Queue, &ok, true).is_empty());
        let unknown = serde_json::json!({"colour": "red"});
        assert_eq!(
            check_settings(EntityKind::Queue, &unknown, true)[0].field,
            "colour"
        );
    }

    #[test]
    fn rule_form_to_spec() {
        let t: RuleForm = serde_json::from_value(serde_json::json!(
            {"name": "$Default", "filter": {"kind": "true"}, "action": null}
        ))
        .unwrap();
        assert_eq!(RuleSpec::try_from(t).unwrap().filter, RuleFilter::True);
        let c: RuleForm = serde_json::from_value(serde_json::json!({
            "name": "eu",
            "filter": {"kind": "correlation", "subject": "order", "properties": [{"name": "region", "value": "eu"}]},
            "action": "SET x = 1"
        }))
        .unwrap();
        let spec = RuleSpec::try_from(c).unwrap();
        assert!(
            matches!(spec.filter, RuleFilter::Correlation { ref subject, ref properties, .. }
            if subject.as_deref() == Some("order") && properties.len() == 1)
        );
        assert_eq!(spec.action.as_deref(), Some("SET x = 1"));
        let empty: RuleForm = serde_json::from_value(serde_json::json!(
            {"name": "c", "filter": {"kind": "correlation", "properties": []}}
        ))
        .unwrap();
        let issues = rule_issues(&empty);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "filter");
        assert!(
            issues[0].message.contains("matches no message"),
            "{}",
            issues[0].message
        );
        let unnamed: RuleForm =
            serde_json::from_value(serde_json::json!({"name": " ", "filter": {"kind": "true"}}))
                .unwrap();
        assert_eq!(rule_issues(&unnamed)[0].field, "name");
    }

    #[test]
    fn policy_connection_string_names_the_entity() {
        let cs = policy_connection_string(
            "myns.servicebus.windows.net",
            false,
            "orders",
            "app",
            "a2V5",
        );
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&cs),
            "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=app;SharedAccessKey=a2V5;EntityPath=orders"
        );
        let em = policy_connection_string("localhost", true, "orders", "app", "a2V5");
        assert!(
            secrecy::ExposeSecret::expose_secret(&em).ends_with(";UseDevelopmentEmulator=true")
        );
    }
}
