//! The CLI's side of the safety gate: `--yes`, `--confirm-prod`, the production pre-check and
//! the gate every mutating command runs through.

// Wired into the mutating commands by Tasks 19-21; until then only the tests use most of it.
#![allow(dead_code)]

use crate::context::Ctx;
use crate::output::sanitize;
use anyhow::Context as _;
use async_trait::async_trait;
use clap::Args;
use dispatched_core::safety::Gate;
use dispatched_core::{
    ActivityEntry, Actor, Audit, EntityPath, Mutation, OpKind, Outcome, Phase, ProfileRef,
};
use horizon_azure_kit::safety::{ConfirmRequest, Decision, Origin, Policy};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use time::OffsetDateTime;

/// Flags every mutating command takes (flattened into each one, not global).
#[derive(Args, Debug, Default, Clone)]
pub struct MutateArgs {
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
    /// Confirm an operation on a production profile by naming its entity (repeat for several entities).
    #[arg(long = "confirm-prod", value_name = "ENTITY")]
    pub confirm_prod: Vec<String>,
    /// Write the per-message result report here (JSON, or NDJSON for .ndjson/.jsonl).
    #[arg(long)]
    pub report: Option<PathBuf>,
}

/// Reads the answer to a confirmation question (the question is passed in).
pub type Prompt = Box<dyn Fn(&str) -> io::Result<String> + Send + Sync>;

/// The confirmation policy of the command line.
pub struct CliPolicy {
    yes: bool,
    confirm_prod: Vec<EntityPath>,
    interactive: bool,
    prompt: Prompt,
    /// The profile name, for the production refusal.
    profile: String,
}

impl CliPolicy {
    /// The policy for `args` on profile `profile`. `interactive` means a prompt can be shown
    /// (stdin and stderr are terminals); `prompt` shows the question and reads the answer.
    pub fn new(
        args: &MutateArgs,
        profile: &str,
        interactive: bool,
        prompt: Prompt,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            yes: args.yes,
            confirm_prod: confirm_prod_entities(args)?,
            interactive,
            prompt,
            profile: profile.to_string(),
        })
    }

    /// The policy for this process: it prompts when stdin and stderr are terminals.
    pub fn for_terminal(args: &MutateArgs, profile: &str) -> anyhow::Result<Self> {
        let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
        Self::new(args, profile, interactive, Box::new(ask_on_terminal))
    }

    #[cfg(test)]
    /// A policy for profile `prod-weu` whose prompt always answers `answer`.
    pub fn for_test(yes: bool, confirm_prod: &[&str], interactive: bool, answer: &str) -> Self {
        let answer = answer.to_string();
        let args = MutateArgs {
            yes,
            confirm_prod: confirm_prod.iter().map(|s| s.to_string()).collect(),
            report: None,
        };
        Self::new(
            &args,
            "prod-weu",
            interactive,
            Box::new(move |_| Ok(answer.clone())),
        )
        .expect("test entities parse")
    }
}

/// Show `question` on stderr and read one line from stdin.
fn ask_on_terminal(question: &str) -> io::Result<String> {
    let mut err = io::stderr().lock();
    err.write_all(question.as_bytes())?;
    err.flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line)
}

/// The `--confirm-prod` values as entities.
fn confirm_prod_entities(args: &MutateArgs) -> anyhow::Result<Vec<EntityPath>> {
    args.confirm_prod
        .iter()
        .map(|s| EntityPath::parse(s).with_context(|| format!("--confirm-prod {s}")))
        .collect()
}

/// Whether some `--confirm-prod` entity is `target` (an entity base), ignoring ASCII case.
fn names(confirm_prod: &[EntityPath], target: &str) -> bool {
    confirm_prod
        .iter()
        .any(|e| e.base().eq_ignore_ascii_case(target))
}

/// The refusal for a production target without the flags.
fn production_refusal(target: &str, profile: &str) -> String {
    format!("{target} is on production profile {profile}: pass --yes --confirm-prod {target}")
}

