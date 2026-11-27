# MCP Client Research

Status: research note, checked against live primary sources on 2026-11-26. Treat it as a draft: the MCP spec, the Rust SDK, and the Anthropic beta all moved several times this year, so re-check the linked pages before building. Claims I could not confirm from a primary source are marked **UNVERIFIED**. Line counts and effort figures are my estimates, not measurements, unless the text says "measured".

Scope: how Copilot's API (Rust, tokio 1.48, axum 0.8, reqwest) should act as an MCP client for user-registered remote servers (URL plus optional headers or bearer token): list tools, hand them to the model, run the tool calls the model asks for, and send the results back. Planned home: the `copilot-mcp` crate behind the `ToolHost` port (`docs/design/copilot.md`).

## Summary

| Question | Answer |
| --- | --- |
| Current MCP spec revision | **`2026-07-28`** ("current"). It removed the `initialize` handshake and protocol sessions; every request carries its version in `_meta`. ([Versioning](https://modelcontextprotocol.io/specification/versioning), [Changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)) |
| Do we still need the old handshake? | **Yes.** A modern-only client cannot talk to a `2025-11-25` server ("Fails" in the compatibility matrix). Copilot must be "dual-era". ([Versioning: Compatibility Matrix](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)) |
| Rust SDK | `rmcp` **3.12.0**, published 2026-11-26, MSRV 1.88, Apache-2.0, official Tier 1 SDK. ([crates.io](https://crates.io/crates/rmcp), [SDK tiers](https://modelcontextprotocol.io/docs/sdk)) |
| rmcp features we need | `default-features = false`, `features = ["client", "transport-streamable-http-client-reqwest"]`, plus `transport-child-process` for opt-in stdio. ([rmcp Cargo.toml](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/Cargo.toml)) |
| Footprint (measured) | +24 crates on top of a tokio + reqwest (rustls) + serde baseline of 95; +2 more for stdio; about 9 s extra `cargo check` on 4 cores. |
| Legacy HTTP+SSE (`2024-11-05`) | Deprecated in the spec; rmcp removed it in 0.11.0 and says it will not add it back. Do not support it in v1. ([rmcp README](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/README.md)) |
| Hand-written client instead? | Possible but not worth it: dual-era Streamable HTTP plus the new `Mcp-Method` / `Mcp-Name` / `x-mcp-header` rules is roughly 800 to 1,000 lines plus tests (estimate), and the transport changed shape twice in 20 months. |
| OAuth for v1? | Bearer token or static headers only is acceptable: authorization is **OPTIONAL** in the spec. Servers that only offer the OAuth flow will not work until v2 (rmcp `auth` feature). ([Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)) |
| Anthropic MCP connector | Useful but not for v1: Anthropic-only beta, public `https://` servers only, no approval pause, no custom headers, not ZDR-eligible. Run tools ourselves for every provider. ([MCP connector](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector)) |

Recommendation in one line: use `rmcp` 3.12 with minimal features, pass it our own SSRF-hardened `reqwest::Client`, connect with `ClientLifecycleMode::Auto` (modern first, legacy fallback), open one connection per server per chat turn, cap result sizes and time out every call ourselves, keep stdio behind `MCP_STDIO_ENABLED`, and leave OAuth and the Anthropic connector for later.

## 1. MCP Spec State

### Revisions

- The current revision is `2026-07-28`. Revisions are "Draft", "Current", or "Final". ([Versioning](https://modelcontextprotocol.io/specification/versioning))
- Earlier revisions Copilot will meet in the wild: `2025-11-25`, `2025-06-18`, `2025-03-26` (Streamable HTTP with sessions) and `2024-11-05` (HTTP+SSE). ([Streamable HTTP: Backward Compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http))

### What Changed in 2026-07-28 (Client-Relevant)

From the [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog):

1. Protocol sessions and the `Mcp-Session-Id` header are gone (SEP-2567). Servers that need state mint explicit handles passed as tool arguments.
2. No `initialize` / `notifications/initialized` handshake (SEP-2575). Every request carries `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` in `_meta`; clients SHOULD send `io.modelcontextprotocol/clientInfo`.
3. New mandatory `server/discover` RPC for up-front version selection; calling it is optional for clients.
4. The HTTP GET stream is replaced by `subscriptions/listen` (opt-in long-lived POST response stream for list-changed notifications).
5. `ping` and `logging/setLevel` are removed.
6. Server-to-client requests (sampling, elicitation, roots) are replaced by Multi Round-Trip Requests: a `tools/call` may return `resultType: "input_required"` and the client retries with `inputResponses` (SEP-2322).
7. Every result carries `resultType`; clients MUST treat a missing field from older servers as `"complete"`.
8. No SSE resumability (`Last-Event-ID`); a broken stream loses the request and the client MUST re-issue it with a new id.
9. `tools/list` results carry `ttlMs` and `cacheScope` cache hints (SEP-2549); servers SHOULD return tools in a deterministic order.
10. Deprecated: Roots, Sampling, Logging, the HTTP+SSE transport, and Dynamic Client Registration (in favor of Client ID Metadata Documents).

### Streamable HTTP in 2026-07-28

From [Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http):

- One MCP endpoint, POST only. "The client **MUST** include an `Accept` header listing both `application/json` and `text/event-stream`." The server answers each request with either one JSON object or an SSE stream scoped to that request; "The client **MUST** support both."
- Required request headers: `MCP-Protocol-Version` (must match the `_meta` value), `Mcp-Method` on all requests, and `Mcp-Name` on `tools/call`, `resources/read`, `prompts/get`. Values that are not plain ASCII use the `=?base64?...?=` sentinel.
- `x-mcp-header`: a tool's `inputSchema` may mark primitive properties to be mirrored as `Mcp-Param-{Name}` headers. "clients **MUST** support this feature", and clients on Streamable HTTP "**MUST** reject tool definitions where any `x-mcp-header` value violates these constraints" by dropping that tool from the list.
- Cancellation: "Closing the SSE response stream **MUST** be treated by the server as cancellation of that request." No `notifications/cancelled` over HTTP.
- Header/body mismatches get `400` with JSON-RPC error `-32020` (`HeaderMismatch`); unsupported versions get `400` with `-32022` (`UnsupportedProtocolVersionError`) listing `supported` versions. ([Versioning](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning))

### Older Shapes Still in Use

- `2025-03-26` through `2025-11-25`: the server may assign `Mcp-Session-Id` (ended with HTTP DELETE), the client may open a GET SSE stream, servers may send JSON-RPC requests on SSE streams, and streams resume with `Last-Event-ID`. ([Streamable HTTP: Earlier Revisions](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http))
- `2024-11-05` HTTP+SSE (two endpoints, GET SSE plus POST): "New implementations **SHOULD NOT** adopt it", and it is eligible for removal. Clients that want it probe with POST, then GET for an `endpoint` event. (same page)

### Era Detection and Why Copilot Must Be Dual-Era

The compatibility matrix in [Versioning](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning):

| Client | Server | Outcome |
| --- | --- | --- |
| Modern | Legacy | **Fails** |
| Legacy | Modern | **Fails** (HTTP: missing headers, `400`) |
| Dual-era | Modern or Legacy | Works |

On HTTP a dual-era client sends a modern request first; on `400` (the compatibility matrix says any `4xx`; a modern server also answers an unknown method with `404` plus JSON-RPC `-32601`) it inspects the body: a recognized modern JSON-RPC error means "modern, retry with a supported version"; anything else means "legacy, fall back to `initialize`". Clients "**SHOULD** cache the result for the lifetime of the ... origin (HTTP)" and "**MAY** persist it". Copilot can store the detected era per tool server. (same page)

## 2. The Official Rust SDK: rmcp

### Version and Status

- Latest: **3.12.0**, published 2026-11-26 14:58 UTC; `rust_version` 1.88 (Copilot's workspace uses 1.94); license Apache-2.0; about 32.6 M downloads in the last 90 days. ([crates.io API](https://crates.io/api/v1/crates/rmcp))
- docs.rs built 3.12.0. ([docs.rs](https://docs.rs/rmcp/3.12.0/rmcp/))
- The Rust SDK is listed as **Tier 1** next to TypeScript, Python, C#, Go and Ruby. ([SDKs](https://modelcontextprotocol.io/docs/sdk))
- "This SDK implements the stable MCP **`2026-07-28`** specification while remaining fully compatible with the **`2025-11-25`** release and earlier versions." ([rust-sdk README](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/README.md))
- Churn: 1.0.0 on 2026-03-03, 2.0.0 on 2026-06-27 (CHANGELOG date; crates.io publish time is 2026-06-29), 3.0.0 on 2026-07-28, then 19 further stable 3.x releases (3.0.1 through 3.12.0, i.e. twelve minor bumps plus patches) by 2026-11-26. That is three breaking majors in five months. ([CHANGELOG](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/CHANGELOG.md)) Mitigation: keep every rmcp type inside `copilot-mcp`, behind the `ToolHost` port, so a major bump touches one crate.

### Feature Flags

Defaults are `base64`, `chrono`, `macros`, `server`, which are server-side extras Copilot does not need. ([rmcp Cargo.toml](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/Cargo.toml), [rmcp README](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/README.md))

| Feature | What it gives | Copilot |
| --- | --- | --- |
| `client` | Client service, `call_tool`, `list_all_tools` | Yes |
| `transport-streamable-http-client` | Streamable HTTP client, transport-agnostic (pulls `client-side-sse`, `transport-worker`) | Implied |
| `transport-streamable-http-client-reqwest` | `impl StreamableHttpClient for reqwest::Client` (reqwest 0.13, `json` + `stream`, no TLS feature) | Yes |
| `reqwest` / `reqwest-native-tls` / `reqwest-tls-no-provider` | Turns on a reqwest TLS backend (rustls, native, or rustls without a crypto provider) | **Not needed** if we pass our own client: cargo unifies reqwest features, so the workspace reqwest decides TLS (measured: compiles without them) |
| `transport-child-process` | `TokioChildProcess` stdio client (adds `tokio/process`, `process-wrap`) | Yes, runtime-gated |
| `auth` | OAuth 2.1 client: discovery, PKCE, RFC 8707, CIMD, DCR, refresh (adds `oauth2`, `url`, `async-trait`) | v2 |
| `client-side-sse` | SSE parsing inside Streamable HTTP. "There is no separate 'SSE transport' to configure." | Implied |
| (none) | Legacy HTTP+SSE client. Removed in 0.11.0 (2025-12-08): "a **deliberate non-goal** ... `rmcp` will not add the legacy transport back." | Not available |

Proposed workspace entry:

```toml
# Cargo.toml [workspace.dependencies]
rmcp = { version = "3.12.0", default-features = false, features = [
  "client",
  "transport-streamable-http-client-reqwest",
  "transport-child-process",
] }
```

### Dependency Footprint (Measured)

Measured on 2026-11-26 with `cargo tree -e normal` in throwaway crates (resolved reqwest 0.13.5):

| Setup | Unique crates | Delta |
| --- | --- | --- |
| Baseline: tokio 1.48, reqwest 0.13 (`rustls`, `json`, `stream`), serde, serde_json, tracing, thiserror | 95 | |
| + rmcp `client` + `transport-streamable-http-client-reqwest` | 119 | +24 |
| + `transport-child-process` | 121 | +2 (`process-wrap`, `nix`) |
| + `auth` (on top of the HTTP client row) | 140 | +21 (`oauth2` 5, `chrono`, a second `rand` 0.8 and `sha2` 0.10, `thiserror` 1) |

The 24 added for the HTTP client: `rmcp`, `futures` (+ executor, io, macro), `indexmap`, `hashbrown`, `equivalent`, `jiff`, `jiff-core`, `sha2` 0.11 (+ `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `typenum`, `const-oid`, `cpufeatures`, `cfg-if`), `sse-stream`, `tokio-stream`, `unicode-normalization`, `tinyvec`, `unicode-general-category`. No `schemars`, no `rmcp-macros`, no `chrono`.

Compile cost: with the baseline already built, adding rmcp took about 8.8 s of `cargo check` on 4 cores (baseline itself: 21.9 s). Release build time and binary size were not measured.

### Minimal Client (Compile-Checked)

This compiles with `cargo check` against rmcp 3.12.0, reqwest 0.13.5 and tokio 1.x (rustc 1.97). I did not run it against a live server. API sources: [streamable_http.rs example](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/examples/clients/src/streamable_http.rs), [config struct](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/streamable_http_client.rs), [service/client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/service/client.rs).

```rust
use std::collections::HashMap;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // Extra headers the user registered for this server (decrypted from headers_sealed).
    let mut headers = HashMap::new();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str("user-secret")?,
    );

    let config = StreamableHttpClientTransportConfig::with_uri("https://mcp.example.com/mcp")
        // rmcp calls reqwest's bearer_auth, so pass the token without "Bearer ".
        .auth_header("user-token")
        .custom_headers(headers)
        .max_json_response_size(1024 * 1024);

    // Our own client: no redirects, connect timeout; the SSRF-checking resolver goes here.
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    let transport = StreamableHttpClientTransport::with_client(http, config);

    let info = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("copilot", env!("CARGO_PKG_VERSION")),
    );
    // Auto: probe the stateless 2026-07-28 `server/discover`, fall back to the legacy
    // `initialize` handshake (and Mcp-Session-Id) for 2025-11-25 and older servers.
    let lifecycle = ClientLifecycleMode::Auto {
        preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        legacy_version: Some(ProtocolVersion::V_2025_11_25),
    };
    let client = tokio::time::timeout(
        Duration::from_secs(15),
        info.serve_with_lifecycle(transport, lifecycle),
    )
    .await??;

    // Follows nextCursor (at most 64 pages since 3.12.0, then ListPaginationExceeded);
    // drops tools with invalid x-mcp-header annotations.
    let tools = client.list_all_tools().await?;
    for tool in &tools {
        tracing::info!(name = %tool.name, "mcp.tools.listed");
    }

    let mut arguments = serde_json::Map::new();
    arguments.insert("query".into(), serde_json::json!("rust"));
    let call = CallToolRequestParams::new("search").with_arguments(arguments);
    // Dropping the call future (here: on timeout) cancels the request on the server.
    let result = tokio::time::timeout(Duration::from_secs(60), client.call_tool(call)).await??;
    tracing::info!(is_error = ?result.is_error, parts = result.content.len(), "mcp.tools.called");

    client.cancel().await?;
    Ok(())
}
```

Notes on the API:

- `Tool` has `name`, `title`, `description`, `input_schema: Arc<JsonObject>`, `output_schema`, `annotations`, `icons`, `_meta`. `CallToolResult` has `content: Vec<ContentBlock>`, `structured_content: Option<Value>`, `is_error: Option<bool>`. ([model/tool.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/model/tool.rs), [model.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/model.rs))
- `ClientLifecycleMode::Auto` falls back "when the peer reports that it is legacy or does not respond within 10 seconds". `serve()` alone uses only the legacy `initialize` handshake, which fails against modern-only servers, so use `serve_with_lifecycle`. ([service/client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/service/client.rs), [rust-sdk README](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/README.md))
- rmcp emits and validates `Mcp-Method`, `Mcp-Name` and `Mcp-Param-*` automatically once `2026-07-28` is negotiated, including the Base64 sentinel. ([README: Standard HTTP Headers](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/README.md))
- `call_tool` drives MRTR rounds automatically through the local `ClientHandler`, up to 10 rounds; `call_tool_once` exposes `InputRequiredResult` for manual handling. With `ClientCapabilities::default()` Copilot advertises no elicitation or sampling. (same README)
- Since 3.6.0 the client cancels an abandoned `tools/call`. ([CHANGELOG](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/CHANGELOG.md))
- Auth failures are classifiable: `ServiceError::is_authorization_required()` and `auth_challenge()` (the `WWW-Authenticate` value). Use them to show "token rejected" on the server card. ([service.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/service.rs))

### Transport Knobs and Defaults

From [streamable_http_client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/streamable_http_client.rs) and [reqwest/streamable_http_client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/common/reqwest/streamable_http_client.rs):

| Field | Default | Copilot suggestion |
| --- | --- | --- |
| `max_json_response_size` | 16 MiB ("Oversized JSON bodies are rejected rather than truncated") | 2 to 4 MiB |
| `max_sse_event_size` | 16 MiB | Same as above |
| `max_error_response_size` | 64 KiB | Keep |
| `max_concurrent_requests` | 16 | Keep |
| `control_request_timeout` | 5 s | Keep |
| `allow_stateless` | `true` | Keep (needed for 2026-07-28) |
| `reinit_on_expired_session` | `true` (re-initializes on legacy `404`) | Keep |
| `auth_header` | none; sent as `Authorization: Bearer <value>` | Bearer token field |
| `custom_headers` | none; rejects `accept`, `mcp-session-id`, `last-event-id` (`mcp-protocol-version` is let through). The check runs when a request is sent, not in the builder, and fails that request with `StreamableHttpError::ReservedHeaderConflict` ([http_header.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/common/http_header.rs), [reqwest/streamable_http_client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/common/reqwest/streamable_http_client.rs)) | Validate names in the API before saving |

The default reqwest client (`from_uri` / `from_config`) follows only same-origin 307/308 redirects (max 10) and sets `pool_max_idle_per_host(0)` "to avoid ~40 ms stalls caused by TCP Delayed ACK". When we pass our own client with `with_client`, neither applies, so copy the pool setting and choose the redirect policy ourselves.

### stdio Child Process Transport

From [child_process.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/child_process.rs) and [async_rw.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/async_rw.rs):

- `TokioChildProcess::new(cmd)` pipes stdin/stdout and **inherits stderr**, so a child's logs would bypass `tracing`. Use `TokioChildProcess::builder(cmd).stderr(Stdio::piped()).spawn()`, which returns the stderr handle, and forward lines as `tracing` events.
- On shutdown it waits up to 3 s, then sends `SIGTERM` (Unix), then kills. Dropping the transport still kills the child.
- Incoming lines are capped at 16 MiB (`DEFAULT_MAX_LINE_LENGTH`).
- The spec says stdio implementations "**SHOULD NOT** follow this specification [authorization], and instead retrieve credentials from the environment." ([Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization))

### OAuth Support (`auth` Feature, for v2)

From [OAUTH_SUPPORT.md](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/docs/OAUTH_SUPPORT.md) and [auth.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/auth.rs):

- `OAuthState` runs discovery (RFC 9728 protected resource metadata, RFC 8414 and OIDC), registration in spec priority order (pre-registered, then Client ID Metadata Document, then Dynamic Client Registration), PKCE S256 ("never falls back to `plain`"), RFC 8707 `resource`, scope selection from `WWW-Authenticate`, and refresh.
- `get_authorization_url()` returns the URL to open; `handle_callback(code, state)` finishes it; `AuthClient::new(reqwest_client, manager)` is then passed to `StreamableHttpClientTransport::with_client`.
- Storage is pluggable: `CredentialStore` (`load`, `save`, `clear`, `on_refresh_token_rejected`) and `StateStore` (keyed by the CSRF state). Both can be Postgres-backed with sealed secrets, which also works for the Lambda binary and multiple API instances.
- `OAuthState::new(&url, Some(reqwest_client))` takes our SSRF-hardened client for discovery and token requests.

## 3. Hand-Written Alternative

If rmcp were too heavy, the minimum is JSON-RPC 2.0 over Streamable HTTP. To interoperate with both eras (required, see Section 1), a hand-written client needs:

| Piece | Rough size (estimate) |
| --- | --- |
| JSON-RPC envelope, ids, error types, content-block types | 150 lines |
| Modern requests: `_meta` fields, `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`, `x-mcp-header` extraction and validation, Base64 sentinel encoding, dropping invalid tools | 200 lines |
| Response handling: JSON or SSE (with a crate such as `sse-stream`), size limits, `resultType`, `-32020` / `-32022` handling | 150 lines |
| Legacy path: `initialize`, `notifications/initialized`, `Mcp-Session-Id`, re-init on `404`, server requests arriving on SSE streams | 200 lines |
| Era probe and fallback rules, `tools/list` pagination, `tools/call`, cancellation by dropping the stream | 150 lines |
| Tests against fake servers for both eras | 400 lines |

Total about 800 to 1,000 lines of production code plus tests, saving 24 small crates and roughly 9 s of `cargo check`. The bigger cost is upkeep: the transport changed incompatibly in `2025-03-26` and again in `2026-07-28`, and the MUST-level `x-mcp-header` rules are easy to get subtly wrong. A modern-only client would be about half the size but cannot reach `2025-11-25` servers, which is most of what users will paste today (**UNVERIFIED**: I found no census of server versions).

Verdict: use rmcp. Revisit only if a future rmcp major makes the client path heavier (watch `cargo tree` in CI).

## 4. Authorization: Is Bearer-Only Acceptable for v1?

Yes.

- "Authorization is **OPTIONAL** for MCP implementations." When supported over HTTP it **SHOULD** follow the OAuth 2.1 profile. ([Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization))
- Token usage rules Copilot must follow even with a static token: "MCP client **MUST** use the Authorization request header field", "authorization **MUST** be included in every HTTP request", and "Access tokens **MUST NOT** be included in the URI query string". (same page) rmcp's `auth_header` sends it on every request.
- Real servers accept static tokens: GitHub's remote server at `https://api.githubcopilot.com/mcp/` documents `"Authorization": "Bearer ${input:github_mcp_pat}"` with a personal access token. ([github-mcp-server README](https://github.com/github/github-mcp-server/blob/main/README.md))
- Not covered in v1: servers that only offer the OAuth flow (401 with `resource_metadata`, then browser consent). Show a clear message when `is_authorization_required()` is true and no token is set.

What a full OAuth client must do (for the v2 plan), per [Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization):

1. On `401`, read `resource_metadata` (and `scope`) from `WWW-Authenticate`; fetch Protected Resource Metadata (RFC 9728); pick an authorization server; fetch its metadata via RFC 8414 or OIDC discovery (clients **MUST** support both).
2. Get a client id: pre-registered, Client ID Metadata Document (**SHOULD** support), or Dynamic Client Registration (**MAY**, now deprecated).
3. Authorization code flow with PKCE, `resource` = canonical server URI ("**MUST** be included in both authorization requests and token requests"), record the expected `issuer` and validate `iss` on the callback (RFC 9207).
4. Refresh tokens kept confidential; on `403 insufficient_scope`, re-authorize with the union of scopes.

Outcome (P7.2, 2026-11-27): implemented, with the redirect URI at the API (`/api/oauth/callback`) as planned below, but without rmcp's `auth` feature. Rechecked that day: the current revision is still `2026-07-28` (Dynamic Client Registration is marked deprecated there, kept for servers without Client ID Metadata Document support), and rmcp 3.12.0 (2026-11-26) is still the newest release; its `auth` feature adds 14 crates to the server's graph (210 to 224), including second copies of `base64`, `rand`, `sha2`, and `thiserror`, and no `aws-lc-rs`. The four OAuth requests are hand-written in `copilot-mcp` on the hardened clients so every URL a server hands back picks its client by URL and network scope; see `docs/design/tracks/p7-2.md`.

Copilot-specific v2 shape: the redirect URI points at the API (for example `/tool-servers/{id}/oauth/callback`), not at the device. Web, the Linux desktop app and Android then all just open the authorization URL in a browser, and tokens never touch the client. A Client ID Metadata Document needs a public HTTPS URL for the Copilot instance; LAN-only installs fall back to DCR or a pre-registered client id.

## 5. Security

### stdio Servers Run Processes on the API Host

- The spec's "Local MCP Server Compromise" section: a client offering one-click local server setup "**MUST** implement proper consent mechanisms prior to executing commands", show "the exact command that will be executed, without truncation", and **SHOULD** sandbox with minimal privileges. ([Security Best Practices](https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices))
- For Copilot the command runs on the server, as the API's OS user, next to `DATABASE_URL` and `SECRET_KEY`. Keep `MCP_STDIO_ENABLED=false` by default (already in `docs/design/copilot.md`) and, when enabled:
  - only admins may create or edit stdio servers;
  - spawn the program directly with an argument list (no shell); clear the environment (`env_clear()`) and pass only the variables the admin listed; set a dedicated working directory;
  - pipe stderr into `tracing` with a dotted key such as `mcp.stdio.stderr`;
  - recommend a container or a separate low-privilege user in the deployment docs;
  - disable it in the Lambda binary (no long-lived processes there).
- stdio servers on a user's own laptop or phone cannot be reached by the API at all. That would need a client-side bridge; out of scope.

### SSRF for User-Supplied URLs

The API fetches URLs that users type, and with OAuth also URLs that the MCP server hands back. The SSRF section of the Security Best Practices page (a docs page linked from the spec, not part of `basic/authorization`) lists the targets (private ranges, `169.254.169.254` cloud metadata, localhost services, DNS rebinding, redirect chains) and says "MCP clients deployed to a server **MUST** consider SSRF risks and implement appropriate mitigations when fetching OAuth-related URLs." The MUST is scoped to OAuth-related URLs (`resource_metadata`, authorization server and token endpoints); the concrete mitigations are SHOULD-level: require HTTPS (except loopback in development), block private and reserved ranges per RFC 9728 Section 7.7, apply the same checks to redirect targets, consider egress proxies such as Smokescreen, and consider pinning DNS between check and use. Applying the same rules to the MCP server URLs users type is Copilot's own choice, and a sensible one, since the threat is identical. It warns: "Avoid implementing IP validation manually. Attackers exploit encoding tricks (octal, hex, IPv4-mapped IPv6)". ([Security Best Practices](https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices))

Copilot plan, sharing one hardened `reqwest::Client` between the provider and MCP adapters where possible:

1. **Parse with `url::Url`** (no string checks). Allow `https`; allow `http` only when `ALLOW_PRIVATE_NETWORK=true` and the host resolves to a private or loopback address (LAN Ollama-style setups).
2. **Filter at resolution time** with a custom `reqwest::dns::Resolve` set via `ClientBuilder::dns_resolver`. reqwest connects to the addresses the resolver returns, so the check and the connection use the same IPs (no rebinding window). ([reqwest 0.13.5 source](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html#method.dns_resolver))
3. **Check IP-literal hosts separately.** hyper-util skips the resolver when the host is already an IP ("If the host is already an IP addr (v4 or v6), skip resolving the dns"), so `https://169.254.169.254/` never reaches a custom resolver. (verified in [hyper-util 0.1.21 `connect/http.rs`](https://docs.rs/crate/hyper-util/0.1.21/source/src/client/legacy/connect/http.rs))
4. **Classify** with std methods, normalizing IPv4-mapped IPv6 first (`Ipv6Addr::to_ipv4_mapped`). Stable: `Ipv4Addr::is_private`, `is_loopback`, `is_link_local`, `is_unspecified`, `is_broadcast`; `Ipv6Addr::is_unique_local`, `is_unicast_link_local`, `is_loopback` (compiled on rustc 1.97 here). `Ipv4Addr::is_global` is still unstable (`E0658`). Add CGNAT `100.64.0.0/10` (Tailscale), `0.0.0.0/8`, multicast and reserved ranges by hand, with table-driven tests.
5. **Always block** link-local (`169.254.0.0/16`, `fe80::/10`, which covers cloud metadata), unspecified and multicast, even when `ALLOW_PRIVATE_NETWORK=true`.
6. **Redirects off** (`redirect::Policy::none()`), or a custom policy that re-runs the URL check on each hop. MCP endpoints rarely redirect.
7. **Proxy**: reqwest reads system proxy variables by default (`auto_sys_proxy`); with a proxy the target is resolved by the proxy and the resolver filter never runs. Either call `.no_proxy()` on this client or document that `HTTPS_PROXY` must point at an SSRF-safe egress proxy.
8. **Errors**: do not echo response bodies or resolved IPs from failed fetches back to the user (reconnaissance). Log them with dotted keys (`mcp.http.blocked`, `mcp.http.failed`).

### Tool Results: Size, Types, Trust

- The spec says clients **SHOULD** "Validate tool results before passing to LLM", "Implement timeouts for tool calls", "Show tool inputs to the user before calling the server", "Prompt for user confirmation on sensitive operations", and "Log tool usage for audit purposes". It also says "clients **MUST** consider tool annotations to be untrusted unless they come from trusted servers." ([Tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools))
- Error handling: protocol errors (unknown tool, malformed request) vs tool execution errors (`isError: true`). "Clients **SHOULD** provide tool execution errors to language models to enable self-correction." (same page)
- Content types in results: `text`, `image`, `audio`, `resource_link`, embedded `resource`, plus optional `structuredContent` (any JSON value; servers SHOULD also send it as text). (same page)
- Two caps:
  - transport cap: `max_json_response_size` / `max_sse_event_size` at 2 to 4 MiB, so a hostile server cannot make the API buffer 16 MiB per call;
  - model cap: after converting to model parts, truncate text to a configurable budget (suggested default 64 KiB) with a visible `[truncated N bytes]` note. Images pass through only when the model's `vision` flag is on and the image is under the provider's size limit; audio and binary blobs become a one-line notice; `resource_link` becomes its URI and name as text.
- Prompt injection: tool output is untrusted text that can instruct the model to call other tools. Default new servers to approval mode `ask`, show arguments on the approval card, and never render tool output as HTML.

### Timeouts and Cancellation

- Wrap each step in `tokio::time::timeout`: connect 10 s (`connect_timeout`), lifecycle start 15 s, `tools/list` 15 s, `tools/call` 60 s default (per-server override, hard max such as 300 s). Do not set reqwest's total `timeout()` on this client, because it would cut long SSE responses; `read_timeout` works as an idle timeout.
- On user "Stop", drop the in-flight futures: on modern HTTP, closing the response stream is the cancellation signal; rmcp sends `notifications/cancelled` where the older protocol needs it.
- Cap tool rounds per turn (the design already says "Stop after a maximum number of tool rounds").

### Secrets and Logging

- Headers and bearer tokens stay sealed at rest (`headers_sealed`, AES-256-GCM per the design) and are decrypted only to build the transport config. Never log header values, tool arguments, or results at `info`; log server id, tool name, duration, status and byte counts.
- Reject a custom `Authorization` header when the bearer field is set (rmcp would send both; reqwest `header()` appends). **UNVERIFIED**: how servers treat a duplicated `Authorization` header varies.

### Tool Names Across Providers

MCP allows names up to 128 characters including `.`, and only asks for uniqueness per server; aggregators "**SHOULD** implement a disambiguation strategy such as prefixing tool names with a server identifier". ([Tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools)) Provider limits differ:

| Provider | Function name rule | Source |
| --- | --- | --- |
| OpenAI Chat Completions | "Must be a-z, A-Z, 0-9, or contain underscores and dashes, with a maximum length of 64." | [Create chat completion](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create) |
| Anthropic Messages (custom tool) | `^[a-zA-Z0-9_-]{1,128}$` | [Messages reference](https://platform.claude.com/docs/en/api/http/messages.md) |
| Gemini `FunctionDeclaration.name` | a-z, A-Z, 0-9, `_`, `:`, `.`, `-`, max 128 (but `FunctionCall.name` excludes `:` and `.`) | [Gemini API reference](https://aistudio.google.com/api-reference/caching) |

So expose tools to every model as `^[a-zA-Z0-9_-]{1,64}$`: `{server_slug}__{tool}` with other characters replaced by `_`, truncated with a short hash suffix when longer than 64, and a per-request map back to `(server_id, original_name)`.

## 6. Anthropic's MCP Connector

### How It Works

From [MCP connector](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector):

- Status: beta. Headers: `mcp-client-2026-09-15` (adds tool-list pinning) or `mcp-client-2025-11-20`; `mcp-client-2025-04-04` is deprecated.
- Request: top-level `mcp_servers: [{ "type": "url", "url": "https://...", "name": "...", "authorization_token": "..." }]` and, for each server, exactly one `{ "type": "mcp_toolset", "mcp_server_name": "...", "default_config": {...}, "configs": {...} }` in `tools` (allowlist, denylist, `defer_loading` per tool, `cache_control`).
- Anthropic's servers call the MCP server during the request and return `mcp_tool_use` and `mcp_tool_result` content blocks (and, with the 2026-09-15 header, `mcp_tool_listing` blocks to send back unchanged).
- Limitations as documented: "only tool calls are currently supported"; "The server must be publicly exposed through HTTP (supports both Streamable HTTP and SSE transports). Local STDIO servers cannot be connected directly."; `url` "Must start with `https://`".
- Auth: "API consumers are expected to handle the OAuth flow and obtain the access token prior to making the API call, and to refresh the token as needed."
- Data: "The MCP connector is not covered by ZDR arrangements"; HIPAA: not eligible.
- Platforms (beta): Claude API, Claude Platform on AWS, Claude Models on Google Cloud (Anthropic managed), Microsoft Foundry; the 2026-09-15 header only on the first and third.
- Anthropic's own guidance: "Use the `mcp_servers` API parameter when you have remote servers accessible by URL and only need tool support. Use the client-side helpers when you need local servers, prompts, resources, or more control over the connection."

### Fit With Copilot

| Need from `docs/design/copilot.md` | Self-run (rmcp) | Anthropic connector |
| --- | --- | --- |
| Same tools for OpenAI-compatible, Anthropic and Gemini models | Yes | Anthropic only |
| Approval mode `ask` (pause, Approve / Deny card) | Yes | No approval step documented; tools run inside the API call |
| LAN or `http://` servers (self-hosters) | Yes, with `ALLOW_PRIVATE_NETWORK` | No: public `https://` only |
| Custom headers (API key headers) | Yes | Only `authorization_token` is documented |
| Our timeouts, size caps, audit log, `tool.requested` / `tool.finished` events | Yes | Partly: we only see the blocks afterwards |
| Unattended schedule runs that deny `ask` tools | Yes | Not enforceable |
| Data stays between Copilot and the MCP server | Yes | Tool definitions and results retained by Anthropic, not ZDR |
| Legacy HTTP+SSE servers | No | Yes |
| Fewer round trips, provider-side tool loop | No | Yes |

Recommendation: do not use the connector in v1. Running tools in `copilot-mcp` gives one code path, one persistence format (`tool call` / `tool result` parts), and the approval gate for every provider. A later, opt-in "let Anthropic call this server directly" toggle is reasonable when all of these hold: Anthropic model, public `https://` URL, approval mode `auto`, bearer-only auth. The Anthropic adapter would then move that server from `tools` into `mcp_servers` and map `mcp_tool_use` / `mcp_tool_result` into the same message parts.

For comparison, OpenAI's Responses API has a similar remote tool (`"type": "mcp"`, `server_url`, `authorization`, `allowed_tools`, `require_approval` with `mcp_approval_request` items), also for public servers. Copilot's OpenAI-compatible adapter targets Chat Completions, where that guide gives no configuration (**UNVERIFIED** whether Chat Completions supports it). ([OpenAI MCP guide](https://developers.openai.com/api/docs/guides/tools-connectors-mcp))

## 7. Recommended Shape for `copilot-mcp`

- **Port**: `ToolHost` in `copilot-core` deals in `ToolSpec { name, description, input_schema }`, `ToolCall`, `ToolOutput { parts, is_error, truncated }`. No rmcp types leak out (keeps `make boundaries` and future rmcp majors contained).
- **Connections**: one rmcp client per enabled server per chat turn, created lazily on first need, closed with `close_with_timeout` when the turn ends. This fits the stateless 2026-07-28 model, the Lambda binary, and legacy servers (rmcp manages `Mcp-Session-Id` within the turn). A pooled, idle-evicted connection cache is a later optimization for stdio servers with slow start-up.
- **Tool list cache**: store the last `tools/list` result per server (with fetch time and `ttlMs` when present) so the composer's tools menu and request building do not connect every time. Refresh on `POST /tool-servers/{id}/test`, on cache expiry, and when a call returns `-32020` (`HeaderMismatch`) or "unknown tool".
- **Era memory**: store the negotiated protocol version per server (`client.peer_info()` returns the server info, which has a `protocol_version` field; that it is filled the same way after the Discover path is **UNVERIFIED**). The spec allows persisting the era, and it avoids the up-to-10 s Auto probe against silent legacy servers.
- **Settings to add** (names are suggestions): `MCP_CALL_TIMEOUT_SECS` (60), `MCP_RESULT_MAX_BYTES` (65536), `MCP_RESPONSE_MAX_BYTES` (4194304), `MCP_MAX_TOOL_ROUNDS`; plus the existing `MCP_STDIO_ENABLED` and `ALLOW_PRIVATE_NETWORK`.
- **Data model additions** to `tool_servers` (names are suggestions): `protocol_version` (text, nullable), `tools_cached` (jsonb), `tools_fetched_at` (timestamptz), `call_timeout_secs` (integer, nullable).

## Open Questions and Unverified Points

- Share of public MCP servers already on `2026-07-28`: **UNVERIFIED**; it decides how often the legacy path runs, not whether it is needed.
- Which MCP revision the Anthropic connector speaks, and whether it accepts headers other than `authorization_token`: **UNVERIFIED** (the docs list only that field).
- Whether OpenAI Chat Completions supports remote MCP: **UNVERIFIED**.
- The minimal client above was compile-checked, not run against a live server.
- Release build time and binary size impact of rmcp: not measured.
- The 800 to 1,000 line figure for a hand-written client is an estimate.

## Sources

- MCP versioning: <https://modelcontextprotocol.io/specification/versioning>
- MCP 2026-07-28 changelog: <https://modelcontextprotocol.io/specification/2026-07-28/changelog>
- MCP versioning and compatibility matrix: <https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning>
- MCP Streamable HTTP: <https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http>
- MCP authorization: <https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization>
- MCP tools: <https://modelcontextprotocol.io/specification/2026-07-28/server/tools>
- MCP security best practices: <https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices>
- MCP SDK tiers: <https://modelcontextprotocol.io/docs/sdk>
- rmcp on crates.io: <https://crates.io/crates/rmcp>
- rmcp docs: <https://docs.rs/rmcp/3.12.0/rmcp/>
- rust-sdk README at tag: <https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/README.md>
- rmcp crate README, Cargo.toml, CHANGELOG at tag: <https://github.com/modelcontextprotocol/rust-sdk/tree/rmcp-v3.12.0/crates/rmcp>
- rmcp client examples at tag: <https://github.com/modelcontextprotocol/rust-sdk/tree/rmcp-v3.12.0/examples/clients>
- rmcp OAuth guide at tag: <https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/docs/OAUTH_SUPPORT.md>
- reqwest `ClientBuilder`: <https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html>
- hyper-util HTTP connector source: <https://docs.rs/crate/hyper-util/0.1.21/source/src/client/legacy/connect/http.rs>
- GitHub MCP server README: <https://github.com/github/github-mcp-server/blob/main/README.md>
- Anthropic MCP connector: <https://platform.claude.com/docs/en/agents-and-tools/mcp-connector>
- Anthropic Messages API reference: <https://platform.claude.com/docs/en/api/http/messages.md>
- OpenAI chat completion reference: <https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create>
- OpenAI MCP guide: <https://developers.openai.com/api/docs/guides/tools-connectors-mcp>
- Gemini API reference (FunctionDeclaration): <https://aistudio.google.com/api-reference/caching>

## Verification

Independent re-check on 2026-11-26 by a second agent, reading the live pages (Markdown exports of modelcontextprotocol.io and platform.claude.com, raw files at the `rmcp-v3.12.0` tag, and the crates.io API) rather than trusting the citations above.

| Claim | Result | Evidence |
| --- | --- | --- |
| `2026-07-28` is the current revision; it removes `initialize` and protocol sessions, adds `server/discover`, MRTR, `subscriptions/listen`, removes SSE resumability | Confirmed | [Versioning](https://modelcontextprotocol.io/specification/versioning) names `2026-07-28` as current; [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog) major changes 1 to 9 |
| Modern-only and legacy-only clients fail across eras; dual-era client probes modern, falls back to `initialize` on a 4xx without a recognized modern error | Confirmed | [Versioning: Compatibility Matrix](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning). Added the `404` / `-32601` nuance from [Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) |
| Streamable HTTP: `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`, `Mcp-Param-*` from `x-mcp-header`, drop invalid tools, closing the SSE stream cancels | Confirmed | [Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) sections Request Metadata, Custom Headers from Tool Parameters, Cancellation |
| rmcp 3.12.0 latest (2026-11-26, MSRV 1.88, Apache-2.0), Tier 1, implements `2026-07-28` and stays compatible with `2025-11-25`, `ClientLifecycleMode::Auto` probes `server/discover` then falls back | Confirmed | [crates.io API](https://crates.io/api/v1/crates/rmcp) (`max_version` 3.12.0, created 2026-11-26T14:58Z, `rust_version` 1.88), [SDK tiers](https://modelcontextprotocol.io/docs/sdk), [rust-sdk README](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/README.md), `DEFAULT_AUTO_DISCOVER_TIMEOUT` of 10 s in [service/client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/service/client.rs) |
| Release churn wording | Corrected | crates.io lists 20 stable 3.x releases (3.0.0 to 3.12.0), not twelve; 2.0.0 was published to crates.io on 2026-06-29 (CHANGELOG says 2026-06-27) |
| Feature flags, defaults `base64`, `chrono`, `macros`, `server`, reqwest 0.13 with only `json` + `stream`, HTTP+SSE removed in 0.11.0 and not coming back | Confirmed | [rmcp Cargo.toml](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/Cargo.toml), [CHANGELOG](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/CHANGELOG.md) 0.11.0 entry "remove SSE transport support (#562)", README "rmcp will not add the legacy transport back" |
| `StreamableHttpClientTransportConfig` fields, 16 MiB defaults, Bearer via `bearer_auth`, reserved headers, `with_client` with our own `reqwest::Client` | Confirmed, one detail added | [streamable_http_client.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.12.0/crates/rmcp/src/transport/streamable_http_client.rs), `DEFAULT_MAX_SSE_EVENT_SIZE = 16 * 1024 * 1024` in `client_side_sse.rs`. Added: the reserved-header check fails at send time, not in the builder |
| Authorization OPTIONAL; Bearer header on every request; never in the query string | Confirmed | [Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), Protocol Requirements and Token Requirements |
| Clients on servers MUST consider SSRF (block private ranges, validate redirects, DNS rebinding) | Corrected scope | Not on the Authorization page. The [Security Best Practices](https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices) page has it, and the MUST covers "fetching OAuth-related URLs"; the listed mitigations are SHOULD. Section 5 now says so |
| Anthropic MCP connector: beta headers `mcp-client-2026-09-15` / `mcp-client-2025-11-20`, tools only, public `https://` only, `authorization_token` only, `mcp_tool_use` / `mcp_tool_result` blocks, not ZDR-eligible | Confirmed | [MCP connector](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector) front matter (`zdr.eligibility: not-eligible`), Limitations, Field descriptions, Required beta headers, Data handling (HIPAA not eligible) |

Also checked: the Minimal Client sample compiles unchanged with `cargo check` (cargo 1.97.0) against rmcp 3.12.0, reqwest 0.13.5 and tokio 1.x with the three features proposed above; with those features the resolved tree has no `chrono` or `schemars`, and `base64` 0.23 is already pulled in by `hyper-util`. `list_all_tools` is capped at 64 pages since 3.12.0 (README Pagination, CHANGELOG #1457); the sample's comment now says so. Not re-checked: the crate-count and compile-time measurements, the GitHub MCP server README, and the provider tool-name rules in Section 5.
