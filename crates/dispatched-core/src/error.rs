//! Dispatched error type. Messages say what to do, not just what failed.

use horizon_azure_kit::KitError;
use std::time::Duration;

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors surfaced to every Dispatched surface (GUI, CLI, MCP).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Credentials were rejected.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// No usable Entra session; the user must sign in again.
    #[error("not signed in: {0}")]
    NotSignedIn(String),
    /// The identity lacks a right. `needed` names it when the service says so.
    #[error("access denied{}: {detail}", needed.as_deref().map(|n| format!(" (needs {n})")).unwrap_or_default())]
    Forbidden {
        /// Missing rights, e.g. `Manage,EntityRead` or `Listen`.
        needed: Option<String>,
        /// Service-provided detail.
        detail: String,
    },
    /// Entity or resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The service is throttling; retry later.
    #[error("the service is busy{}; try again", retry_after.map(|d| format!(" (retry after {}s)", d.as_secs())).unwrap_or_default())]
    Throttled {
        /// Server-suggested back-off.
        retry_after: Option<Duration>,
    },
    /// An operation exceeded its deadline.
    #[error("timed out: {0}")]
    Timeout(String),
    /// Cancelled by the user.
    #[error("cancelled")]
    Cancelled,
    /// The target (e.g. the emulator) does not support this capability.
    #[error("not supported here: {capability}")]
    Unsupported {
        /// What is unsupported.
        capability: String,
    },
    /// Invalid user input (connection string, entity path, ...).
    #[error("{0}")]
    Invalid(String),
    /// Network / transport failure.
    #[error("network error: {0}")]
    Network(String),
    /// Unexpected protocol response.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// Local input/output failure (writing an export file, ...).
    #[error("{0}")]
    Io(String),
}

impl From<KitError> for Error {
    fn from(e: KitError) -> Self {
        match e {
            KitError::AuthFailed(m) => Error::Auth(m),
            KitError::NotSignedIn(m) => Error::NotSignedIn(m),
            KitError::NotFound(m) => Error::NotFound(m),
            KitError::Forbidden(m) => Error::Forbidden {
                needed: None,
                detail: m,
            },
            KitError::Network(m) => Error::Network(m),
            KitError::Remote { status, body } => match status {
                429 | 503 => Error::Throttled { retry_after: None },
                408 | 504 => Error::Timeout(format!("service returned {status}")),
                _ => Error::Protocol(format!("service returned {status}: {body}")),
            },
            KitError::SecretStore(m) => Error::Auth(format!(
                "the OS keychain is unavailable ({m}); on Linux start a Secret Service such as gnome-keyring"
            )),
            KitError::PolicyDenied(m) => Error::Invalid(format!("denied: {m}")),
            KitError::Cancelled => Error::Cancelled,
            KitError::Invalid(m) => Error::Invalid(m),
            other => Error::Protocol(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_names_the_missing_right() {
        let e = Error::Forbidden {
            needed: Some("Listen".into()),
            detail: "Unauthorized".into(),
        };
        assert_eq!(e.to_string(), "access denied (needs Listen): Unauthorized");
    }

    #[test]
    fn remote_throttling_and_timeout_statuses_are_mapped() {
        let busy: Error = KitError::Remote {
            status: 429,
            body: "slow down".into(),
        }
        .into();
        assert!(matches!(busy, Error::Throttled { retry_after: None }));
        let unavailable: Error = KitError::Remote {
            status: 503,
            body: String::new(),
        }
        .into();
        assert!(matches!(unavailable, Error::Throttled { .. }));
        let gateway: Error = KitError::Remote {
            status: 504,
            body: String::new(),
        }
        .into();
        assert!(matches!(gateway, Error::Timeout(ref m) if m == "service returned 504"));
        let request: Error = KitError::Remote {
            status: 408,
            body: String::new(),
        }
        .into();
        assert!(matches!(request, Error::Timeout(_)));
        let other: Error = KitError::Remote {
            status: 500,
            body: "boom".into(),
        }
        .into();
        assert!(matches!(other, Error::Protocol(ref m) if m == "service returned 500: boom"));
    }

    #[test]
    fn secret_store_failure_says_what_to_do() {
        let e: Error = KitError::SecretStore("no secret service".into()).into();
        assert!(
            matches!(e, Error::Auth(ref m) if m.contains("no secret service") && m.contains("gnome-keyring"))
        );
    }

    #[test]
    fn kit_not_signed_in_is_preserved() {
        let e: Error = KitError::NotSignedIn("sign in again".into()).into();
        assert!(matches!(e, Error::NotSignedIn(_)));
    }
}
