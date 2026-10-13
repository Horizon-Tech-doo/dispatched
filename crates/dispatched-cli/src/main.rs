//! `dispatched`: Azure Service Bus explorer CLI.

mod output;
mod resolve;
mod state;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use dispatched_core::discovery::discover_namespaces;
use dispatched_core::{EntityPath, Explorer, MessageBody, PeekedMessage, SubQueue};
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::arm::ArmClient;
use output::{ellipsize, table};
use std::process::ExitCode;
use time::format_description::well_known::Rfc3339;

#[derive(Parser)]
#[command(
    name = "dispatched",
    version,
    about = "Fast, cross-platform Azure Service Bus explorer"
)]
struct Cli {
    /// Service Bus connection string (SAS or emulator).
    #[arg(
        long,
        env = "DISPATCHED_CONNECTION_STRING",
        global = true,
        hide_env_values = true
    )]
    connection_string: Option<String>,
    /// Namespace name or host, used with the signed-in Entra account.
    #[arg(long, env = "DISPATCHED_NAMESPACE", global = true)]
    namespace: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Table, global = true)]
    format: Format,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
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
    Namespaces,
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
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn count_cell(n: Option<u64>) -> String {
    n.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
}

fn body_cell(body: &MessageBody) -> String {
    match body.as_text() {
        Some(text) => ellipsize(&text, 60),
        None => format!("<{} bytes>", body.len()),
    }
}