#[async_trait]
impl Policy for CliPolicy {
    async fn confirm(&self, req: &ConfirmRequest) -> Decision {
        if req.production {
            // No interactive way around it: both flags, naming this entity.
            return if self.yes && names(&self.confirm_prod, &req.target) {
                Decision::ConfirmedTyped(req.target.clone())
            } else {
                Decision::Deny(production_refusal(&req.target, &self.profile))
            };
        }
        if self.yes {
            return Decision::Allow;
        }
        if !self.interactive {
            return Decision::Deny(format!(
                "{}. Pass --yes to confirm without a prompt",
                req.summary
            ));
        }
        let question = format!("{}\nContinue? [y/N] ", sanitize(&req.summary));
        match (self.prompt)(&question) {
            Ok(answer) if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") => {
                Decision::Allow
            }
            Ok(_) => Decision::Deny("not confirmed; nothing was changed".into()),
            Err(e) => Decision::Deny(format!(
                "could not read the confirmation ({e}); nothing was changed"
            )),
        }
    }
}

/// The mutation kind a command verb is logged under (the pre-check has only the verb).
/// Unknown verbs count as deletes, the stricter reading.
fn mutation_for(verb: &str) -> Mutation {
    match verb {
        "send" | "schedule" | "import" => Mutation::Send,
        "complete" | "abandon" | "defer" | "dead-letter" => Mutation::Settle,
        "purge" => Mutation::Purge,
        "set-session-state" | "clear-session-state" => Mutation::SessionState,
        _ => Mutation::Delete,
    }
}

/// The CLI's production check, made **before any I/O** (before connecting, counting for a
/// purge plan or resolving a `--all` selection): on a production profile every entity must be
/// named by a `--confirm-prod` and `--yes` must be given. A refusal is logged as one finished,
/// denied entry and fails with the same text `CliPolicy` gives. `CliPolicy` in the gate stays
/// the authoritative check.
pub fn precheck_production(
    ctx: &Ctx,
    profile: &ProfileRef,
    args: &MutateArgs,
    verb: &str,
    entities: &[EntityPath],
) -> anyhow::Result<()> {
    if !profile.production {
        return Ok(());
    }
    let confirm_prod = confirm_prod_entities(args)?;
    let Some(target) = entities
        .iter()
        .map(EntityPath::base)
        .find(|t| !args.yes || !names(&confirm_prod, t))
    else {
        return Ok(());
    };
    let refusal = production_refusal(&target, &profile.name);
    let entry = ActivityEntry {
        id: 0,
        op_id: format!("op-{}", uuid::Uuid::new_v4().simple()),
        at: OffsetDateTime::now_utc(),
        origin: Origin::Cli,
        profile: profile.name.clone(),
        entity: Some(target.clone()),
        action: verb.to_string(),
        kind: OpKind::Mutating(mutation_for(verb)),
        phase: Phase::Finished,
        outcome: Outcome::Denied,
        count: None,
        succeeded: None,
        failed: None,
        skipped: None,
        summary: format!("{verb} on {target} refused before connecting"),
        error: Some(refusal.clone()),
        note: None,
    };
    if let Err(e) = ctx.activity().and_then(|log| Ok(log.record(&entry)?)) {
        anyhow::bail!("{refusal} (the refusal could not be written to the activity log: {e:#})");
    }
    anyhow::bail!(refusal)
}

