//! The MCP side of the safety gate: every confirmation is asked of the human through an
//! elicitation form in the agent's client, one form per request, never answered by the server.
//!
//! Off production the form has one checkbox; on production the human types the entity name and
//! the kit's `require` compares it with the target. A client that cannot show forms gets a
//! refusal: nothing is ever allowed silently.

use async_trait::async_trait;
use horizon_azure_kit::safety::{ConfirmRequest, Decision, Policy};
use rmcp::service::{ElicitationError, ElicitationMode};
use rmcp::{Peer, RoleServer};
use serde::Deserialize;

/// The refusal when the client cannot show a form.
pub const NO_FORMS: &str =
    "this MCP client cannot ask you to confirm changes; use the Dispatched app or the CLI";

/// The form for a change off production: one checkbox.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConfirmForm {
    /// Tick to make this change.
    pub confirm: bool,
}

/// The form for a change on production: the entity name, typed.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TypedForm {
    /// The entity name, typed exactly.
    pub name: String,
}

rmcp::elicit_safe!(ConfirmForm, TypedForm);

/// How the human answered one form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<T> {
    /// Submitted, with this content.
    Accepted(T),
    /// Refused explicitly.
    Declined,
    /// Dismissed without a choice.
    Cancelled,
    /// The client declared no form elicitation.
    Unsupported,
    /// The form could not be shown or read (the reason has no secret).
    Failed(String),
}

/// Asks the human through the client. The real one is the call's [`Peer`]; tests answer
/// without a client.
#[async_trait]
pub trait Elicit: Send + Sync {
    /// Whether the client can show forms.
    fn supports_forms(&self) -> bool;
    /// Show a confirmation checkbox.
    async fn confirm(&self, message: String) -> Answer<bool>;
    /// Ask for the entity name.
    async fn typed(&self, message: String) -> Answer<String>;
}

fn answer<T, F>(result: Result<Option<F>, ElicitationError>, take: impl Fn(F) -> T) -> Answer<T> {
    match result {
        Ok(Some(form)) => Answer::Accepted(take(form)),
        Ok(None) | Err(ElicitationError::NoContent) => {
            Answer::Failed("the client returned an empty form".into())
        }
        Err(ElicitationError::UserDeclined) => Answer::Declined,
        Err(ElicitationError::UserCancelled) => Answer::Cancelled,
        Err(ElicitationError::CapabilityNotSupported) => Answer::Unsupported,
        Err(ElicitationError::ParseError { .. }) => {
            Answer::Failed("the client returned a form that does not match the request".into())
        }
        Err(e) => Answer::Failed(e.to_string()),
    }
}

#[async_trait]
impl Elicit for Peer<RoleServer> {
    fn supports_forms(&self) -> bool {
        self.supported_elicitation_modes()
            .contains(&ElicitationMode::Form)
    }

    async fn confirm(&self, message: String) -> Answer<bool> {
        answer(self.elicit::<ConfirmForm>(message).await, |f| f.confirm)
    }

    async fn typed(&self, message: String) -> Answer<String> {
        answer(self.elicit::<TypedForm>(message).await, |f| f.name)
    }
}

/// The confirmation policy for one tool call: it asks through that call's client.
pub struct McpPolicy<E> {
    elicit: E,
}

impl<E: Elicit> McpPolicy<E> {
    pub fn new(elicit: E) -> Self {
        Self { elicit }
    }
}

/// The text a form shows: the core's exact sentence, the count, whether it can be undone and,
/// on production, what to type.
pub fn form_message(req: &ConfirmRequest) -> String {
    let mut out = req.summary.clone();
    if let Some(n) = req.item_count {
        out.push_str(&format!(
            "\n\nItems affected: {}.",
            dispatched_core::count_text(n)
        ));
    } else {
        out.push('\n');
    }
    if !req.reversible {
        out.push_str(" This can't be undone.");
    }
    if req.production {
        out.push_str(&format!(
            "\n\nThis connection is marked as production. Type {} to confirm.",
            req.target
        ));
    }
    out
}

/// The refusal for an answer that is not a yes.
fn refusal<T>(a: &Answer<T>) -> String {
    match a {
        Answer::Accepted(_) => "not confirmed; nothing was changed".into(),
        Answer::Declined => "declined in the MCP client; nothing was changed".into(),
        Answer::Cancelled => "cancelled in the MCP client; nothing was changed".into(),
        Answer::Unsupported => NO_FORMS.into(),
        Answer::Failed(why) => {
            format!("the confirmation could not be asked ({why}); nothing was changed")
        }
    }
}

