//! Saved connections. Metadata in SQLite; secrets in the OS keychain.

pub mod store;

pub use store::ProfileStore;

use crate::connstr::ConnectionString;
use crate::namespace::{Namespace, ProxyConfig, Transport};
use crate::tokens::EntraSource;
use crate::{Error, Result};
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::CredentialStore;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// How a profile authenticates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProfileAuth {
    /// A connection string kept in the OS keychain.
    ConnectionString,
    /// A signed-in Entra identity.
    Entra {
        /// Sign-in id (see `SignIn::id`).
        sign_in_id: String,
        /// Tenant to request tokens in (home tenant if `None`).
        tenant: Option<String>,
    },
}

/// A saved connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
    /// Stable id.
    pub id: String,
    /// User-visible name (unique, case-insensitive).
    pub name: String,
    /// Namespace host.
    pub host: String,
    /// Authentication.
    pub auth: ProfileAuth,
    /// Service Bus Emulator.
    pub emulator: bool,
    /// Production: destructive operations need typed confirmation (M2).
    pub production: bool,
    /// Optional colour label.
    pub color: Option<String>,
    /// AMQP transport.
    pub transport: Transport,
    /// `http://host:port` (credentials only via `HTTPS_PROXY`).
    pub proxy: Option<String>,
    /// Entities to show when the identity cannot list the namespace.
    pub pinned_entities: Vec<String>,
}

impl Profile {
    fn base(name: &str, host: &str, auth: ProfileAuth, emulator: bool) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            host: host.to_ascii_lowercase(),
            auth,
            emulator,
            production: false,
            color: None,
            transport: Transport::Amqp,
            proxy: None,
            pinned_entities: Vec::new(),
        }
    }

    /// A connection-string profile (secret stored separately).
    pub fn connection_string(name: &str, cs: &ConnectionString) -> Self {
        let mut p = Self::base(
            name,
            &cs.host,
            ProfileAuth::ConnectionString,
            cs.use_development_emulator,
        );
        if let Some(e) = &cs.entity_path {
            p.pinned_entities.push(e.clone());
        }
        p
    }

    /// An Entra profile.
    pub fn entra(name: &str, host: &str, sign_in_id: &str, tenant: Option<&str>) -> Self {
        Self::base(
            name,
            host,
            ProfileAuth::Entra {
                sign_in_id: sign_in_id.to_string(),
                tenant: tenant.map(str::to_string),
            },
            false,
        )
    }
}

/// What a UI submits to create (`id: None`) or edit a profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDraft {
    /// Existing profile id when editing.
    #[serde(default)]
    pub id: Option<String>,
    /// Display name.
    pub name: String,
    /// How it authenticates.
    pub auth: DraftAuth,
    /// Production profile.
    #[serde(default)]
    pub production: bool,
    /// Optional colour label.
    #[serde(default)]
    pub color: Option<String>,
    /// AMQP transport.
    #[serde(default)]
    pub transport: Transport,
    /// `http://host:port` proxy.
    #[serde(default)]
    pub proxy: Option<String>,
    /// Entities to show when the identity cannot list the namespace.
    #[serde(default)]
    pub pinned_entities: Vec<String>,
}

/// The authentication part of a [`ProfileDraft`].
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DraftAuth {
    /// A connection string; `None` on an edit keeps the stored one.
    ConnectionString {
        /// The connection string (a secret).
        #[serde(default)]
        connection_string: Option<String>,
    },
    /// The current Entra sign-in, for a namespace name or host.
    Entra {
        /// `myns` or `myns.servicebus.windows.net`.
        namespace: String,
        /// Tenant to request tokens in (home tenant if `None`).
        #[serde(default)]
        tenant: Option<String>,
    },
}

impl std::fmt::Debug for DraftAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftAuth::ConnectionString { connection_string } => f
                .debug_struct("ConnectionString")
                .field(
                    "connection_string",
                    &connection_string.as_ref().map(|_| "<redacted>"),
                )
                .finish(),
            DraftAuth::Entra { namespace, tenant } => f
                .debug_struct("Entra")
                .field("namespace", namespace)
                .field("tenant", tenant)
                .finish(),
        }
    }
}

/// Keychain key of a profile's connection string.
pub fn secret_key(profile_id: &str) -> String {
    format!("dispatched:profile:{profile_id}:connection-string")
}

/// A bare namespace name expands to the public-cloud host; a dotted input is
/// taken as the full host. Only hostname characters (letters, digits, `.`,
/// `-`) are accepted, so a URL, port or path is rejected rather than guessed at.
pub fn namespace_host(input: &str) -> Result<String> {
    let input = input.trim().to_ascii_lowercase();
    let valid = !input.is_empty()
        && !input.starts_with(['.', '-'])
        && !input.ends_with(['.', '-'])
        && !input.contains("..")
        && input
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !valid {
        return Err(Error::Invalid(
            "invalid namespace: expected a name like `myns` or a host like \
             `myns.servicebus.windows.net` (letters, digits, `.` and `-` only)"
                .into(),
        ));
    }
    Ok(if input.contains('.') {
        input
    } else {
        format!("{input}.servicebus.windows.net")
    })
}

/// Apply `HTTPS_PROXY` unless the namespace is the emulator or already proxied.
pub fn with_env_proxy(ns: Namespace) -> Result<Namespace> {
    apply_proxy(ns, ProxyConfig::from_env())
}

/// Apply `proxy` unless the namespace is the emulator or already proxied.
pub fn apply_proxy(ns: Namespace, proxy: Option<ProxyConfig>) -> Result<Namespace> {
    if ns.is_emulator() || ns.proxy().is_some() {
        return Ok(ns);
    }
    ns.with_proxy(proxy)
}

