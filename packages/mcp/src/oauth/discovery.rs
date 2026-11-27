//! Discovery (MCP authorization, revision `2026-07-28`, Authorization Server Discovery): the
//! server's `401` challenge, its protected resource metadata (RFC 9728), and the first listed
//! authorization server's metadata (RFC 8414 or OpenID Connect Discovery), with the checks the
//! client must make.

use copilot_core::{
    AuthServer, BaseUrl, Discovery, DomainError, DomainResult, NetworkScope, OAUTH_DISCOVERY,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE, WWW_AUTHENTICATE};
use serde_json::{Map, Value, json};
use url::Url;

use crate::oauth::challenge::Challenge;
use crate::oauth::http::{Answer, Http};
use crate::oauth::trouble;

/// The MCP revision the probe names.
const PROBE_VERSION: &str = "2026-07-28";

pub(crate) async fn discover(http: &Http<'_>, server: &BaseUrl) -> DomainResult<Discovery> {
    let url = server.url();
    let challenge = probe(http, url).await?;
    let metadata = resource_metadata(http, url, &challenge).await?;
    let issuer = metadata
        .authorization_servers
        .first()
        .ok_or_else(|| refused("no-authorization-server"))?;
    let authorization = authorization_server(http, issuer).await?;
    let scopes = match &challenge.scope {
        Some(scope) => words(scope),
        None => metadata.scopes,
    };
    tracing::info!(
        scopes = scopes.len(),
        registration = authorization.registration_endpoint.is_some(),
        client_metadata = authorization.client_metadata,
        "mcp.oauth.discovered"
    );
    Ok(Discovery {
        resource: metadata.resource,
        scopes,
        server: authorization,
    })
}

/// An unauthenticated MCP request, for the server's challenge. Any answer but a `401` with a
/// `Bearer` challenge means no challenge (the well-known URIs are tried next).
async fn probe(http: &Http<'_>, url: &Url) -> DomainResult<Challenge> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    let answer = http
        .send(url, |client| {
            client
                .post(url.clone())
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROBE_VERSION)
                .body(body.to_string())
        })
        .await
        .map_err(|error| trouble("probe", error))?;
    if answer.status != 401 {
        return Ok(Challenge::default());
    }
    let headers = answer
        .headers
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok());
    Ok(Challenge::parse(headers))
}

/// Protected resource metadata, validated.
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes: Vec<String>,
}

/// The challenge's metadata URL, else the well-known URIs in the specification's order (with the
/// endpoint's path inserted, then at the root).
async fn resource_metadata(
    http: &Http<'_>,
    url: &Url,
    challenge: &Challenge,
) -> DomainResult<ResourceMetadata> {
    let candidates = match &challenge.resource_metadata {
        Some(location) => vec![url.join(location).map_err(|_| refused("metadata-url"))?],
        None => well_known(url, "oauth-protected-resource"),
    };
    for candidate in candidates {
        let answer = http
            .get(&candidate)
            .await
            .map_err(|error| trouble("resource-metadata", error))?;
        if !answer.success() {
            continue;
        }
        let object = answer
            .object()
            .ok_or_else(|| refused("resource-metadata"))?;
        return resource(url, &object);
    }
    Err(DomainError::upstream(
        "The tool server does not say where to sign in (it publishes no protected resource metadata). Use a bearer token or headers instead.",
        OAUTH_DISCOVERY,
    ))
}

/// Checks the metadata against the server's URL: its `resource` must cover the URL (same
/// origin, and the URL's path at or below the resource's), else the metadata is someone
/// else's.
fn resource(url: &Url, object: &Map<String, Value>) -> DomainResult<ResourceMetadata> {
    let resource = match object.get("resource").and_then(Value::as_str) {
        Some(resource) => {
            let parsed = Url::parse(resource).map_err(|_| refused("resource"))?;
            if !covers(&parsed, url) {
                return Err(DomainError::upstream(
                    "The tool server's sign-in metadata belongs to another address. Check its URL.",
                    OAUTH_DISCOVERY,
                ));
            }
            resource.to_owned()
        }
        None => canonical(url),
    };
    let authorization_servers = strings(object.get("authorization_servers"));
    if authorization_servers.is_empty() {
        return Err(refused("no-authorization-server"));
    }
    Ok(ResourceMetadata {
        resource,
        authorization_servers,
        scopes: strings(object.get("scopes_supported")),
    })
}

