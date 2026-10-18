//! The CLI's side of the safety gate: `--yes`, `--confirm-prod`, the production pre-check and
//! the gate every mutating command runs through.

use crate::context::Ctx;
use crate::output::sanitize;
use crate::resolve::Resolved;
use anyhow::Context as _;
use async_trait::async_trait;
use clap::Args;
use dispatched_core::safety::Gate;
use dispatched_core::{
    ActivityEntry, Actor, Audit, EntityPath, Mutation, OpKind, Outcome, Phase, ProfileRef,
};
use horizon_azure_kit::safety::{CancellationToken, ConfirmRequest, Decision, Origin, Policy};
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
    /// Replace the --report file if it exists.
    #[arg(long, requires = "report")]
    pub force_report: bool,
}

/// What a mutating command will do, known before any I/O: the same kind and verb as the
/// [`dispatched_core::Operation`] it builds later (`Operation.mutation`, `Operation::verb()`).
/// Each command declares its own as a constant, e.g. `Planned { kind: Mutation::Purge, verb:
/// "purge" }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Planned {
    /// The mutation kind the activity log records.
    pub kind: Mutation,
    /// The short verb (the activity entry's action).
    pub verb: &'static str,
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
            ..MutateArgs::default()
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

/// Show `question` on stderr and read one line from stdin. [`cli_gate`] arms Ctrl-C
/// ([`crate::jobs::arm_ctrl_c`]) only after the gate confirmed, so a Ctrl-C at this prompt
/// ends the process (the default signal handling) instead of only cancelling a token.
fn ask_on_terminal(question: &str) -> io::Result<String> {
    let mut err = io::stderr().lock();
    err.write_all(question.as_bytes())?;
    err.flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line)
}

/// The `--confirm-prod` values as entities. A sub-queue path (`orders/$DeadLetterQueue`) is
/// refused with the entity to name instead: confirmations name the entity.
fn confirm_prod_entities(args: &MutateArgs) -> anyhow::Result<Vec<EntityPath>> {
    args.confirm_prod
        .iter()
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            for suffix in ["/$transfer/$deadletterqueue", "/$deadletterqueue"] {
                if let Some(base) = lower
                    .trim_end_matches('/')
                    .strip_suffix(suffix)
                    .map(|b| &s[..b.len()])
                {
                    anyhow::bail!(
                        "--confirm-prod {s}: name the entity without the sub-queue: --confirm-prod {base}"
                    );
                }
            }
            EntityPath::parse(s).with_context(|| format!("--confirm-prod {s}"))
        })
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

/// The refusal when a production operation names no entity yet.
fn production_refusal_without_entity(profile: &str) -> String {
    format!("this operation is on production profile {profile}: pass --yes --confirm-prod <entity>")
}

/// Refuse an existing `--report` file (unless `--force-report`), a directory, or a path whose
/// directory does not exist, before anything runs: a job that ran must never fail on its
/// report path.
pub fn check_report_path(args: &MutateArgs) -> anyhow::Result<()> {
    let Some(path) = &args.report else {
        return Ok(());
    };
    let shown = sanitize(&path.display().to_string());
    if path.is_dir() {
        anyhow::bail!("--report {shown} is a directory; choose a file (nothing was changed)");
    }
    if path.exists() && !args.force_report && !dispatched_core::export::is_special(path) {
        anyhow::bail!(
            "{shown} exists; pass --force-report to replace it, or choose another --report \
             (nothing was changed)"
        );
    }
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    if !dir.is_dir() {
        anyhow::bail!(
            "the directory of --report {shown} does not exist; create it or choose another \
             --report (nothing was changed)"
        );
    }
    Ok(())
}

/// Everything a mutating command checks right after `resolve`, before any I/O: the report
/// path, then the production flags.
pub fn preflight(
    ctx: &Ctx,
    resolved: &Resolved,
    args: &MutateArgs,
    planned: Planned,
    entities: &[EntityPath],
) -> anyhow::Result<()> {
    check_report_path(args)?;
    precheck_production(ctx, &resolved.profile, args, planned, entities)
}

