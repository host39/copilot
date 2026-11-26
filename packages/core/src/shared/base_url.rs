//! Base URLs of provider connections and tool servers.

use std::fmt;
use std::net::IpAddr;

use url::{Host, Url};

use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::net::{AddressClass, NetworkScope, classify};

/// A validated `http` or `https` URL that Copilot connects to.
///
/// Rules (`docs/design/copilot.md`, Outbound Requests): `https` for public hosts; `http` only for
/// private hosts the connection may reach; no user name or password; no query or fragment (keys
/// never travel in a URL). A host is private when it is a private or loopback IP literal, or a
/// name that only resolves locally (`localhost`, a single label such as `ollama`, or a name under
/// `.localhost`, `.local`, `.internal`, `.lan`, or `.home.arpa`). The outbound client checks the
/// resolved addresses again at connect time, so a public-looking name that resolves to a private
/// address is still blocked for a public-only connection.
#[derive(Clone, Eq, PartialEq)]
pub struct BaseUrl {
    url: Url,
    text: String,
}

impl BaseUrl {
    /// Validates user input for a connection with `scope`.
    pub fn parse(input: &str, scope: NetworkScope) -> DomainResult<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(DomainError::validation("Enter a URL."));
        }
        let url = Url::parse(trimmed).map_err(|_| {
            DomainError::validation("Enter a valid URL, for example https://api.example.com/v1.")
        })?;
        let https = match url.scheme() {
            "https" => true,
            "http" => false,
            _ => return Err(DomainError::validation("The URL must start with https://.")),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(DomainError::validation(
                "The URL must not contain a user name or password.",
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(DomainError::validation(
                "The URL must not contain a query or fragment.",
            ));
        }
        let private = match host_class(&url) {
            Some(AddressClass::Blocked) => {
                return Err(DomainError::validation("This address is not allowed."));
            }
            Some(AddressClass::Private) => true,
            Some(AddressClass::Public) => false,
            None => return Err(DomainError::validation("The URL must name a host.")),
        };
        if private && scope == NetworkScope::Public {
            return Err(DomainError::validation(
                "This account cannot connect to private network addresses.",
            ));
        }
        if !https && !private {
            return Err(DomainError::validation(
                "Use https:// for this host. Plain http:// is allowed only for private network addresses.",
            ));
        }
        Ok(Self::wrap(url))
    }

    /// A URL read back from storage. It was validated when it was saved; only the syntax is
    /// checked again, because the scope may have changed since (the outbound client enforces it).
    pub fn restore(stored: &str) -> DomainResult<Self> {
        let url = Url::parse(stored)
            .map_err(|error| DomainError::internal("core.base-url.restore.invalid", error))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(DomainError::Internal {
                key: "core.base-url.restore.scheme".to_owned(),
                source: None,
            });
        }
        Ok(Self::wrap(url))
    }

    fn wrap(url: Url) -> Self {
        let text = url.as_str().trim_end_matches('/').to_owned();
        Self { url, text }
    }

    /// The URL without a trailing slash, ready for `{base}/chat/completions`.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Same scheme, host, and port (default ports included).
    pub fn same_origin(&self, other: &Self) -> bool {
        self.url.origin() == other.url.origin()
    }
}

impl fmt::Debug for BaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("BaseUrl").field(&self.text).finish()
    }
}

impl fmt::Display for BaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

/// The class of the URL's host; `None` when it has none.
fn host_class(url: &Url) -> Option<AddressClass> {
    match url.host()? {
        Host::Ipv4(ip) => Some(classify(IpAddr::V4(ip))),
        Host::Ipv6(ip) => Some(classify(IpAddr::V6(ip))),
        Host::Domain(name) => Some(if is_local_name(name) {
            AddressClass::Private
        } else {
            AddressClass::Public
        }),
    }
}

