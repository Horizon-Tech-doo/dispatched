//! Turn CLI flags (or saved profiles) into a `Namespace` and the profile it runs under.

use crate::context::StoreSource;
use anyhow::{Context, bail};
use dispatched_core::profiles;
pub use dispatched_core::profiles::namespace_host;
use dispatched_core::tokens::EntraSource;
use dispatched_core::{ConnectionString, Namespace, ProfileRef};
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

/// The selected namespace and the profile operations on it are recorded under.
pub struct Resolved {
    /// Where to connect.
    pub namespace: Namespace,
    /// The profile name (or the host, for a connection string or `--namespace`) and whether
    /// it is production.
    pub profile: ProfileRef,
}

/// Shows the host and profile only: never the credentials.
impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("host", &self.namespace.host())
            .field("profile", &self.profile)
            .finish()
    }
}

/// What a selection points at, before the production check.
enum Chosen {
    /// A connection string or `--namespace`: no profile, named by its host.
    Host(Namespace, String),
    /// A saved profile.
    Profile(Namespace, profiles::Profile),
}

/// Resolution order: `--connection-string`, `--namespace` (with the current
/// sign-in), `--profile`, then the default profile. The profile store is only
/// opened for the last two.
fn choose(
    sel: &Selection<'_>,
    store: &dyn StoreSource,
    secrets: &dyn CredentialStore,
    session: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> anyhow::Result<Chosen> {
    if let Some(cs) = sel.connection_string {
        let cs = ConnectionString::parse(cs)?;
        let ns = profiles::with_env_proxy(Namespace::from_connection_string(&cs)?)?;
        return Ok(Chosen::Host(ns, cs.host.clone()));
    }
    if let Some(ns) = sel.namespace {
        let sign_in = sign_in.cloned().context(
            "not signed in: run `dispatched login`, or pass --connection-string or --profile",
        )?;
        let host = namespace_host(ns)?;
        let tokens = Arc::new(EntraSource::new(session.clone(), sign_in, None));
        let ns = profiles::with_env_proxy(Namespace::new(host.clone(), tokens))?;
        return Ok(Chosen::Host(ns, host));
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
    let ns = profiles::namespace_for(&profile, secrets, session, sign_in)?;
    Ok(Chosen::Profile(ns, profile))
}

/// The namespace only, for read commands: a connection string or bare namespace never opens
/// the profile store (reads do not need the production check).
pub fn resolve_namespace(
    sel: &Selection<'_>,
    store: &dyn StoreSource,
    secrets: &dyn CredentialStore,
    session: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> anyhow::Result<Namespace> {
    Ok(match choose(sel, store, secrets, session, sign_in)? {
        Chosen::Host(ns, _) | Chosen::Profile(ns, _) => ns,
    })
}

/// The namespace and its profile, for mutating commands. Production follows the namespace,
/// not the selection path: a connection string, `--namespace` or profile whose host is the
/// host of a saved production profile (ASCII case-insensitive) is production too. This opens
/// the profile store for every selection; if it cannot be read, nothing runs.
pub fn resolve(
    sel: &Selection<'_>,
    store: &dyn StoreSource,
    secrets: &dyn CredentialStore,
    session: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> anyhow::Result<Resolved> {
    let (namespace, name, marked) = match choose(sel, store, secrets, session, sign_in)? {
        Chosen::Host(ns, host) => (ns, host, false),
        Chosen::Profile(ns, p) => (ns, p.name, p.production),
    };
    let production = marked || host_is_production(store, namespace.host())?;
    Ok(Resolved {
        namespace,
        profile: ProfileRef { name, production },
    })
}

/// Whether any saved production profile points at `host`.
fn host_is_production(store: &dyn StoreSource, host: &str) -> anyhow::Result<bool> {
    let check = || -> anyhow::Result<bool> {
        Ok(store
            .store()?
            .list()?
            .iter()
            .any(|p| p.production && p.host.eq_ignore_ascii_case(host)))
    };
    check().with_context(|| {
        format!("cannot check whether {host} is saved as a production profile; nothing was changed")
    })
}

/// [`resolve`] for the selection, reading the sign-in (the keychain) only when it uses Entra.
pub fn resolve_selected(
    ctx: &crate::context::Ctx,
    sel: &Selection<'_>,
) -> anyhow::Result<Resolved> {
    let sign_in = if needs_sign_in(sel, ctx)? {
        ctx.current_sign_in()?
    } else {
        None
    };
    resolve(
        sel,
        ctx,
        ctx.secrets.as_ref(),
        &ctx.session,
        sign_in.as_ref(),
    )
}

#[cfg(test)]
/// Test selections, one per selection path.
pub(crate) enum TestSelection {
    ConnectionString(String),
    Namespace(String),
    Profile(String),
}

#[cfg(test)]
/// [`resolve`] over the context's store, signed in as the account test profiles use.
pub(crate) fn resolve_for_test(
    ctx: &crate::context::Ctx,
    sel: TestSelection,
) -> anyhow::Result<Resolved> {
    let (cs, ns, p) = match &sel {
        TestSelection::ConnectionString(s) => (Some(s.as_str()), None, None),
        TestSelection::Namespace(s) => (None, Some(s.as_str()), None),
        TestSelection::Profile(s) => (None, None, Some(s.as_str())),
    };
    let sign_in = SignIn {
        id: "o".into(),
        user_principal: "a@x.io".into(),
        tenant_id: "t".into(),
        display_name: None,
        cloud: horizon_azure_kit::AzureCloud::Public,
    };
    resolve(
        &Selection {
            connection_string: cs,
            namespace: ns,
            profile: p,
        },
        ctx,
        ctx.secrets.as_ref(),
        &ctx.session,
        Some(&sign_in),
    )
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
                .namespace
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
                .namespace
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
                .namespace
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

    mod production {
        use super::super::{TestSelection as Selection, resolve_for_test};
        use super::EMU;
        use crate::context::Ctx;
        use dispatched_core::ProfileRef;

        #[test]
        fn a_connection_string_to_a_host_saved_as_production_is_production() {
            let ctx = Ctx::for_test_with_profiles(&[
                ("prod-weu", "prod.servicebus.windows.net", true),
                ("dev", "dev.servicebus.windows.net", false),
            ]);
            let cs = |host: &str| {
                format!("Endpoint=sb://{host}/;SharedAccessKeyName=k;SharedAccessKey=x")
            };
            let r = resolve_for_test(
                &ctx,
                Selection::ConnectionString(cs("PROD.servicebus.windows.net")),
            )
            .unwrap();
            assert_eq!(
                r.profile,
                ProfileRef {
                    // The connection string parser lowercases the host (hosts are
                    // case-insensitive), so the name is the canonical host.
                    name: "prod.servicebus.windows.net".into(),
                    production: true
                },
                "matched on the host, ignoring case"
            );
            let r = resolve_for_test(
                &ctx,
                Selection::Namespace("prod.servicebus.windows.net".into()),
            )
            .unwrap();
            assert!(r.profile.production, "--namespace too");
            assert!(
                !resolve_for_test(
                    &ctx,
                    Selection::ConnectionString(cs("dev.servicebus.windows.net"))
                )
                .unwrap()
                .profile
                .production
            );
            assert!(
                !resolve_for_test(&ctx, Selection::ConnectionString(EMU.into()))
                    .unwrap()
                    .profile
                    .production,
                "the emulator host is saved nowhere"
            );
            let r = resolve_for_test(&ctx, Selection::Profile("prod-weu".into())).unwrap();
            assert_eq!(
                r.profile,
                ProfileRef {
                    name: "prod-weu".into(),
                    production: true
                }
            );
        }

        #[test]
        fn a_profile_on_a_production_host_is_production_even_if_not_marked() {
            // Saved hosts are lowercased, so this tests the "not marked" rule, not case.
            let ctx = Ctx::for_test_with_profiles(&[
                ("prod-weu", "prod.servicebus.windows.net", true),
                ("prod-alias", "prod.servicebus.windows.net", false),
            ]);
            let r = resolve_for_test(&ctx, Selection::Profile("prod-alias".into())).unwrap();
            assert_eq!(
                r.profile,
                ProfileRef {
                    name: "prod-alias".into(),
                    production: true
                }
            );
        }
    }
}