/// The CLI's production check, made **before any I/O** (before connecting, counting for a
/// purge plan or resolving a `--all` selection): on a production profile `--yes` must be given
/// and every entity must be named by a `--confirm-prod`; an operation that names no entity is
/// refused. A refusal is logged as one finished, denied entry and fails with the same text
/// `CliPolicy` gives. `CliPolicy` in the gate stays the authoritative check.
pub fn precheck_production(
    ctx: &Ctx,
    profile: &ProfileRef,
    args: &MutateArgs,
    planned: Planned,
    entities: &[EntityPath],
) -> anyhow::Result<()> {
    if !profile.production {
        return Ok(());
    }
    let confirm_prod = confirm_prod_entities(args)?;
    let (target, refusal) = if entities.is_empty() {
        (None, production_refusal_without_entity(&profile.name))
    } else {
        let Some(target) = entities
            .iter()
            .map(EntityPath::base)
            .find(|t| !args.yes || !names(&confirm_prod, t))
        else {
            return Ok(());
        };
        let refusal = production_refusal(&target, &profile.name);
        (Some(target), refusal)
    };
    let verb = planned.verb;
    let entry = ActivityEntry {
        id: 0,
        op_id: format!("op-{}", uuid::Uuid::new_v4().simple()),
        at: OffsetDateTime::now_utc(),
        origin: Origin::Cli,
        profile: profile.name.clone(),
        entity: target.clone(),
        action: verb.to_string(),
        kind: OpKind::Mutating(planned.kind),
        phase: Phase::Finished,
        outcome: Outcome::Denied,
        count: None,
        succeeded: None,
        failed: None,
        skipped: None,
        summary: match &target {
            Some(t) => format!("{verb} on {t} refused before connecting"),
            None => format!("{verb} refused before connecting"),
        },
        error: Some(refusal.clone()),
        note: None,
    };
    if let Err(e) = ctx.activity().and_then(|log| Ok(log.record(&entry)?)) {
        anyhow::bail!("{refusal} (the refusal could not be written to the activity log: {e:#})");
    }
    anyhow::bail!(refusal)
}

/// A policy that runs `on_confirm` once, the first time `inner` lets an operation proceed.
struct ArmOnConfirm<P> {
    inner: P,
    on_confirm: Box<dyn Fn() + Send + Sync>,
    armed: std::sync::Once,
}

#[async_trait]
impl<P: Policy> Policy for ArmOnConfirm<P> {
    async fn confirm(&self, req: &ConfirmRequest) -> Decision {
        let decision = self.inner.confirm(req).await;
        if !matches!(decision, Decision::Deny(_)) {
            self.armed.call_once(|| (self.on_confirm)());
        }
        decision
    }
}

/// The gate for a mutating command: [`CliPolicy`] for this terminal, the activity log, origin
/// `cli`. The profile comes from [`Resolved`] only, so production cannot be set by hand.
///
/// Also returns the job's cancel token. Ctrl-C cancels it only from the first confirmation
/// on ([`crate::jobs::arm_ctrl_c`]): a Ctrl-C at the prompt still ends the process.
pub fn cli_gate(
    ctx: &Ctx,
    resolved: &Resolved,
    args: &MutateArgs,
) -> anyhow::Result<(Gate, CancellationToken)> {
    cli_gate_with(ctx, resolved, args, crate::jobs::arm_ctrl_c)
}

/// [`cli_gate`] with the function that arms the returned token on the first confirmation.
fn cli_gate_with(
    ctx: &Ctx,
    resolved: &Resolved,
    args: &MutateArgs,
    arm: impl Fn(&CancellationToken) + Send + Sync + 'static,
) -> anyhow::Result<(Gate, CancellationToken)> {
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let policy = ArmOnConfirm {
        inner: CliPolicy::for_terminal(args, &resolved.profile.name)?,
        on_confirm: Box::new(move || arm(&token)),
        armed: std::sync::Once::new(),
    };
    Ok((gate_with(ctx, resolved, Arc::new(policy))?, cancel))
}

