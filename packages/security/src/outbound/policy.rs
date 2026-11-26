//! Which addresses a client may connect to.

use std::net::IpAddr;

use copilot_core::{NetworkScope, is_allowed, is_allowed_plaintext};

/// The address rule of one client. The resolver and the peer check both apply it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Policy {
    /// An `https` client: core's [`is_allowed`] for the scope.
    Scope(NetworkScope),
    /// The plain `http` client: private and loopback addresses only ([`is_allowed_plaintext`]),
    /// so a key never travels in the clear to a public address. Only a connection with
    /// [`NetworkScope::Private`] gets this client.
    Plaintext,
}

impl Policy {
    pub(crate) fn allows(self, ip: IpAddr) -> bool {
        match self {
            Self::Scope(scope) => is_allowed(ip, scope),
            Self::Plaintext => is_allowed_plaintext(ip, NetworkScope::Private),
        }
    }

    /// `https` clients refuse plain `http` URLs outright.
    pub(crate) fn https_only(self) -> bool {
        matches!(self, Self::Scope(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("a valid test address")
    }

    /// Every address class against every client: `(address, public, private, plaintext)`.
    #[test]
    fn applies_the_core_policy_per_client() {
        let cases = [
            // Public
            ("8.8.8.8", true, true, false),
            ("2606:4700:4700::1111", true, true, false),
            ("::ffff:8.8.8.8", true, true, false),
            ("64:ff9b::808:808", true, true, false),
            ("2002:808:808::1", true, true, false),
            // Private and loopback, including CGNAT and IPv6 forms that carry them
            ("10.0.0.1", false, true, true),
            ("172.16.0.1", false, true, true),
            ("192.168.1.1", false, true, true),
            ("127.0.0.1", false, true, true),
            ("::1", false, true, true),
            ("fd12::1", false, true, true),
            ("100.64.0.1", false, true, true),
            ("100.127.255.254", false, true, true),
            ("::ffff:127.0.0.1", false, true, true),
            ("::ffff:10.0.0.1", false, true, true),
            ("64:ff9b::a00:1", false, true, true),
            ("2002:a00:1::1", false, true, true),
            ("::7f00:1", false, true, true),
            // Always blocked: link-local, metadata, unspecified, multicast, broadcast
            ("169.254.169.254", false, false, false),
            ("169.254.0.1", false, false, false),
            ("fe80::1", false, false, false),
            ("fd00:ec2::254", false, false, false),
            ("100.100.100.200", false, false, false),
            ("0.0.0.0", false, false, false),
            ("::", false, false, false),
            ("224.0.0.1", false, false, false),
            ("ff02::1", false, false, false),
            ("255.255.255.255", false, false, false),
            ("::ffff:169.254.169.254", false, false, false),
            ("64:ff9b::a9fe:a9fe", false, false, false),
            ("2002:a9fe:a9fe::1", false, false, false),
            ("::a9fe:a9fe", false, false, false),
            ("64:ff9b:1::a00:1", false, false, false),
        ];
        let public = Policy::Scope(NetworkScope::Public);
        let private = Policy::Scope(NetworkScope::Private);
        for (address, in_public, in_private, in_plaintext) in cases {
            let address_ip = ip(address);
            assert_eq!(public.allows(address_ip), in_public, "public {address}");
            assert_eq!(private.allows(address_ip), in_private, "private {address}");
            assert_eq!(
                Policy::Plaintext.allows(address_ip),
                in_plaintext,
                "plaintext {address}"
            );
        }
    }

    #[test]
    fn only_the_plaintext_client_takes_http() {
        assert!(Policy::Scope(NetworkScope::Public).https_only());
        assert!(Policy::Scope(NetworkScope::Private).https_only());
        assert!(!Policy::Plaintext.https_only());
    }
}
