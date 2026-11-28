//! `/api/oauth/*`: the public end of tool server sign-in (`docs/design/copilot.md`, Tool Server
//! Sign-In). Neither route needs a session: the browser that comes back from an authorization
//! server carries none (in the native apps it is the system browser), and authorization servers
//! fetch the client document themselves.
//!
//! - `GET /api/oauth/callback` and `GET /api/oauth/callback/{tag}` take a sign-in's redirect
//!   (`ToolService::complete`): the `state` names the flow, used once, and the path must be the
//!   flow's own redirect URI (`{tag}` names its authorization server; the bare path belongs to
//!   sign-ins with the client document). The browser is sent on to the web app's `/oauth/done`
//!   page with a short result; when the code was exchanged, the confirmation code that finishes
//!   the sign-in goes in the URL's fragment, which no server sees. The authorization server's
//!   error text is never shown.
//! - `GET /api/oauth/client` is this install's Client ID Metadata Document, for authorization
//!   servers that accept a URL as the client id (`https` `PUBLIC_URL` only).
//!
//! Without `PUBLIC_URL` both answer `404`.

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, LOCATION, REFERRER_POLICY};
use axum::response::{IntoResponse, Response};
use copilot_contracts::ErrorResponse;
use copilot_core::{CLIENT_NAME, Callback, DomainError, Secret, SignInOutcome, SignInResult};
use copilot_middleware::HttpError;
use serde_json::{Value, json};
use url::form_urlencoded;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;

/// Longest `state`, `code`, `iss`, or `error` value read from a callback.
const PARAMETER_MAX_CHARS: usize = 2_048;
/// Longest issuer tag read from a callback's path (a tag is 22 characters).
const TAG_MAX_CHARS: usize = 64;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(callback))
        .routes(routes!(tagged))
        .routes(routes!(client))
}

#[utoipa::path(
    get,
    path = "/oauth/callback",
    operation_id = "oauth_callback",
    tag = "tool-servers",
    params(
        ("state" = Option<String>, Query, description = "The sign-in's `state`"),
        ("code" = Option<String>, Query, description = "The authorization code"),
        ("iss" = Option<String>, Query, description = "The issuer (RFC 9207)"),
        ("error" = Option<String>, Query, description = "The authorization server's error code")
    ),
    responses(
        (status = SEE_OTHER, description = "To `<PUBLIC_URL>/oauth/done?result=confirm|denied|expired|failed&server=<id>`, with `#<confirmation code>` after `confirm`"),
        (status = NOT_FOUND, description = "Sign-in is off (no `PUBLIC_URL`)", body = ErrorResponse)
    )
)]
pub async fn callback(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Response, HttpError> {
    finish(&state, None, query.as_deref()).await
}

#[utoipa::path(
    get,
    path = "/oauth/callback/{tag}",
    operation_id = "oauth_callback_tagged",
    tag = "tool-servers",
    params(
        ("tag" = String, Path, description = "The authorization server's tag: sign-ins with a pre-registered or registered client come back here"),
        ("state" = Option<String>, Query, description = "The sign-in's `state`"),
        ("code" = Option<String>, Query, description = "The authorization code"),
        ("iss" = Option<String>, Query, description = "The issuer (RFC 9207)"),
        ("error" = Option<String>, Query, description = "The authorization server's error code")
    ),
    responses(
        (status = SEE_OTHER, description = "As `/oauth/callback`"),
        (status = NOT_FOUND, description = "Sign-in is off (no `PUBLIC_URL`)", body = ErrorResponse)
    )
)]
pub async fn tagged(
    State(state): State<AppState>,
    Path(tag): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, HttpError> {
    // A tag is base64url; anything else names no authorization server and fails as a mismatch.
    let tag = if tag.len() <= TAG_MAX_CHARS
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        tag
    } else {
        "-".to_owned()
    };
    finish(&state, Some(tag), query.as_deref()).await
}

/// Takes a callback that came back to `/oauth/callback` (`tag` `None`) or
/// `/oauth/callback/{tag}` and sends the browser on to `/oauth/done`.
async fn finish(
    state: &AppState,
    tag: Option<String>,
    query: Option<&str>,
) -> Result<Response, HttpError> {
    let Some(public) = state.config.public_url.clone() else {
        return Err(off());
    };
    let mut callback = parse(query.unwrap_or_default());
    callback.tag = tag;
    let outcome = if callback.state.is_empty() {
        SignInOutcome {
            server_id: None,
            result: SignInResult::Expired,
            confirmation: None,
        }
    } else {
        state.tools.complete(&callback).await
    };
    let mut target = public
        .join("/oauth/done")
        .map_err(|error| DomainError::internal("api.oauth.done-url.invalid", error))?;
    {
        let mut pairs = target.query_pairs_mut();
        pairs.append_pair("result", outcome.result.as_str());
        if let Some(server) = outcome.server_id {
            pairs.append_pair("server", &server.to_string());
        }
    }
    // Only the browser that came back gets the code: a fragment never reaches a server's logs.
    if let Some(code) = outcome.confirmation.as_ref().and_then(Secret::text) {
        target.set_fragment(Some(code));
    }
    Ok((
        StatusCode::SEE_OTHER,
        [
            (LOCATION, target.to_string()),
            (CACHE_CONTROL, "no-store".to_owned()),
            (REFERRER_POLICY, "no-referrer".to_owned()),
        ],
    )
        .into_response())
}

