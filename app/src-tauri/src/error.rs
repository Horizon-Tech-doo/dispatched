use dispatched_core::Error;
use serde::Serialize;

/// Error shape returned to the frontend: a stable `kind` the UI branches on
/// and a sentence for people. Never contains secrets (core errors don't).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        Self {
            kind: dispatched_core::diagnostics::error_kind(&e),
            message: e.to_string(),
        }
    }
}

impl From<horizon_azure_kit::KitError> for CommandError {
    fn from(e: horizon_azure_kit::KitError) -> Self {
        Error::from(e).into()
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

    #[test]
    fn kit_errors_map_through_core() {
        let e: CommandError = horizon_azure_kit::KitError::NotSignedIn("expired".into()).into();
        assert_eq!(e.kind, "not_signed_in");
    }

    #[test]
    fn io_and_protocol_have_their_own_kinds() {
        assert_eq!(CommandError::from(Error::Io("disk".into())).kind, "io");
        assert_eq!(
            CommandError::from(Error::Protocol("x".into())).kind,
            "protocol"
        );
    }
}