fn print_messages(messages: &[PeekedMessage], format: Format, dlq: bool) -> anyhow::Result<()> {
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(messages)?);
        return Ok(());
    }
    let mut headers = vec!["SEQ", "ENQUEUED", "MESSAGE ID", "DELIV", "SUBJECT"];
    if dlq {
        headers.push("REASON");
    }
    headers.push("BODY");
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            let mut row = vec![
                m.sequence_number.to_string(),
                m.enqueued_time
                    .and_then(|t| t.format(&Rfc3339).ok())
                    .unwrap_or_default(),
                ellipsize(m.message_id.as_deref().unwrap_or(""), 36),
                m.delivery_count.to_string(),
                ellipsize(m.subject.as_deref().unwrap_or(""), 24),
            ];
            if dlq {
                row.push(ellipsize(m.dead_letter_reason.as_deref().unwrap_or(""), 32));
            }
            row.push(body_cell(&m.body));
            row
        })
        .collect();
    println!("{}", table(&headers, &rows));
    if messages.is_empty() {
        eprintln!("(no messages)");
    }
    Ok(())
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let session = state::entra_session();
    let mut st = state::load();
    match cli.command {
        Command::Login {
            device_code,
            tenant,
        } => {
            let sign_in = if device_code {
                session
                    .login_device_code(&tenant, |dc| eprintln!("{}", dc.message))
                    .await?
            } else {
                session
                    .login_browser(&tenant, |url| {
                        eprintln!(
                            "Opening your browser to sign in. If it does not open, visit:\n{url}"
                        );
                        let _ = open::that(url);
                    })
                    .await?
            };
            eprintln!(
                "Signed in as {} (tenant {}).",
                sign_in.user_principal, sign_in.tenant_id
            );
            if let Some(old) = st.sign_in.take().filter(|old| old.id != sign_in.id)
                && let Err(e) = session.logout(&old)
                && !matches!(e, horizon_azure_kit::KitError::NotFound(_))
            {
                eprintln!(
                    "note: could not clear the previous sign-in for {}: {e}",
                    old.user_principal
                );
            }
            st.sign_in = Some(sign_in);
            state::save(&st)?;
        }
        Command::Logout => {
            if let Some(sign_in) = st.sign_in.take() {
                session.logout(&sign_in)?;
                state::save(&st)?;
                eprintln!("Signed out {}.", sign_in.user_principal);
            } else {
                eprintln!("Not signed in.");
            }
        }
        Command::Whoami => match &st.sign_in {
            Some(s) => println!("{} (tenant {})", s.user_principal, s.tenant_id),
            None => println!("Not signed in."),
        },
        Command::Namespaces => {
            let sign_in = st
                .sign_in
                .clone()
                .context("not signed in: run `dispatched login`")?;
            let token = session
                .token_for(&sign_in, None, &AzureCloud::Public.arm_scope())
                .await?;
            let arm = ArmClient::new(AzureCloud::Public, token.token);
            let found = discover_namespaces(&arm).await?;
            if let Format::Json = cli.format {
                println!("{}", serde_json::to_string_pretty(&found)?);
            } else {
                for s in &found.skipped {
                    eprintln!(
                        "warning: skipped subscription {}: {}",
                        ellipsize(&s.subscription_name, 80),
                        ellipsize(&s.reason, 200)
                    );
                }
                let rows: Vec<Vec<String>> = found
                    .namespaces
                    .iter()
                    .map(|n| {
                        vec![
                            n.name.clone(),
                            n.host.clone(),
                            n.tier.clone().unwrap_or_default(),
                            n.location.clone().unwrap_or_default(),
                            n.subscription_name.clone(),
                            if n.local_auth_disabled {
                                "Entra only".into()
                            } else {
                                String::new()
                            },
                        ]
                    })
                    .collect();
                println!(
                    "{}",
                    table(
                        &["NAME", "HOST", "TIER", "LOCATION", "SUBSCRIPTION", "AUTH"],
                        &rows
                    )
                );
            }
        }
        Command::Ls { topics } => {
            let ns = resolve::namespace(
                cli.connection_string.as_deref(),
                cli.namespace.as_deref(),
                &session,
                &st,
            )?;
            let explorer = Explorer::new(ns);
            let queues = explorer.list_queues().await?;
            if let Format::Json = cli.format {
                let mut value = serde_json::json!({ "queues": queues });
                if topics {
                    let mut list = Vec::new();
                    for t in explorer.list_topics().await? {
                        let subs = explorer.list_subscriptions(&t.name).await?;
                        list.push(serde_json::json!({ "topic": t, "subscriptions": subs }));
                    }
                    value["topics"] = serde_json::Value::Array(list);
                }
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = queues
                .iter()
                .map(|q| {
                    vec![
                        q.name.clone(),
                        count_cell(q.counts.as_ref().map(|c| c.active)),
                        count_cell(q.counts.as_ref().map(|c| c.dead_letter)),
                        count_cell(q.counts.as_ref().map(|c| c.scheduled)),
                        if q.requires_session {
                            "yes".into()
                        } else {
                            String::new()
                        },
                        q.status.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            println!(
                "{}",
                table(
                    &["QUEUE", "ACTIVE", "DLQ", "SCHEDULED", "SESSIONS", "STATUS"],
                    &rows
                )
            );
            if topics {
                let mut rows = Vec::new();
                for t in explorer.list_topics().await? {
                    for s in explorer.list_subscriptions(&t.name).await? {
                        rows.push(vec![
                            format!("{}/subscriptions/{}", t.name, s.name),
                            count_cell(s.counts.as_ref().map(|c| c.active)),
                            count_cell(s.counts.as_ref().map(|c| c.dead_letter)),
                            s.status.clone().unwrap_or_default(),
                        ]);
                    }
                }
                println!(
                    "\n{}",
                    table(&["SUBSCRIPTION", "ACTIVE", "DLQ", "STATUS"], &rows)
                );
            }
        }
        Command::Peek {
            entity,
            dlq,
            transfer_dlq,
            top,
            from_seq,
        } => {
            let ns = resolve::namespace(
                cli.connection_string.as_deref(),
                cli.namespace.as_deref(),
                &session,
                &st,
            )?;
            let explorer = Explorer::new(ns);
            let path = EntityPath::parse(&entity)?;
            let sub_queue = match (dlq, transfer_dlq) {
                (true, _) => SubQueue::DeadLetter,
                (_, true) => SubQueue::TransferDeadLetter,
                _ => SubQueue::Active,
            };
            let messages = explorer.peek(&path, sub_queue, from_seq, top).await?;
            explorer.close().await;
            print_messages(&messages, cli.format, dlq || transfer_dlq)?;
        }
    }
    Ok(())
}
