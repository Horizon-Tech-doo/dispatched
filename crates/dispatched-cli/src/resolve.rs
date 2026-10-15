//! Turn CLI flags (or saved profiles) into a `Namespace`.

use crate::context::StoreSource;
use anyhow::{Context, bail};
use dispatched_core::profiles;
pub use dispatched_core::profiles::namespace_host;
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

pub const ENV_CONNECTION_STRING: &str = "DISPATCHED_CONNECTION_STRING";
pub const ENV_NAMESPACE: &str = "DISPATCHED_NAMESPACE";
pub const ENV_PROFILE: &str = "DISPATCHED_PROFILE";

/// A selection that owns its strings (after merging flags with the environment).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OwnedSelection {
    pub connection_string: Option<String>,
    pub namespace: Option<String>,
    pub profile: Option<String>,
}

impl OwnedSelection {
    pub fn borrow(&self) -> Selection<'_> {
        Selection {
            connection_string: self.connection_string.as_deref(),
            namespace: self.namespace.as_deref(),
            profile: self.profile.as_deref(),
        }
    }
}

/// Merge explicit flags with the environment. Precedence: an explicit flag
/// (clap guarantees at most one) beats every environment variable; otherwise
/// `DISPATCHED_CONNECTION_STRING`, then `DISPATCHED_NAMESPACE`, then
/// `DISPATCHED_PROFILE`; with none of them the default profile applies later.
/// Environment variables never conflict with each other or with flags.
pub fn select(flags: &Selection<'_>, env: &dyn Fn(&str) -> Option<String>) -> OwnedSelection {
    let own = |v: Option<&str>| v.map(str::to_string);
    if flags.connection_string.is_some() || flags.namespace.is_some() || flags.profile.is_some() {
        return OwnedSelection {
            connection_string: own(flags.connection_string),
            namespace: own(flags.namespace),
            profile: own(flags.profile),
        };
    }
    let get = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    if let Some(cs) = get(ENV_CONNECTION_STRING) {
        return OwnedSelection {
            connection_string: Some(cs),
            ..Default::default()
        };
    }
    if let Some(ns) = get(ENV_NAMESPACE) {
        return OwnedSelection {
            namespace: Some(ns),
            ..Default::default()
        };
    }
    OwnedSelection {
        profile: get(ENV_PROFILE),
        ..Default::default()
    }
}

/// Whether resolving `sel` needs the Entra sign-in (so the keychain is only
/// read when it matters). A connection string or bare namespace never opens
/// the profile store here. Unknown profiles return false; `resolve` reports them.
pub fn needs_sign_in(sel: &Selection<'_>, store: &dyn StoreSource) -> anyhow::Result<bool> {
    if sel.connection_string.is_some() {
        return Ok(false);
    }
    if sel.namespace.is_some() {
        return Ok(true);
    }
    let store = store.store()?;
    let profile = match sel.profile {
        Some(name) => store.get(name)?,
        None => store.default_profile()?,
    };
    Ok(matches!(
        profile.map(|p| p.auth),
        Some(profiles::ProfileAuth::Entra { .. })
    ))
}

/// Resolution order: `--connection-string`, `--namespace` (with the current
/// sign-in), `--profile`, then the default profile. The profile store is only
/// opened for the last two.
pub fn resolve(
    sel: &Selection<'_>,
    store: &dyn StoreSource,
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
            namespace_host(ns)?,
            tokens,
        ))?);
    }
    let store = store.store()?;
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
        assert_eq!(
            namespace_host("MyNs").unwrap(),
            "myns.servicebus.windows.net"
        );
        assert_eq!(
            namespace_host(" x.servicebus.chinacloudapi.cn ").unwrap(),
            "x.servicebus.chinacloudapi.cn"
        );
    }

    #[test]
    fn namespace_input_must_be_a_hostname() {
        for bad in [
            "",
            "  ",
            "https://myns.servicebus.windows.net",
            "myns.servicebus.windows.net:5671",
            "myns/queue",
            "my ns",
            "user@myns",
            "sb://myns",
            "my_ns",
            ".myns",
            "myns.",
            "-myns",
            "a..b",
            "my\u{1b}[31mns",
        ] {
            let e = namespace_host(bad).unwrap_err().to_string();
            assert!(e.contains("expected a name like"), "{bad:?}: {e}");
            assert!(!e.contains('\x1b'), "{e}");
        }
    }

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    fn sel(cs: Option<&str>, ns: Option<&str>, p: Option<&str>) -> OwnedSelection {
        OwnedSelection {
            connection_string: cs.map(Into::into),
            namespace: ns.map(Into::into),
            profile: p.map(Into::into),
        }
    }

    const ALL_ENV: [(&str, &str); 3] = [
        (ENV_CONNECTION_STRING, "env-cs"),
        (ENV_NAMESPACE, "env-ns"),
        (ENV_PROFILE, "env-profile"),
    ];

    #[test]
    fn selection_precedence_table() {
        let flag = |cs, ns, p| Selection {
            connection_string: cs,
            namespace: ns,
            profile: p,
        };
        // (flags, env, expected)
        type Case<'a> = (Selection<'a>, Vec<(&'a str, &'a str)>, OwnedSelection);
        let cases: Vec<Case<'_>> = vec![
            // Nothing anywhere: the default profile applies later.
            (flag(None, None, None), vec![], sel(None, None, None)),
            // Environment only, in order of precedence.
            (
                flag(None, None, None),
                ALL_ENV.to_vec(),
                sel(Some("env-cs"), None, None),
            ),
            (
                flag(None, None, None),
                ALL_ENV[1..].to_vec(),
                sel(None, Some("env-ns"), None),
            ),
            (
                flag(None, None, None),
                ALL_ENV[2..].to_vec(),
                sel(None, None, Some("env-profile")),
            ),
            // An explicit flag beats every environment variable (and only it applies).
            (
                flag(None, None, Some("p")),
                ALL_ENV.to_vec(),
                sel(None, None, Some("p")),
            ),
            (
                flag(None, Some("n"), None),
                ALL_ENV.to_vec(),
                sel(None, Some("n"), None),
            ),
            (
                flag(Some("c"), None, None),
                ALL_ENV.to_vec(),
                sel(Some("c"), None, None),
            ),
            // Empty variables count as unset.
            (
                flag(None, None, None),
                vec![
                    (ENV_CONNECTION_STRING, ""),
                    (ENV_NAMESPACE, "  "),
                    (ENV_PROFILE, "q"),
                ],
                sel(None, None, Some("q")),
            ),
        ];
        for (flags, env, expected) in cases {
            assert_eq!(select(&flags, &env_of(&env)), expected, "env {env:?}");
        }
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
    fn sign_in_is_only_needed_for_entra_selections() {
        let (store, secrets) = ctx();
        store
            .add_connection_string("emu", EMU, &secrets, false)
            .unwrap();
        let entra =
            dispatched_core::profiles::Profile::entra("ent", "x.servicebus.windows.net", "o", None);
        store.insert(&entra).unwrap();
        let needs = |cs, ns, p| {
            needs_sign_in(
                &Selection {
                    connection_string: cs,
                    namespace: ns,
                    profile: p,
                },
                &store,
            )
            .unwrap()
        };
        assert!(!needs(Some("x"), None, None));
        assert!(needs(None, Some("n"), None));
        assert!(!needs(None, None, Some("emu")));
        assert!(needs(None, None, Some("ENT")));
        assert!(!needs(None, None, Some("ghost")));
        assert!(!needs(None, None, None));
        store.set_default(Some("ent")).unwrap();
        assert!(needs(None, None, None));
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
