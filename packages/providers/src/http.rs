//! What every provider request shares: the client choice, credentials, sending, and the mapping
//! of HTTP failures to human-readable errors (`docs/design/copilot.md`, Adapter Rules).
//!
//! Failures never echo a response body or a resolved address: the client reads a short message
//! and a stable code, and the log gets the status and the provider's request id.

use std::error::Error;

use copilot_core::shared::constants::MCP_RESPONSE_MAX_BYTES;
use copilot_core::{Blocked, DomainError, DomainResult, Outbound, ProviderTarget, read_capped};
use reqwest::header::{HeaderValue, RETRY_AFTER};
use reqwest::{Client, RequestBuilder, Response, StatusCode};

/// The largest whole body (a model list) and the largest SSE event a provider may send: the
/// same cap as MCP responses (`docs/design/copilot.md`, Outbound Requests, item 7).
pub(crate) const RESPONSE_MAX_BYTES: usize = MCP_RESPONSE_MAX_BYTES;

/// The headers that name a request in the provider's own logs (`x-request-id` on OpenAI and
/// most compatible servers, `request-id` on Anthropic), logged with failures so an operator can
/// match them.
const PROVIDER_REQUEST_IDS: [&str; 2] = ["x-request-id", "request-id"];

pub(crate) const PROVIDER_AUTH: &str = "PROVIDER_AUTH";
pub(crate) const PROVIDER_RATE_LIMITED: &str = "PROVIDER_RATE_LIMITED";
pub(crate) const PROVIDER_HTTP: &str = "PROVIDER_HTTP";
pub(crate) const PROVIDER_BLOCKED: &str = "PROVIDER_BLOCKED";
pub(crate) const PROVIDER_UNREACHABLE: &str = "PROVIDER_UNREACHABLE";
pub(crate) const PROVIDER_TIMEOUT: &str = "PROVIDER_TIMEOUT";
pub(crate) const PROVIDER_FAILED: &str = "PROVIDER_FAILED";
pub(crate) const PROVIDER_BAD_RESPONSE: &str = "PROVIDER_BAD_RESPONSE";
pub(crate) const PROVIDER_KEY_INVALID: &str = "PROVIDER_KEY_INVALID";

/// The outbound client for `target`: the scope and scheme pick one of the three, and an
/// IP-literal host must pass the address policy before anything is sent.
pub(crate) fn client<'a>(
    clients: &'a Outbound<Client>,
    target: &ProviderTarget,
) -> DomainResult<&'a Client> {
    clients
        .pick(target.base_url.url(), target.network)
        .map_err(|blocked| {
            tracing::warn!(
                provider_id = %target.provider_id,
                reason = %blocked,
                "provider.http.blocked"
            );
            blocked_error()
        })
}

/// `Authorization: Bearer <key>` when the connection has a key (OpenAI-compatible servers take
/// credentials in this header only, never in the URL). The value is marked sensitive.
pub(crate) fn bearer(target: &ProviderTarget) -> DomainResult<Option<HeaderValue>> {
    key_header(target, |key| format!("Bearer {key}"))
}

/// The `x-api-key` value when the connection has a key (Anthropic connections take credentials
/// in this header only). The value is marked sensitive.
pub(crate) fn api_key(target: &ProviderTarget) -> DomainResult<Option<HeaderValue>> {
    key_header(target, str::to_owned)
}

/// A sensitive header value made of the connection's key, or `None` without a key.
fn key_header(
    target: &ProviderTarget,
    value: impl FnOnce(&str) -> String,
) -> DomainResult<Option<HeaderValue>> {
    let Some(key) = &target.api_key else {
        return Ok(None);
    };
    let key = key.text().map(str::trim).unwrap_or_default();
    if key.is_empty() {
        return Ok(None);
    }
    let mut header = HeaderValue::from_str(&value(key)).map_err(|_| {
        DomainError::upstream(
            "The API key contains characters that cannot be sent. Enter it again in the provider settings.",
            PROVIDER_KEY_INVALID,
        )
    })?;
    header.set_sensitive(true);
    Ok(Some(header))
}

/// Sends `request` and returns a successful response; connection failures and error statuses
/// become human-readable errors (logged without the URL).
pub(crate) async fn send(
    target: &ProviderTarget,
    request: RequestBuilder,
) -> DomainResult<Response> {
    let response = request
        .send()
        .await
        .map_err(|error| failed(target, error))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let request_id = PROVIDER_REQUEST_IDS
        .into_iter()
        .find_map(|name| response.headers().get(name)?.to_str().ok())
        .map(str::to_owned);
    tracing::warn!(
        provider_id = %target.provider_id,
        status = status.as_u16(),
        provider_request_id = request_id.as_deref(),
        "provider.http.failed"
    );
    Err(status_error(status, retry_after(&response)))
}

/// Reads a whole body of at most [`RESPONSE_MAX_BYTES`].
pub(crate) async fn body(target: &ProviderTarget, response: Response) -> DomainResult<Vec<u8>> {
    let declared = response.content_length();
    read_capped(declared, response.bytes_stream(), RESPONSE_MAX_BYTES)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| {
            tracing::warn!(
                provider_id = %target.provider_id,
                error = %error,
                "provider.http.body-failed"
            );
            bad_response()
        })
}

