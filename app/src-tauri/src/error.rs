use dispatched_core::Error;
use serde::Serialize;

/// Error shape returned to the frontend.
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        let kind = match &e {
            Error::Auth(_) => "auth",
            Error::NotSignedIn(_) => "not_signed_in",
            Error::Forbidden { .. } => "forbidden",
            Error::NotFound(_) => "not_found",
            Error::Throttled { .. } => "throttled",
            Error::Timeout(_) => "timeout",
            Error::Cancelled => "cancelled",
            Error::Unsupported { .. } => "unsupported",
            Error::Invalid(_) => "invalid",
            Error::Network(_) => "network",
            _ => "protocol",
        };
        Self {
            kind,
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_maps_to_kind_and_message() {
        let e: CommandError = Error::Forbidden {
            needed: Some("Listen".into()),
            detail: "no".into(),
        }
        .into();
        assert_eq!(e.kind, "forbidden");
        assert!(e.message.contains("needs Listen"));
    }
}
