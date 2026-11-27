//! The MCP adapter against an in-process rmcp server in both eras, through the hardened
//! outbound clients the composition root injects (`copilot_security::clients(Purpose::Tools)`).
//! The server listens on loopback over plain `http`, which only a private-scope connection may
//! reach.

mod server;

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bytes::Bytes;
use copilot_core::shared::constants::{MCP_RESPONSE_MAX_BYTES, MCP_TOOLS_MAX};
use copilot_core::{
    BaseUrl, DomainError, NetworkScope, Secret, TOOL_TIMEOUT, ToolContent, ToolEndpoint,
    ToolHeaders, ToolHost, ToolServerId,
};
use copilot_mcp::{
    Limits, McpHost, TOOL_SERVER_AUTH, TOOL_SERVER_BLOCKED, TOOL_SERVER_FAILED,
    TOOL_SERVER_TOO_LARGE, TOOL_SERVER_UNREACHABLE, TOOL_UNKNOWN,
};
use copilot_security::{Purpose, clients};
use serde_json::{Value, json};

use crate::server::{Era, Options, PNG, Running, start, tool};

fn host() -> McpHost {
    let limits = Limits {
        connect: Duration::from_secs(5),
        list: Duration::from_secs(10),
        ..Limits::default()
    };
    McpHost::with_limits(clients(Purpose::Tools).expect("clients build"), limits)
}

fn endpoint(url: &str, network: NetworkScope) -> ToolEndpoint {
    ToolEndpoint {
        tokens: None,
        server_id: ToolServerId::generate(),
        url: BaseUrl::restore(url).expect("a URL"),
        headers: None,
        protocol_version: None,
        network,
    }
}

fn local(server: &Running) -> ToolEndpoint {
    endpoint(&server.url, NetworkScope::Private)
}

fn credentials(bearer: Option<&str>, headers: &[(&str, &str)]) -> Secret {
    let headers: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Secret::new(ToolHeaders::new(bearer, &headers).unwrap().to_json())
}

const CALL: Duration = Duration::from_secs(10);

async fn call(
    host: &McpHost,
    endpoint: &ToolEndpoint,
    name: &str,
    arguments: Value,
) -> Result<copilot_core::ToolOutput, DomainError> {
    let session = host.open(vec![endpoint.clone()]).await.unwrap();
    let output = session
        .call(endpoint.server_id, name, arguments, CALL)
        .await;
    session.close().await;
    output
}

