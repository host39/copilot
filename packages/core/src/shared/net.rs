//! Network scope and the outbound address policy.
//!
//! `ALLOW_PRIVATE_NETWORK` ([`PrivateNetwork`]) and the caller's `is_admin` decide a connection's
//! [`NetworkScope`]. [`is_allowed`] is the pure address check: `copilot-security` calls it from the
//! outbound client's DNS resolver and for IP-literal hosts, and [`crate::shared::base_url`] calls
//! it when a URL is saved.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::shared::actor::Actor;

/// Which connections may reach private and loopback addresses (`ALLOW_PRIVATE_NETWORK`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrivateNetwork {
    /// Nobody.
    None,
    /// Connections owned by an administrator (the default).
    #[default]
    Admins,
    /// Everyone.
    All,
}

impl PrivateNetwork {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Admins => "admins",
            Self::All => "all",
        }
    }

    /// The scope of a connection owned by `owner`.
    pub fn scope(self, owner: &Actor) -> NetworkScope {
        match self {
            Self::All => NetworkScope::Private,
            Self::Admins if owner.is_admin => NetworkScope::Private,
            Self::Admins | Self::None => NetworkScope::Public,
        }
    }
}

impl fmt::Display for PrivateNetwork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PrivateNetwork {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "admins" => Ok(Self::Admins),
            "all" => Ok(Self::All),
            other => Err(other.to_owned()),
        }
    }
}

/// Where a connection may go: public addresses only, or private and loopback ones too.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NetworkScope {
    Public,
    Private,
}

/// What an address is, for the policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressClass {
    /// Reachable by every connection.
    Public,
    /// RFC 1918, loopback, unique local (`fc00::/7`), and CGNAT (`100.64.0.0/10`): needs
    /// [`NetworkScope::Private`].
    Private,
    /// Never reachable: link-local and cloud metadata addresses, `0.0.0.0/8`, `::`, multicast,
    /// and broadcast.
    Blocked,
}

/// Classifies an address. IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is classified as its IPv4 address.
pub fn classify(ip: IpAddr) -> AddressClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => classify_v4(v4),
            None => classify_v6(v6),
        },
    }
}

/// Whether a connection with `scope` may connect to `ip`.
pub fn is_allowed(ip: IpAddr, scope: NetworkScope) -> bool {
    match classify(ip) {
        AddressClass::Public => true,
        AddressClass::Private => scope == NetworkScope::Private,
        AddressClass::Blocked => false,
    }
}

/// The Alibaba Cloud metadata address, inside the CGNAT range but always blocked.
const ALIBABA_METADATA: Ipv4Addr = Ipv4Addr::new(100, 100, 100, 200);
/// The AWS IPv6 metadata address, inside `fc00::/7` but always blocked.
const AWS_METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);

fn classify_v4(ip: Ipv4Addr) -> AddressClass {
    let [first, second, ..] = ip.octets();
    if first == 0
        || ip.is_link_local()
        || ip == ALIBABA_METADATA
        || ip.is_multicast()
        || ip.is_broadcast()
    {
        return AddressClass::Blocked;
    }
    let cgnat = first == 100 && (second & 0b1100_0000) == 0b0100_0000;
    if ip.is_private() || ip.is_loopback() || cgnat {
        return AddressClass::Private;
    }
    AddressClass::Public
}

fn classify_v6(ip: Ipv6Addr) -> AddressClass {
    let first = ip.segments()[0];
    let link_local = (first & 0xffc0) == 0xfe80;
    if ip.is_unspecified() || link_local || ip == AWS_METADATA_V6 || ip.is_multicast() {
        return AddressClass::Blocked;
    }
    let unique_local = (first & 0xfe00) == 0xfc00;
    if ip.is_loopback() || unique_local {
        return AddressClass::Private;
    }
    AddressClass::Public
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::ids::UserId;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("a valid test address")
    }

    #[test]
    fn classifies_addresses() {
        let cases = [
            ("8.8.8.8", AddressClass::Public),
            ("1.1.1.1", AddressClass::Public),
            ("2606:4700:4700::1111", AddressClass::Public),
            ("10.0.0.1", AddressClass::Private),
            ("172.16.5.4", AddressClass::Private),
            ("172.31.255.255", AddressClass::Private),
            ("172.32.0.1", AddressClass::Public),
            ("192.168.1.10", AddressClass::Private),
            ("127.0.0.1", AddressClass::Private),
            ("127.8.9.10", AddressClass::Private),
            ("::1", AddressClass::Private),
            ("fc00::1", AddressClass::Private),
            ("fd12:3456::1", AddressClass::Private),
            ("100.64.0.1", AddressClass::Private),
            ("100.127.255.254", AddressClass::Private),
            ("100.128.0.1", AddressClass::Public),
            ("169.254.169.254", AddressClass::Blocked),
            ("169.254.0.1", AddressClass::Blocked),
            ("fe80::1", AddressClass::Blocked),
            ("febf::1", AddressClass::Blocked),
            ("fd00:ec2::254", AddressClass::Blocked),
            ("100.100.100.200", AddressClass::Blocked),
            ("0.0.0.0", AddressClass::Blocked),
            ("0.1.2.3", AddressClass::Blocked),
            ("::", AddressClass::Blocked),
            ("224.0.0.1", AddressClass::Blocked),
            ("239.255.255.250", AddressClass::Blocked),
            ("ff02::1", AddressClass::Blocked),
            ("255.255.255.255", AddressClass::Blocked),
        ];
        for (address, expected) in cases {
            assert_eq!(classify(ip(address)), expected, "{address}");
        }
    }

    #[test]
    fn normalizes_ipv4_mapped_ipv6() {
        assert_eq!(classify(ip("::ffff:127.0.0.1")), AddressClass::Private);
        assert_eq!(
            classify(ip("::ffff:169.254.169.254")),
            AddressClass::Blocked
        );
        assert_eq!(classify(ip("::ffff:10.1.2.3")), AddressClass::Private);
        assert_eq!(classify(ip("::ffff:8.8.8.8")), AddressClass::Public);
    }

    #[test]
    fn allows_private_addresses_only_in_private_scope() {
        assert!(is_allowed(ip("8.8.8.8"), NetworkScope::Public));
        assert!(!is_allowed(ip("192.168.1.2"), NetworkScope::Public));
        assert!(is_allowed(ip("192.168.1.2"), NetworkScope::Private));
        assert!(!is_allowed(ip("169.254.169.254"), NetworkScope::Private));
        assert!(!is_allowed(ip("fd00:ec2::254"), NetworkScope::Private));
    }

    #[test]
    fn derives_the_scope_from_the_policy_and_the_owner() {
        let user = Actor::user(UserId::new(uuid::Uuid::now_v7()));
        let admin = Actor::admin(UserId::new(uuid::Uuid::now_v7()));
        assert_eq!(PrivateNetwork::None.scope(&admin), NetworkScope::Public);
        assert_eq!(PrivateNetwork::Admins.scope(&user), NetworkScope::Public);
        assert_eq!(PrivateNetwork::Admins.scope(&admin), NetworkScope::Private);
        assert_eq!(PrivateNetwork::All.scope(&user), NetworkScope::Private);
    }

    #[test]
    fn parses_the_setting() {
        assert_eq!(PrivateNetwork::default(), PrivateNetwork::Admins);
        for policy in [
            PrivateNetwork::None,
            PrivateNetwork::Admins,
            PrivateNetwork::All,
        ] {
            assert_eq!(policy.as_str().parse::<PrivateNetwork>(), Ok(policy));
        }
        assert!("some".parse::<PrivateNetwork>().is_err());
    }
}
