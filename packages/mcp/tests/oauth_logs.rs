//! What reaches the application's log from a sign-in's OAuth requests, with the filter
//! `copilot-logger` installs at its most verbose: step names, statuses, and failure kinds, never
//! a token, a code, a verifier, a client secret, a URL, or the authorization server's text.
//!
//! One test per binary: the subscriber is process-global.

use std::io::Write;
use std::sync::{Arc, Mutex};

use copilot_core::{
    BaseUrl, ClientAuth, Grant, LogLevel, NetworkScope, OAuthClient, Secret, TokenRequest, ToolAuth,
};
use copilot_mcp::McpAuth;
use copilot_security::{Purpose, clients};
use serde_json::json;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use url::Url;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn tokens_and_server_text_never_reach_the_log() {
    let logged = Lines::default();
    let writer = logged.clone();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(move || writer.clone())
                .with_filter(copilot_logger::targets(LogLevel::Debug)),
        )
        .init();

    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("POST"))
        .and(path("/s/path-secret-1/mcp"))
        .respond_with(ResponseTemplate::new(401).insert_header(
            "WWW-Authenticate",
            format!(r#"Bearer resource_metadata="{uri}/prm""#).as_str(),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/prm"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{uri}/s/path-secret-1/mcp"),
            "authorization_servers": [uri],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": uri,
            "authorization_endpoint": format!("{uri}/authorize"),
            "token_endpoint": format!("{uri}/token"),
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("code=code-secret-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-secret-1", "token_type": "Bearer",
            "refresh_token": "refresh-secret-1",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=refresh-secret-1"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "description-secret-1",
        })))
        .mount(&server)
        .await;

    let auth = McpAuth::new(clients(Purpose::Tools).expect("clients build"));
    let url = BaseUrl::restore(&format!("{uri}/s/path-secret-1/mcp")).unwrap();
    auth.discover(&url, NetworkScope::Private).await.unwrap();
    let request = |grant| TokenRequest {
        endpoint: Url::parse(&format!("{uri}/token")).unwrap(),
        client: OAuthClient {
            id: "client-1".to_owned(),
            secret: Some(Secret::new("client-secret-1")),
            auth: ClientAuth::Post,
        },
        resource: url.as_str().to_owned(),
        grant,
    };
    let code = Grant::Code {
        code: Secret::new("code-secret-1"),
        verifier: Secret::new("verifier-secret-1"),
        redirect_uri: Url::parse("http://localhost:4000/api/oauth/callback").unwrap(),
    };
    auth.token(&request(code), NetworkScope::Private)
        .await
        .unwrap();
    let refresh = Grant::Refresh {
        token: Secret::new("refresh-secret-1"),
    };
    auth.token(&request(refresh), NetworkScope::Private)
        .await
        .unwrap_err();

    let text = String::from_utf8(logged.0.lock().unwrap().clone()).unwrap();
    assert!(text.contains("mcp.oauth.discovered"), "{text}");
    assert!(text.contains("mcp.oauth.token-rejected"), "{text}");
    for secret in [
        "path-secret-1",
        "code-secret-1",
        "verifier-secret-1",
        "client-secret-1",
        "access-secret-1",
        "refresh-secret-1",
        "description-secret-1",
    ] {
        assert!(!text.contains(secret), "{secret} reached the log: {text}");
    }
}
