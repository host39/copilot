//! The DNS resolver of the outbound clients.

use std::net::SocketAddr;

use copilot_core::{Blocked, BoxError};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::outbound::policy::Policy;

/// Resolves with the system resolver and keeps only the addresses the client's [`Policy`]
/// allows. reqwest connects to exactly the addresses returned here, so the check and the
/// connection use the same IPs (no DNS rebinding window). A name with no allowed address fails
/// with [`Blocked::Address`]. IP-literal hosts never reach a resolver; the peer check in
/// [`crate::outbound::peer`] covers them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resolver {
    policy: Policy,
}

impl Resolver {
    pub(crate) fn new(policy: Policy) -> Self {
        Self { policy }
    }
}

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.policy;
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let allowed = admit(policy, found);
            if allowed.is_empty() {
                tracing::debug!(host, "security.outbound.dns.blocked");
                return Err(Box::new(Blocked::Address) as BoxError);
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

/// The addresses `policy` allows, in the resolver's order.
pub(crate) fn admit(
    policy: Policy,
    found: impl IntoIterator<Item = SocketAddr>,
) -> Vec<SocketAddr> {
    found
        .into_iter()
        .filter(|address| policy.allows(address.ip()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use copilot_core::NetworkScope;

    fn addresses(list: &[&str]) -> Vec<SocketAddr> {
        list.iter()
            .map(|text| SocketAddr::new(text.parse().expect("a valid test address"), 0))
            .collect()
    }

    #[test]
    fn keeps_only_allowed_addresses() {
        let found = addresses(&["10.0.0.1", "8.8.8.8", "169.254.169.254", "::1"]);
        assert_eq!(
            admit(Policy::Scope(NetworkScope::Public), found.clone()),
            addresses(&["8.8.8.8"])
        );
        assert_eq!(
            admit(Policy::Scope(NetworkScope::Private), found.clone()),
            addresses(&["10.0.0.1", "8.8.8.8", "::1"])
        );
        assert_eq!(
            admit(Policy::Plaintext, found),
            addresses(&["10.0.0.1", "::1"])
        );
    }

    #[tokio::test]
    async fn refuses_a_name_with_no_allowed_address() {
        let resolver = Resolver::new(Policy::Scope(NetworkScope::Public));
        let error = match resolver
            .resolve("localhost".parse().expect("a valid name"))
            .await
        {
            Ok(_) => panic!("localhost must not resolve for a public-only client"),
            Err(error) => error,
        };
        assert_eq!(Blocked::find(error.as_ref()), Some(Blocked::Address));
    }

    #[tokio::test]
    async fn resolves_allowed_names() {
        let resolver = Resolver::new(Policy::Plaintext);
        let found: Vec<SocketAddr> = match resolver
            .resolve("localhost".parse().expect("a valid name"))
            .await
        {
            Ok(addresses) => addresses.collect(),
            Err(error) => panic!("localhost resolves for the plaintext client: {error}"),
        };
        assert!(!found.is_empty());
        assert!(found.iter().all(|address| address.ip().is_loopback()));
    }
}
