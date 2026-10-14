//! `dispatched login | logout | whoami | namespaces`.

use crate::context::Ctx;
use crate::output::{ellipsize, table};
use crate::{Format, LOGIN_HINT};
use anyhow::Context as _;
use dispatched_core::discovery::{
    DiscoveredNamespace, Discovery, SkippedSubscription, discover_namespaces,
};
use dispatched_core::profiles::ProfileAuth;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::arm::ArmClient;
use horizon_azure_kit::auth::SignIn;
use serde::Serialize;

pub async fn login(ctx: &Ctx, device_code: bool, tenant: &str) -> anyhow::Result<()> {
    // Read the previous sign-in before the new token exists, so a database
    // problem here can only mean "no previous", never a stranded new token.
    let previous = ctx.store.current_sign_in().ok().flatten();
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
    for note in finish_login(ctx, &sign_in, previous.as_ref())? {
        eprintln!("note: {note}");
    }
    Ok(())
}

/// Bookkeeping after a successful login: remember the new sign-in as current,
/// then (on an account switch) clear the previous account's refresh token as
/// M0 did. Returns notes for the user, e.g. which saved profiles used the
/// previous account.
fn finish_login(ctx: &Ctx, new: &SignIn, previous: Option<&SignIn>) -> anyhow::Result<Vec<String>> {
    ctx.store.save_sign_in(new)?;
    ctx.store.set_current_sign_in(Some(&new.id))?;
    let mut notes = Vec::new();
    let Some(old) = previous.filter(|old| old.id != new.id) else {
        return Ok(notes);
    };
    let users: Vec<String> = ctx
        .store
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(
            |p| matches!(&p.auth, ProfileAuth::Entra { sign_in_id, .. } if *sign_in_id == old.id),
        )
        .map(|p| format!("`{}`", p.name))
        .collect();
    if let Err(e) = ctx.session.logout(old)
        && !matches!(e, horizon_azure_kit::KitError::NotFound(_))
    {
        notes.push(format!(
            "could not clear the previous sign-in for {}: {e}",
            old.user_principal
        ));
    }
    let _ = ctx.store.remove_sign_in(&old.id);
    if !users.is_empty() {
        let label = if users.len() == 1 {
            "Profile"
        } else {
            "Profiles"
        };
        notes.push(format!(
            "{label} {} use {}; sign in as that account again to use {}.",
            users.join(", "),
            old.user_principal,
            if users.len() == 1 { "it" } else { "them" }
        ));
    }
    Ok(notes)
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
    let mut tenants_total = 0;
    if all_tenants {
        let home = ctx.session.token_for(&sign_in, None, &scope).await?;
        let tenants = ArmClient::new(AzureCloud::Public, home.token)
            .list_tenants()
            .await?;
        tenants_total = tenants.len();
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
    let outcome = all_skipped(tenants_total, &out.skipped_tenants);
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(&out)?);
        return outcome;
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
    outcome
}

/// `--all-tenants` fails when no tenant could be searched at all.
fn all_skipped(total: usize, skipped: &[SkippedTenant]) -> anyhow::Result<()> {
    if total > 0 && skipped.len() == total {
        anyhow::bail!(
            "every tenant was skipped ({total}); nothing could be listed. Try `dispatched login` again, \
             or `--tenant <id>` for a tenant you can sign in to"
        );
    }
    Ok(())
}

fn merge(out: &mut NamespacesOutput, d: Discovery) {
    out.namespaces.extend(d.namespaces);
    out.skipped.extend(d.skipped);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::profiles::{Profile, ProfileStore};
    use horizon_azure_kit::secrets::MemoryStore;
    use std::sync::Arc;

    fn sign_in(id: &str, upn: &str) -> SignIn {
        SignIn {
            id: id.into(),
            user_principal: upn.into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        }
    }

    fn ctx() -> Ctx {
        Ctx::from_parts(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        )
    }

    #[test]
    fn first_login_becomes_current() {
        let ctx = ctx();
        let a = sign_in("a", "a@x.io");
        assert!(finish_login(&ctx, &a, None).unwrap().is_empty());
        assert_eq!(ctx.store.current_sign_in().unwrap().unwrap().id, "a");
    }

    #[test]
    fn account_switch_forgets_the_old_sign_in_and_names_its_profiles() {
        let ctx = ctx();
        let (a, b) = (sign_in("a", "a@x.io"), sign_in("b", "b@x.io"));
        ctx.store.save_sign_in(&a).unwrap();
        ctx.store.set_current_sign_in(Some("a")).unwrap();
        for (name, owner) in [("one", "a"), ("two", "a"), ("other", "b")] {
            ctx.store
                .insert(&Profile::entra(
                    name,
                    "n.servicebus.windows.net",
                    owner,
                    None,
                ))
                .unwrap();
        }
        let notes = finish_login(&ctx, &b, Some(&a)).unwrap();
        assert_eq!(
            notes,
            ["Profiles `one`, `two` use a@x.io; sign in as that account again to use them."]
        );
        assert_eq!(ctx.store.current_sign_in().unwrap().unwrap().id, "b");
        assert!(ctx.store.sign_in("a").unwrap().is_none());
    }

    #[test]
    fn switch_without_dependent_profiles_or_same_account_has_no_note() {
        let ctx = ctx();
        let (a, b) = (sign_in("a", "a@x.io"), sign_in("b", "b@x.io"));
        assert!(finish_login(&ctx, &b, Some(&a)).unwrap().is_empty());
        ctx.store
            .insert(&Profile::entra("p", "n.servicebus.windows.net", "b", None))
            .unwrap();
        // Same account again: nothing is cleared, no note.
        assert!(finish_login(&ctx, &b, Some(&b)).unwrap().is_empty());
        assert!(ctx.store.sign_in("b").unwrap().is_some());
    }

    #[test]
    fn all_tenants_fails_only_when_every_tenant_was_skipped() {
        let sk = |n: usize| {
            (0..n)
                .map(|i| SkippedTenant {
                    tenant_id: format!("t{i}"),
                    reason: "mfa".into(),
                })
                .collect::<Vec<_>>()
        };
        assert!(all_skipped(0, &sk(0)).is_ok());
        assert!(all_skipped(3, &sk(2)).is_ok());
        assert!(all_skipped(3, &sk(0)).is_ok());
        assert!(
            all_skipped(2, &sk(2))
                .unwrap_err()
                .to_string()
                .contains("every tenant")
        );
    }
}
