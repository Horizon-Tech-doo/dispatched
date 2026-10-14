//! `dispatched profile …`

use crate::Format;
use crate::context::Ctx;
use crate::output::{sanitize, table};
use crate::resolve::namespace_host;
use anyhow::{Context, bail};
use clap::Subcommand;
use dispatched_core::diagnostics::{Check, ConnectionReport, test_connection};
use dispatched_core::namespace::{ProxyConfig, Transport};
use dispatched_core::profiles::{Profile, ProfileAuth, namespace_for};
use dispatched_core::{ConnectionString, EntityPath, Explorer};
use std::io::{IsTerminal, Read};

#[derive(Subcommand)]
pub enum ProfileCmd {
    /// List saved profiles.
    Ls,
    /// Show one profile.
    Show { name: String },
    /// Save a profile.
    Add {
        name: String,
        /// Read the connection string from this environment variable.
        #[arg(long, conflicts_with_all = ["connection_string_stdin", "namespace"])]
        connection_string_env: Option<String>,
        /// Read the connection string from standard input (pipe it in).
        #[arg(long, conflicts_with = "namespace")]
        connection_string_stdin: bool,
        /// Namespace (name or host) used with the current Entra sign-in.
        #[arg(long)]
        namespace: Option<String>,
        /// Tenant for Entra tokens (default: the sign-in's home tenant).
        #[arg(long, requires = "namespace")]
        tenant: Option<String>,
        /// Mark as production (destructive operations need typed confirmation).
        #[arg(long)]
        production: bool,
        /// Colour label.
        #[arg(long)]
        color: Option<String>,
        /// amqp or websockets.
        #[arg(long)]
        transport: Option<String>,
        /// http://host:port (credentials only via HTTPS_PROXY).
        #[arg(long)]
        proxy: Option<String>,
    },
    /// Delete a profile and its stored secret.
    Rm { name: String },
    /// Set or clear the default profile.
    Default {
        name: Option<String>,
        #[arg(long, conflicts_with = "name")]
        clear: bool,
    },
    /// Remember an entity to show when the identity cannot list the namespace.
    Pin { name: String, entity: String },
    /// Forget a pinned entity.
    Unpin { name: String, entity: String },
    /// Check which auth path is used and which rights work. Exits non-zero
    /// when a check fails. Without a name: --profile / DISPATCHED_PROFILE, then
    /// the default profile.
    Test {
        name: Option<String>,
        #[arg(long)]
        entity: Option<String>,
    },
}

fn auth_label(p: &Profile) -> String {
    match &p.auth {
        ProfileAuth::ConnectionString => "connection string".into(),
        ProfileAuth::Entra { tenant, .. } => {
            format!(
                "Entra{}",
                tenant
                    .as_deref()
                    .map(|t| format!(" ({t})"))
                    .unwrap_or_default()
            )
        }
    }
}

/// Transport and proxy settings from `profile add`, checked before anything is saved.
#[derive(Debug, Default, PartialEq, Eq)]
struct NetworkOptions {
    transport: Option<Transport>,
    /// `http://host:port`, without credentials.
    proxy: Option<String>,
}

impl NetworkOptions {
    fn parse(transport: Option<&str>, proxy: Option<&str>) -> anyhow::Result<Self> {
        let mut transport = transport.map(Transport::parse).transpose()?;
        let proxy = match proxy {
            None => None,
            Some(p) => {
                let parsed = ProxyConfig::parse(p)?;
                if parsed.basic_auth().is_some() {
                    bail!(
                        "profiles store only http://host:port; put proxy credentials in HTTPS_PROXY"
                    );
                }
                if transport == Some(Transport::Amqp) {
                    bail!("a proxy carries AMQP over WebSockets; drop --transport amqp");
                }
                // A proxy forces WebSockets.
                transport = Some(Transport::WebSockets);
                Some(parsed.without_credentials_url())
            }
        };
        Ok(Self { transport, proxy })
    }

