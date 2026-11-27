//! `McpAuth` against a scripted MCP server and authorization server (wiremock on loopback),
//! through the hardened clients the composition root injects. Plain `http` on loopback is open to
//! private-scope accounts only, as for any tool server URL.

use copilot_core::{
    AuthServer, BaseUrl, ClientAuth, Grant, NetworkScope, OAUTH_BLOCKED, OAUTH_DISCOVERY,
    OAUTH_FAILED, OAUTH_REGISTRATION, OAUTH_REJECTED, OAuthClient, Registration, Secret,
    TokenRequest, ToolAuth,
};
use copilot_mcp::McpAuth;
use copilot_security::{Purpose, clients};
use serde_json::{Value, json};
use url::Url;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn auth() -> McpAuth {
    McpAuth::new(clients(Purpose::Tools).expect("clients build"))
}

fn base(server: &MockServer, path: &str) -> BaseUrl {
    BaseUrl::restore(&format!("{}{path}", server.uri())).unwrap()
}

/// Authorization server metadata for `issuer` on `server`.
fn metadata(server: &MockServer, issuer: &str) -> Value {
    let uri = server.uri();
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{uri}/authorize"),
        "token_endpoint": format!("{uri}/token"),
        "registration_endpoint": format!("{uri}/register"),
        "code_challenge_methods_supported": ["S256"],
        "response_types_supported": ["code"],
        "scopes_supported": ["files:read", "offline_access"],
        "authorization_response_iss_parameter_supported": true,
        "client_id_metadata_document_supported": true,
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
    })
}

async fn json_at(server: &MockServer, route: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// An MCP endpoint at `/mcp` that answers `401` with `challenge`.
async fn protected(server: &MockServer, challenge: &str) {
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(401).insert_header("WWW-Authenticate", challenge))
        .mount(server)
        .await;
}

#[tokio::test]
async fn discovers_through_the_challenge_and_openid_path_insertion() {
    let server = MockServer::start().await;
    let uri = server.uri();
    protected(
        &server,
        &format!(r#"Bearer resource_metadata="{uri}/meta/mcp", scope="files:read files:write""#),
    )
    .await;
    json_at(
        &server,
        "/meta/mcp",
        json!({
            "resource": format!("{uri}/mcp"),
            "authorization_servers": [format!("{uri}/tenant")],
            "scopes_supported": ["ignored"],
        }),
    )
    .await;
    // RFC 8414 with the path inserted is missing; OpenID Connect with it inserted answers.
    json_at(
        &server,
        "/.well-known/openid-configuration/tenant",
        metadata(&server, &format!("{uri}/tenant")),
    )
    .await;

    let found = auth()
        .discover(&base(&server, "/mcp"), NetworkScope::Private)
        .await
        .unwrap();
    assert_eq!(found.resource, format!("{uri}/mcp"));
    assert_eq!(
        found.scopes,
        ["files:read", "files:write"],
        "the challenge wins"
    );
    let authorization = &found.server;
    assert_eq!(authorization.issuer, format!("{uri}/tenant"));
    assert_eq!(authorization.authorization_endpoint.path(), "/authorize");
    assert_eq!(authorization.token_endpoint.path(), "/token");
    assert!(authorization.registration_endpoint.is_some());
    assert!(authorization.client_metadata && authorization.iss_parameter);
    assert!(authorization.offline_access);
    assert_eq!(authorization.auth_methods, ["none", "client_secret_post"]);

    let requests = server.received_requests().await.unwrap();
    let paths: Vec<&str> = requests.iter().map(|request| request.url.path()).collect();
    assert_eq!(
        paths,
        [
            "/mcp",
            "/meta/mcp",
            "/.well-known/oauth-authorization-server/tenant",
            "/.well-known/openid-configuration/tenant",
        ]
    );
    assert!(
        requests[0].headers.get("authorization").is_none(),
        "the probe carries no credentials"
    );
}

#[tokio::test]
async fn falls_back_to_the_well_known_uris() {
    let server = MockServer::start().await;
    let uri = server.uri();
    protected(&server, "Bearer").await;
    json_at(
        &server,
        "/.well-known/oauth-protected-resource",
        json!({
            "resource": uri,
            "authorization_servers": [uri],
            "scopes_supported": ["files:read"],
        }),
    )
    .await;
    json_at(
        &server,
        "/.well-known/oauth-authorization-server",
        metadata(&server, &uri),
    )
    .await;
    let found = auth()
        .discover(&base(&server, "/mcp"), NetworkScope::Private)
        .await
        .unwrap();
    assert_eq!(found.resource, uri, "the metadata's resource, as given");
    assert_eq!(
        found.scopes,
        ["files:read"],
        "no challenge scope: the metadata's"
    );
    let paths: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "/mcp",
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-authorization-server",
        ]
    );
}

