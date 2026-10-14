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
        let kind = dispatched_core::diagnostics::error_kind(&e);
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

    #[test]
    fn io_and_protocol_have_their_own_kinds() {
        assert_eq!(CommandError::from(Error::Io("disk".into())).kind, "io");
        assert_eq!(
            CommandError::from(Error::Protocol("x".into())).kind,
            "protocol"
        );
    }
}