fn texts(content: &[ToolContent]) -> Vec<String> {
    content
        .iter()
        .filter_map(|item| match item {
            ToolContent::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn lists_every_page_of_a_modern_server_without_sessions() {
    let server = start(
        Era::Modern,
        Options {
            ttl_ms: Some(120_000),
            ..Options::default()
        },
    )
    .await;
    let listing = host().list(&local(&server)).await.unwrap();

    assert_eq!(listing.protocol_version, "2026-07-28");
    let names: Vec<&str> = listing
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["echo", "region", "sleep", "big", "picture", "fail"],
        "both pages, in the server's order"
    );
    let echo = &listing.tools[0];
    assert_eq!(echo.title.as_deref(), Some("Echo"));
    assert_eq!(echo.description, "Echoes text.");
    assert!(echo.read_only, "readOnlyHint kept as a hint");
    assert_eq!(echo.input_schema["properties"]["text"]["type"], "string");
    assert!(!listing.tools[1].read_only);
    let ttl = listing.ttl.expect("every page had ttlMs");
    assert!(
        ttl <= Duration::from_secs(60) && ttl > Duration::from_secs(50),
        "the smaller ttlMs of the pages: {ttl:?}"
    );

    let seen = server.seen();
    assert_eq!(seen.count("server/discover"), 1);
    assert_eq!(
        seen.count("initialize"),
        0,
        "no handshake in the modern era"
    );
    assert_eq!(seen.count("tools/list"), 2);
    assert_eq!(seen.sessions, 0, "stateless");
}

#[tokio::test]
async fn lists_a_legacy_server_through_initialize_and_a_session() {
    for era in [Era::Legacy, Era::LegacyDiscover] {
        let server = start(era, Options::default()).await;
        let listing = host().list(&local(&server)).await.unwrap();
        assert_eq!(listing.protocol_version, "2025-11-25", "{era:?}");
        assert_eq!(listing.tools.len(), 6, "{era:?}");
        assert_eq!(listing.ttl, None, "no ttlMs from this server");

        let seen = server.seen();
        assert_eq!(
            seen.count("server/discover"),
            1,
            "{era:?}: one modern probe"
        );
        assert_eq!(seen.count("initialize"), 1, "{era:?}");
        assert!(seen.sessions > 0, "{era:?}: requests carry Mcp-Session-Id");
    }
}

#[tokio::test]
async fn remembers_the_era_of_a_legacy_server() {
    let server = start(Era::Legacy, Options::default()).await;
    let host = host();
    let endpoint = local(&server);
    host.list(&endpoint).await.unwrap();
    assert_eq!(
        host.remembered(endpoint.server_id, &server.url).as_deref(),
        Some("2025-11-25")
    );

    // The same process: no second probe.
    host.list(&endpoint).await.unwrap();
    call(&host, &endpoint, "echo", json!({"text": "hi"}))
        .await
        .unwrap();
    // A new process with the revision core stored from the listing: no probe either.
    let stored = ToolEndpoint {
        tokens: None,
        protocol_version: Some("2025-11-25".to_owned()),
        ..endpoint.clone()
    };
    self::host().list(&stored).await.unwrap();

    let seen = server.seen();
    assert_eq!(
        seen.count("server/discover"),
        1,
        "only the first connection probed"
    );
    assert_eq!(seen.count("initialize"), 4);
}

#[tokio::test]
async fn probes_again_when_the_remembered_era_is_out_of_date() {
    let server = start(Era::ModernOnly, Options::default()).await;
    let stored = ToolEndpoint {
        tokens: None,
        protocol_version: Some("2025-11-25".to_owned()),
        ..local(&server)
    };
    let host = host();
    let listing = host.list(&stored).await.unwrap();
    assert_eq!(listing.protocol_version, "2026-07-28");
    assert_eq!(
        host.remembered(stored.server_id, &server.url).as_deref(),
        Some("2026-07-28")
    );
    let seen = server.seen();
    assert_eq!(seen.count("initialize"), 1, "the remembered era first");
    assert_eq!(seen.count("server/discover"), 1, "then the probe");
}

#[tokio::test]
async fn sends_the_bearer_token_and_custom_headers_on_every_request() {
    for era in [Era::Modern, Era::Legacy] {
        let server = start(
            era,
            Options {
                require: Some("Bearer tok-1".to_owned()),
                ..Options::default()
            },
        )
        .await;
        let endpoint = ToolEndpoint {
            tokens: None,
            headers: Some(credentials(Some("tok-1"), &[("X-Team", "core")])),
            ..local(&server)
        };
        let host = host();
        host.list(&endpoint).await.unwrap();
        let output = call(&host, &endpoint, "echo", json!({"text": "hi"}))
            .await
            .unwrap();
        assert_eq!(texts(&output.content), ["echo: hi"]);

        let seen = server.seen();
        assert!(seen.methods.len() >= 4, "{era:?}: {:?}", seen.methods);
        assert!(
            seen.authorization
                .iter()
                .all(|value| value == "Bearer tok-1"),
            "{era:?}"
        );
        assert_eq!(
            seen.authorization.len(),
            seen.requests,
            "{era:?}: one Authorization header on every request"
        );
        assert!(seen.team.iter().all(|value| value == "core"), "{era:?}");
        assert_eq!(seen.team.len(), seen.requests, "{era:?}");
    }
}

#[tokio::test]
async fn reports_rejected_and_missing_credentials() {
    let server = start(
        Era::Modern,
        Options {
            require: Some("Bearer right".to_owned()),
            ..Options::default()
        },
    )
    .await;
    let wrong = ToolEndpoint {
        tokens: None,
        headers: Some(credentials(Some("wrong"), &[])),
        ..local(&server)
    };
    let error = host().list(&wrong).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_AUTH);
    assert_eq!(
        error.to_string(),
        "The tool server rejected the credentials. Check its bearer token and headers."
    );

    let error = host().list(&local(&server)).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_AUTH);
    assert_eq!(
        error.to_string(),
        "The tool server requires sign-in. Add a bearer token or headers, or sign in with OAuth, in its settings."
    );

    let error = call(&host(), &wrong, "echo", json!({})).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_AUTH);
}

