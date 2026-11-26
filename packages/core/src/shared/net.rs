//! Network scope and the outbound address policy.
//!
//! `ALLOW_PRIVATE_NETWORK` ([`PrivateNetwork`]) and the caller's `is_admin` decide a connection's
//! [`NetworkScope`]. [`is_allowed`] is the pure address check: `copilot-security` calls it from the
//! outbound clients' DNS resolver and peer check, [`Outbound::pick`] calls it for IP-literal
//! hosts, and [`crate::shared::base_url`] calls it when a URL is saved. A plain `http` URL uses
//! [`is_allowed_plaintext`] instead, so a name that only looks local never sends a key in the
//! clear to a public address.
//!
//! The provider and MCP adapters receive an [`Outbound`] of the hardened `reqwest` clients that
//! `copilot-security` builds, pick one per request with [`Outbound::pick`], and tell a refused
//! request from a failed one with [`Blocked::find`]. Both types live here because the adapters
//! may not depend on `copilot-security` (`make boundaries`).

use std::error::Error;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use url::{Host, Url};

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
    /// broadcast, and the local-use NAT64 prefix `64:ff9b:1::/48` (its IPv4 layout is the
    /// operator's choice, so the address it reaches cannot be read).
    Blocked,
}

/// Classifies an address. An IPv6 address that carries an IPv4 address a gateway on the path
/// connects to is classified as that IPv4 address: IPv4-mapped (`::ffff:a.b.c.d`), the NAT64
/// well-known prefix (`64:ff9b::/96`), 6to4 (`2002::/16`), and IPv4-compatible (`::a.b.c.d`).
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

/// Whether a plain `http` request of a connection with `scope` may go to `ip`: only a private or
/// loopback address the scope allows, never a public one. A URL is saved with `http` only for a
/// host that looks private (a private IP literal or a local name such as `ollama` or `gpu.lan`),
/// and this check keeps such a name from sending keys in the clear when it resolves publicly.
pub fn is_allowed_plaintext(ip: IpAddr, scope: NetworkScope) -> bool {
    classify(ip) == AddressClass::Private && scope == NetworkScope::Private
}

/// Why an outbound request was refused before any request bytes were sent. `Display` is a dotted
/// key, and no variant carries a resolved address, so a refusal never echoes one to a client.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Blocked {
    /// The host is, or resolves only to, an address the connection may not reach.
    #[error("outbound.address.blocked")]
    Address,
    /// Not `https`, or plain `http` for a connection that may not reach private addresses.
    #[error("outbound.scheme.blocked")]
    Scheme,
    /// The URL carries a user name or password.
    #[error("outbound.credentials.blocked")]
    Credentials,
}

impl Blocked {
    /// The refusal in an error's source chain, if the policy caused it. reqwest reports a refusal
    /// by the outbound resolver or peer check as a connect error whose source chain holds it;
    /// adapters log those as `*.http.blocked` and other failures as `*.http.failed`.
    pub fn find(error: &(dyn Error + 'static)) -> Option<Self> {
        let mut current = Some(error);
        while let Some(error) = current {
            if let Some(blocked) = error.downcast_ref::<Self>() {
                return Some(*blocked);
            }
            current = error.source();
        }
        None
    }
}

/// The three outbound clients (`docs/design/copilot.md`, Outbound Requests): public-only and
/// private-allowed `https` clients, and a plain `http` client that reaches only private and
/// loopback addresses. `copilot-security` builds them as `reqwest::Client`s.
#[derive(Clone, Debug)]
pub struct Outbound<T> {
    public: T,
    private: T,
    plaintext: T,
}

impl<T> Outbound<T> {
    pub fn new(public: T, private: T, plaintext: T) -> Self {
        Self {
            public,
            private,
            plaintext,
        }
    }

    /// The client for a request to `url` by a connection with `scope`, after the checks that need
    /// no network: `https`, or plain `http` only for a private-scope connection (and then only
    /// to private addresses); no user name or password; and an IP-literal host must pass the
    /// policy here, because no resolver sees it. The clients check every resolved and connected
    /// address again.
    pub fn pick(&self, url: &Url, scope: NetworkScope) -> Result<&T, Blocked> {
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Blocked::Credentials);
        }
        let (client, plaintext) = match (url.scheme(), scope) {
            ("https", NetworkScope::Public) => (&self.public, false),
            ("https", NetworkScope::Private) => (&self.private, false),
            ("http", NetworkScope::Private) => (&self.plaintext, true),
            _ => return Err(Blocked::Scheme),
        };
        let allowed = match literal(url) {
            Some(ip) if plaintext => is_allowed_plaintext(ip, scope),
            Some(ip) => is_allowed(ip, scope),
            None => true,
        };
        if allowed {
            Ok(client)
        } else {
            Err(Blocked::Address)
        }
    }
}

