//! `diagnose_subscription`: gathers what the core's analysis needs (the subscription's
//! description, its topic's status, its rules, its counts and, when it has dead-lettered
//! messages, their reasons) and returns the findings with the facts they rest on.

use super::{Server, ToolResult, reply};
use dispatched_core::diagnose::{SubscriptionFacts, diagnose_subscription};
use dispatched_core::messaging::duration_text;
use dispatched_core::{EntityKind, EntityPath, SubQueue};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiagnoseArgs {
    /// A saved profile to use instead of the server's default (see list_profiles).
    pub profile: Option<String>,
    /// The topic.
    pub topic: String,
    /// The subscription name.
    pub subscription: String,
}

/// Names cannot be empty or hold a path separator.
fn name_arg(value: &str, what: &str) -> Result<String, rmcp::ErrorData> {
    let v = value.trim();
    if v.is_empty() || v.contains('/') {
        return Err(super::invalid(format!(
            "{what} `{value}`: give the bare name, without slashes"
        )));
    }
    Ok(v.to_string())
}

#[tool_router(router = diagnose_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Explain in plain sentences why messages may not arrive at a subscription: \
            the topic's and subscription's status, forwarding (ForwardTo, \
            ForwardDeadLetteredMessagesTo), its rules, active and dead-letter counts and reasons, \
            MaxDeliveryCount and LockDuration, time to live and dead-lettering on expiry, and \
            sessions. Findings are ordered problem, warning, info.",
        annotations(read_only_hint = true)
    )]
    async fn diagnose_subscription(&self, Parameters(a): Parameters<DiagnoseArgs>) -> ToolResult {
        let topic = name_arg(&a.topic, "topic")?;
        let subscription = name_arg(&a.subscription, "subscription")?;
        reply(
            async {
                let t = self.target(a.profile.as_deref())?;
                let ex = &t.explorer;
                let mgmt = ex.management_client();
                // The subscription must exist: its description is the one read that may fail
                // the tool.
                let settings = mgmt.get_subscription(&topic, &subscription).await?.settings;
                // Best effort from here on: what cannot be read is said so in the findings.
                let topic_status = mgmt
                    .get_topic_description(&topic)
                    .await
                    .ok()
                    .and_then(|d| d.settings.status);
                let rules = ex.list_rules(&topic, &subscription).await.ok();
                let path = EntityPath::subscription(&topic, &subscription);
                let counts = ex
                    .entity_details(EntityKind::Subscription, &path.base())
                    .await
                    .ok()
                    .and_then(|d| d.counts);
                let dead_letter_reasons = if counts.as_ref().is_some_and(|c| c.dead_letter > 0) {
                    ex.dlq_summary(&path, SubQueue::DeadLetter)
                        .await
                        .map(|s| s.groups)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let facts = SubscriptionFacts {
                    topic: topic.clone(),
                    subscription: subscription.clone(),
                    topic_status,
                    settings,
                    rules,
                    counts,
                    dead_letter_reasons,
                };
                let findings = diagnose_subscription(&facts);
                let s = &facts.settings;
                Ok(json!({
                    "findings": findings,
                    "topic": facts.topic,
                    "subscription": facts.subscription,
                    "topic_status": facts.topic_status.map(|s| s.as_str()),
                    "status": s.status.map(|s| s.as_str()),
                    "forward_to": s.forward_to,
                    "forward_dead_lettered_messages_to": s.forward_dead_lettered_messages_to,
                    "requires_session": s.requires_session,
                    "lock_duration": s.lock_duration.map(duration_text),
                    "max_delivery_count": s.max_delivery_count,
                    "default_message_time_to_live": s.default_message_time_to_live.map(duration_text),
                    "dead_lettering_on_message_expiration": s.dead_lettering_on_message_expiration,
                    "rules": facts.rules,
                    "counts": facts.counts,
                    "dead_letter_reasons": facts.dead_letter_reasons,
                }))
            }
            .await,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_bare() {
        assert_eq!(name_arg(" events ", "topic").unwrap(), "events");
        assert!(name_arg("", "topic").is_err());
        assert!(name_arg("events/subscriptions/x", "topic").is_err());
    }
}
