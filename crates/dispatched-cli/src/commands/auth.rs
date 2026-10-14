//! `dispatched login | logout | whoami | namespaces`.

use crate::context::Ctx;
use crate::output::{ellipsize, table};
use crate::{Format, LOGIN_HINT};
use anyhow::Context as _;
use dispatched_core::discovery::{
    DiscoveredNamespace, Discovery, SkippedSubscription, discover_namespaces,
};
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::arm::ArmClient;
use horizon_azure_kit::auth::SignIn;
use serde::Serialize;

pub async fn login(ctx: &Ctx, device_code: bool, tenant: &str) -> anyhow::Result<()> {
    let sign_in = if device_code {
        ctx.session
            .login_device_code(tenant, |dc| eprintln!("{}", dc.message))
            .await?
    } else {
        ctx.session
            .login_browser(tenant, |url| {
                eprintln!("Opening your browser to sign in. If it does not open, visit:\n{url}");
                let _ = open::that(url);
            })
            .await?
    };
    eprintln!(
        "Signed in as {} (tenant {}).",
        sign_in.user_principal, sign_in.tenant_id
    );
    let old = ctx.store.current_sign_in()?;
    ctx.store.save_sign_in(&sign_in)?;
    ctx.store.set_current_sign_in(Some(&sign_in.id))?;
    if let Some(old) = old.filter(|old| old.id != sign_in.id) {
        forget_previous(ctx, &old);
    }
    Ok(())
}

/// Clear the previous account's refresh token, unless a saved profile still
/// uses that account (then it stays usable once the user signs back in).
fn forget_previous(ctx: &Ctx, old: &SignIn) {
    let in_use = ctx.store.list().map_or(true, |profiles| {
        profiles.iter().any(|p| {
            matches!(&p.auth, dispatched_core::profiles::ProfileAuth::Entra { sign_in_id, .. } if *sign_in_id == old.id)
        })
    });
    if in_use {
        eprintln!(
            "note: kept the sign-in for {}; saved profiles use it.",
            old.user_principal
        );
        return;
    }
    if let Err(e) = ctx.session.logout(old)
        && !matches!(e, horizon_azure_kit::KitError::NotFound(_))
    {
        eprintln!(
            "note: could not clear the previous sign-in for {}: {e}",
            old.user_principal
        );
    }
    let _ = ctx.store.remove_sign_in(&old.id);
}

pub async fn logout(ctx: &Ctx) -> anyhow::Result<()> {
    if let Some(sign_in) = ctx.store.current_sign_in()? {
        ctx.session.logout(&sign_in)?;
        ctx.store.set_current_sign_in(None)?;
        ctx.store.remove_sign_in(&sign_in.id)?;
        eprintln!("Signed out {}.", sign_in.user_principal);
    } else {
        eprintln!("Not signed in.");
    }
    Ok(())
}

pub async fn whoami(ctx: &Ctx, format: Format) -> anyhow::Result<()> {
    let sign_in = ctx.current_sign_in()?;
    if let Format::Json = format {
        let value = serde_json::json!({
            "signed_in": sign_in.is_some(),
            "user": sign_in.as_ref().map(|s| &s.user_principal),
            "tenant": sign_in.as_ref().map(|s| &s.tenant_id),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    match sign_in {
        Some(s) => println!(
            "{} (tenant {})",
            crate::output::sanitize(&s.user_principal),
            crate::output::sanitize(&s.tenant_id)
        ),
        None => println!("Not signed in; {LOGIN_HINT}."),
    }
    Ok(())
}

/// A tenant that could not be searched (for example MFA or Conditional Access).
#[derive(Debug, Serialize)]
pub struct SkippedTenant {
    pub tenant_id: String,
    pub reason: String,
}

/// The `namespaces --format json` document.
#[derive(Debug, Serialize)]
pub struct NamespacesOutput {
    pub namespaces: Vec<DiscoveredNamespace>,
    pub skipped: Vec<SkippedSubscription>,
    pub skipped_tenants: Vec<SkippedTenant>,
}

pub async fn namespaces(
    ctx: &Ctx,
    tenant: Option<String>,
    all_tenants: bool,
    format: Format,
) -> anyhow::Result<()> {
    let sign_in = ctx
        .current_sign_in()?
        .context(format!("not signed in; {LOGIN_HINT}"))?;
    let scope = AzureCloud::Public.arm_scope();
    let mut out = NamespacesOutput {
        namespaces: Vec::new(),
        skipped: Vec::new(),
        skipped_tenants: Vec::new(),
    };
    if all_tenants {
        let home = ctx.session.token_for(&sign_in, None, &scope).await?;
        let tenants = ArmClient::new(AzureCloud::Public, home.token)
            .list_tenants()
            .await?;
        for t in tenants {
            let found = async {
                let token = ctx
                    .session
                    .token_for(&sign_in, Some(&t.tenant_id), &scope)
                    .await
                    .map_err(anyhow::Error::from)?;
                discover_namespaces(&ArmClient::new(AzureCloud::Public, token.token))
                    .await
                    .map_err(anyhow::Error::from)
            }
            .await;
            match found {
                Ok(d) => merge(&mut out, d),
                Err(e) => {
                    let reason = format!("{e:#}");
                    eprintln!(
                        "warning: skipped tenant {}: {}",
                        ellipsize(&t.tenant_id, 80),
                        ellipsize(&reason, 200)
                    );
                    out.skipped_tenants.push(SkippedTenant {
                        tenant_id: t.tenant_id,
                        reason,
                    });
                }
            }
        }
        out.namespaces.sort_by(|a, b| a.name.cmp(&b.name));
    } else {
        let token = ctx
            .session
            .token_for(&sign_in, tenant.as_deref(), &scope)
            .await?;
        let found = discover_namespaces(&ArmClient::new(AzureCloud::Public, token.token)).await?;
        merge(&mut out, found);
    }
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    for s in &out.skipped {
        eprintln!(
            "warning: skipped subscription {}: {}",
            ellipsize(&s.subscription_name, 80),
            ellipsize(&s.reason, 200)
        );
    }
    let rows: Vec<Vec<String>> = out
        .namespaces
        .iter()
        .map(|n| {
            vec![
                n.name.clone(),
                n.host.clone(),
                n.tier.clone().unwrap_or_default(),
                n.location.clone().unwrap_or_default(),
                n.subscription_name.clone(),
                n.tenant_id.clone().unwrap_or_default(),
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
            &[
                "NAME",
                "HOST",
                "TIER",
                "LOCATION",
                "SUBSCRIPTION",
                "TENANT",
                "AUTH"
            ],
            &rows
        )
    );
    Ok(())
}

fn merge(out: &mut NamespacesOutput, d: Discovery) {
    out.namespaces.extend(d.namespaces);
    out.skipped.extend(d.skipped);
}
