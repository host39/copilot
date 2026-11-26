//! The process-wide rustls crypto provider.
//!
//! reqwest uses `rustls-no-provider`, so it panics when a client is built before a provider is
//! installed. Copilot installs ring (sqlx already uses it), which keeps aws-lc-rs out of the tree.

use std::sync::Once;

static INSTALL: Once = Once::new();

/// Installs ring as the process default rustls provider, once. [`crate::Outbound::build`] calls
/// it, so a client never exists without a provider; binaries may call it earlier too. A provider
/// installed before the first call stays in place.
pub fn install() {
    INSTALL.call_once(|| {
        if rustls::crypto::ring::default_provider()
            .install_default()
            .is_err()
        {
            tracing::debug!("security.crypto.provider.present");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_a_default_provider_once() {
        install();
        install();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