/// A gate over `policy`, the activity log and origin `cli`.
fn gate_with(ctx: &Ctx, resolved: &Resolved, policy: Arc<dyn Policy>) -> anyhow::Result<Gate> {
    let audit: Arc<dyn Audit> = ctx.activity()?.clone();
    Ok(Gate::new(
        policy,
        audit,
        Actor {
            origin: Origin::Cli,
            profile: resolved.profile.clone(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::ActivityQuery;

    const PURGE: Planned = Planned {
        kind: Mutation::Purge,
        verb: "purge",
    };

    /// A resolved production target (a connection string to a host saved as production).
    fn prod_resolved(ctx: &Ctx) -> Resolved {
        crate::resolve::resolve_for_test(
            ctx,
            crate::resolve::TestSelection::ConnectionString(
                "Endpoint=sb://prod.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=x"
                    .into(),
            ),
        )
        .unwrap()
    }

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
        assert!(!p.mutate.force_report);
        assert!(
            Purge::try_parse_from(["purge", "orders", "--force-report"]).is_err(),
            "--force-report needs --report"
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
            ..MutateArgs::default()
        };
        let both = [EntityPath::queue("orders"), EntityPath::queue("events")];
        let e = precheck_production(&ctx, &prod, &flags(false, &[]), PURGE, &both[..1])
            .unwrap_err()
            .to_string();
        assert!(e.contains("--confirm-prod orders"), "{e}");
        let e = precheck_production(&ctx, &prod, &flags(true, &["orders"]), PURGE, &both)
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
            PURGE,
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
            PURGE,
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
            PURGE,
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
        let ctx = Ctx::for_test_with_profiles(&[("prod-weu", "prod.servicebus.windows.net", true)]);
        let resolved = prod_resolved(&ctx);
        assert!(resolved.profile.production);
        let (gate, _cancel) = cli_gate(&ctx, &resolved, &MutateArgs::default()).unwrap();
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

    #[test]
    fn production_with_no_entities_is_refused_and_logged() {
        let ctx = Ctx::for_test_with_profiles(&[]);
        let prod = ProfileRef {
            name: "prod-weu".into(),
            production: true,
        };
        let args = MutateArgs {
            yes: true,
            confirm_prod: vec!["orders".into()],
            ..MutateArgs::default()
        };
        let e = precheck_production(&ctx, &prod, &args, PURGE, &[])
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "this operation is on production profile prod-weu: pass --yes --confirm-prod <entity>"
        );
        let log = ctx
            .activity()
            .unwrap()
            .list(&ActivityQuery::default())
            .unwrap();
        let [entry] = log.as_slice() else {
            panic!("one entry: {log:?}")
        };
        assert_eq!(entry.outcome, Outcome::Denied);
        assert_eq!(entry.entity, None);
        assert_eq!(entry.summary, "purge refused before connecting");
        // Off production an empty list is the command's business.
        precheck_production(
            &ctx,
            &ProfileRef {
                name: "dev".into(),
                production: false,
            },
            &args,
            PURGE,
            &[],
        )
        .unwrap();
    }

    #[test]
    fn the_precheck_logs_the_planned_kind() {
        let ctx = Ctx::for_test_with_profiles(&[]);
        let prod = ProfileRef {
            name: "prod-weu".into(),
            production: true,
        };
        let resubmit = Planned {
            kind: Mutation::Send,
            verb: "resubmit",
        };
        precheck_production(
            &ctx,
            &prod,
            &MutateArgs::default(),
            resubmit,
            &[EntityPath::queue("orders")],
        )
        .unwrap_err();
        let log = ctx
            .activity()
            .unwrap()
            .list(&ActivityQuery::default())
            .unwrap();
        assert_eq!(log[0].kind, OpKind::Mutating(Mutation::Send));
        assert_eq!(log[0].action, "resubmit");
    }

    #[test]
    fn an_existing_report_file_is_refused_before_anything_runs() {
        let ctx = Ctx::for_test_with_profiles(&[("prod-weu", "prod.servicebus.windows.net", true)]);
        let resolved = prod_resolved(&ctx);
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("r.json");
        std::fs::write(&existing, "{}").unwrap();
        // All production flags given: only the report path stops it.
        let args = |report: std::path::PathBuf, force_report| MutateArgs {
            yes: true,
            confirm_prod: vec!["orders".into()],
            report: Some(report),
            force_report,
        };
        let orders = [EntityPath::queue("orders")];
        let e = preflight(
            &ctx,
            &resolved,
            &args(existing.clone(), false),
            PURGE,
            &orders,
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("--force-report") && e.contains("nothing was changed"),
            "{e}"
        );
        preflight(
            &ctx,
            &resolved,
            &args(existing.clone(), true),
            PURGE,
            &orders,
        )
        .unwrap();
        let e = preflight(
            &ctx,
            &resolved,
            &args(dir.path().into(), true),
            PURGE,
            &orders,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("is a directory"), "{e}");
        let e = preflight(
            &ctx,
            &resolved,
            &args(dir.path().join("missing/r.json"), false),
            PURGE,
            &orders,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("does not exist"), "{e}");
        preflight(
            &ctx,
            &resolved,
            &args(dir.path().join("new.json"), false),
            PURGE,
            &orders,
        )
        .unwrap();
        assert!(
            ctx.activity()
                .unwrap()
                .list(&ActivityQuery::default())
                .unwrap()
                .is_empty(),
            "report path refusals happen before anything is logged or run"
        );
    }

    #[test]
    fn confirm_prod_must_name_the_entity_not_a_sub_queue() {
        for (input, base) in [
            ("orders/$DeadLetterQueue", "orders"),
            (
                "events/subscriptions/all/$deadletterqueue",
                "events/subscriptions/all",
            ),
            ("orders/$Transfer/$DeadLetterQueue", "orders"),
        ] {
            let e = CliPolicy::new(
                &MutateArgs {
                    confirm_prod: vec![input.into()],
                    ..MutateArgs::default()
                },
                "p",
                false,
                Box::new(|_| Ok(String::new())),
            )
            .err()
            .expect("refused")
            .to_string();
            assert!(
                e.ends_with(&format!(
                    "name the entity without the sub-queue: --confirm-prod {base}"
                )),
                "{e}"
            );
        }
    }

    #[tokio::test]
    async fn ctrl_c_is_armed_only_once_the_gate_confirmed() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let armed = Arc::new(AtomicUsize::new(0));
        let policy = |answer: &str| {
            let seen = armed.clone();
            ArmOnConfirm {
                inner: CliPolicy::for_test(false, &[], true, answer),
                on_confirm: Box::new(move || {
                    seen.fetch_add(1, Ordering::SeqCst);
                }),
                armed: std::sync::Once::new(),
            }
        };
        let denied = policy("n");
        assert!(matches!(
            denied.confirm(&req("orders", false)).await,
            Decision::Deny(_)
        ));
        assert_eq!(armed.load(Ordering::SeqCst), 0, "a denial arms nothing");
        let allowed = policy("y");
        assert_eq!(
            allowed.confirm(&req("orders", false)).await,
            Decision::Allow
        );
        assert_eq!(
            allowed.confirm(&req("events", false)).await,
            Decision::Allow
        );
        assert_eq!(
            armed.load(Ordering::SeqCst),
            1,
            "armed once, on the first confirmation"
        );
    }

    #[tokio::test]
    async fn cli_gate_arms_the_token_it_returns_once_a_run_is_confirmed() {
        use dispatched_core::{Operation, SubQueue, Target};
        let ctx = Ctx::for_test_with_profiles(&[]);
        let dev = crate::resolve::resolve_for_test(
            &ctx,
            crate::resolve::TestSelection::ConnectionString(
                "Endpoint=sb://dev.servicebus.invalid/;SharedAccessKeyName=k;SharedAccessKey=x"
                    .into(),
            ),
        )
        .unwrap();
        let args = MutateArgs {
            yes: true,
            ..MutateArgs::default()
        };
        // The test's "arm" cancels the token it is given, so the returned token shows it.
        let (gate, cancel) = cli_gate_with(&ctx, &dev, &args, |t| t.cancel()).unwrap();
        assert!(
            !cancel.is_cancelled(),
            "nothing armed before a confirmation"
        );
        let op = Operation::send(
            Target::new(EntityPath::queue("q"), SubQueue::Active),
            1,
            None,
        );
        gate.run(&op, |_| async { Ok(()) }).await.unwrap();
        assert!(cancel.is_cancelled(), "the returned token was armed");
    }

    #[tokio::test]
    async fn the_prompt_shows_the_summary_and_the_question() {
        let asked = Arc::new(std::sync::Mutex::new(String::new()));
        let seen = asked.clone();
        let policy = CliPolicy::new(
            &MutateArgs::default(),
            "dev",
            true,
            Box::new(move |q| {
                *seen.lock().unwrap() = q.to_string();
                Ok("y".into())
            }),
        )
        .unwrap();
        assert_eq!(policy.confirm(&req("orders", false)).await, Decision::Allow);
        assert_eq!(
            *asked.lock().unwrap(),
            "Remove 3 messages from orders/$DeadLetterQueue on prod-weu\nContinue? [y/N] "
        );
    }
}
