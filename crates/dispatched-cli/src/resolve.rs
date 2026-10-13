//! Turn CLI flags into a `Namespace`.

use crate::state::CliState;
use anyhow::{Context, bail};
use dispatched_core::tokens::EntraSource;
use dispatched_core::{ConnectionString, Namespace};
use horizon_azure_kit::auth::EntraSession;
use std::sync::Arc;

pub fn namespace_host(input: &str) -> String {
    let input = input.trim().to_ascii_lowercase();
    if input.contains('.') {
        input
    } else {
        format!("{input}.servicebus.windows.net")
    }
}

pub fn namespace(
    connection_string: Option<&str>,
    namespace: Option<&str>,
    session: &Arc<EntraSession>,
    state: &CliState,
) -> anyhow::Result<Namespace> {
    if let Some(cs) = connection_string {
        return Ok(Namespace::from_connection_string(
            &ConnectionString::parse(cs)?,
        )?);
    }
    if let Some(ns) = namespace {
        let sign_in = state
            .sign_in
            .clone()
            .context("not signed in: run `dispatched login`, or pass --connection-string")?;
        let tokens = Arc::new(EntraSource::new(session.clone(), sign_in, None));
        return Ok(Namespace::new(namespace_host(ns), tokens));
    }
    bail!(
        "no namespace: pass --connection-string (or set DISPATCHED_CONNECTION_STRING), or --namespace after `dispatched login`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_expand_to_public_cloud_host() {
        assert_eq!(namespace_host("MyNs"), "myns.servicebus.windows.net");
        assert_eq!(
            namespace_host("x.servicebus.chinacloudapi.cn"),
            "x.servicebus.chinacloudapi.cn"
        );
    }

    #[test]
    fn namespace_without_sign_in_explains_what_to_do() {
        let session = crate::state::entra_session();
        let err = namespace(None, Some("myns"), &session, &CliState::default()).unwrap_err();
        assert!(err.to_string().contains("dispatched login"));
    }

    #[test]
    fn nothing_given_explains_both_options() {
        let session = crate::state::entra_session();
        let err = namespace(None, None, &session, &CliState::default()).unwrap_err();
        assert!(err.to_string().contains("--connection-string"));
    }
}
