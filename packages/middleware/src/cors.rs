//! CORS for browser clients.
//!
//! An origin is allowed when its hostname is `localhost` / `127.0.0.1` (with `allow_localhost`),
//! equals an entry in `allowed_origins`, or ends with an entry that starts with a dot
//! (`.example.com` allows `app.example.com`).

use axum::http::{HeaderValue, Method};
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};

use crate::request_id::REQUEST_ID_HEADER;

#[derive(Clone, Debug, Default)]
pub struct CorsOptions {
    /// Allow requests from localhost (useful for local development).
    pub allow_localhost: bool,
    /// Extra exact hostnames or hostname suffixes to allow (for example `.example.com`).
    pub allowed_origins: Vec<String>,
}

pub fn cors(options: CorsOptions) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            move |origin: &HeaderValue, _parts| {
                origin
                    .to_str()
                    .ok()
                    .and_then(hostname)
                    .is_some_and(|host| is_allowed(host, &options))
            },
        ))
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::POST,
            Method::DELETE,
            Method::PATCH,
        ])
        .allow_headers(AllowHeaders::mirror_request())
        .expose_headers([REQUEST_ID_HEADER])
}

fn is_allowed(host: &str, options: &CorsOptions) -> bool {
    if options.allow_localhost && matches!(host, "localhost" | "127.0.0.1") {
        return true;
    }
    options.allowed_origins.iter().any(|allowed| {
        if allowed.starts_with('.') {
            host.ends_with(allowed.as_str())
        } else {
            host == allowed
        }
    })
}

/// Hostname of an `Origin` header value such as `https://app.example.com:8443`.
fn hostname(origin: &str) -> Option<&str> {
    let (scheme, rest) = origin.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split('/').next()?;
    let host = match authority.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next()?,
        None => authority.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(allow_localhost: bool, allowed: &[&str]) -> CorsOptions {
        CorsOptions {
            allow_localhost,
            allowed_origins: allowed.iter().map(|entry| (*entry).to_owned()).collect(),
        }
    }

    #[test]
    fn extracts_hostnames() {
        assert_eq!(hostname("http://localhost:4000"), Some("localhost"));
        assert_eq!(hostname("https://app.example.com"), Some("app.example.com"));
        assert_eq!(hostname("http://[::1]:4000"), Some("::1"));
        assert_eq!(hostname("file://"), None);
    }

    #[test]
    fn allows_localhost_only_when_enabled() {
        assert!(is_allowed("localhost", &options(true, &[])));
        assert!(!is_allowed("localhost", &options(false, &[])));
    }

    #[test]
    fn matches_exact_hosts_and_suffixes() {
        let rules = options(false, &["api.example.com", ".example.org"]);
        assert!(is_allowed("api.example.com", &rules));
        assert!(is_allowed("app.example.org", &rules));
        assert!(!is_allowed("example.com", &rules));
    }
}