#[tokio::test]
async fn calls_tools_in_both_eras_on_one_connection_per_run() {
    for (era, json) in [
        (Era::Modern, false),
        (Era::Modern, true),
        (Era::Legacy, false),
        (Era::LegacyDiscover, true),
    ] {
        let server = start(
            era,
            Options {
                json,
                ..Options::default()
            },
        )
        .await;
        let endpoint = local(&server);
        let session = host().open(vec![endpoint.clone()]).await.unwrap();
        let id = endpoint.server_id;

        let echo = session
            .call(id, "echo", json!({"text": "hello"}), CALL)
            .await
            .unwrap();
        assert!(!echo.is_error);
        assert_eq!(texts(&echo.content), ["echo: hello"], "{era:?}");

        let picture = session.call(id, "picture", json!({}), CALL).await.unwrap();
        assert_eq!(
            picture.content,
            vec![
                ToolContent::Image {
                    media_type: "image/png".to_owned(),
                    data: Bytes::from_static(PNG),
                },
                ToolContent::Text("a pixel".to_owned()),
            ],
            "{era:?}"
        );

        let failed = session.call(id, "fail", json!({}), CALL).await.unwrap();
        assert!(failed.is_error, "a tool error is a result, not a failure");
        assert_eq!(texts(&failed.content), ["the disk is full"]);
        session.close().await;

        let seen = server.seen();
        assert_eq!(seen.count("tools/call"), 3, "{era:?}");
        assert_eq!(
            seen.count("server/discover") + seen.count("initialize"),
            match era {
                Era::Modern | Era::ModernOnly => 1,
                Era::Legacy | Era::LegacyDiscover => 2,
            },
            "{era:?}: one connection for the run"
        );
    }
}

#[tokio::test]
async fn times_out_a_slow_call_and_cancels_it_on_the_server() {
    for era in [Era::Modern, Era::Legacy] {
        let server = start(era, Options::default()).await;
        let endpoint = local(&server);
        let session = host().open(vec![endpoint.clone()]).await.unwrap();
        let id = endpoint.server_id;
        // Connect first, so the timeout below covers the call only.
        session.call(id, "echo", json!({}), CALL).await.unwrap();

        let started = Instant::now();
        let error = session
            .call(
                id,
                "sleep",
                json!({"ms": 30_000}),
                Duration::from_millis(300),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), TOOL_TIMEOUT, "{era:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "{era:?}");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !server.server.cancelled.load(Ordering::SeqCst) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            server.server.cancelled.load(Ordering::SeqCst),
            "{era:?}: the server saw the request cancelled"
        );

        let after = session
            .call(id, "echo", json!({"text": "still here"}), CALL)
            .await
            .unwrap();
        assert_eq!(texts(&after.content), ["echo: still here"], "{era:?}");
        session.close().await;
    }
}

