//! `dispatched`: Azure Service Bus explorer CLI.

mod commands;
mod context;
mod output;
mod resolve;

use clap::{Parser, Subcommand, ValueEnum};
use commands::profile::ProfileCmd;
use context::Ctx;
use dispatched_core::PeekView;
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
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Table, global = true)]
    format: Format,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Table,
    Json,
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
        #[arg(long)]
        topics: bool,
    },
    /// Peek messages without removing them.
    Peek {
        /// `queue` or `topic/subscriptions/subscription`.
        entity: String,
        /// Read the dead-letter queue.
        #[arg(long, conflicts_with = "transfer_dlq")]
        dlq: bool,
        /// Read the transfer dead-letter queue.
        #[arg(long)]
        transfer_dlq: bool,
        /// Maximum messages.
        #[arg(long, default_value_t = 20)]
        top: u32,
        /// Start at this sequence number.
        #[arg(long, default_value_t = 0)]
        from_seq: i64,
    },
    /// Manage saved profiles.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", format_error(&e));
            ExitCode::FAILURE
        }
    }
}

pub const LOGIN_HINT: &str = "run `dispatched login`";

/// One-line, terminal-safe error text. A not-signed-in root cause gets the
/// login hint appended; control characters from remote text are neutralised.
fn format_error(e: &anyhow::Error) -> String {
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
    output::sanitize(&text)
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let ctx = Ctx::open()?;
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
    match cli.command {
        Command::Login {
            device_code,
            tenant,
        } => commands::auth::login(&ctx, device_code, &tenant).await,
        Command::Logout => commands::auth::logout(&ctx).await,
        Command::Whoami => commands::auth::whoami(&ctx, cli.format).await,
        Command::Namespaces {
            tenant,
            all_tenants,
        } => commands::auth::namespaces(&ctx, tenant, all_tenants, cli.format).await,
        Command::Ls { topics } => commands::browse::ls(&ctx, &sel, topics, cli.format).await,
        Command::Peek {
            entity,
            dlq,
            transfer_dlq,
            top,
            from_seq,
        } => {
            let view = match (dlq, transfer_dlq) {
                (true, _) => PeekView::DeadLetter,
                (_, true) => PeekView::TransferDeadLetter,
                _ => PeekView::Active,
            };
            commands::browse::peek(&ctx, &sel, &entity, view, top, from_seq, cli.format).await
        }
        Command::Profile { cmd } => {
            commands::profile::run(&ctx, cmd, cli.format, profile_hint.as_deref()).await
        }
    }
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
    use dispatched_core::Error as CoreError;
    use horizon_azure_kit::KitError;

    #[test]
    fn core_not_signed_in_gets_the_login_hint() {
        let e = anyhow::Error::from(CoreError::NotSignedIn("a@x.io is not signed in".into()));
        assert_eq!(
            format_error(&e),
            "not signed in: a@x.io is not signed in; run `dispatched login`"
        );
    }

    #[test]
    fn kit_not_signed_in_gets_the_login_hint_even_with_context() {
        let e = anyhow::Error::from(KitError::NotSignedIn("session expired".into()))
            .context("listing namespaces");
        assert!(format_error(&e).ends_with("; run `dispatched login`"));
    }

    #[test]
    fn other_errors_get_no_hint_and_are_sanitised() {
        let e = anyhow::Error::from(CoreError::Protocol("bad\x1b[31m\nthing".into()));
        let text = format_error(&e);
        assert!(!text.contains("login"));
        assert!(!text.contains('\x1b') && !text.contains('\n'));
    }

    #[test]
    fn hint_is_not_duplicated() {
        let e = anyhow::anyhow!("not signed in; run `dispatched login`");
        assert_eq!(format_error(&e), "not signed in; run `dispatched login`");
    }
}
