//! Dynamic client registration (RFC 7591) and the token endpoint (OAuth 2.1: the authorization
//! code grant with PKCE, and refresh), with the `resource` parameter (RFC 8707) on every token
//! request.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use copilot_core::{
    AuthServer, ClientAuth, DomainError, DomainResult, Grant, OAUTH_FAILED, OAUTH_REGISTRATION,
    OAUTH_REJECTED, OAuthClient, Registration, Secret, TokenRequest, TokenSet,
};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Map, Value, json};
use url::form_urlencoded;

use crate::oauth::http::Http;
use crate::oauth::trouble;

pub(crate) async fn register(
    http: &Http<'_>,
    server: &AuthServer,
    registration: &Registration,
) -> DomainResult<OAuthClient> {
    let Some(endpoint) = &server.registration_endpoint else {
        return Err(registration_refused());
    };
    let mut body = json!({
        "client_name": registration.client_name,
        "redirect_uris": [registration.redirect_uri.as_str()],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": registration.application_type,
    });
    if let Some(scope) = &registration.scope {
        body["scope"] = json!(scope);
    }
    let answer = http
        .send(endpoint, |client| {
            client
                .post(endpoint.clone())
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json")
                .body(body.to_string())
        })
        .await
        .map_err(|error| trouble("registration", error))?;
    if !answer.success() {
        tracing::warn!(status = answer.status, "mcp.oauth.registration-refused");
        return Err(registration_refused());
    }
    let object = answer.object().ok_or_else(registration_refused)?;
    let id = text(&object, "client_id").ok_or_else(registration_refused)?;
    let secret = text(&object, "client_secret").map(Secret::new);
    let auth = match (&secret, text(&object, "token_endpoint_auth_method")) {
        (None, _) => ClientAuth::None,
        (Some(_), Some(method)) => ClientAuth::parse(&method).unwrap_or(ClientAuth::Basic),
        (Some(_), None) => ClientAuth::Basic,
    };
    tracing::info!(auth = auth.as_str(), "mcp.oauth.registered");
    Ok(OAuthClient { id, secret, auth })
}

fn registration_refused() -> DomainError {
    DomainError::upstream(
        "The sign-in server refused to register Copilot as a client. Enter a client ID in the server's settings instead.",
        OAUTH_REGISTRATION,
    )
}

pub(crate) async fn token(http: &Http<'_>, request: &TokenRequest) -> DomainResult<TokenSet> {
    let step = match request.grant {
        Grant::Code { .. } => "token",
        Grant::Refresh { .. } => "refresh",
    };
    let body = form(request)?;
    let client = &request.client;
    let basic = match (client.auth, &client.secret) {
        (ClientAuth::Basic, Some(secret)) => Some(basic(&client.id, secret)?),
        _ => None,
    };
    let answer = http
        .send(&request.endpoint, |http| {
            let builder = http
                .post(request.endpoint.clone())
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(ACCEPT, "application/json")
                .body(body);
            match basic {
                Some(value) => builder.header(AUTHORIZATION, value),
                None => builder,
            }
        })
        .await
        .map_err(|error| trouble(step, error))?;
    if answer.success() {
        return answer
            .object()
            .and_then(|object| tokens(&object))
            .ok_or_else(|| {
                tracing::warn!(step, "mcp.oauth.token-unreadable");
                DomainError::upstream(
                    "The sign-in server sent a token answer Copilot cannot read.",
                    OAUTH_FAILED,
                )
            });
    }
    // RFC 6749 Section 5.2: an OAuth error is a 400 (or a 401 for client authentication) with
    // a JSON `error`. Its description is the server's text and is never passed on.
    let error = answer.object().and_then(|object| text(&object, "error"));
    match (answer.status, error) {
        (400 | 401, Some(error)) => {
            tracing::warn!(step, status = answer.status, error = %short(&error), "mcp.oauth.token-rejected");
            Err(DomainError::upstream(
                "The sign-in server did not accept the sign-in. Sign in again.",
                OAUTH_REJECTED,
            ))
        }
        (status, _) => {
            tracing::warn!(step, status, "mcp.oauth.failed");
            Err(DomainError::upstream(
                format!("The sign-in server answered with HTTP {status}."),
                OAUTH_FAILED,
            ))
        }
    }
}

/// The form body: the grant, the `resource`, and the client id unless HTTP Basic carries it
/// (one authentication method per request), plus the secret for `client_secret_post`.
fn form(request: &TokenRequest) -> DomainResult<String> {
    let invalid = || DomainError::upstream("The sign-in values cannot be sent.", OAUTH_FAILED);
    let text = |secret: &Secret| secret.text().map(ToOwned::to_owned).ok_or_else(invalid);
    let mut form = form_urlencoded::Serializer::new(String::new());
    match &request.grant {
        Grant::Code {
            code,
            verifier,
            redirect_uri,
        } => {
            form.append_pair("grant_type", "authorization_code")
                .append_pair("code", &text(code)?)
                .append_pair("redirect_uri", redirect_uri.as_str())
                .append_pair("code_verifier", &text(verifier)?);
        }
        Grant::Refresh { token } => {
            form.append_pair("grant_type", "refresh_token")
                .append_pair("refresh_token", &text(token)?);
        }
    }
    form.append_pair("resource", &request.resource);
    let client = &request.client;
    match (client.auth, &client.secret) {
        (ClientAuth::Basic, Some(_)) => {}
        (ClientAuth::Post, Some(secret)) => {
            form.append_pair("client_id", &client.id)
                .append_pair("client_secret", &text(secret)?);
        }
        _ => {
            form.append_pair("client_id", &client.id);
        }
    }
    Ok(form.finish())
}