#[tokio::test]
async fn refuses_a_response_over_the_cap_and_leaves_result_cuts_to_core() {
    for json in [true, false] {
        let server = start(
            Era::Modern,
            Options {
                json,
                ..Options::default()
            },
        )
        .await;
        let endpoint = local(&server);
        let session = host().open(vec![endpoint.clone()]).await.unwrap();
        let id = endpoint.server_id;

        let error = session
            .call(id, "big", json!({"bytes": MCP_RESPONSE_MAX_BYTES}), CALL)
            .await
            .unwrap_err();
        if json {
            assert_eq!(error.code(), TOOL_SERVER_TOO_LARGE);
            assert_eq!(
                error.to_string(),
                "The tool server sent more than Copilot accepts in one response (4 MiB)."
            );
        } else {
            // rmcp ends the connection for an oversized SSE event and reports only the end.
            assert_eq!(error.code(), TOOL_SERVER_FAILED);
            assert!(error.to_string().contains("(4 MiB)"), "{error}");
        }

        // Under the response cap the text arrives whole (core cuts it to
        // MCP_RESULT_MAX_BYTES with its note), on a new connection when the old one ended.
        let whole = session
            .call(id, "big", json!({"bytes": 200_000}), CALL)
            .await
            .unwrap();
        assert_eq!(texts(&whole.content)[0].len(), 200_000, "json: {json}");
        session.close().await;
    }
}

#[tokio::test]
async fn refuses_a_tool_list_over_the_cap() {
    let server = start(Era::Modern, Options::default()).await;
    let huge = "d".repeat(MCP_RESPONSE_MAX_BYTES / 4);
    let tools = (0..5)
        .map(|index| {
            let mut tool = tool("t", "", json!({}));
            tool.name = format!("tool{index}").into();
            tool.description = Some(huge.clone().into());
            tool
        })
        .collect();
    server.set_tools(tools);
    let error = host().list(&local(&server)).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_TOO_LARGE);
}

#[tokio::test]
async fn refuses_a_tool_list_with_too_many_tools() {
    let options = Options {
        page_size: Some(200),
        ..Options::default()
    };
    let server = start(Era::Modern, options).await;
    let named = |count: usize| {
        (0..count)
            .map(|index| {
                let mut tool = tool("t", "", json!({}));
                tool.name = format!("tool{index}").into();
                tool
            })
            .collect()
    };
    server.set_tools(named(MCP_TOOLS_MAX));
    let listing = host().list(&local(&server)).await.unwrap();
    assert_eq!(listing.tools.len(), MCP_TOOLS_MAX);
    server.set_tools(named(MCP_TOOLS_MAX + 1));
    let error = host().list(&local(&server)).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_TOO_LARGE);
    assert!(error.to_string().contains("more than 512 tools"), "{error}");
}

#[tokio::test]
async fn lists_again_and_retries_after_a_header_mismatch() {
    let server = start(Era::Modern, Options::default()).await;
    let endpoint = local(&server);
    // A fresh connection has no input schemas yet, so the first call lacks the
    // Mcp-Param-Region header that the tool's x-mcp-header asks for.
    let output = call(&host(), &endpoint, "region", json!({"region": "eu-west"}))
        .await
        .unwrap();
    assert_eq!(texts(&output.content), ["region: eu-west"]);

    let seen = server.seen();
    assert_eq!(seen.count("tools/call"), 2, "rejected once, then retried");
    assert_eq!(
        seen.count("tools/list"),
        2,
        "listed again in between (two pages)"
    );
}

#[tokio::test]
async fn fails_a_tool_the_server_no_longer_lists() {
    let server = start(Era::Modern, Options::default()).await;
    let endpoint = local(&server);
    let listing = host().list(&endpoint).await.unwrap();
    assert!(listing.tools.iter().any(|tool| tool.name == "echo"));

    server.set_tools(vec![tool("other", "Something else.", json!({}))]);
    let error = call(&host(), &endpoint, "echo", json!({"text": "hi"}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), TOOL_UNKNOWN);
    assert_eq!(
        error.to_string(),
        "The tool server no longer offers this tool. Test the server to refresh its tool list."
    );
    let seen = server.seen();
    assert_eq!(seen.count("tools/call"), 1, "never retried");
}