/// The host as an address, when the URL names one. The URL parser has already normalized the
/// other spellings (`2130706433`, `0x7f.1`, a trailing dot) to the dotted form.
fn literal(url: &Url) -> Option<IpAddr> {
    match url.host()? {
        Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        Host::Domain(_) => None,
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
    let segments = ip.segments();
    if segments[..3] == [0x64, 0xff9b, 1] {
        return AddressClass::Blocked;
    }
    if let Some(v4) = embedded(ip) {
        return classify_v4(v4);
    }
    let first = segments[0];
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

/// The IPv4 address a NAT64 (`64:ff9b::/96`), 6to4 (`2002::/16`), or IPv4-compatible
/// (`::a.b.c.d`, other than `::` and `::1`) address carries.
fn embedded(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let octets = ip.octets();
    let last = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
    match ip.segments() {
        [0x64, 0xff9b, 0, 0, 0, 0, _, _] => Some(last),
        [0x2002, ..] => Some(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5])),
        [0, 0, 0, 0, 0, 0, _, _] if !ip.is_unspecified() && !ip.is_loopback() => Some(last),
        _ => None,
    }
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
    fn classifies_ipv6_that_carries_an_ipv4_address_as_that_address() {
        let cases = [
            ("64:ff9b::a00:1", AddressClass::Private),
            ("64:ff9b::7f00:1", AddressClass::Private),
            ("64:ff9b::a9fe:a9fe", AddressClass::Blocked),
            ("64:ff9b::808:808", AddressClass::Public),
            ("64:ff9b:1::a00:1", AddressClass::Blocked),
            ("64:ff9b:1::808:808", AddressClass::Blocked),
            ("2002:a9fe:a9fe::1", AddressClass::Blocked),
            ("2002:a00:1::1", AddressClass::Private),
            ("2002:808:808::1", AddressClass::Public),
            ("::7f00:1", AddressClass::Private),
            ("::a9fe:a9fe", AddressClass::Blocked),
            ("::808:808", AddressClass::Public),
            ("::1", AddressClass::Private),
            ("::", AddressClass::Blocked),
        ];
        for (address, expected) in cases {
            assert_eq!(classify(ip(address)), expected, "{address}");
        }
        assert!(!is_allowed(ip("64:ff9b::a9fe:a9fe"), NetworkScope::Private));
        assert!(!is_allowed(ip("64:ff9b::a00:1"), NetworkScope::Public));
        assert!(!is_allowed(ip("2002:a00:1::1"), NetworkScope::Public));
    }

    #[test]
    fn plain_http_reaches_only_private_addresses_the_scope_allows() {
        assert!(is_allowed_plaintext(ip("10.0.0.5"), NetworkScope::Private));
        assert!(is_allowed_plaintext(ip("::1"), NetworkScope::Private));
        assert!(!is_allowed_plaintext(ip("8.8.8.8"), NetworkScope::Private));
        assert!(!is_allowed_plaintext(ip("10.0.0.5"), NetworkScope::Public));
        assert!(!is_allowed_plaintext(
            ip("169.254.169.254"),
            NetworkScope::Private
        ));
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

    fn outbound() -> Outbound<&'static str> {
        Outbound::new("public", "private", "plaintext")
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a valid test URL")
    }

    #[test]
    fn picks_the_client_by_scheme_and_scope() {
        let outbound = outbound();
        let cases = [
            ("https://api.example.com/v1", NetworkScope::Public, "public"),
            (
                "https://api.example.com/v1",
                NetworkScope::Private,
                "private",
            ),
            ("https://gpu.lan/v1", NetworkScope::Private, "private"),
            ("http://ollama:11434/v1", NetworkScope::Private, "plaintext"),
            (
                "http://10.0.0.5:1234/v1",
                NetworkScope::Private,
                "plaintext",
            ),
        ];
        for (text, scope, client) in cases {
            assert_eq!(outbound.pick(&url(text), scope), Ok(&client), "{text}");
        }
    }

    #[test]
    fn refuses_other_schemes_and_plain_http_for_public_connections() {
        let outbound = outbound();
        let cases = [
            ("http://ollama:11434/v1", NetworkScope::Public),
            ("http://api.example.com/v1", NetworkScope::Public),
            ("ftp://example.com/", NetworkScope::Private),
            ("ws://example.com/", NetworkScope::Private),
            ("file:///etc/passwd", NetworkScope::Private),
        ];
        for (text, scope) in cases {
            assert_eq!(
                outbound.pick(&url(text), scope),
                Err(Blocked::Scheme),
                "{text}"
            );
        }
    }

    #[test]
    fn refuses_urls_with_credentials() {
        let outbound = outbound();
        for text in ["https://user@example.com/", "https://user:pw@example.com/"] {
            assert_eq!(
                outbound.pick(&url(text), NetworkScope::Private),
                Err(Blocked::Credentials),
                "{text}"
            );
        }
    }

    /// IP-literal hosts, which no resolver sees: `(url, scope, allowed)`.
    #[test]
    fn checks_ip_literal_hosts_before_any_request() {
        use NetworkScope::{Private, Public};
        let cases = [
            ("https://8.8.8.8/", Public, true),
            ("https://[2606:4700:4700::1111]/", Public, true),
            ("https://[::ffff:8.8.8.8]/", Public, true),
            ("https://127.0.0.1/", Public, false),
            ("https://2130706433/", Public, false),
            ("https://0x7f.1/", Public, false),
            ("https://0177.0.0.1/", Public, false),
            ("https://127.0.0.1./", Public, false),
            ("https://[::1]/", Public, false),
            ("https://10.0.0.1/", Public, false),
            ("https://100.64.0.1/", Public, false),
            ("https://[::ffff:127.0.0.1]/", Public, false),
            ("https://[::ffff:7f00:1]/", Public, false),
            ("https://[64:ff9b::a00:1]/", Public, false),
            ("https://[2002:a00:1::1]/", Public, false),
            ("https://[::7f00:1]/", Public, false),
            ("https://10.0.0.1/", Private, true),
            ("https://100.64.0.1/", Private, true),
            ("https://[::ffff:192.168.1.9]/", Private, true),
            ("https://169.254.169.254/", Private, false),
            ("https://[::ffff:169.254.169.254]/", Private, false),
            ("https://[64:ff9b::a9fe:a9fe]/", Private, false),
            ("https://[2002:a9fe:a9fe::1]/", Private, false),
            ("https://[64:ff9b:1::a00:1]/", Private, false),
            ("https://[fe80::1]/", Private, false),
            ("https://[fd00:ec2::254]/", Private, false),
            ("https://100.100.100.200/", Private, false),
            ("https://0.0.0.0/", Private, false),
            ("https://[::]/", Private, false),
            ("https://224.0.0.1/", Private, false),
            ("https://255.255.255.255/", Private, false),
            ("http://10.0.0.5:1234/", Private, true),
            ("http://[::1]:8080/", Private, true),
            ("http://8.8.8.8/", Private, false),
            ("http://[::ffff:8.8.8.8]/", Private, false),
            ("http://169.254.169.254/latest/meta-data/", Private, false),
        ];
        let outbound = outbound();
        for (text, scope, allowed) in cases {
            let result = outbound.pick(&url(text), scope);
            if allowed {
                assert!(result.is_ok(), "{text} is allowed for {scope:?}");
            } else {
                assert_eq!(result, Err(Blocked::Address), "{text} {scope:?}");
            }
        }
    }

    #[test]
    fn leaves_names_to_the_resolver() {
        let outbound = outbound();
        for text in ["https://localhost/", "https://metadata.google.internal/"] {
            assert_eq!(
                outbound.pick(&url(text), NetworkScope::Public),
                Ok(&"public")
            );
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Box<dyn Error + Send + Sync>);

    #[test]
    fn finds_the_refusal_in_a_source_chain() {
        let error = Outer(Box::new(Outer(Box::new(Blocked::Address))));
        assert_eq!(Blocked::find(&error), Some(Blocked::Address));
        assert_eq!(Blocked::find(&Blocked::Scheme), Some(Blocked::Scheme));
        let other = Outer(Box::new(std::io::Error::other("refused")));
        assert_eq!(Blocked::find(&other), None);
        assert_eq!(Blocked::Address.to_string(), "outbound.address.blocked");
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