/// Whether `resource` names `url` or a path above it on the same origin.
fn covers(resource: &Url, url: &Url) -> bool {
    if resource.origin() != url.origin() || resource.fragment().is_some() {
        return false;
    }
    let base = resource.path().trim_end_matches('/');
    let path = url.path().trim_end_matches('/');
    base.is_empty() || path == base || path.starts_with(&format!("{base}/"))
}

/// The server's URL as a resource identifier: no fragment, and no trailing slash on a bare
/// origin.
fn canonical(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    let text = url.to_string();
    if url.path() == "/" && url.query().is_none() {
        text.trim_end_matches('/').to_owned()
    } else {
        text
    }
}

/// The well-known URIs of `suffix` for `url`: with its path inserted (when it has one), then at
/// the root.
fn well_known(url: &Url, suffix: &str) -> Vec<Url> {
    let path = url.path().trim_end_matches('/');
    let mut candidates = Vec::new();
    let mut at = |path: String| {
        let mut candidate = url.clone();
        candidate.set_path(&path);
        candidate.set_query(None);
        candidate.set_fragment(None);
        candidates.push(candidate);
    };
    if !path.is_empty() {
        at(format!("/.well-known/{suffix}{path}"));
    }
    at(format!("/.well-known/{suffix}"));
    candidates
}

/// The issuer's metadata from the RFC 8414 and OpenID Connect URLs in the specification's order,
/// validated.
async fn authorization_server(http: &Http<'_>, issuer: &str) -> DomainResult<AuthServer> {
    let parsed = Url::parse(issuer).map_err(|_| refused("issuer"))?;
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(refused("issuer"));
    }
    http.allows(&parsed).map_err(|blocked| {
        trouble(
            "authorization-metadata",
            crate::oauth::http::Trouble::Blocked(blocked),
        )
    })?;
    for candidate in metadata_urls(&parsed) {
        let answer = http
            .get(&candidate)
            .await
            .map_err(|error| trouble("authorization-metadata", error))?;
        if !answer.success() {
            continue;
        }
        return validate(http, issuer, &answer);
    }
    Err(DomainError::upstream(
        "Could not read the sign-in server's metadata.",
        OAUTH_DISCOVERY,
    ))
}

/// For an issuer with a path: RFC 8414 and OpenID Connect with the path inserted, then OpenID
/// Connect with it appended; without one: RFC 8414, then OpenID Connect.
fn metadata_urls(issuer: &Url) -> Vec<Url> {
    let path = issuer.path().trim_end_matches('/');
    let at = |path: String| {
        let mut url = issuer.clone();
        url.set_path(&path);
        url
    };
    if path.is_empty() {
        vec![
            at("/.well-known/oauth-authorization-server".to_owned()),
            at("/.well-known/openid-configuration".to_owned()),
        ]
    } else {
        vec![
            at(format!("/.well-known/oauth-authorization-server{path}")),
            at(format!("/.well-known/openid-configuration{path}")),
            at(format!("{path}/.well-known/openid-configuration")),
        ]
    }
}