    /// The emulator supports neither WebSockets nor a proxy.
    fn check_emulator(&self, emulator: bool) -> anyhow::Result<()> {
        if emulator && (self.proxy.is_some() || self.transport == Some(Transport::WebSockets)) {
            bail!("the Service Bus emulator supports neither WebSockets nor a proxy");
        }
        Ok(())
    }

    fn apply(self, profile: &mut Profile) {
        if let Some(t) = self.transport {
            profile.transport = t;
        }
        if self.proxy.is_some() {
            profile.proxy = self.proxy;
        }
    }
}

fn find(ctx: &Ctx, name: &str) -> anyhow::Result<Profile> {
    ctx.store()?
        .get(name)?
        .with_context(|| format!("no profile named `{name}`; see `dispatched profile ls`"))
}

/// A connection string from the environment or standard input; never echoed.
fn read_connection_string(env_var: Option<String>) -> anyhow::Result<String> {
    if let Some(var) = env_var {
        return std::env::var(&var)
            .with_context(|| format!("environment variable `{var}` is not set"));
    }
    if std::io::stdin().is_terminal() {
        bail!(
            "--connection-string-stdin reads a piped connection string (it would be echoed in a terminal); \
             pipe it in, or use --connection-string-env VAR"
        );
    }
    let mut cs = String::new();
    std::io::stdin()
        .read_to_string(&mut cs)
        .context("reading the connection string from standard input")?;
    Ok(cs)
}

#[allow(clippy::too_many_arguments)]
fn add(
    ctx: &Ctx,
    name: &str,
    connection_string: Option<String>,
    namespace: Option<String>,
    tenant: Option<String>,
    production: bool,
    color: Option<String>,
    net: NetworkOptions,
) -> anyhow::Result<Profile> {
    let color = color
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    if let Some(cs) = connection_string {
        let cs = cs.trim();
        net.check_emulator(ConnectionString::parse(cs)?.use_development_emulator)?;
        let mut profile =
            ctx.store()?
                .add_connection_string(name, cs, ctx.secrets.as_ref(), production)?;
        profile.color = color;
        net.apply(&mut profile);
        if let Err(e) = ctx.store()?.update(&profile) {
            // Do not leave a half-configured profile (and its secret) behind.
            let _ = ctx.store()?.delete(&profile.name, ctx.secrets.as_ref());
            return Err(e.into());
        }
        Ok(profile)
    } else if let Some(ns) = namespace {
        let sign_in = ctx
            .current_sign_in()?
            .context("not signed in: run `dispatched login` first")?;
        ctx.store()?.save_sign_in(&sign_in)?;
        let mut profile =
            Profile::entra(name, &namespace_host(&ns)?, &sign_in.id, tenant.as_deref());
        profile.production = production;
        profile.color = color;
        net.apply(&mut profile);
        ctx.store()?.insert(&profile)?;
        Ok(profile)
    } else {
        bail!(
            "pass --connection-string-env VAR, --connection-string-stdin, or --namespace (after `dispatched login`)"
        );
    }
}

/// `Err` naming the failed checks and their kinds, if any check failed.
fn failure_summary(report: &ConnectionReport) -> anyhow::Result<()> {
    let failed: Vec<String> = [
        ("management", &report.management),
        ("messaging", &report.messaging),
    ]
    .into_iter()
    .filter_map(|(name, c)| match c {
        Check::Failed { kind, .. } => Some(format!("{name} ({kind})")),
        _ => None,
    })
    .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        bail!("connection test failed: {}", failed.join(", "))
    }
}