/// `Basic` with the form-encoded id and secret (RFC 6749 Section 2.3.1).
fn basic(id: &str, secret: &Secret) -> DomainResult<String> {
    let secret = secret
        .text()
        .ok_or_else(|| DomainError::upstream("The client secret cannot be sent.", OAUTH_FAILED))?;
    let encode =
        |value: &str| form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    let pair = format!("{}:{}", encode(id), encode(secret));
    Ok(format!("Basic {}", STANDARD.encode(pair)))
}

/// A token answer: a `Bearer` access token (other token types, such as DPoP, are not
/// supported), with its lifetime, refresh token, and scope when given.
fn tokens(object: &Map<String, Value>) -> Option<TokenSet> {
    let access = text(object, "access_token").filter(|token| !token.is_empty())?;
    let kind = text(object, "token_type")?;
    if !kind.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let expires_in = match object.get("expires_in") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    };
    Some(TokenSet {
        access: Secret::new(access),
        refresh: text(object, "refresh_token")
            .filter(|token| !token.is_empty())
            .map(Secret::new),
        expires_in: expires_in.map(Duration::from_secs),
        scope: text(object, "scope"),
    })
}

fn text(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToOwned::to_owned)
}

/// An OAuth error code for the log: the registered codes are short ASCII words; anything else is
/// cut.
fn short(error: &str) -> String {
    error
        .chars()
        .filter(|character| character.is_ascii_graphic())
        .take(40)
        .collect()
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    fn request(auth: ClientAuth, secret: Option<&str>, grant: Grant) -> TokenRequest {
        TokenRequest {
            endpoint: Url::parse("https://auth.example.com/token").unwrap(),
            client: OAuthClient {
                id: "client 1".to_owned(),
                secret: secret.map(Secret::new),
                auth,
            },
            resource: "https://mcp.example.com/mcp".to_owned(),
            grant,
        }
    }

    fn code() -> Grant {
        Grant::Code {
            code: Secret::new("code-1"),
            verifier: Secret::new("verifier-1"),
            redirect_uri: Url::parse("https://copilot.example.com/api/oauth/callback").unwrap(),
        }
    }

    fn pairs(body: &str) -> Vec<(String, String)> {
        form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    #[test]
    fn sends_the_code_verifier_resource_and_public_client_id() {
        let body = form(&request(ClientAuth::None, None, code())).unwrap();
        assert_eq!(
            pairs(&body),
            [
                ("grant_type".to_owned(), "authorization_code".to_owned()),
                ("code".to_owned(), "code-1".to_owned()),
                (
                    "redirect_uri".to_owned(),
                    "https://copilot.example.com/api/oauth/callback".to_owned()
                ),
                ("code_verifier".to_owned(), "verifier-1".to_owned()),
                (
                    "resource".to_owned(),
                    "https://mcp.example.com/mcp".to_owned()
                ),
                ("client_id".to_owned(), "client 1".to_owned()),
            ]
        );
    }

    #[test]
    fn authenticates_confidential_clients_one_way_only() {
        let refresh = Grant::Refresh {
            token: Secret::new("refresh-1"),
        };
        let basic_body = form(&request(ClientAuth::Basic, Some("s:x"), refresh.clone())).unwrap();
        assert!(!basic_body.contains("client_id"), "{basic_body}");
        assert!(basic_body.contains("grant_type=refresh_token"));
        assert!(basic_body.contains("refresh_token=refresh-1"));
        assert_eq!(
            basic("client 1", &Secret::new("s:x")).unwrap(),
            format!("Basic {}", STANDARD.encode("client+1:s%3Ax"))
        );
        let post = form(&request(ClientAuth::Post, Some("s"), refresh)).unwrap();
        assert!(post.contains("client_id=client+1") && post.contains("client_secret=s"));
    }

    #[test]
    fn reads_bearer_tokens_only() {
        let object = |value: Value| value.as_object().unwrap().clone();
        let set = tokens(&object(json!({
            "access_token": "a", "token_type": "Bearer", "expires_in": 3600,
            "refresh_token": "r", "scope": "x y",
        })))
        .unwrap();
        assert_eq!(set.access, Secret::new("a"));
        assert_eq!(set.refresh, Some(Secret::new("r")));
        assert_eq!(set.expires_in, Some(Duration::from_secs(3600)));
        assert_eq!(set.scope.as_deref(), Some("x y"));
        let text_lifetime = tokens(&object(json!({
            "access_token": "a", "token_type": "bearer", "expires_in": "60",
        })))
        .unwrap();
        assert_eq!(text_lifetime.expires_in, Some(Duration::from_secs(60)));
        assert_eq!(text_lifetime.refresh, None);
        assert!(tokens(&object(json!({"access_token": "a", "token_type": "DPoP"}))).is_none());
        assert!(tokens(&object(json!({"access_token": "", "token_type": "Bearer"}))).is_none());
        assert!(tokens(&object(json!({"token_type": "Bearer"}))).is_none());
    }

    #[test]
    fn cuts_error_codes_for_the_log() {
        assert_eq!(short("invalid_grant"), "invalid_grant");
        assert_eq!(short("a b\nc"), "abc");
        assert_eq!(short(&"x".repeat(100)).len(), 40);
    }
}
