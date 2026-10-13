//! Credentials for Service Bus, as tokens per audience (CBS for AMQP,
//! `Authorization` header for the management API).

use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;

/// Entra scope for Service Bus data and management-plane (ATOM) access.
pub const SERVICEBUS_SCOPE: &str = "https://servicebus.azure.net/.default";

const SAS_TTL: Duration = Duration::from_secs(60 * 60);

/// The kind of token, which decides the CBS token type and HTTP header format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Entra access token.
    Jwt,
    /// Shared Access Signature.
    Sas,
}

impl TokenKind {
    /// CBS `type` application property for `put-token`.
    pub fn cbs_type(self) -> &'static str {
        match self {
            TokenKind::Jwt => "jwt",
            TokenKind::Sas => "servicebus.windows.net:sastoken",
        }
    }
}

/// A token valid for one audience.
#[derive(Clone)]
pub struct ServiceBusToken {
    /// Raw token (`eyJ...` or `SharedAccessSignature sr=...`).
    pub token: SecretString,
    /// Token kind.
    pub kind: TokenKind,
    /// Expiry (UTC).
    pub expires_on: OffsetDateTime,
}

impl ServiceBusToken {
    /// Value for an HTTP `Authorization` header.
    pub fn http_authorization(&self) -> String {
        match self.kind {
            TokenKind::Jwt => format!("Bearer {}", self.token.expose_secret()),
            TokenKind::Sas => self.token.expose_secret().to_string(),
        }
    }
}

/// Produces tokens for Service Bus audiences.
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// Token for `audience` (e.g. `amqps://ns.servicebus.windows.net/orders`).
    async fn token(&self, audience: &str) -> Result<ServiceBusToken>;
    /// Human description of the auth path, e.g. "SAS key RootManageSharedAccessKey".
    fn describe(&self) -> String;
}

/// Signs SAS tokens locally from a key.
pub struct SasKeySource {
    key_name: String,
    key: SecretString,
}

impl SasKeySource {
    /// Build from a key name and key.
    pub fn new(key_name: impl Into<String>, key: SecretString) -> Self {
        Self {
            key_name: key_name.into(),
            key,
        }
    }
}

#[async_trait]
impl TokenSource for SasKeySource {
    async fn token(&self, audience: &str) -> Result<ServiceBusToken> {
        let (token, expires_on) = horizon_azure_kit::sas::sas_token(
            audience,
            &self.key_name,
            self.key.expose_secret(),
            SAS_TTL,
        );
        Ok(ServiceBusToken {
            token: SecretString::from(token),
            kind: TokenKind::Sas,
            expires_on,
        })
    }

    fn describe(&self) -> String {
        format!("SAS key {}", self.key_name)
    }
}

/// A pre-signed SAS from a connection string. Valid until its `se=` expiry.
pub struct PresignedSasSource {
    token: SecretString,
    expires_on: OffsetDateTime,
}

impl PresignedSasSource {
    /// Parse `SharedAccessSignature sr=..&sig=..&se=..&skn=..` (the prefix is optional).
    pub fn parse(token: SecretString) -> Result<Self> {
        let raw = token.expose_secret().trim();
        let full = if raw.starts_with("SharedAccessSignature ") {
            raw.to_string()
        } else {
            format!("SharedAccessSignature {raw}")
        };
        let se = full
            .trim_start_matches("SharedAccessSignature ")
            .split('&')
            .find_map(|kv| kv.strip_prefix("se="))
            .ok_or_else(|| Error::Invalid("SharedAccessSignature has no `se=` expiry".into()))?;
        let unix: i64 = se.parse().map_err(|_| {
            Error::Invalid(format!(
                "SharedAccessSignature expiry `{se}` is not a number"
            ))
        })?;
        let expires_on = OffsetDateTime::from_unix_timestamp(unix).map_err(|_| {
            Error::Invalid(format!(
                "SharedAccessSignature expiry `{se}` is out of range"
            ))
        })?;
        Ok(Self {
            token: SecretString::from(full),
            expires_on,
        })
    }
}

#[async_trait]
impl TokenSource for PresignedSasSource {
    async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
        if self.expires_on <= OffsetDateTime::now_utc() {
            return Err(Error::Auth(
                "the SharedAccessSignature in this connection string has expired".into(),
            ));
        }
        Ok(ServiceBusToken {
            token: self.token.clone(),
            kind: TokenKind::Sas,
            expires_on: self.expires_on,
        })
    }

    fn describe(&self) -> String {
        "pre-signed SAS".into()
    }
}

/// Entra ID tokens from a kit `EntraSession`.
pub struct EntraSource {
    session: Arc<EntraSession>,
    sign_in: SignIn,
    tenant: Option<String>,
}

impl EntraSource {
    /// Use `sign_in`, optionally in a tenant other than its home tenant.
    pub fn new(session: Arc<EntraSession>, sign_in: SignIn, tenant: Option<String>) -> Self {
        Self {
            session,
            sign_in,
            tenant,
        }
    }
}

#[async_trait]
impl TokenSource for EntraSource {
    async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
        let t = self
            .session
            .token_for(&self.sign_in, self.tenant.as_deref(), SERVICEBUS_SCOPE)
            .await?;
        Ok(ServiceBusToken {
            token: t.token,
            kind: TokenKind::Jwt,
            expires_on: t.expires_on,
        })
    }

    fn describe(&self) -> String {
        format!("Entra ID {}", self.sign_in.user_principal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sas_key_source_signs_audience() {
        let src = SasKeySource::new(
            "RootManageSharedAccessKey",
            SecretString::from("SAS_KEY_VALUE"),
        );
        let t = src.token("amqp://localhost/orders").await.unwrap();
        assert_eq!(t.kind, TokenKind::Sas);
        assert!(
            t.http_authorization()
                .starts_with("SharedAccessSignature sr=amqp%3a%2f%2flocalhost%2forders&")
        );
        assert_eq!(src.describe(), "SAS key RootManageSharedAccessKey");
    }

    #[tokio::test]
    async fn presigned_sas_parses_expiry_and_rejects_expired() {
        let future = OffsetDateTime::now_utc().unix_timestamp() + 600;
        let ok =
            PresignedSasSource::parse(SecretString::from(format!("sr=x&sig=y&se={future}&skn=k")))
                .unwrap();
        let t = ok.token("ignored").await.unwrap();
        assert!(
            t.http_authorization()
                .starts_with("SharedAccessSignature sr=x")
        );
        let expired = PresignedSasSource::parse(SecretString::from(
            "SharedAccessSignature sr=x&sig=y&se=1&skn=k",
        ))
        .unwrap();
        assert!(matches!(
            expired.token("a").await.err(),
            Some(Error::Auth(_))
        ));
    }

    #[test]
    fn cbs_types() {
        assert_eq!(TokenKind::Jwt.cbs_type(), "jwt");
        assert_eq!(TokenKind::Sas.cbs_type(), "servicebus.windows.net:sastoken");
    }
}