/// A server with resource metadata at the root and authorization server metadata `edit`ed.
async fn with_metadata(edit: impl FnOnce(&mut Value)) -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    protected(&server, "Bearer").await;
    json_at(
        &server,
        "/.well-known/oauth-protected-resource",
        json!({"resource": format!("{uri}/mcp"), "authorization_servers": [uri]}),
    )
    .await;
    let mut document = metadata(&server, &uri);
    edit(&mut document);
    json_at(&server, "/.well-known/oauth-authorization-server", document).await;
    server
}

/// A change to authorization server metadata.
type Edit = Box<dyn FnOnce(&mut Value)>;

#[tokio::test]
async fn refuses_metadata_that_breaks_the_rules() {
    let cases: Vec<(&str, Edit)> = vec![
        (
            "another issuer",
            Box::new(|document| document["issuer"] = json!("https://honest.example")),
        ),
        (
            "no PKCE",
            Box::new(|document| {
                document
                    .as_object_mut()
                    .unwrap()
                    .remove("code_challenge_methods_supported");
            }),
        ),
        (
            "plain PKCE only",
            Box::new(|document| document["code_challenge_methods_supported"] = json!(["plain"])),
        ),
        (
            "a browser endpoint that is not http(s)",
            Box::new(|document| document["authorization_endpoint"] = json!("javascript:alert(1)")),
        ),
        (
            "a token endpoint at a metadata address",
            Box::new(|document| {
                document["token_endpoint"] = json!("http://169.254.169.254/token");
            }),
        ),
    ];
    for (case, edit) in cases {
        let server = with_metadata(edit).await;
        let error = auth()
            .discover(&base(&server, "/mcp"), NetworkScope::Private)
            .await
            .unwrap_err();
        assert_eq!(error.code(), OAUTH_DISCOVERY, "{case}");
    }
}

#[tokio::test]
async fn refuses_resource_metadata_of_another_server() {
    let server = MockServer::start().await;
    let uri = server.uri();
    protected(&server, "Bearer").await;
    json_at(
        &server,
        "/.well-known/oauth-protected-resource",
        json!({"resource": "https://elsewhere.example/mcp", "authorization_servers": [uri]}),
    )
    .await;
    let error = auth()
        .discover(&base(&server, "/mcp"), NetworkScope::Private)
        .await
        .unwrap_err();
    assert_eq!(error.code(), OAUTH_DISCOVERY);
    assert!(error.to_string().contains("another address"), "{error}");

    let bare = MockServer::start().await;
    protected(&bare, "Bearer").await;
    let error = auth()
        .discover(&base(&bare, "/mcp"), NetworkScope::Private)
        .await
        .unwrap_err();
    assert_eq!(error.code(), OAUTH_DISCOVERY, "no metadata anywhere");
}

#[tokio::test]
async fn follows_the_private_network_policy() {
    let server = with_metadata(|_| {}).await;
    let error = auth()
        .discover(&base(&server, "/mcp"), NetworkScope::Public)
        .await
        .unwrap_err();
    assert_eq!(error.code(), OAUTH_BLOCKED);
    assert!(
        !error.to_string().contains("127.0.0.1"),
        "no address in the message"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

fn authorization(server: &MockServer) -> AuthServer {
    let at = |route: &str| Url::parse(&format!("{}{route}", server.uri())).unwrap();
    AuthServer {
        issuer: server.uri(),
        authorization_endpoint: at("/authorize"),
        token_endpoint: at("/token"),
        registration_endpoint: Some(at("/register")),
        client_metadata: false,
        iss_parameter: false,
        offline_access: false,
        auth_methods: Vec::new(),
    }
}

fn registration() -> Registration {
    Registration {
        client_name: "Copilot".to_owned(),
        redirect_uri: Url::parse("http://localhost:4000/api/oauth/callback").unwrap(),
        application_type: "native",
        scope: Some("files:read".to_owned()),
    }
}

#[tokio::test]
async fn registers_a_public_client() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/register"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"client_id": "dyn-1"})))
        .mount(&server)
        .await;
    let client = auth()
        .register(
            &authorization(&server),
            &registration(),
            NetworkScope::Private,
        )
        .await
        .unwrap();
    assert_eq!(client.id, "dyn-1");
    assert_eq!(client.secret, None);
    assert_eq!(client.auth, ClientAuth::None);
    let request = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["client_name"], "Copilot");
    assert_eq!(
        body["redirect_uris"],
        json!(["http://localhost:4000/api/oauth/callback"])
    );
    assert_eq!(body["application_type"], "native");
    assert_eq!(
        body["grant_types"],
        json!(["authorization_code", "refresh_token"])
    );
    assert_eq!(body["token_endpoint_auth_method"], "none");
    assert_eq!(body["scope"], "files:read");
}

