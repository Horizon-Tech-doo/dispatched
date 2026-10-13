//! Service Bus connection strings, as copied from the Azure portal, the
//! emulator docs, or other tools.

use crate::{Error, Result};
use secrecy::SecretString;

/// A parsed connection string. Unknown keys (e.g. `TransportType`) are ignored.
#[derive(Debug, Clone)]
pub struct ConnectionString {
    /// Namespace host, lowercased (`myns.servicebus.windows.net`, `localhost`).
    pub host: String,
    /// Explicit port in `Endpoint`, if any.
    pub port: Option<u16>,
    /// `SharedAccessKeyName`.
    pub shared_access_key_name: Option<String>,
    /// `SharedAccessKey`.
    pub shared_access_key: Option<SecretString>,
    /// Pre-signed `SharedAccessSignature`.
    pub shared_access_signature: Option<SecretString>,
    /// `EntityPath` (entity-scoped connection strings).
    pub entity_path: Option<String>,
    /// `UseDevelopmentEmulator=true`.
    pub use_development_emulator: bool,
}

impl ConnectionString {
    /// Parse `input`. Keys are case-insensitive; values keep their case.
    pub fn parse(input: &str) -> Result<Self> {
        let mut endpoint = None;
        let mut key_name = None;
        let mut key = None;
        let mut sas = None;
        let mut entity_path = None;
        let mut emulator = false;
        for segment in input.trim().split(';') {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            let Some((k, v)) = segment.split_once('=') else {
                return Err(Error::Invalid(format!(
                    "connection string segment `{}` has no `=`",
                    segment.chars().take(24).collect::<String>()
                )));
            };
            let v = v.trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "endpoint" => endpoint = Some(v.to_string()),
                "sharedaccesskeyname" => key_name = Some(v.to_string()),
                "sharedaccesskey" => key = Some(SecretString::from(v.to_string())),
                "sharedaccesssignature" => sas = Some(SecretString::from(v.to_string())),
                "entitypath" => entity_path = Some(v.to_string()).filter(|s| !s.is_empty()),
                "usedevelopmentemulator" => emulator = v.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
        let endpoint = endpoint
            .ok_or_else(|| Error::Invalid("connection string has no `Endpoint=`".into()))?;
        let url = url::Url::parse(&endpoint).map_err(|e| {
            Error::Invalid(format!("`Endpoint={endpoint}` is not a valid URL: {e}"))
        })?;
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::Invalid(format!("`Endpoint={endpoint}` has no host")))?
            .to_ascii_lowercase();
        if sas.is_none() && (key_name.is_none() || key.is_none()) {
            return Err(Error::Invalid(
                "connection string needs `SharedAccessKeyName` and `SharedAccessKey`, or `SharedAccessSignature`".into(),
            ));
        }
        Ok(Self {
            host,
            port: url.port(),
            shared_access_key_name: key_name,
            shared_access_key: key,
            shared_access_signature: sas,
            entity_path,
            use_development_emulator: emulator,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn portal_namespace_string() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://MyNs.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=abc+def/123=",
        )
        .unwrap();
        assert_eq!(cs.host, "myns.servicebus.windows.net");
        assert_eq!(cs.port, None);
        assert_eq!(
            cs.shared_access_key_name.as_deref(),
            Some("RootManageSharedAccessKey")
        );
        assert_eq!(
            cs.shared_access_key.unwrap().expose_secret(),
            "abc+def/123="
        );
        assert!(!cs.use_development_emulator);
    }

    #[test]
    fn trailing_semicolon_and_mixed_case_keys() {
        let cs = ConnectionString::parse(
            "endpoint=sb://ns.servicebus.windows.net;sharedaccesskeyname=k;SHAREDACCESSKEY=v;",
        )
        .unwrap();
        assert_eq!(cs.host, "ns.servicebus.windows.net");
    }

    #[test]
    fn entity_path_is_kept() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=orders",
        )
        .unwrap();
        assert_eq!(cs.entity_path.as_deref(), Some("orders"));
    }

    #[test]
    fn presigned_sas_is_enough() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessSignature=SharedAccessSignature sr=x&sig=y&se=1&skn=k",
        )
        .unwrap();
        assert!(cs.shared_access_signature.is_some());
        assert!(cs.shared_access_key.is_none());
    }

    #[test]
    fn emulator_with_port() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://localhost:5300;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;",
        )
        .unwrap();
        assert_eq!(cs.host, "localhost");
        assert_eq!(cs.port, Some(5300));
        assert!(cs.use_development_emulator);
    }

    #[test]
    fn missing_endpoint_is_named() {
        let err = ConnectionString::parse("SharedAccessKeyName=k;SharedAccessKey=v").unwrap_err();
        assert!(err.to_string().contains("Endpoint="), "{err}");
    }

    #[test]
    fn missing_key_is_named() {
        let err = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k",
        )
        .unwrap_err();
        assert!(err.to_string().contains("SharedAccessKey"), "{err}");
    }

    #[test]
    fn garbage_segment_is_rejected_without_echoing_secrets() {
        let err = ConnectionString::parse(
            "Endpoint=sb://ns/;thisisnotakeyvaluepairbutlongsecretmaterial",
        )
        .unwrap_err();
        assert!(!err.to_string().contains("secretmaterial"), "{err}");
    }
}