/// The callback's parameters; the first of each name counts, and an overlong value is dropped.
fn parse(query: &str) -> Callback {
    let mut callback = Callback::default();
    let mut seen = Vec::new();
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        if seen.contains(&name) || value.chars().count() > PARAMETER_MAX_CHARS {
            continue;
        }
        let value = value.into_owned();
        match name.as_ref() {
            "state" => callback.state = value,
            "code" => callback.code = Some(value),
            "iss" => callback.iss = Some(value),
            "error" => callback.error = Some(value),
            _ => continue,
        }
        seen.push(name);
    }
    callback
}

#[utoipa::path(
    get,
    path = "/oauth/client",
    operation_id = "oauth_client",
    tag = "tool-servers",
    responses(
        (status = OK, description = "This install's OAuth Client ID Metadata Document: `client_id` is this URL", content_type = "application/json"),
        (status = NOT_FOUND, description = "No `https` `PUBLIC_URL`, or `OAUTH_CLIENT_METADATA=false`", body = ErrorResponse)
    )
)]
pub async fn client(State(state): State<AppState>) -> Result<Response, HttpError> {
    let config = state.tools.sign_in_config().ok_or_else(off)?;
    let document = config.client_metadata.as_ref().ok_or_else(off)?;
    let public = state.config.public_url.as_ref().ok_or_else(off)?;
    let body: Value = json!({
        "client_id": document.as_str(),
        "client_name": CLIENT_NAME,
        "client_uri": public.as_str(),
        // Only the bare callback: sign-ins with this document come back there, and only from
        // authorization servers that send `iss` (`ToolService::sign_in`).
        "redirect_uris": [config.callback.as_str()],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "web",
    });
    Ok(([(CACHE_CONTROL, "public, max-age=3600")], axum::Json(body)).into_response())
}

fn off() -> HttpError {
    HttpError::Domain(DomainError::not_found("Sign-in", "oauth"))
}

#[cfg(test)]
mod tests {
    use axum::http::Method;
    use axum::http::StatusCode;
    use url::Url;

    use super::*;
    use crate::helpers::testing::{app_with, config, get, request, send};

    #[test]
    fn reads_the_first_of_each_parameter_and_drops_overlong_ones() {
        let long = "x".repeat(PARAMETER_MAX_CHARS + 1);
        let callback = parse(&format!(
            "state=s1&code=c%201&iss=https%3A%2F%2Fa.example&state=s2&error=access_denied&error_description=text&code={long}"
        ));
        assert_eq!(callback.state, "s1");
        assert_eq!(callback.code.as_deref(), Some("c 1"));
        assert_eq!(callback.iss.as_deref(), Some("https://a.example"));
        assert_eq!(callback.error.as_deref(), Some("access_denied"));
        let dropped = parse(&format!("state={long}"));
        assert_eq!(dropped.state, "");
    }

    async fn app(public: Option<&str>) -> axum::Router {
        let mut config = config();
        config.public_url = public.map(|value| Url::parse(value).unwrap());
        let db = copilot_database::create_pool(&config.db);
        app_with(config, db).await
    }

    #[tokio::test]
    async fn is_off_without_a_public_url() {
        let app = app(None).await;
        for uri in [
            "/api/oauth/callback?state=x&code=y",
            "/api/oauth/callback/f7mcEwYDYdtgG-u6lN8a0g?state=x&code=y",
            "/api/oauth/client",
        ] {
            let response = get(app.clone(), uri).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn serves_the_client_metadata_document_on_https_only() {
        let document = get(
            app(Some("https://copilot.example.com")).await,
            "/api/oauth/client",
        )
        .await;
        assert_eq!(document.status, StatusCode::OK);
        assert_eq!(
            document.body["client_id"], "https://copilot.example.com/api/oauth/client",
            "the client id is the document's own URL"
        );
        assert_eq!(
            document.body["redirect_uris"],
            json!(["https://copilot.example.com/api/oauth/callback"])
        );
        assert_eq!(document.body["client_name"], "Copilot");
        assert_eq!(document.body["token_endpoint_auth_method"], "none");

        let local = get(
            app(Some("http://localhost:4000")).await,
            "/api/oauth/client",
        )
        .await;
        assert_eq!(
            local.status,
            StatusCode::NOT_FOUND,
            "a document must be https"
        );
    }

    /// A callback without a `state` never reaches the database (this app's is unreachable).
    #[tokio::test]
    async fn a_callback_without_a_state_goes_to_the_done_page_as_expired() {
        let app = app(Some("https://copilot.example.com")).await;
        for uri in [
            "/api/oauth/callback?code=c",
            "/api/oauth/callback",
            "/api/oauth/callback?error=access_denied&error_description=%3Cscript%3E",
            "/api/oauth/callback/f7mcEwYDYdtgG-u6lN8a0g?code=c",
            "/api/oauth/callback/not%20a%20tag",
        ] {
            let response = send(app.clone(), request(Method::GET, uri, None, None)).await;
            assert_eq!(response.status, StatusCode::SEE_OTHER, "{uri}");
            assert_eq!(
                response.headers[LOCATION], "https://copilot.example.com/oauth/done?result=expired",
                "{uri}"
            );
            assert_eq!(response.headers[CACHE_CONTROL], "no-store");
            assert_eq!(response.headers[REFERRER_POLICY], "no-referrer");
        }
    }
}