#[tokio::test]
async fn reads_a_confidential_registration_and_reports_a_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/register"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "client_id": "dyn-2",
            "client_secret": "dyn-secret",
            "token_endpoint_auth_method": "client_secret_post",
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/register"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_redirect_uri",
            "error_description": "secret server text",
        })))
        .mount(&server)
        .await;
    let client = auth()
        .register(
            &authorization(&server),
            &registration(),
            NetworkScope::Private,
        )
        .await
        .unwrap();
    assert_eq!(client.secret, Some(Secret::new("dyn-secret")));
    assert_eq!(client.auth, ClientAuth::Post);
    let error = auth()
        .register(
            &authorization(&server),
            &registration(),
            NetworkScope::Private,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), OAUTH_REGISTRATION);
    assert!(!error.to_string().contains("secret server text"));
}

fn token_request(server: &MockServer, grant: Grant) -> TokenRequest {
    TokenRequest {
        endpoint: Url::parse(&format!("{}/token", server.uri())).unwrap(),
        client: OAuthClient {
            id: "client-1".to_owned(),
            secret: None,
            auth: ClientAuth::None,
        },
        resource: "https://mcp.example.com/mcp".to_owned(),
        grant,
    }
}

#[tokio::test]
async fn exchanges_a_code_and_refreshes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code_verifier=verifier-1"))
        .and(body_string_contains(
            "resource=https%3A%2F%2Fmcp.example.com%2Fmcp",
        ))
        .and(body_string_contains("client_id=client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-1", "token_type": "Bearer", "expires_in": 3600,
            "refresh_token": "refresh-1", "scope": "files:read",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=refresh-1"))
        .and(body_string_contains(
            "resource=https%3A%2F%2Fmcp.example.com%2Fmcp",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-2", "token_type": "bearer",
        })))
        .mount(&server)
        .await;
    let code = Grant::Code {
        code: Secret::new("code-1"),
        verifier: Secret::new("verifier-1"),
        redirect_uri: Url::parse("http://localhost:4000/api/oauth/callback").unwrap(),
    };
    let tokens = auth()
        .token(&token_request(&server, code), NetworkScope::Private)
        .await
        .unwrap();
    assert_eq!(tokens.access, Secret::new("access-1"));
    assert_eq!(tokens.refresh, Some(Secret::new("refresh-1")));
    assert_eq!(
        tokens.expires_in,
        Some(std::time::Duration::from_secs(3600))
    );

    let refresh = Grant::Refresh {
        token: Secret::new("refresh-1"),
    };
    let refreshed = auth()
        .token(&token_request(&server, refresh), NetworkScope::Private)
        .await
        .unwrap();
    assert_eq!(refreshed.access, Secret::new("access-2"));
    assert_eq!(refreshed.refresh, None, "no rotation");
}

#[tokio::test]
async fn tells_a_rejected_grant_from_a_failed_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=revoked"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "secret server text",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=busy"))
        .respond_with(ResponseTemplate::new(503).set_body_string("secret server text"))
        .mount(&server)
        .await;
    let refresh = |token: &str| Grant::Refresh {
        token: Secret::new(token),
    };
    let rejected = auth()
        .token(
            &token_request(&server, refresh("revoked")),
            NetworkScope::Private,
        )
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), OAUTH_REJECTED);
    assert!(!rejected.to_string().contains("secret server text"));
    let failed = auth()
        .token(
            &token_request(&server, refresh("busy")),
            NetworkScope::Private,
        )
        .await
        .unwrap_err();
    assert_eq!(failed.code(), OAUTH_FAILED);
    assert!(!failed.to_string().contains("secret server text"));

    let blocked = auth()
        .token(
            &token_request(&server, refresh("busy")),
            NetworkScope::Public,
        )
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), OAUTH_BLOCKED);
}
