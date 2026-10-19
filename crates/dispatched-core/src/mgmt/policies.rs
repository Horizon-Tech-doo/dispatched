//! Entity SAS policies: add, remove and regenerate keys on a [`Description`], which the caller
//! then writes back with an update (read-modify-write). Subscriptions have no policies.

use super::model::{AccessPolicy, AccessRight, Description};
use crate::{Error, Result};
use base64::Engine;
use secrecy::SecretString;
use serde::Serialize;

/// The most SAS policies one entity may have (a service limit).
pub const MAX_POLICIES: usize = 12;

/// Which key of a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySlot {
    /// The primary key.
    Primary,
    /// The secondary key.
    Secondary,
}

impl KeySlot {
    /// `primary` or `secondary`.
    pub fn as_str(self) -> &'static str {
        match self {
            KeySlot::Primary => "primary",
            KeySlot::Secondary => "secondary",
        }
    }
}

/// A policy without its keys: what listings show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyView {
    /// Policy name.
    pub key_name: String,
    /// Granted rights.
    pub rights: Vec<AccessRight>,
}

impl From<&AccessPolicy> for PolicyView {
    fn from(p: &AccessPolicy) -> Self {
        Self {
            key_name: p.key_name.clone(),
            rights: p.rights.clone(),
        }
    }
}

/// The policies without their keys.
pub fn views(policies: &[AccessPolicy]) -> Vec<PolicyView> {
    policies.iter().map(PolicyView::from).collect()
}

/// A new key: 32 random bytes from the OS, base64, as the service generates them.
pub fn new_key() -> SecretString {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random number generator is available");
    SecretString::from(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn position<T>(d: &Description<T>, name: &str) -> Option<usize> {
    d.policies
        .iter()
        .position(|p| p.key_name.eq_ignore_ascii_case(name))
}

fn not_found(name: &str) -> Error {
    Error::NotFound(format!("policy {name}"))
}

/// Add policy `name` with `rights` and two new keys. Refuses an empty or taken name (names are
/// compared ignoring case), no rights, `Manage` without `Listen` and `Send` (the service
/// requires both), and more than [`MAX_POLICIES`].
pub fn add_policy<T>(d: &mut Description<T>, name: &str, rights: &[AccessRight]) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("a policy needs a name".into()));
    }
    if position(d, name).is_some() {
        return Err(Error::Conflict(format!("policy {name} already exists")));
    }
    let mut rights = rights.to_vec();
    rights.sort();
    rights.dedup();
    if rights.is_empty() {
        return Err(Error::Invalid(format!(
            "policy {name} needs at least one right: listen, send or manage"
        )));
    }
    if rights.contains(&AccessRight::Manage)
        && !(rights.contains(&AccessRight::Listen) && rights.contains(&AccessRight::Send))
    {
        return Err(Error::Invalid(
            "the Manage right needs Listen and Send as well; use --rights listen,send,manage"
                .into(),
        ));
    }
    if d.policies.len() >= MAX_POLICIES {
        return Err(Error::Invalid(format!(
            "an entity has at most {MAX_POLICIES} policies; remove one first"
        )));
    }
    d.policies.push(AccessPolicy {
        key_name: name.to_string(),
        rights,
        primary_key: new_key(),
        secondary_key: new_key(),
    });
    Ok(())
}

/// Remove policy `name`. A missing one is `NotFound`.
pub fn remove_policy<T>(d: &mut Description<T>, name: &str) -> Result<()> {
    let at = position(d, name).ok_or_else(|| not_found(name))?;
    d.policies.remove(at);
    Ok(())
}

/// Replace one key of policy `name` with a new one; the other key is unchanged.
pub fn regenerate<T>(d: &mut Description<T>, name: &str, which: KeySlot) -> Result<()> {
    let at = position(d, name).ok_or_else(|| not_found(name))?;
    let p = &mut d.policies[at];
    match which {
        KeySlot::Primary => p.primary_key = new_key(),
        KeySlot::Secondary => p.secondary_key = new_key(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::model::{AccessRight, Description, QueueSettings};
    use secrecy::ExposeSecret;

    fn queue() -> Description<QueueSettings> {
        Description::new(QueueSettings::default())
    }

    #[test]
    fn add_then_remove_policy() {
        let mut d = queue();
        add_policy(&mut d, "app", &[AccessRight::Send, AccessRight::Listen]).unwrap();
        assert_eq!(d.policies.len(), 1);
        let p = &d.policies[0];
        assert_eq!(p.key_name, "app");
        assert_eq!(p.rights, [AccessRight::Listen, AccessRight::Send]);
        assert_ne!(
            p.primary_key.expose_secret(),
            p.secondary_key.expose_secret()
        );
        let e = add_policy(&mut d, "APP", &[AccessRight::Listen]).unwrap_err();
        assert!(e.to_string().contains("already"), "{e}");
        assert!(add_policy(&mut d, " ", &[AccessRight::Listen]).is_err());
        assert!(add_policy(&mut d, "none", &[]).is_err());
        remove_policy(&mut d, "app").unwrap();
        assert!(d.policies.is_empty());
        assert!(matches!(
            remove_policy(&mut d, "app"),
            Err(crate::Error::NotFound(_))
        ));
    }

    #[test]
    fn manage_needs_send_and_listen() {
        let mut d = queue();
        let e = add_policy(&mut d, "admin", &[AccessRight::Manage, AccessRight::Send]).unwrap_err();
        assert!(e.to_string().contains("Listen and Send"), "{e}");
        add_policy(
            &mut d,
            "admin",
            &[AccessRight::Manage, AccessRight::Send, AccessRight::Listen],
        )
        .unwrap();
    }

    #[test]
    fn regenerate_changes_only_that_key() {
        let mut d = queue();
        add_policy(&mut d, "app", &[AccessRight::Listen]).unwrap();
        let before = d.policies[0].clone();
        regenerate(&mut d, "app", KeySlot::Primary).unwrap();
        let after = &d.policies[0];
        assert_ne!(
            after.primary_key.expose_secret(),
            before.primary_key.expose_secret()
        );
        assert_eq!(
            after.secondary_key.expose_secret(),
            before.secondary_key.expose_secret()
        );
        regenerate(&mut d, "app", KeySlot::Secondary).unwrap();
        assert_ne!(
            d.policies[0].secondary_key.expose_secret(),
            before.secondary_key.expose_secret()
        );
        assert!(regenerate(&mut d, "ghost", KeySlot::Primary).is_err());
    }

    #[test]
    fn policy_view_has_no_keys() {
        let mut d = queue();
        add_policy(&mut d, "app", &[AccessRight::Listen, AccessRight::Send]).unwrap();
        let views = views(&d.policies);
        let json = serde_json::to_string(&views).unwrap();
        assert_eq!(json, r#"[{"key_name":"app","rights":["listen","send"]}]"#);
        for p in &d.policies {
            assert!(!json.contains(p.primary_key.expose_secret()));
            assert!(!json.contains(p.secondary_key.expose_secret()));
        }
    }

    #[test]
    fn new_keys_are_32_random_bytes() {
        use base64::Engine;
        let a = new_key();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(a.expose_secret())
            .unwrap();
        assert_eq!(bytes.len(), 32);
        assert_ne!(a.expose_secret(), new_key().expose_secret());
    }
}