/// Names that only resolve on a local network.
fn is_local_name(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost"
        || !name.contains('.')
        || [".localhost", ".local", ".internal", ".lan", ".home.arpa"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(result: DomainResult<BaseUrl>) -> String {
        result.expect_err("the URL is rejected").to_string()
    }

    #[test]
    fn accepts_public_https_urls_and_drops_the_trailing_slash() {
        let url = BaseUrl::parse(" https://api.openai.com/v1/ ", NetworkScope::Public).unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1");
        let root = BaseUrl::parse("https://api.anthropic.com", NetworkScope::Public).unwrap();
        assert_eq!(root.as_str(), "https://api.anthropic.com");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            message(BaseUrl::parse("  ", NetworkScope::Private)),
            "Enter a URL."
        );
        assert!(
            message(BaseUrl::parse("not a url", NetworkScope::Private))
                .starts_with("Enter a valid URL")
        );
        assert_eq!(
            message(BaseUrl::parse("ftp://example.com", NetworkScope::Private)),
            "The URL must start with https://."
        );
        assert_eq!(
            message(BaseUrl::parse(
                "https://user:pw@example.com",
                NetworkScope::Private
            )),
            "The URL must not contain a user name or password."
        );
        assert_eq!(
            message(BaseUrl::parse(
                "https://user@example.com",
                NetworkScope::Private
            )),
            "The URL must not contain a user name or password."
        );
        assert_eq!(
            message(BaseUrl::parse(
                "https://example.com/v1?key=abc",
                NetworkScope::Private
            )),
            "The URL must not contain a query or fragment."
        );
        assert_eq!(
            message(BaseUrl::parse(
                "https://example.com/#top",
                NetworkScope::Private
            )),
            "The URL must not contain a query or fragment."
        );
    }

    #[test]
    fn requires_https_for_public_hosts() {
        let error = message(BaseUrl::parse(
            "http://api.example.com/v1",
            NetworkScope::Private,
        ));
        assert!(error.starts_with("Use https:// for this host."));
        let error = message(BaseUrl::parse("http://8.8.8.8/v1", NetworkScope::Private));
        assert!(error.starts_with("Use https:// for this host."));
    }

    #[test]
    fn allows_http_for_private_hosts_in_private_scope() {
        for input in [
            "http://localhost:11434/v1",
            "http://ollama:11434/v1",
            "http://gpu.lan:8000/v1",
            "http://box.internal/v1",
            "http://10.0.0.5:1234/v1",
            "http://[::1]:8080/v1",
            "http://[::ffff:192.168.1.9]/v1",
        ] {
            assert!(
                BaseUrl::parse(input, NetworkScope::Private).is_ok(),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_private_hosts_in_public_scope() {
        for input in [
            "http://localhost:11434/v1",
            "https://localhost/v1",
            "https://ollama/v1",
            "https://192.168.1.2/v1",
            "https://100.64.1.1/v1",
        ] {
            assert_eq!(
                message(BaseUrl::parse(input, NetworkScope::Public)),
                "This account cannot connect to private network addresses.",
                "{input}"
            );
        }
    }

    #[test]
    fn always_rejects_blocked_addresses() {
        for input in [
            "http://169.254.169.254/latest",
            "https://[fe80::1]/",
            "https://[fd00:ec2::254]/",
            "http://100.100.100.200/",
            "https://0.0.0.0/",
            "https://[::]/",
            "https://[::ffff:169.254.169.254]/",
        ] {
            assert_eq!(
                message(BaseUrl::parse(input, NetworkScope::Private)),
                "This address is not allowed.",
                "{input}"
            );
        }
    }

    #[test]
    fn compares_origins() {
        let scope = NetworkScope::Private;
        let base = BaseUrl::parse("https://api.example.com/v1", scope).unwrap();
        let same = BaseUrl::parse("https://api.example.com:443/v2", scope).unwrap();
        let other_host = BaseUrl::parse("https://proxy.example.com/v1", scope).unwrap();
        let other_port = BaseUrl::parse("https://api.example.com:8443/v1", scope).unwrap();
        let other_scheme = BaseUrl::parse("http://localhost/v1", scope).unwrap();
        assert!(base.same_origin(&same));
        assert!(!base.same_origin(&other_host));
        assert!(!base.same_origin(&other_port));
        assert!(!base.same_origin(&other_scheme));
    }

    #[test]
    fn restores_stored_urls_without_the_scope_check() {
        let url = BaseUrl::restore("http://localhost:11434/v1").unwrap();
        assert_eq!(url.to_string(), "http://localhost:11434/v1");
        assert!(BaseUrl::restore("nonsense").is_err());
        assert!(BaseUrl::restore("ftp://example.com").is_err());
    }
}