/// The error for an HTTP status other than success.
pub(crate) fn status_error(status: StatusCode, retry_after: Option<u64>) -> DomainError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            DomainError::upstream("Key rejected: open provider settings.", PROVIDER_AUTH)
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let message = match retry_after {
                Some(1) => {
                    "The provider is rate limiting requests. Try again in 1 second.".to_owned()
                }
                Some(seconds) => format!(
                    "The provider is rate limiting requests. Try again in {seconds} seconds."
                ),
                None => "The provider is rate limiting requests. Try again later.".to_owned(),
            };
            DomainError::upstream(message, PROVIDER_RATE_LIMITED)
        }
        StatusCode::PAYLOAD_TOO_LARGE => {
            DomainError::upstream("The request is too large for this provider.", PROVIDER_HTTP)
        }
        // 529 is Anthropic's "overloaded".
        status if status.as_u16() == 529 => DomainError::upstream(
            "The provider is overloaded. Try again in a moment.",
            PROVIDER_HTTP,
        ),
        status => DomainError::upstream(
            format!("The provider answered with HTTP {}.", status.as_u16()),
            PROVIDER_HTTP,
        ),
    }
}

/// The error for a reply that is not what the protocol promises (unreadable JSON, a body over
/// the cap).
pub(crate) fn bad_response() -> DomainError {
    DomainError::upstream(
        "The provider sent a reply Copilot cannot read.",
        PROVIDER_BAD_RESPONSE,
    )
}

/// The error of an address the outbound policy refused (or a base URL that cannot be one).
pub(crate) fn blocked_error() -> DomainError {
    DomainError::upstream(
        "This connection may not reach that address. Check the base URL.",
        PROVIDER_BLOCKED,
    )
}

/// A request that never got a response: refused by the outbound policy, unreachable, or too
/// slow to connect or to answer.
fn failed(target: &ProviderTarget, error: reqwest::Error) -> DomainError {
    let error = error.without_url();
    if let Some(blocked) = Blocked::find(&error as &(dyn Error + 'static)) {
        tracing::warn!(
            provider_id = %target.provider_id,
            reason = %blocked,
            "provider.http.blocked"
        );
        return blocked_error();
    }
    tracing::warn!(
        provider_id = %target.provider_id,
        error = %error,
        "provider.http.failed"
    );
    if error.is_timeout() {
        DomainError::upstream("The provider did not answer in time.", PROVIDER_TIMEOUT)
    } else if error.is_connect() {
        DomainError::upstream(
            "Could not connect to the provider. Check the base URL.",
            PROVIDER_UNREACHABLE,
        )
    } else {
        DomainError::upstream("The request to the provider failed.", PROVIDER_FAILED)
    }
}

/// `Retry-After` in seconds (the HTTP-date form is ignored).
fn retry_after(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use copilot_core::{BaseUrl, NetworkScope, Preset, ProviderId, ProviderKind, Secret};

    use super::*;

    fn target(key: Option<&str>) -> ProviderTarget {
        ProviderTarget {
            provider_id: ProviderId::generate(),
            kind: ProviderKind::OpenAi,
            preset: Preset::OpenAi,
            api: None,
            base_url: BaseUrl::parse("https://api.example.com/v1", NetworkScope::Public)
                .expect("valid"),
            api_key: key.map(Secret::new),
            network: NetworkScope::Public,
        }
    }

    #[test]
    fn maps_statuses_to_readable_errors() {
        let auth = status_error(StatusCode::UNAUTHORIZED, None);
        assert_eq!(auth.code(), PROVIDER_AUTH);
        assert_eq!(auth.to_string(), "Key rejected: open provider settings.");
        assert_eq!(
            status_error(StatusCode::FORBIDDEN, None).code(),
            PROVIDER_AUTH
        );

        let limited = status_error(StatusCode::TOO_MANY_REQUESTS, Some(20));
        assert_eq!(limited.code(), PROVIDER_RATE_LIMITED);
        assert!(limited.to_string().ends_with("Try again in 20 seconds."));
        let later = status_error(StatusCode::TOO_MANY_REQUESTS, None);
        assert!(later.to_string().ends_with("Try again later."));

        let other = status_error(StatusCode::SERVICE_UNAVAILABLE, None);
        assert_eq!(other.code(), PROVIDER_HTTP);
        assert_eq!(other.to_string(), "The provider answered with HTTP 503.");

        let large = status_error(StatusCode::PAYLOAD_TOO_LARGE, None);
        assert_eq!(large.code(), PROVIDER_HTTP);
        assert_eq!(
            large.to_string(),
            "The request is too large for this provider."
        );
        let overloaded = status_error(StatusCode::from_u16(529).expect("a status"), None);
        assert_eq!(overloaded.code(), PROVIDER_HTTP);
        assert_eq!(
            overloaded.to_string(),
            "The provider is overloaded. Try again in a moment."
        );
    }

    #[test]
    fn sends_a_sensitive_bearer_header_only_with_a_key() {
        assert_eq!(bearer(&target(None)).expect("no key"), None);
        assert_eq!(bearer(&target(Some("  "))).expect("blank key"), None);
        let value = bearer(&target(Some("sk-test")))
            .expect("valid key")
            .expect("a header");
        assert_eq!(value.to_str().expect("ascii"), "Bearer sk-test");
        assert!(value.is_sensitive());
    }

    #[test]
    fn sends_a_sensitive_api_key_header_only_with_a_key() {
        assert_eq!(api_key(&target(None)).expect("no key"), None);
        assert_eq!(api_key(&target(Some(" "))).expect("blank key"), None);
        let value = api_key(&target(Some(" sk-ant-test ")))
            .expect("valid key")
            .expect("a header");
        assert_eq!(value.to_str().expect("ascii"), "sk-ant-test");
        assert!(value.is_sensitive());
    }

    #[test]
    fn refuses_a_key_that_cannot_be_a_header() {
        for header in [bearer, api_key] {
            let error = header(&target(Some("sk\ntest"))).expect_err("control character");
            assert_eq!(error.code(), PROVIDER_KEY_INVALID);
            assert!(!error.to_string().contains("sk"), "never echoes the key");
        }
    }
}