/// The metadata's issuer must be identical to the one its URL was built from, PKCE `S256` must be
/// listed, the endpoints the server calls must pass the outbound policy, and the authorization
/// endpoint (which the user's browser opens) must be `https` (plain `http` only for an account
/// that may reach private addresses).
fn validate(http: &Http<'_>, issuer: &str, answer: &Answer) -> DomainResult<AuthServer> {
    let object = answer
        .object()
        .ok_or_else(|| refused("authorization-metadata"))?;
    if object.get("issuer").and_then(Value::as_str) != Some(issuer) {
        tracing::warn!("mcp.oauth.issuer-mismatch");
        return Err(DomainError::upstream(
            "The sign-in server's metadata names another issuer, so Copilot does not use it.",
            OAUTH_DISCOVERY,
        ));
    }
    if !strings(object.get("code_challenge_methods_supported"))
        .iter()
        .any(|method| method == "S256")
    {
        return Err(DomainError::upstream(
            "The sign-in server does not support PKCE with S256, which MCP sign-in requires.",
            OAUTH_DISCOVERY,
        ));
    }
    if object.contains_key("response_types_supported")
        && !strings(object.get("response_types_supported"))
            .iter()
            .any(|kind| kind == "code")
    {
        return Err(refused("response-types"));
    }
    let endpoint = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .and_then(|text| Url::parse(text).ok())
    };
    let insecure = || {
        DomainError::upstream(
            "The sign-in server's endpoints must use HTTPS.",
            OAUTH_DISCOVERY,
        )
    };
    let authorization_endpoint = endpoint("authorization_endpoint").ok_or_else(insecure)?;
    let browser_safe = match authorization_endpoint.scheme() {
        "https" => true,
        "http" => http.network == NetworkScope::Private,
        _ => false,
    };
    if !browser_safe
        || !authorization_endpoint.username().is_empty()
        || authorization_endpoint.password().is_some()
    {
        return Err(insecure());
    }
    let token_endpoint = endpoint("token_endpoint").ok_or_else(insecure)?;
    http.allows(&token_endpoint).map_err(|_| insecure())?;
    let registration_endpoint =
        endpoint("registration_endpoint").filter(|url| http.allows(url).is_ok());
    let flag = |key: &str| object.get(key).and_then(Value::as_bool).unwrap_or(false);
    Ok(AuthServer {
        issuer: issuer.to_owned(),
        authorization_endpoint,
        token_endpoint,
        registration_endpoint,
        client_metadata: flag("client_id_metadata_document_supported"),
        iss_parameter: flag("authorization_response_iss_parameter_supported"),
        offline_access: strings(object.get("scopes_supported"))
            .iter()
            .any(|scope| scope == "offline_access"),
        auth_methods: strings(object.get("token_endpoint_auth_methods_supported")),
    })
}

/// The strings of a JSON array; anything else is empty.
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(ToOwned::to_owned).collect()
}

/// Metadata that breaks a rule; `reason` goes to the log only.
fn refused(reason: &'static str) -> DomainError {
    tracing::warn!(reason, "mcp.oauth.metadata-refused");
    DomainError::upstream(
        "The tool server's sign-in metadata is not usable.",
        OAUTH_DISCOVERY,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn builds_the_well_known_resource_uris_in_order() {
        let found: Vec<String> = well_known(
            &url("https://a.example/public/mcp?x=1"),
            "oauth-protected-resource",
        )
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            found,
            [
                "https://a.example/.well-known/oauth-protected-resource/public/mcp",
                "https://a.example/.well-known/oauth-protected-resource",
            ]
        );
        let root: Vec<String> = well_known(&url("https://a.example/"), "oauth-protected-resource")
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            root,
            ["https://a.example/.well-known/oauth-protected-resource"]
        );
    }

    #[test]
    fn builds_the_metadata_uris_of_the_specification() {
        let with_path: Vec<String> = metadata_urls(&url("https://auth.example.com/tenant1"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            with_path,
            [
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant1",
                "https://auth.example.com/.well-known/openid-configuration/tenant1",
                "https://auth.example.com/tenant1/.well-known/openid-configuration",
            ]
        );
        let bare: Vec<String> = metadata_urls(&url("https://auth.example.com"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            bare,
            [
                "https://auth.example.com/.well-known/oauth-authorization-server",
                "https://auth.example.com/.well-known/openid-configuration",
            ]
        );
    }

    #[test]
    fn a_resource_covers_its_own_paths_on_its_origin_only() {
        let server = url("https://mcp.example.com/team/mcp");
        for resource in [
            "https://mcp.example.com",
            "https://mcp.example.com/",
            "https://mcp.example.com/team",
            "https://mcp.example.com/team/mcp",
            "https://MCP.example.com/team/mcp/",
        ] {
            assert!(covers(&url(resource), &server), "{resource}");
        }
        for resource in [
            "https://mcp.example.com/teams",
            "https://mcp.example.com/other/mcp",
            "https://evil.example.com/team/mcp",
            "http://mcp.example.com/team/mcp",
            "https://mcp.example.com:8443/team/mcp",
            "https://mcp.example.com/team/mcp#x",
        ] {
            assert!(!covers(&url(resource), &server), "{resource}");
        }
    }

    #[test]
    fn writes_canonical_resource_identifiers() {
        assert_eq!(
            canonical(&url("https://MCP.example.com/")),
            "https://mcp.example.com"
        );
        assert_eq!(
            canonical(&url("https://mcp.example.com/mcp#frag")),
            "https://mcp.example.com/mcp"
        );
    }
}
