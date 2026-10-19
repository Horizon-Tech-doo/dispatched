//! `dispatched`: Azure Service Bus explorer CLI.

mod commands;
mod context;
mod jobs;
mod output;
mod resolve;
mod safety;

use clap::{Parser, Subcommand, ValueEnum};
use commands::activity::ActivityCmd;
use commands::browse::ViewArgs;
use commands::profile::ProfileCmd;
use context::Ctx;
use jobs::JobExit;
use resolve::Selection;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "dispatched",
    version,
    about = "Fast, cross-platform Azure Service Bus explorer"
)]
struct Cli {
    /// Service Bus connection string (SAS or emulator). Prefer the
    /// DISPATCHED_CONNECTION_STRING environment variable (used when no
    /// selection flag is given): arguments are visible in shell history and
    /// process lists.
    #[arg(long, global = true, conflicts_with_all = ["namespace", "profile"])]
    connection_string: Option<String>,
    /// Namespace name or host, used with the signed-in Entra account
    /// (environment: DISPATCHED_NAMESPACE).
    #[arg(long, global = true, conflicts_with_all = ["connection_string", "profile"])]
    namespace: Option<String>,
    /// Saved profile to use, see `dispatched profile ls` (environment:
    /// DISPATCHED_PROFILE). Without any selection, the default profile is used.
    /// Precedence: flag, then DISPATCHED_CONNECTION_STRING, DISPATCHED_NAMESPACE,
    /// DISPATCHED_PROFILE, then the default profile.
    #[arg(long, global = true, conflicts_with_all = ["connection_string", "namespace"])]
    profile: Option<String>,
    /// Output format: table (default) or json; `export` and `activity export`
    /// also take ndjson (their default is json).
    #[arg(long, value_enum, global = true)]
    format: Option<Format>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Table,
    Json,
    /// One JSON object per line; `export` and `activity export` only.
    #[value(alias = "jsonl")]
    Ndjson,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in with Microsoft Entra ID (browser by default).
    Login {
        /// Use the device-code flow (for SSH / headless).
        #[arg(long)]
        device_code: bool,
        /// Tenant id or domain.
        #[arg(long, default_value = "organizations")]
        tenant: String,
    },
    /// Sign out and forget the stored refresh token.
    Logout,
    /// Show the current sign-in.
    Whoami,
    /// List Service Bus namespaces visible to the signed-in account.
    Namespaces {
        /// Search this tenant (id or domain) instead of the home tenant.
        #[arg(long, conflicts_with = "all_tenants")]
        tenant: Option<String>,
        /// Search every tenant the account can access.
        #[arg(long)]
        all_tenants: bool,
    },
    /// List queues (and with --topics, topics and subscriptions) with counts.
    Ls {
        /// Also list topics and their subscriptions.
        #[arg(long)]
        topics: bool,
    },
    /// Peek messages without removing them.
    Peek {
        #[command(flatten)]
        view: ViewArgs,
        /// Maximum matching messages.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..))]
        top: u32,
        /// The last N matching messages instead of the first (no continuation). Scans back
        /// from the end of the entity; --top is limited to 5000.
        #[arg(long, conflicts_with = "from_seq")]
        from_end: bool,
    },
    /// Write peeked messages to a file or stdout (JSON or NDJSON).
    Export {
        #[command(flatten)]
        view: ViewArgs,
        /// Maximum matching messages to write.
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..))]
        top: u32,
        /// Output file, or `-` for stdout.
        #[arg(long)]
        out: String,
        /// Replace the output file if it exists.
        #[arg(long)]
        force: bool,
    },
    /// Show an entity's properties and counts.
    Show {
        /// `queue`, `topic` or `topic/subscriptions/subscription`.
        entity: String,
    },
    /// Sessions of a session-enabled queue or subscription.
    Session {
        #[command(subcommand)]
        cmd: SessionCmd,
    },
    /// Rules of a subscription: list, add and remove.
    Rule {
        #[command(subcommand)]
        cmd: RuleCmd,
    },
    /// Create, update, delete, copy, export and import queues, topics and subscriptions.
    Entity {
        #[command(subcommand)]
        cmd: commands::manage::EntityCmd,
    },
    /// SAS policies of a queue or topic.
    Policy {
        #[command(subcommand)]
        cmd: commands::manage::PolicyCmd,
    },
    /// Show messages live as they arrive, until Ctrl-C (--complete or --delete removes them).
    Listen(commands::listen::ListenArgs),
    /// Manage saved profiles.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    /// The log of mutating operations (what was changed, where, by which surface).
    Activity {
        #[command(subcommand)]
        cmd: ActivityCmd,
    },
    /// Send messages to a queue or topic, or schedule them.
    Send(Box<commands::send::SendArgs>),
    /// Scheduled messages.
    Scheduled {
        #[command(subcommand)]
        cmd: commands::send::ScheduledCmd,
    },
    /// Send the messages of an export file to a queue or topic.
    Import(commands::send::ImportArgs),
    /// Receive messages and settle them (--then) or delete them (--delete).
    Receive(commands::receive::ReceiveArgs),
    /// Deferred messages.
    Deferred {
        #[command(subcommand)]
        cmd: commands::receive::DeferredCmd,
    },
    /// Delete messages by sequence number, or all that match (--all with filters).
    Delete(commands::repair::DeleteArgs),
    /// Move messages to another queue or topic (sent first, then removed).
    Move(commands::repair::MoveArgs),
    /// Send edited copies of messages to a queue or topic, optionally removing the originals.
    Resubmit(Box<commands::repair::ResubmitArgs>),
    /// Remove every message of one or more entities (each counted and confirmed on its own).
    Purge(commands::repair::PurgeArgs),
    /// The dead-letter queue: summary, resubmit, move, delete and purge.
    Dlq {
        #[command(subcommand)]
        cmd: commands::repair::DlqCmd,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCmd {
    /// List sessions with message counts.
    Ls { entity: String },
    /// Read, set or clear the state of a session.
    State {
        #[command(subcommand)]
        cmd: commands::state::StateCmd,
    },
}

#[derive(Subcommand, Debug)]
enum RuleCmd {
    /// List the rules of a subscription.
    Ls { subscription: String },
    /// Add a rule (a SQL or correlation filter, with an optional action).
    Add(commands::manage::RuleAddArgs),
    /// Remove a rule.
    Rm(commands::manage::RuleRmArgs),
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    set_sigpipe(mutates(&cli.command));
    match run(cli).await {
        Ok(JobExit::Ok) => ExitCode::SUCCESS,
        // A job ran but did not finish every item (plan decision 17).
        Ok(JobExit::Incomplete) => ExitCode::from(2),
        Err((e, hints)) => {
            eprintln!("error: {}", format_error(&e, hints));
            ExitCode::FAILURE
        }
    }
}

/// Rust ignores SIGPIPE, so `println!` panics when the reader of a pipe exits early
/// (`dispatched peek q | head`). For reads, restore the default so the process ends quietly,
/// like other command-line tools. A mutating command keeps ignoring it: it has already changed
/// the broker, so a closed stdout must not kill it before its `--report` file and activity
/// entry are written. Its output goes through writers that turn the broken pipe into the
/// "result could not be printed" warning and exit 2.
fn set_sigpipe(mutating: bool) {
    #[cfg(unix)]
    // SAFETY: setting a signal disposition is process-wide and thread-safe;
    // this runs once, before anything is written.
    unsafe {
        libc::signal(
            libc::SIGPIPE,
            if mutating {
                libc::SIG_IGN
            } else {
                libc::SIG_DFL
            },
        );
    }
    #[cfg(not(unix))]
    let _ = mutating;
}

/// Whether the command can change the broker (its output must survive a closed pipe).
fn mutates(command: &Command) -> bool {
    match command {
        Command::Send(_)
        | Command::Scheduled { .. }
        | Command::Import(_)
        | Command::Receive(_)
        | Command::Delete(_)
        | Command::Move(_)
        | Command::Resubmit(_)
        | Command::Purge(_) => true,
        Command::Deferred { cmd } => !matches!(cmd, commands::receive::DeferredCmd::Ls(_)),
        Command::Dlq { cmd } => !matches!(cmd, commands::repair::DlqCmd::Summary(_)),
        Command::Rule { cmd } => !matches!(cmd, RuleCmd::Ls { .. }),
        Command::Entity { cmd } => !matches!(cmd, commands::manage::EntityCmd::Export(_)),
        Command::Policy { cmd } => !matches!(
            cmd,
            commands::manage::PolicyCmd::Ls { .. } | commands::manage::PolicyCmd::Keys { .. }
        ),
        Command::Listen(a) => a.complete || a.delete,
        Command::Session {
            cmd: SessionCmd::State { cmd },
        } => !matches!(cmd, commands::state::StateCmd::Get(_)),
        Command::Login { .. }
        | Command::Logout
        | Command::Whoami
        | Command::Namespaces { .. }
        | Command::Ls { .. }
        | Command::Peek { .. }
        | Command::Export { .. }
        | Command::Show { .. }
        | Command::Session { .. }
        | Command::Profile { .. }
        | Command::Activity { .. } => false,
    }
}

pub const LOGIN_HINT: &str = "run `dispatched login`";
pub const TENANT_HINT: &str = "if this namespace is in another tenant, add a profile with \
    `dispatched profile add <name> --namespace <ns> --tenant <tenant-id>`";

/// Extra context for `format_error`.
#[derive(Clone, Copy, Default)]
struct Hints {
    /// The namespace came from `--namespace` / DISPATCHED_NAMESPACE, whose
    /// tokens are always issued by the sign-in's home tenant.
    namespace_selected: bool,
}

/// One-line, terminal-safe error text. A not-signed-in root cause gets the
/// login hint appended; a rejected credential on a `--namespace` selection
/// gets the other-tenant hint; secrets are redacted and control characters
/// from remote text are neutralised.
fn format_error(e: &anyhow::Error, hints: Hints) -> String {
    let mut text = format!("{e:#}");
    let not_signed_in = matches!(
        e.root_cause().downcast_ref::<dispatched_core::Error>(),
        Some(dispatched_core::Error::NotSignedIn(_))
    ) || matches!(
        e.root_cause().downcast_ref::<horizon_azure_kit::KitError>(),
        Some(horizon_azure_kit::KitError::NotSignedIn(_))
    );
    if not_signed_in && !text.contains("dispatched login") {
        text.push_str("; ");
        text.push_str(LOGIN_HINT);
    }
    let token_rejected = matches!(
        e.root_cause().downcast_ref::<dispatched_core::Error>(),
        Some(dispatched_core::Error::Auth(_) | dispatched_core::Error::Forbidden { .. })
    ) || matches!(
        e.root_cause().downcast_ref::<horizon_azure_kit::KitError>(),
        Some(
            horizon_azure_kit::KitError::AuthFailed(_) | horizon_azure_kit::KitError::Forbidden(_)
        )
    );
    if hints.namespace_selected && token_rejected {
        text.push_str("; ");
        text.push_str(TENANT_HINT);
    }
    output::sanitize(&dispatched_core::activity::redact(&text))
}

/// Run the command; on failure also report which hints apply.
async fn run(cli: Cli) -> Result<JobExit, (anyhow::Error, Hints)> {
    let env = |k: &str| std::env::var(k).ok();
    let flags = Selection {
        connection_string: cli.connection_string.as_deref(),
        namespace: cli.namespace.as_deref(),
        profile: cli.profile.as_deref(),
    };
    let hints = Hints {
        namespace_selected: matches!(
            cli.command,
            Command::Ls { .. }
                | Command::Peek { .. }
                | Command::Export { .. }
                | Command::Show { .. }
                | Command::Session { .. }
                | Command::Rule { .. }
                | Command::Send(_)
                | Command::Scheduled { .. }
                | Command::Import(_)
                | Command::Receive(_)
                | Command::Deferred { .. }
                | Command::Delete(_)
                | Command::Move(_)
                | Command::Resubmit(_)
                | Command::Purge(_)
                | Command::Dlq { .. }
                | Command::Entity { .. }
                | Command::Policy { .. }
                | Command::Listen(_)
        ) && resolve::select(&flags, &env).namespace.is_some(),
    };
    // Opens nothing: the profile store, activity log and keychain are touched
    // only by commands and selections that need them.
    let ctx = Ctx::open();
    run_command(&ctx, cli).await.map_err(|e| (e, hints))
}

/// NDJSON is a file layout: only `export` and `activity export` write it.
fn check_format(cli: &Cli) -> anyhow::Result<Format> {
    let format = cli.format.unwrap_or(Format::Table);
    let writes_ndjson = matches!(
        cli.command,
        Command::Export { .. }
            | Command::Activity {
                cmd: ActivityCmd::Export { .. }
            }
    );
    if matches!(format, Format::Ndjson) && !writes_ndjson {
        anyhow::bail!(
            "--format ndjson is only supported by `export` and `activity export`; use table or json"
        );
    }
    Ok(format)
}

#[cfg(test)]
fn check_format_for_test<const N: usize>(args: [&str; N]) -> anyhow::Result<Format> {
    check_format(&Cli::try_parse_from(args)?)
}

#[cfg(test)]
/// Run a command line against `ctx` (in-memory store and activity log).
pub(crate) async fn run_for_test<const N: usize>(
    ctx: &Ctx,
    args: [&str; N],
) -> anyhow::Result<JobExit> {
    run_command(ctx, Cli::try_parse_from(args)?).await
}

async fn run_command(ctx: &Ctx, cli: Cli) -> anyhow::Result<JobExit> {
    let env = |k: &str| std::env::var(k).ok();
    let flags = Selection {
        connection_string: cli.connection_string.as_deref(),
        namespace: cli.namespace.as_deref(),
        profile: cli.profile.as_deref(),
    };
    let owned = resolve::select(&flags, &env);
    let sel = owned.borrow();
    // `profile test` is about a profile whatever else is selected.
    let profile_hint = cli
        .profile
        .clone()
        .or_else(|| env(resolve::ENV_PROFILE).filter(|v| !v.trim().is_empty()));
    let format = check_format(&cli)?;
    // Commands that run jobs return their exit directly; the rest map to `Ok`.
    let done: anyhow::Result<()> = match cli.command {
        Command::Login {
            device_code,
            tenant,
        } => commands::auth::login(ctx, device_code, &tenant).await,
        Command::Logout => commands::auth::logout(ctx).await,
        Command::Whoami => commands::auth::whoami(ctx, format).await,
        Command::Namespaces {
            tenant,
            all_tenants,
        } => commands::auth::namespaces(ctx, tenant, all_tenants, format).await,
        Command::Ls { topics } => commands::browse::ls(ctx, &sel, topics, format).await,
        Command::Peek {
            view,
            top,
            from_end,
        } => commands::browse::peek(ctx, &sel, &view, top, from_end, format).await,
        Command::Show { entity } => commands::inspect::show(ctx, &sel, &entity, format).await,
        Command::Session {
            cmd: SessionCmd::Ls { entity },
        } => commands::inspect::sessions(ctx, &sel, &entity, format).await,
        Command::Session {
            cmd: SessionCmd::State { cmd },
        } => return commands::state::run(ctx, &sel, &cmd, format).await,
        Command::Rule {
            cmd: RuleCmd::Ls { subscription },
        } => commands::inspect::rules(ctx, &sel, &subscription, format).await,
        Command::Rule {
            cmd: RuleCmd::Add(args),
        } => return commands::manage::rule_add(ctx, &sel, &args, format).await,
        Command::Rule {
            cmd: RuleCmd::Rm(args),
        } => return commands::manage::rule_rm(ctx, &sel, &args, format).await,
        Command::Entity { cmd } => return commands::manage::entity(ctx, &sel, &cmd, format).await,
        Command::Policy { cmd } => return commands::manage::policy(ctx, &sel, &cmd, format).await,
        Command::Listen(args) => return commands::listen::listen(ctx, &sel, &args, format).await,
        Command::Export {
            view,
            top,
            out,
            force,
        } => {
            let fmt = commands::browse::export_format(cli.format)?;
            commands::browse::export(ctx, &sel, &view, top, &out, fmt, force).await
        }
        Command::Profile { cmd } => {
            commands::profile::run(ctx, cmd, format, profile_hint.as_deref()).await
        }
        Command::Activity { cmd } => {
            commands::activity::run(ctx, cmd, cli.format, cli.profile.as_deref())
        }
        Command::Send(args) => return commands::send::send(ctx, &sel, &args, format).await,
        Command::Scheduled { cmd } => {
            return commands::send::scheduled(ctx, &sel, &cmd, format).await;
        }
        Command::Import(args) => return commands::send::import(ctx, &sel, &args, format).await,
        Command::Receive(args) => {
            return commands::receive::receive(ctx, &sel, &args, format).await;
        }
        Command::Deferred { cmd } => {
            return commands::receive::deferred(ctx, &sel, &cmd, format).await;
        }
        Command::Delete(args) => return commands::repair::delete(ctx, &sel, &args, format).await,
        Command::Move(args) => {
            return commands::repair::move_messages(ctx, &sel, &args, format).await;
        }
        Command::Resubmit(args) => {
            return commands::repair::resubmit(ctx, &sel, &args, format).await;
        }
        Command::Purge(args) => return commands::repair::purge(ctx, &sel, &args, format).await,
        Command::Dlq { cmd } => return commands::repair::dlq(ctx, &sel, &cmd, format).await,
    };
    done.map(|()| JobExit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn mutating_commands_survive_a_closed_pipe_and_reads_do_not() {
        let mutates_line = |args: &[&str]| {
            let cli =
                Cli::try_parse_from(std::iter::once("dispatched").chain(args.iter().copied()))
                    .unwrap();
            mutates(&cli.command)
        };
        for args in [
            &["receive", "q", "--delete"][..],
            &["send", "q", "--body", "x"],
            &["purge", "q"],
            &["delete", "q", "--seq", "1"],
            &["move", "q", "--seq", "1", "--to", "r"],
            &[
                "deferred", "receive", "q", "--seq", "1", "--then", "complete",
            ],
            &["dlq", "purge", "q"],
            &["scheduled", "cancel", "q", "--all"],
            &["session", "state", "clear", "q", "s"],
            &["entity", "delete", "q"],
            &["entity", "create", "queue", "q"],
            &["entity", "import", "e.xml"],
            &["rule", "add", "t/subscriptions/s", "r", "--sql", "1=1"],
            &["rule", "rm", "t/subscriptions/s", "r"],
            &["policy", "renew", "q", "p", "primary"],
            &["listen", "q", "--delete"],
            &["listen", "q", "--complete"],
        ] {
            assert!(mutates_line(args), "{args:?}");
        }
        for args in [
            &["peek", "q"][..],
            &["ls"],
            &["deferred", "ls", "q"],
            &["dlq", "summary", "q"],
            &["session", "state", "get", "q", "s"],
            &["session", "ls", "q"],
            &["activity", "ls"],
            &["entity", "export", "--out", "e.xml"],
            &["policy", "ls", "q"],
            &["policy", "keys", "q", "p"],
            &["rule", "ls", "t/subscriptions/s"],
            &["listen", "q"],
        ] {
            assert!(!mutates_line(args), "{args:?}");
        }
    }

    #[test]
    fn profile_add_namespace_is_its_own_option() {
        let cli = Cli::try_parse_from([
            "dispatched",
            "profile",
            "add",
            "p",
            "--namespace",
            "ns",
            "--tenant",
            "t",
        ])
        .unwrap();
        let Command::Profile {
            cmd: ProfileCmd::Add {
                namespace, tenant, ..
            },
        } = cli.command
        else {
            panic!("not profile add")
        };
        assert_eq!(namespace.as_deref(), Some("ns"));
        assert_eq!(tenant.as_deref(), Some("t"));
        assert!(
            Cli::try_parse_from([
                "dispatched",
                "profile",
                "add",
                "p",
                "--namespace",
                "ns",
                "--connection-string-stdin"
            ])
            .is_err()
        );
    }

    #[test]
    fn explicit_selection_flags_conflict() {
        let e = Cli::try_parse_from([
            "dispatched",
            "--profile",
            "p",
            "--connection-string",
            "c",
            "ls",
        ]);
        assert!(e.is_err());
        let cli = Cli::try_parse_from(["dispatched", "--profile", "p", "ls"]).unwrap();
        assert_eq!(cli.profile.as_deref(), Some("p"));
        assert!(cli.connection_string.is_none());
    }
    #[test]
    fn peek_and_export_flags_parse() {
        let cli = Cli::try_parse_from([
            "dispatched",
            "peek",
            "q",
            "--view",
            "deferred",
            "--top",
            "5",
            "--body-contains",
            "x",
        ])
        .unwrap();
        let Command::Peek { view, top, .. } = cli.command else {
            panic!("not peek")
        };
        assert_eq!(view.entity.as_deref(), Some("q"));
        assert_eq!(view.view.as_deref(), Some("deferred"));
        assert_eq!(top, 5);

        let cli = Cli::try_parse_from([
            "dispatched",
            "export",
            "--out",
            "-",
            "--format",
            "ndjson",
            "--dlq",
        ])
        .unwrap();
        assert!(matches!(cli.format, Some(Format::Ndjson)));
        let Command::Export {
            view,
            top,
            out,
            force,
        } = cli.command
        else {
            panic!("not export")
        };
        assert!(view.entity.is_none() && view.dlq);
        assert_eq!((top, out.as_str(), force), (1000, "-", false));
        // `jsonl` is an alias; --out is required; --top 0 is refused.
        let cli = Cli::try_parse_from(["dispatched", "--format", "jsonl", "export", "--out", "x"])
            .unwrap();
        assert!(matches!(cli.format, Some(Format::Ndjson)));
        assert!(Cli::try_parse_from(["dispatched", "export"]).is_err());
        assert!(Cli::try_parse_from(["dispatched", "peek", "--top", "0"]).is_err());
        assert!(Cli::try_parse_from(["dispatched", "peek", "--dlq", "--view", "active"]).is_err());
    }

    #[test]
    fn peek_from_end_conflicts_with_from_seq_and_session_parses() {
        let cli = Cli::try_parse_from([
            "dispatched",
            "peek",
            "q",
            "--from-end",
            "--session",
            "RMA-1",
            "--top",
            "5",
        ])
        .unwrap();
        let Command::Peek { view, from_end, .. } = cli.command else {
            panic!("not peek")
        };
        assert!(from_end);
        assert_eq!(view.session.as_deref(), Some("RMA-1"));
        assert!(
            Cli::try_parse_from(["dispatched", "peek", "q", "--from-end", "--from-seq", "9"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["dispatched", "session", "ls", "q"]).is_ok());
        assert!(Cli::try_parse_from(["dispatched", "rule", "ls", "t/subscriptions/s"]).is_ok());
        assert!(Cli::try_parse_from(["dispatched", "show", "q"]).is_ok());
        // --session also works for export.
        assert!(
            Cli::try_parse_from(["dispatched", "export", "q", "--session", "s", "--out", "-"])
                .is_ok()
        );
    }

    #[test]
    fn ndjson_is_allowed_for_activity_export() {
        assert!(
            check_format_for_test([
                "dispatched",
                "--format",
                "ndjson",
                "activity",
                "export",
                "--out",
                "a.ndjson"
            ])
            .is_ok()
        );
        assert!(
            check_format_for_test(["dispatched", "--format", "ndjson", "activity", "ls"]).is_err()
        );
        assert!(
            check_format_for_test(["dispatched", "--format", "ndjson", "export", "--out", "-"])
                .is_ok()
        );
        assert!(check_format_for_test(["dispatched", "--format", "ndjson", "ls"]).is_err());
        assert!(matches!(
            check_format_for_test(["dispatched", "--format", "json", "ls"]),
            Ok(Format::Json)
        ));
    }

    #[test]
    fn activity_commands_parse() {
        assert!(
            Cli::try_parse_from([
                "dispatched",
                "activity",
                "ls",
                "--origin",
                "gui",
                "--limit",
                "5"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(["dispatched", "activity", "ls", "--origin", "robot"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["dispatched", "activity", "export"]).is_err(),
            "--out is required"
        );
        // The global --profile filters `activity ls`.
        let cli =
            Cli::try_parse_from(["dispatched", "activity", "ls", "--profile", "prod-weu"]).unwrap();
        assert_eq!(cli.profile.as_deref(), Some("prod-weu"));
        assert!(Cli::try_parse_from(["dispatched", "activity", "ls", "--limit", "0"]).is_err());
    }

    #[tokio::test]
    async fn activity_ls_and_export_read_the_log() {
        use dispatched_core::{ActivityEntry, Audit, Mutation, OpKind, Outcome, Phase};
        use horizon_azure_kit::safety::Origin;
        let ctx = Ctx::for_test_with_profiles(&[]);
        let entry = |origin, profile: &str| ActivityEntry {
            id: 0,
            op_id: "op-1".into(),
            at: time::OffsetDateTime::now_utc(),
            origin,
            profile: profile.into(),
            entity: Some("orders".into()),
            action: "purge".into(),
            kind: OpKind::Mutating(Mutation::Purge),
            phase: Phase::Finished,
            outcome: Outcome::Succeeded,
            count: Some(3),
            succeeded: Some(3),
            failed: Some(0),
            skipped: Some(0),
            summary: "Remove 3 messages from orders on dev".into(),
            error: None,
            note: None,
        };
        let log = ctx.activity().unwrap();
        log.record(&entry(Origin::Cli, "dev")).unwrap();
        log.record(&entry(Origin::Gui, "prod-weu")).unwrap();
        assert_eq!(
            run_for_test(&ctx, ["dispatched", "activity", "ls"])
                .await
                .unwrap(),
            JobExit::Ok
        );
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.ndjson");
        let out_s = out.to_str().unwrap();
        run_for_test(
            &ctx,
            [
                "dispatched",
                "--format",
                "ndjson",
                "activity",
                "export",
                "--out",
                out_s,
            ],
        )
        .await
        .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text.lines().count(), 2);
        let e = run_for_test(&ctx, ["dispatched", "activity", "export", "--out", out_s])
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("--force"), "{e}");
        run_for_test(
            &ctx,
            [
                "dispatched",
                "activity",
                "export",
                "--out",
                out_s,
                "--force",
            ],
        )
        .await
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2, "JSON by default");
    }

    use dispatched_core::Error as CoreError;
    use horizon_azure_kit::KitError;

    #[test]
    fn core_not_signed_in_gets_the_login_hint() {
        let e = anyhow::Error::from(CoreError::NotSignedIn("a@x.io is not signed in".into()));
        assert_eq!(
            format_error(&e, Hints::default()),
            "not signed in: a@x.io is not signed in; run `dispatched login`"
        );
    }

    #[test]
    fn kit_not_signed_in_gets_the_login_hint_even_with_context() {
        let e = anyhow::Error::from(KitError::NotSignedIn("session expired".into()))
            .context("listing namespaces");
        assert!(format_error(&e, Hints::default()).ends_with("; run `dispatched login`"));
    }

    #[test]
    fn other_errors_get_no_hint_and_are_sanitised() {
        let e = anyhow::Error::from(CoreError::Protocol("bad\x1b[31m\nthing".into()));
        let text = format_error(&e, Hints::default());
        assert!(!text.contains("login"));
        assert!(!text.contains('\x1b') && !text.contains('\n'));
    }

    #[test]
    fn rejected_token_on_a_namespace_selection_hints_at_the_tenant() {
        let on = Hints {
            namespace_selected: true,
        };
        for e in [
            anyhow::Error::from(CoreError::Auth("401".into())),
            anyhow::Error::from(CoreError::Forbidden {
                needed: None,
                detail: "d".into(),
            }),
            anyhow::Error::from(KitError::AuthFailed("AADSTS".into())).context("sending"),
        ] {
            let text = format_error(&e, on);
            assert!(
                text.ends_with(TENANT_HINT) && text.contains("--namespace <ns> --tenant"),
                "{text}"
            );
            // Not a `--namespace` selection: no hint.
            assert!(!format_error(&e, Hints::default()).contains("tenant"));
        }
        // Other failures on a namespace selection get none.
        let e = anyhow::Error::from(CoreError::Network("down".into()));
        assert!(!format_error(&e, on).contains("tenant"));
    }

    #[test]
    fn error_text_is_redacted() {
        let e = anyhow::anyhow!(
            "broker said Endpoint=sb://x.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=topsecretvalue"
        );
        let text = format_error(&e, Hints::default());
        assert!(!text.contains("topsecretvalue"), "{text}");
    }

    #[test]
    fn hint_is_not_duplicated() {
        let e = anyhow::anyhow!("not signed in; run `dispatched login`");
        assert_eq!(
            format_error(&e, Hints::default()),
            "not signed in; run `dispatched login`"
        );
    }
}