pub async fn run(
    ctx: &Ctx,
    cmd: ProfileCmd,
    format: Format,
    selected_profile: Option<&str>,
) -> anyhow::Result<()> {
    match cmd {
        ProfileCmd::Ls => {
            let profiles = ctx.store()?.list()?;
            let default_id = ctx.store()?.default_profile()?.map(|p| p.id);
            if let Format::Json = format {
                println!("{}", serde_json::to_string_pretty(&profiles)?);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = profiles
                .iter()
                .map(|p| {
                    vec![
                        format!(
                            "{}{}",
                            if default_id.as_deref() == Some(p.id.as_str()) {
                                "* "
                            } else {
                                ""
                            },
                            p.name
                        ),
                        p.host.clone(),
                        auth_label(p),
                        if p.production {
                            "PROD".into()
                        } else {
                            String::new()
                        },
                        match p.transport {
                            Transport::Amqp => "amqp".into(),
                            Transport::WebSockets => "websockets".into(),
                        },
                    ]
                })
                .collect();
            println!(
                "{}",
                table(&["PROFILE", "HOST", "AUTH", "", "TRANSPORT"], &rows)
            );
        }
        ProfileCmd::Show { name } => {
            let p = find(ctx, &name)?;
            println!("{}", serde_json::to_string_pretty(&p)?);
        }
        ProfileCmd::Add {
            name,
            connection_string_env,
            connection_string_stdin,
            namespace,
            tenant,
            production,
            color,
            transport,
            proxy,
        } => {
            let net = NetworkOptions::parse(transport.as_deref(), proxy.as_deref())?;
            let connection_string = if connection_string_env.is_some() || connection_string_stdin {
                Some(read_connection_string(connection_string_env)?)
            } else {
                None
            };
            let profile = add(
                ctx,
                &name,
                connection_string,
                namespace,
                tenant,
                production,
                color,
                net,
            )?;
            eprintln!(
                "Saved profile `{}` ({}).",
                sanitize(&profile.name),
                sanitize(&profile.host)
            );
        }
        ProfileCmd::Rm { name } => {
            if ctx.store()?.delete(&name, ctx.secrets.as_ref())? {
                eprintln!(
                    "Removed profile `{}` and its stored secret.",
                    sanitize(&name)
                );
            } else {
                bail!("no profile named `{name}`; see `dispatched profile ls`");
            }
        }
        ProfileCmd::Default { name, clear } => {
            if clear {
                ctx.store()?.set_default(None)?;
                eprintln!("Cleared the default profile.");
            } else if let Some(n) = name {
                ctx.store()?.set_default(Some(&n))?;
                eprintln!("Default profile: {}", sanitize(&n));
            } else {
                match ctx.store()?.default_profile()? {
                    Some(p) => println!("{}", sanitize(&p.name)),
                    None => println!("(none)"),
                }
            }
        }
        ProfileCmd::Pin { name, entity } => {
            let mut p = find(ctx, &name)?;
            EntityPath::parse(&entity)?;
            let msg = if p
                .pinned_entities
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&entity))
            {
                format!("`{entity}` is already pinned on profile `{}`.", p.name)
            } else {
                let msg = format!("Pinned `{entity}` on profile `{}`.", p.name);
                p.pinned_entities.push(entity);
                ctx.store()?.update(&p)?;
                msg
            };
            eprintln!("{}", sanitize(&msg));
        }
        ProfileCmd::Unpin { name, entity } => {
            let mut p = find(ctx, &name)?;
            let before = p.pinned_entities.len();
            p.pinned_entities
                .retain(|e| !e.eq_ignore_ascii_case(&entity));
            let msg = if p.pinned_entities.len() == before {
                format!("`{entity}` is not pinned on profile `{}`.", p.name)
            } else {
                ctx.store()?.update(&p)?;
                format!("Unpinned `{entity}` from profile `{}`.", p.name)
            };
            eprintln!("{}", sanitize(&msg));
        }
        ProfileCmd::Test { name, entity } => {
            let p = match name.as_deref().or(selected_profile) {
                Some(n) => find(ctx, n)?,
                None => ctx
                    .store()?
                    .default_profile()?
                    .context("no default profile; pass a name")?,
            };
            let sign_in = ctx.current_sign_in()?;
            let ns = namespace_for(&p, ctx.secrets.as_ref(), &ctx.session, sign_in.as_ref())?;
            let explorer = Explorer::new(ns)?;
            let entity = entity
                .or_else(|| p.pinned_entities.first().cloned())
                .map(|e| EntityPath::parse(&e))
                .transpose()?;
            let report = test_connection(&explorer, entity.as_ref()).await;
            explorer.close().await;
            if let Format::Json = format {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return failure_summary(&report);
            }
            let show = |c: &Check| match c {
                Check::Ok { detail } => format!("ok — {}", sanitize(detail)),
                Check::Failed { message, .. } => format!("FAILED — {}", sanitize(message)),
                Check::Skipped { reason } => format!("skipped — {}", sanitize(reason)),
            };
            println!(
                "namespace   {}{}",
                sanitize(&report.host),
                if report.emulator { " (emulator)" } else { "" }
            );
            println!("auth        {}", sanitize(&report.auth));
            println!("management  {}", show(&report.management));
            println!("messaging   {}", show(&report.messaging));
            return failure_summary(&report);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::profiles::{ProfileStore, secret_key};
    use horizon_azure_kit::secrets::MemoryStore;
    use std::sync::Arc;

    const EMU: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";
    const SAS: &str = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE";

    fn ctx() -> Ctx {
        Ctx::from_parts(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        )
    }

    fn net(transport: Option<&str>, proxy: Option<&str>) -> anyhow::Result<NetworkOptions> {
        NetworkOptions::parse(transport, proxy)
    }

    fn add_cs(ctx: &Ctx, name: &str, cs: &str, net: NetworkOptions) -> anyhow::Result<Profile> {
        add(ctx, name, Some(cs.into()), None, None, false, None, net)
    }

    fn report(management: Check, messaging: Check) -> ConnectionReport {
        ConnectionReport {
            host: "h".into(),
            emulator: false,
            transport: Transport::Amqp,
            auth: "a".into(),
            management,
            messaging,
        }
    }

    #[test]
    fn failed_checks_make_the_test_fail_with_a_summary() {
        let ok = || Check::Ok { detail: "d".into() };
        let skipped = || Check::Skipped { reason: "r".into() };
        let failed = |kind| Check::Failed {
            kind,
            message: "secret-free".into(),
        };
        assert!(failure_summary(&report(ok(), ok())).is_ok());
        assert!(failure_summary(&report(ok(), skipped())).is_ok());
        let e = failure_summary(&report(failed("forbidden"), skipped()))
            .unwrap_err()
            .to_string();
        assert_eq!(e, "connection test failed: management (forbidden)");
        let e = failure_summary(&report(failed("auth"), failed("timeout")))
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "connection test failed: management (auth), messaging (timeout)"
        );
    }

    #[test]
    fn proxy_is_stored_without_credentials_and_forces_websockets() {
        let o = net(None, Some("http://proxy.corp:3128")).unwrap();
        assert_eq!(o.proxy.as_deref(), Some("http://proxy.corp:3128"));
        assert_eq!(o.transport, Some(Transport::WebSockets));
        let o = net(Some("websockets"), Some("http://[::1]:3128")).unwrap();
        assert_eq!(o.proxy.as_deref(), Some("http://[::1]:3128"));
    }

    #[test]
    fn proxy_credentials_and_amqp_are_rejected() {
        let e = net(None, Some("http://u:pw@proxy:3128"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("HTTPS_PROXY") && !e.contains("pw"), "{e}");
        assert!(net(Some("amqp"), Some("http://proxy:3128")).is_err());
        assert!(net(None, Some("socks5://proxy:1")).is_err());
        assert!(net(Some("carrier-pigeon"), None).is_err());
        assert_eq!(net(None, None).unwrap(), NetworkOptions::default());
    }

    #[test]
    fn add_stores_options_and_the_secret_only_in_the_credential_store() {
        let ctx = ctx();
        let o = net(None, Some("http://proxy.corp:3128")).unwrap();
        let p = add(
            &ctx,
            "prod",
            Some(format!("  {SAS}\n")),
            None,
            None,
            true,
            Some(" red ".into()),
            o,
        )
        .unwrap();
        let saved = ctx.store().unwrap().get("PROD").unwrap().unwrap();
        assert_eq!(saved, p);
        assert!(saved.production);
        assert_eq!(saved.color.as_deref(), Some("red"));
        assert_eq!(saved.transport, Transport::WebSockets);
        assert_eq!(saved.proxy.as_deref(), Some("http://proxy.corp:3128"));
        assert!(ctx.secrets.get(&secret_key(&saved.id)).unwrap().is_some());
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("SAS_KEY_VALUE")
        );
    }

    #[test]
    fn emulator_rejects_proxy_and_websockets_before_saving() {
        let ctx = ctx();
        for o in [
            net(None, Some("http://proxy:3128")).unwrap(),
            net(Some("websockets"), None).unwrap(),
        ] {
            let e = add_cs(&ctx, "emu", EMU, o).unwrap_err().to_string();
            assert!(e.contains("emulator"), "{e}");
        }
        assert!(ctx.store().unwrap().list().unwrap().is_empty());
        add_cs(&ctx, "emu", EMU, NetworkOptions::default()).unwrap();
    }

    #[test]
    fn duplicate_and_blank_names_are_rejected() {
        let ctx = ctx();
        add_cs(&ctx, "a", SAS, NetworkOptions::default()).unwrap();
        assert!(add_cs(&ctx, "A", SAS, NetworkOptions::default()).is_err());
        assert!(add_cs(&ctx, "  ", SAS, NetworkOptions::default()).is_err());
        assert_eq!(ctx.store().unwrap().list().unwrap().len(), 1);
    }

    #[test]
    fn entra_profile_needs_a_sign_in() {
        let ctx = ctx();
        let e = add(
            &ctx,
            "x",
            None,
            Some("myns".into()),
            None,
            false,
            None,
            NetworkOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("dispatched login"), "{e}");
        let e = add(
            &ctx,
            "x",
            None,
            None,
            None,
            false,
            None,
            NetworkOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("--connection-string-env"), "{e}");
    }

    #[tokio::test]
    async fn default_pin_unpin_and_rm_round_trip() {
        let ctx = ctx();
        let p = add_cs(&ctx, "emu", EMU, NetworkOptions::default()).unwrap();
        run(
            &ctx,
            ProfileCmd::Default {
                name: Some("EMU".into()),
                clear: false,
            },
            Format::Table,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            ctx.store().unwrap().default_profile().unwrap().unwrap().id,
            p.id
        );

        let pin = |e: &str| ProfileCmd::Pin {
            name: "emu".into(),
            entity: e.into(),
        };
        run(&ctx, pin("orders"), Format::Table, None).await.unwrap();
        run(&ctx, pin("ORDERS"), Format::Table, None).await.unwrap();
        assert_eq!(
            ctx.store()
                .unwrap()
                .get("emu")
                .unwrap()
                .unwrap()
                .pinned_entities,
            ["orders"]
        );
        assert!(
            run(&ctx, pin("a/b/c/d"), Format::Table, None)
                .await
                .is_err()
        );
        run(
            &ctx,
            ProfileCmd::Unpin {
                name: "emu".into(),
                entity: "Orders".into(),
            },
            Format::Table,
            None,
        )
        .await
        .unwrap();
        assert!(
            ctx.store()
                .unwrap()
                .get("emu")
                .unwrap()
                .unwrap()
                .pinned_entities
                .is_empty()
        );
        assert!(
            run(
                &ctx,
                ProfileCmd::Pin {
                    name: "ghost".into(),
                    entity: "q".into()
                },
                Format::Table,
                None
            )
            .await
            .is_err()
        );

        run(
            &ctx,
            ProfileCmd::Default {
                name: None,
                clear: true,
            },
            Format::Table,
            None,
        )
        .await
        .unwrap();
        assert!(ctx.store().unwrap().default_profile().unwrap().is_none());

        run(
            &ctx,
            ProfileCmd::Rm { name: "emu".into() },
            Format::Table,
            None,
        )
        .await
        .unwrap();
        assert!(ctx.store().unwrap().list().unwrap().is_empty());
        assert!(ctx.secrets.get(&secret_key(&p.id)).unwrap().is_none());
        let e = run(
            &ctx,
            ProfileCmd::Rm { name: "emu".into() },
            Format::Table,
            None,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("emu"), "{e}");
    }
}
