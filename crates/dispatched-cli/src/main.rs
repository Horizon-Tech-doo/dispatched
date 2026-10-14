//! `dispatched`: Azure Service Bus explorer CLI.

mod commands;
mod context;
mod output;
mod resolve;

use clap::{Parser, Subcommand, ValueEnum};
use commands::browse::ViewArgs;
use commands::profile::ProfileCmd;
use context::Ctx;
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
    /// Output format: table (default) or json; `export` also takes ndjson
    /// (its default is json).
    #[arg(long, value_enum, global = true)]
    format: Option<Format>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Table,
    Json,
    /// One JSON object per line; `export` only.
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
        Err((e, hints)) => {
            eprintln!("error: {}", format_error(&e, hints));
            ExitCode::FAILURE
        }
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
/// gets the other-tenant hint; control characters from remote text are
/// neutralised.
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
    output::sanitize(&text)
}

/// Run the command; on failure also report which hints apply.
async fn run(cli: Cli) -> Result<(), (anyhow::Error, Hints)> {
    let env = |k: &str| std::env::var(k).ok();
    let flags = Selection {
        connection_string: cli.connection_string.as_deref(),
        namespace: cli.namespace.as_deref(),
        profile: cli.profile.as_deref(),
    };
    let hints = Hints {
        namespace_selected: matches!(
            cli.command,
            Command::Ls { .. } | Command::Peek { .. } | Command::Export { .. }
        ) && resolve::select(&flags, &env).namespace.is_some(),
    };
    run_command(cli).await.map_err(|e| (e, hints))
}

async fn run_command(cli: Cli) -> anyhow::Result<()> {
    // Opens nothing: the profile store and keychain are touched only by
    // commands and selections that need them.
    let ctx = Ctx::open();
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
    // NDJSON is a file layout: only `export` writes it.
    let format = cli.format.unwrap_or(Format::Table);
    if matches!(format, Format::Ndjson) && !matches!(cli.command, Command::Export { .. }) {
        anyhow::bail!("--format ndjson is only supported by `export`; use table or json");
    }
    match cli.command {
        Command::Login {
            device_code,
            tenant,
        } => commands::auth::login(&ctx, device_code, &tenant).await,
        Command::Logout => commands::auth::logout(&ctx).await,
        Command::Whoami => commands::auth::whoami(&ctx, format).await,
        Command::Namespaces {
            tenant,
            all_tenants,
        } => commands::auth::namespaces(&ctx, tenant, all_tenants, format).await,
        Command::Ls { topics } => commands::browse::ls(&ctx, &sel, topics, format).await,
        Command::Peek { view, top } => commands::browse::peek(&ctx, &sel, &view, top, format).await,
        Command::Export {
            view,
            top,
            out,
            force,
        } => {
            let fmt = commands::browse::export_format(cli.format)?;
            commands::browse::export(&ctx, &sel, &view, top, &out, fmt, force).await
        }
        Command::Profile { cmd } => {
            commands::profile::run(&ctx, cmd, format, profile_hint.as_deref()).await
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
        let Command::Peek { view, top } = cli.command else {
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
    fn hint_is_not_duplicated() {
        let e = anyhow::anyhow!("not signed in; run `dispatched login`");
        assert_eq!(
            format_error(&e, Hints::default()),
            "not signed in; run `dispatched login`"
        );
    }
}