/// The gate for a mutating command: [`CliPolicy`] for this terminal, the activity log, origin
/// `cli`.
pub fn cli_gate(ctx: &Ctx, profile: ProfileRef, args: &MutateArgs) -> anyhow::Result<Gate> {
    let policy = CliPolicy::for_terminal(args, &profile.name)?;
    let audit: Arc<dyn Audit> = ctx.activity()?.clone();
    Ok(Gate::new(
        Arc::new(policy),
        audit,
        Actor {
            origin: Origin::Cli,
            profile,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::ActivityQuery;

    fn req(target: &str, production: bool) -> ConfirmRequest {
        ConfirmRequest {
            verb: "purge".into(),
            target: target.into(),
            summary: "Remove 3 messages from orders/$DeadLetterQueue on prod-weu".into(),
            item_count: Some(3),
            reversible: false,
            production,
            origin: Origin::Cli,
        }
    }

    #[test]
    fn mutate_args_flatten_into_a_command() {
        use clap::Parser;
        #[derive(Parser)]
        struct Purge {
            entity: String,
            #[command(flatten)]
            mutate: MutateArgs,
        }
        let p = Purge::try_parse_from([
            "purge",
            "orders",
            "--yes",
            "--confirm-prod",
            "orders",
            "--confirm-prod",
            "events",
            "--report",
            "r.ndjson",
        ])
        .unwrap();
        assert_eq!(p.entity, "orders");
        assert!(p.mutate.yes);
        assert_eq!(p.mutate.confirm_prod, ["orders", "events"]);
        assert_eq!(
            p.mutate.report.as_deref(),
            Some(std::path::Path::new("r.ndjson"))
        );
        let e = CliPolicy::new(
            &MutateArgs {
                confirm_prod: vec!["a//b".into()],
                ..MutateArgs::default()
            },
            "p",
            false,
            Box::new(|_| Ok(String::new())),
        )
        .err()
        .expect("a bad entity is refused")
        .to_string();
        assert!(e.contains("--confirm-prod a//b"), "{e}");
    }

    #[tokio::test]
    async fn production_needs_yes_and_matching_confirm_prod() {
        let p = |yes, prod: &[&str]| CliPolicy::for_test(yes, prod, false, "n");
        assert!(
            matches!(p(true, &[]).confirm(&req("orders", true)).await, Decision::Deny(m) if m.contains("--confirm-prod orders"))
        );
        assert!(
            matches!(
                p(false, &["orders"]).confirm(&req("orders", true)).await,
                Decision::Deny(_)
            ),
            "--yes is also needed"
        );
        assert!(matches!(
            p(true, &["other"]).confirm(&req("orders", true)).await,
            Decision::Deny(_)
        ));
        assert_eq!(
            p(true, &["ORDERS"]).confirm(&req("orders", true)).await,
            Decision::ConfirmedTyped("orders".into())
        );
        assert_eq!(
            p(true, &["events/subscriptions/all"])
                .confirm(&req("events/Subscriptions/all", true))
                .await,
            Decision::ConfirmedTyped("events/Subscriptions/all".into())
        );
    }

    #[tokio::test]
    async fn production_refusal_names_the_target_profile_and_flags() {
        let d = CliPolicy::for_test(false, &[], true, "y")
            .confirm(&req("orders", true))
            .await;
        assert_eq!(
            d,
            Decision::Deny(
                "orders is on production profile prod-weu: pass --yes --confirm-prod orders".into()
            ),
            "no interactive way around it"
        );
    }

    #[tokio::test]
    async fn interactive_prompt_accepts_only_yes() {
        assert_eq!(
            CliPolicy::for_test(false, &[], true, "y")
                .confirm(&req("orders", false))
                .await,
            Decision::Allow
        );
        assert_eq!(
            CliPolicy::for_test(false, &[], true, " YES\n")
                .confirm(&req("orders", false))
                .await,
            Decision::Allow
        );
        assert!(matches!(
            CliPolicy::for_test(false, &[], true, "")
                .confirm(&req("orders", false))
                .await,
            Decision::Deny(_)
        ));
        assert!(matches!(
            CliPolicy::for_test(false, &[], true, "yep")
                .confirm(&req("orders", false))
                .await,
            Decision::Deny(_)
        ));
    }

    #[tokio::test]
    async fn non_tty_without_yes_is_refused() {
        assert!(
            matches!(CliPolicy::for_test(false, &[], false, "y").confirm(&req("orders", false)).await, Decision::Deny(m) if m.contains("--yes"))
        );
        assert_eq!(
            CliPolicy::for_test(true, &[], false, "n")
                .confirm(&req("orders", false))
                .await,
            Decision::Allow,
            "--yes needs no prompt off production"
        );
    }

    #[test]
    fn production_flags_are_checked_before_any_connection() {
        let ctx = Ctx::for_test_with_profiles(&[]);
        let prod = ProfileRef {
            name: "prod-weu".into(),
            production: true,
        };
        let flags = |yes, confirm: &[&str]| MutateArgs {
            yes,
            confirm_prod: confirm.iter().map(|s| s.to_string()).collect(),
            report: None,
        };
        let both = [EntityPath::queue("orders"), EntityPath::queue("events")];
        let e = precheck_production(&ctx, &prod, &flags(false, &[]), "purge", &both[..1])
            .unwrap_err()
            .to_string();
        assert!(e.contains("--confirm-prod orders"), "{e}");
        let e = precheck_production(&ctx, &prod, &flags(true, &["orders"]), "purge", &both)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("--confirm-prod events"),
            "each named entity needs its own flag: {e}"
        );
        precheck_production(
            &ctx,
            &prod,
            &flags(true, &["ORDERS", "events"]),
            "purge",
            &both,
        )
        .unwrap();
        precheck_production(
            &ctx,
            &ProfileRef {
                name: "dev".into(),
                production: false,
            },
            &flags(false, &[]),
            "purge",
            &both,
        )
        .unwrap();
        let log = ctx
            .activity()
            .unwrap()
            .list(&ActivityQuery::default())
            .unwrap();
        assert_eq!(log.len(), 2, "each refusal is logged");
        assert!(
            log.iter()
                .all(|e| e.outcome == Outcome::Denied && e.phase == Phase::Finished)
        );
    }

    #[test]
    fn a_precheck_refusal_entry_says_what_was_refused() {
        let ctx = Ctx::for_test_with_profiles(&[]);
        let prod = ProfileRef {
            name: "prod-weu".into(),
            production: true,
        };
        let e = precheck_production(
            &ctx,
            &prod,
            &MutateArgs::default(),
            "purge",
            &[EntityPath::subscription("events", "all")],
        )
        .unwrap_err()
        .to_string();
        let expected = "events/Subscriptions/all is on production profile prod-weu: \
                        pass --yes --confirm-prod events/Subscriptions/all";
        assert_eq!(e, expected);
        let log = ctx
            .activity()
            .unwrap()
            .list(&ActivityQuery::default())
            .unwrap();
        let [entry] = log.as_slice() else {
            panic!("one entry: {log:?}")
        };
        assert_eq!(entry.action, "purge");
        assert_eq!(entry.origin, Origin::Cli);
        assert_eq!(entry.profile, "prod-weu");
        assert_eq!(entry.entity.as_deref(), Some("events/Subscriptions/all"));
        assert_eq!(
            entry.summary,
            "purge on events/Subscriptions/all refused before connecting"
        );
        assert_eq!(entry.error.as_deref(), Some(expected));
    }

    #[tokio::test]
    async fn the_gate_denies_production_without_flags_and_logs_it() {
        use dispatched_core::{CountEstimate, Operation, SubQueue, Target};
        let ctx = Ctx::for_test_with_profiles(&[]);
        let prod = ProfileRef {
            name: "prod-weu".into(),
            production: true,
        };
        let gate = cli_gate(&ctx, prod, &MutateArgs::default()).unwrap();
        let op = Operation::purge(
            Target::new(EntityPath::queue("orders"), SubQueue::DeadLetter),
            CountEstimate::Exact(3),
        );
        let ran = std::sync::atomic::AtomicBool::new(false);
        let r = gate
            .run(&op, |_| async {
                ran.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert!(r.unwrap_err().to_string().contains("--confirm-prod orders"));
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        let log = ctx
            .activity()
            .unwrap()
            .list(&ActivityQuery::default())
            .unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].outcome, Outcome::Denied);
        assert_eq!(log[0].origin, Origin::Cli);
    }
}