#[tokio::test]
async fn refuses_private_and_metadata_addresses_through_the_hardened_client() {
    let server = start(Era::Modern, Options::default()).await;
    let port = server
        .url
        .split(':')
        .nth(2)
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let cases = [
        // Plain http is only for private-scope connections.
        endpoint(&server.url, NetworkScope::Public),
        // A name that resolves to loopback, for a public-only connection: the resolver refuses.
        endpoint(
            &format!("https://localhost:{port}/mcp"),
            NetworkScope::Public,
        ),
        // Cloud metadata, whatever the scope.
        endpoint("http://169.254.169.254/mcp", NetworkScope::Private),
        // A loopback literal for a public-only connection.
        endpoint(
            &format!("https://127.0.0.1:{port}/mcp"),
            NetworkScope::Public,
        ),
    ];
    for refused in cases {
        let error = host().list(&refused).await.unwrap_err();
        assert_eq!(error.code(), TOOL_SERVER_BLOCKED, "{}", refused.url);
        let error = call(&host(), &refused, "echo", json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.code(), TOOL_SERVER_BLOCKED, "{}", refused.url);
    }
    assert!(
        server.seen().methods.is_empty(),
        "nothing reached the server"
    );
}

#[tokio::test]
async fn reports_an_unreachable_server_and_fails_later_calls_without_retrying() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = endpoint(&format!("http://{address}/mcp"), NetworkScope::Private);
    let error = host().list(&endpoint).await.unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_UNREACHABLE);
    assert!(
        !error.to_string().contains(&address.to_string()),
        "no address echoed"
    );

    let session = host().open(vec![endpoint.clone()]).await.unwrap();
    for _ in 0..2 {
        let error = session
            .call(endpoint.server_id, "echo", json!({}), CALL)
            .await
            .unwrap_err();
        assert_eq!(error.code(), TOOL_SERVER_UNREACHABLE);
    }
    session.close().await;
}

#[tokio::test]
async fn fails_a_call_to_a_server_outside_the_run() {
    let server = start(Era::Modern, Options::default()).await;
    let session = host().open(vec![local(&server)]).await.unwrap();
    let error = session
        .call(ToolServerId::generate(), "echo", json!({}), CALL)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "This tool server is not part of this run."
    );
    session.close().await;
    assert!(server.seen().methods.is_empty());
}