/// Build the namespace a profile points at.
pub fn namespace_for(
    profile: &Profile,
    secrets: &dyn CredentialStore,
    entra: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> Result<Namespace> {
    let ns = match &profile.auth {
        ProfileAuth::ConnectionString => {
            let secret = secrets.get(&secret_key(&profile.id))?.ok_or_else(|| {
                Error::Invalid(format!(
                    "profile `{}` has no stored connection string; edit the connection and paste it again",
                    profile.name
                ))
            })?;
            Namespace::from_connection_string(&ConnectionString::parse(secret.expose_secret())?)?
        }
        ProfileAuth::Entra { sign_in_id, tenant } => {
            let sign_in = sign_in.filter(|s| &s.id == sign_in_id).ok_or_else(|| {
                Error::NotSignedIn(format!(
                    "profile `{}` needs the account it was created with",
                    profile.name
                ))
            })?;
            Namespace::new(
                profile.host.clone(),
                Arc::new(EntraSource::new(
                    entra.clone(),
                    sign_in.clone(),
                    tenant.clone(),
                )),
            )
        }
    };
    let ns = ns.with_transport(profile.transport)?;
    let proxy = profile
        .proxy
        .as_deref()
        .map(ProxyConfig::parse)
        .transpose()?;
    with_env_proxy(ns.with_proxy(proxy)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_azure_kit::AzureCloud;
    use horizon_azure_kit::secrets::MemoryStore;

    fn session() -> Arc<EntraSession> {
        Arc::new(EntraSession::new(
            AzureCloud::Public,
            Arc::new(MemoryStore::new()),
            "test",
        ))
    }

    #[test]
    fn connection_string_profile_resolves_to_its_namespace() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let cs = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";
        let p = store
            .add_connection_string("emu", cs, &secrets, false)
            .unwrap();
        assert!(p.emulator);
        let ns = namespace_for(&p, &secrets, &session(), None).unwrap();
        assert!(ns.is_emulator());
        assert_eq!(ns.host(), "localhost");
    }

    #[test]
    fn missing_secret_error_names_the_phrase_the_ui_matches() {
        // The app recognises this case by the words "no stored connection string": keep them.
        let cs = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v",
        )
        .unwrap();
        let p = Profile::connection_string("Orphan", &cs);
        let e = namespace_for(&p, &MemoryStore::new(), &session(), None).unwrap_err();
        assert!(matches!(e, Error::Invalid(_)));
        assert!(e.to_string().contains("no stored connection string"), "{e}");
        // ... and says how to fix it.
        assert!(
            e.to_string()
                .contains("edit the connection and paste it again"),
            "{e}"
        );
    }

    #[test]
    fn entra_profile_without_its_sign_in_says_so() {
        let p = Profile::entra("cloud", "ns.servicebus.windows.net", "oid-1", None);
        let e = namespace_for(&p, &MemoryStore::new(), &session(), None).unwrap_err();
        assert!(matches!(e, Error::NotSignedIn(_)));
    }

    #[test]
    fn entity_path_becomes_a_pin() {
        let cs = ConnectionString::parse("Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=orders").unwrap();
        assert_eq!(
            Profile::connection_string("x", &cs).pinned_entities,
            ["orders"]
        );
    }

    #[test]
    fn profile_proxy_forces_websockets() {
        let mut p = Profile::entra("cloud", "ns.servicebus.windows.net", "oid-1", None);
        p.proxy = Some("http://proxy.local:8080".into());
        let si = SignIn {
            id: "oid-1".into(),
            user_principal: "a@x.io".into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        };
        let ns = namespace_for(&p, &MemoryStore::new(), &session(), Some(&si)).unwrap();
        assert_eq!(ns.transport(), Transport::WebSockets);
        assert!(ns.proxy().is_some());
    }

    fn namespace(cs: &str) -> Namespace {
        Namespace::from_connection_string(&ConnectionString::parse(cs).unwrap()).unwrap()
    }

    const REAL: &str =
        "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v";
    const EMU: &str = "Endpoint=sb://localhost;SharedAccessKeyName=k;SharedAccessKey=v;UseDevelopmentEmulator=true;";

    #[test]
    fn apply_proxy_leaves_the_emulator_alone() {
        let ns = apply_proxy(namespace(EMU), ProxyConfig::parse("http://p:8080").ok()).unwrap();
        assert!(ns.proxy().is_none());
        assert_eq!(ns.transport(), Transport::Amqp);
    }

    #[test]
    fn apply_proxy_forces_websockets() {
        let ns = apply_proxy(namespace(REAL), ProxyConfig::parse("http://p:8080").ok()).unwrap();
        assert_eq!(ns.proxy().unwrap().host(), "p");
        assert_eq!(ns.transport(), Transport::WebSockets);
    }

    #[test]
    fn apply_proxy_keeps_an_existing_proxy() {
        let ns = namespace(REAL)
            .with_proxy(ProxyConfig::parse("http://first:1").ok())
            .unwrap();
        let ns = apply_proxy(ns, ProxyConfig::parse("http://second:2").ok()).unwrap();
        assert_eq!(ns.proxy().unwrap().host(), "first");
    }

    #[test]
    fn namespace_host_expands_names_and_rejects_urls() {
        assert_eq!(
            namespace_host("MyNs").unwrap(),
            "myns.servicebus.windows.net"
        );
        assert_eq!(
            namespace_host(" x.servicebus.chinacloudapi.cn ").unwrap(),
            "x.servicebus.chinacloudapi.cn"
        );
        for bad in [
            "",
            "https://x.servicebus.windows.net",
            "x:5671",
            ".x",
            "x..y",
            "a/b",
        ] {
            let e = namespace_host(bad).unwrap_err().to_string();
            assert!(e.contains("invalid namespace"), "{bad}: {e}");
        }
    }
}
