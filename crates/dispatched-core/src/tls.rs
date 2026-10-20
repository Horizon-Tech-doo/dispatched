//! The process's TLS crypto provider.

use std::sync::Once;

/// Install aws-lc-rs as the process's rustls crypto provider, once, before any TLS. A build that
/// links both rustls providers (the desktop app: a Tauri dependency brings `ring`) cannot pick
/// one by itself, and its first TLS connection panics. Installing it when another provider is
/// already installed changes nothing.
pub fn ensure_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crypto_provider_is_installed_before_tls() {
        ensure_crypto_provider();
        ensure_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        // A client configuration builds without choosing a provider by hand.
        let _ = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
    }
}