#[tokio::test]
async fn never_echoes_the_body_of_a_url_that_is_not_mcp() {
    let router = axum::Router::new().fallback(|| async {
        (
            axum::http::StatusCode::NOT_FOUND,
            "<html>secret-body</html>",
        )
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let endpoint = endpoint(&format!("http://{address}/nothing"), NetworkScope::Private);

    let error = host().list(&endpoint).await.unwrap_err();
    assert!(!error.to_string().contains("secret-body"), "{error}");
    assert_eq!(error.to_string(), "The tool server answered with HTTP 404.");
}

/// Access tokens that start at `current` and change to `fresh` once one is rejected, recording
/// every rejection; with `refusal`, a rejection fails with that code instead.
struct Tokens {
    current: std::sync::Mutex<String>,
    fresh: String,
    rejected: std::sync::Mutex<Vec<String>>,
    refusal: Option<&'static str>,
}

impl Tokens {
    fn new(current: &str, fresh: &str) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            current: std::sync::Mutex::new(current.to_owned()),
            fresh: fresh.to_owned(),
            rejected: std::sync::Mutex::default(),
            refusal: None,
        })
    }

    fn rejections(&self) -> Vec<String> {
        self.rejected.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl copilot_core::AccessTokens for Tokens {
    async fn current(&self) -> copilot_core::DomainResult<Secret> {
        Ok(Secret::new(self.current.lock().unwrap().clone()))
    }

    async fn rejected(&self, token: &Secret) -> copilot_core::DomainResult<Secret> {
        self.rejected
            .lock()
            .unwrap()
            .push(token.text().unwrap().to_owned());
        if let Some(code) = self.refusal {
            return Err(DomainError::business_rule("Sign in again.", code));
        }
        *self.current.lock().unwrap() = self.fresh.clone();
        Ok(Secret::new(self.fresh.clone()))
    }
}

fn signed_in(server: &Running, tokens: std::sync::Arc<Tokens>) -> ToolEndpoint {
    ToolEndpoint {
        tokens: Some(copilot_core::TokenSource(tokens)),
        ..local(server)
    }
}

#[tokio::test]
async fn sends_the_access_token_and_refreshes_it_once_on_401() {
    for era in [Era::Modern, Era::Legacy] {
        let server = start(
            era,
            Options {
                require: Some("Bearer fresh".to_owned()),
                ..Options::default()
            },
        )
        .await;
        let tokens = Tokens::new("stale", "fresh");
        let endpoint = signed_in(&server, tokens.clone());
        let listing = host().list(&endpoint).await.unwrap();
        assert_eq!(listing.tools.len(), 6, "{era:?}");
        assert_eq!(tokens.rejections(), ["stale"], "{era:?}: one refresh");
        let seen = server.seen();
        assert_eq!(seen.authorization[0], "Bearer stale", "{era:?}");
        assert!(
            seen.authorization[1..]
                .iter()
                .all(|value| value == "Bearer fresh"),
            "{era:?}: {:?}",
            seen.authorization
        );
    }
}

#[tokio::test]
async fn refreshes_a_token_the_server_rejects_in_the_middle_of_a_run() {
    let server = start(
        Era::Modern,
        Options {
            require: Some("Bearer one".to_owned()),
            ..Options::default()
        },
    )
    .await;
    let tokens = Tokens::new("one", "two");
    let endpoint = signed_in(&server, tokens.clone());
    let host = host();
    let session = host.open(vec![endpoint.clone()]).await.unwrap();
    let first = session
        .call(endpoint.server_id, "echo", json!({"text": "a"}), CALL)
        .await
        .unwrap();
    assert_eq!(texts(&first.content), ["echo: a"]);
    assert!(tokens.rejections().is_empty());

    server.require("Bearer two");
    let second = session
        .call(endpoint.server_id, "echo", json!({"text": "b"}), CALL)
        .await
        .unwrap();
    assert_eq!(texts(&second.content), ["echo: b"]);
    assert_eq!(tokens.rejections(), ["one"]);
    assert_eq!(
        server.seen().count("tools/call"),
        3,
        "the first call, the refused one, and its retry"
    );
    session.close().await;
}

#[tokio::test]
async fn a_failed_refresh_fails_with_its_own_error() {
    let server = start(
        Era::Modern,
        Options {
            require: Some("Bearer other".to_owned()),
            ..Options::default()
        },
    )
    .await;
    let tokens = std::sync::Arc::new(Tokens {
        refusal: Some(copilot_core::SIGN_IN_REQUIRED),
        ..std::sync::Arc::into_inner(Tokens::new("stale", "unused")).unwrap()
    });
    let endpoint = signed_in(&server, tokens.clone());
    let error = host().list(&endpoint).await.unwrap_err();
    assert_eq!(error.code(), copilot_core::SIGN_IN_REQUIRED);
    let error = call(&host(), &endpoint, "echo", json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), copilot_core::SIGN_IN_REQUIRED);
    assert_eq!(tokens.rejections(), ["stale", "stale"]);
}

#[tokio::test]
async fn a_refreshed_token_the_server_still_rejects_asks_for_sign_in() {
    let server = start(
        Era::Modern,
        Options {
            require: Some("Bearer neither".to_owned()),
            ..Options::default()
        },
    )
    .await;
    let tokens = Tokens::new("stale", "fresh");
    let error = host()
        .list(&signed_in(&server, tokens.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), TOOL_SERVER_AUTH);
    assert_eq!(
        error.to_string(),
        "The tool server did not accept the sign-in. Sign in to it again in its settings."
    );
    assert_eq!(tokens.rejections(), ["stale"], "one refresh only");
}
