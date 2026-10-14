//! Turn CLI flags (or saved profiles) into a `Namespace`.

use anyhow::{Context, bail};
use dispatched_core::profiles::{self, ProfileStore};
use dispatched_core::tokens::EntraSource;
use dispatched_core::{ConnectionString, Namespace};
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::CredentialStore;
use std::sync::Arc;

/// Which namespace the user selected.
pub struct Selection<'a> {
    pub connection_string: Option<&'a str>,
    pub namespace: Option<&'a str>,
    pub profile: Option<&'a str>,
}

pub fn namespace_host(input: &str) -> String {
    let input = input.trim().to_ascii_lowercase();
    if input.contains('.') {
        input
    } else {
        format!("{input}.servicebus.windows.net")
    }
}

/// Resolution order: `--connection-string`, `--namespace` (with the current
/// sign-in), `--profile`, then the default profile.
pub fn resolve(
    sel: &Selection<'_>,
    store: &ProfileStore,
    secrets: &dyn CredentialStore,
    session: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> anyhow::Result<Namespace> {
    if let Some(cs) = sel.connection_string {
        return Ok(profiles::with_env_proxy(
            Namespace::from_connection_string(&ConnectionString::parse(cs)?)?,
        )?);
    }
    if let Some(ns) = sel.namespace {
        let sign_in = sign_in.cloned().context(
            "not signed in: run `dispatched login`, or pass --connection-string or --profile",
        )?;
        let tokens = Arc::new(EntraSource::new(session.clone(), sign_in, None));
        return Ok(profiles::with_env_proxy(Namespace::new(
            namespace_host(ns),
            tokens,
        ))?);
    }
    let profile = match sel.profile {
        Some(name) => store
            .get(name)?
            .with_context(|| format!("no profile named `{name}`; see `dispatched profile ls`"))?,
        None => match store.default_profile()? {
            Some(p) => p,
            None => bail!(
                "no namespace selected: pass --profile, --connection-string (or DISPATCHED_CONNECTION_STRING), \
                 or --namespace after `dispatched login`; or set a default with `dispatched profile default <name>`"
            ),
        },
    };
    Ok(profiles::namespace_for(
        &profile, secrets, session, sign_in,
    )?)
}

#[cfg(test)]
pub(crate) fn test_session() -> Arc<EntraSession> {
    Arc::new(EntraSession::new(
        horizon_azure_kit::AzureCloud::Public,
        Arc::new(horizon_azure_kit::secrets::MemoryStore::new()),
        "test",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::profiles::ProfileStore;
    use horizon_azure_kit::secrets::MemoryStore;

    fn ctx() -> (ProfileStore, MemoryStore) {
        (ProfileStore::open_in_memory().unwrap(), MemoryStore::new())
    }

    const EMU: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

    #[test]
    fn short_names_expand_to_public_cloud_host() {
        assert_eq!(namespace_host("MyNs"), "myns.servicebus.windows.net");
        assert_eq!(
            namespace_host("x.servicebus.chinacloudapi.cn"),
            "x.servicebus.chinacloudapi.cn"
        );
    }

    #[test]
    fn connection_string_wins() {
        let (store, secrets) = ctx();
        let sel = Selection {
            connection_string: Some(EMU),
            namespace: None,
            profile: None,
        };
        assert!(
            resolve(&sel, &store, &secrets, &test_session(), None)
                .unwrap()
                .is_emulator()
        );
    }

    #[test]
    fn named_profile_then_default_profile() {
        let (store, secrets) = ctx();
        store
            .add_connection_string("emu", EMU, &secrets, false)
            .unwrap();
        let sel = Selection {
            connection_string: None,
            namespace: None,
            profile: Some("EMU"),
        };
        assert!(
            resolve(&sel, &store, &secrets, &test_session(), None)
                .unwrap()
                .is_emulator()
        );
        let none = Selection {
            connection_string: None,
            namespace: None,
            profile: None,
        };
        let e = resolve(&none, &store, &secrets, &test_session(), None)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("--profile")
                && e.contains("--connection-string")
                && e.contains("profile default"),
            "{e}"
        );
        store.set_default(Some("emu")).unwrap();
        assert!(
            resolve(&none, &store, &secrets, &test_session(), None)
                .unwrap()
                .is_emulator()
        );
    }

    #[test]
    fn unknown_profile_is_named() {
        let (store, secrets) = ctx();
        let sel = Selection {
            connection_string: None,
            namespace: None,
            profile: Some("ghost"),
        };
        assert!(
            resolve(&sel, &store, &secrets, &test_session(), None)
                .unwrap_err()
                .to_string()
                .contains("ghost")
        );
    }

    #[test]
    fn namespace_without_sign_in_explains_what_to_do() {
        let (store, secrets) = ctx();
        let sel = Selection {
            connection_string: None,
            namespace: Some("myns"),
            profile: None,
        };
        assert!(
            resolve(&sel, &store, &secrets, &test_session(), None)
                .unwrap_err()
                .to_string()
                .contains("dispatched login")
        );
    }
}