#[async_trait]
impl<E: Elicit> Policy for McpPolicy<E> {
    async fn confirm(&self, req: &ConfirmRequest) -> Decision {
        if !self.elicit.supports_forms() {
            return Decision::Deny(NO_FORMS.into());
        }
        let message = form_message(req);
        if req.production {
            // The kit's `require` compares the typed text with the target.
            return match self.elicit.typed(message).await {
                Answer::Accepted(name) => Decision::ConfirmedTyped(name),
                other => Decision::Deny(refusal(&other)),
            };
        }
        match self.elicit.confirm(message).await {
            Answer::Accepted(true) => Decision::Allow,
            other => Decision::Deny(refusal(&other)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use horizon_azure_kit::safety::{Origin, require};
    use std::sync::Mutex;

    /// Answers every form from a script and records the messages it was shown.
    pub(crate) struct Scripted {
        pub forms: bool,
        pub confirm: Answer<bool>,
        pub typed: Answer<String>,
        pub shown: Mutex<Vec<String>>,
    }

    impl Scripted {
        pub fn confirming(a: Answer<bool>) -> Self {
            Self {
                forms: true,
                confirm: a,
                typed: Answer::Declined,
                shown: Mutex::new(Vec::new()),
            }
        }

        pub fn typing(a: Answer<String>) -> Self {
            Self {
                forms: true,
                confirm: Answer::Declined,
                typed: a,
                shown: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Elicit for Scripted {
        fn supports_forms(&self) -> bool {
            self.forms
        }
        async fn confirm(&self, message: String) -> Answer<bool> {
            self.shown.lock().unwrap().push(message);
            self.confirm.clone()
        }
        async fn typed(&self, message: String) -> Answer<String> {
            self.shown.lock().unwrap().push(message);
            self.typed.clone()
        }
    }

    fn req(production: bool) -> ConfirmRequest {
        ConfirmRequest {
            verb: "purge".into(),
            target: "orders".into(),
            summary: "Remove 1,284 messages from orders/$DeadLetterQueue on prod-weu".into(),
            item_count: Some(1_284),
            reversible: false,
            production,
            origin: Origin::Mcp,
        }
    }

    async fn decide(e: Scripted, production: bool) -> (Decision, Vec<String>) {
        let p = McpPolicy::new(e);
        let d = p.confirm(&req(production)).await;
        let shown = p.elicit.shown.lock().unwrap().clone();
        (d, shown)
    }

    fn denied(d: &Decision, needle: &str) -> bool {
        matches!(d, Decision::Deny(r) if r.contains(needle))
    }

    #[tokio::test]
    async fn a_ticked_form_allows_and_shows_the_exact_sentence() {
        let (d, shown) = decide(Scripted::confirming(Answer::Accepted(true)), false).await;
        assert_eq!(d, Decision::Allow);
        assert_eq!(shown.len(), 1);
        assert!(
            shown[0].starts_with("Remove 1,284 messages from orders/$DeadLetterQueue on prod-weu"),
            "{}",
            shown[0]
        );
        assert!(shown[0].contains("Items affected: 1,284."), "{}", shown[0]);
        assert!(shown[0].contains("This can't be undone."), "{}", shown[0]);
        assert!(!shown[0].contains("production"), "{}", shown[0]);
    }

    #[tokio::test]
    async fn an_unticked_declined_or_cancelled_form_denies() {
        let (d, _) = decide(Scripted::confirming(Answer::Accepted(false)), false).await;
        assert!(denied(&d, "not confirmed"), "{d:?}");
        let (d, _) = decide(Scripted::confirming(Answer::Declined), false).await;
        assert!(denied(&d, "declined"), "{d:?}");
        let (d, _) = decide(Scripted::confirming(Answer::Cancelled), false).await;
        assert!(denied(&d, "cancelled"), "{d:?}");
        let (d, _) = decide(Scripted::confirming(Answer::Failed("x".into())), false).await;
        assert!(denied(&d, "could not be asked (x)"), "{d:?}");
    }

    #[tokio::test]
    async fn a_client_without_forms_is_refused_without_asking() {
        for production in [false, true] {
            let mut e = Scripted::confirming(Answer::Accepted(true));
            e.forms = false;
            let (d, shown) = decide(e, production).await;
            assert_eq!(d, Decision::Deny(NO_FORMS.into()));
            assert!(shown.is_empty(), "nothing was asked");
        }
        // A client that claimed forms but refused the request is refused the same way.
        let (d, _) = decide(Scripted::confirming(Answer::Unsupported), false).await;
        assert_eq!(d, Decision::Deny(NO_FORMS.into()));
    }

    #[tokio::test]
    async fn production_needs_the_typed_name_and_require_checks_it() {
        let p = McpPolicy::new(Scripted::typing(Answer::Accepted("orders".into())));
        require(&p, &req(true))
            .await
            .expect("the right name passes");
        let shown = p.elicit.shown.lock().unwrap().clone();
        assert!(
            shown[0].ends_with("This connection is marked as production. Type orders to confirm."),
            "{}",
            shown[0]
        );

        let p = McpPolicy::new(Scripted::typing(Answer::Accepted("Orders ".into())));
        let e = require(&p, &req(true)).await.unwrap_err().to_string();
        assert!(e.contains("does not match"), "{e}");

        let p = McpPolicy::new(Scripted::typing(Answer::Declined));
        let e = require(&p, &req(true)).await.unwrap_err().to_string();
        assert!(e.contains("declined"), "{e}");

        let p = McpPolicy::new(Scripted::typing(Answer::Cancelled));
        let e = require(&p, &req(true)).await.unwrap_err().to_string();
        assert!(e.contains("cancelled"), "{e}");

        // A checkbox is never enough on production: the policy asks for the name.
        let p = McpPolicy::new(Scripted {
            forms: true,
            confirm: Answer::Accepted(true),
            typed: Answer::Declined,
            shown: Mutex::new(Vec::new()),
        });
        assert!(require(&p, &req(true)).await.is_err());
    }

    #[test]
    fn a_reversible_request_without_a_count_says_neither() {
        let mut r = req(false);
        r.item_count = None;
        r.reversible = true;
        let m = form_message(&r);
        assert!(
            !m.contains("Items affected") && !m.contains("undone"),
            "{m}"
        );
    }
}
