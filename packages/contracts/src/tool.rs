//! `/tool-servers`: MCP servers, remote (Streamable HTTP) or a program on the Copilot server
//! (stdio, administrators only). Credentials (a bearer token or custom headers, an OAuth client
//! secret, OAuth tokens, a stdio program's environment values) go in and never come out;
//! responses say whether they are stored and, for OAuth, the sign-in's state.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::shared::{KeyState, redacted};

/// A server's default for its tools.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Every call waits for the user's decision. The default for new servers.
    #[default]
    Ask,
    /// Calls run without asking.
    Auto,
}

/// One tool's setting; a tool without one follows the server's [`ApprovalMode`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ToolMode {
    /// Never sent to the model.
    Off,
    Ask,
    Auto,
}

/// How Copilot reaches a server. Set when the server is added; it never changes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Streamable HTTP at the server's `url`.
    #[default]
    Http,
    /// A program on the Copilot server that speaks MCP over its stdin and stdout. Only
    /// administrators add and run these, and only while `MCP_STDIO_ENABLED` is on
    /// (`GET /capabilities`).
    Stdio,
}

/// A stdio server's program as stored. The environment's values never come out.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct StdioProgram {
    /// The program's absolute path.
    pub command: String,
    /// Passed to the program as they are: no shell, no expansion.
    pub args: Vec<String>,
    /// The names of its environment variables, the only ones it gets.
    pub env_names: Vec<String>,
    /// Whether variables are stored; `unreadable` after a `SECRET_KEY` change.
    pub env: KeyState,
    /// Its working directory, relative to the server's `MCP_STDIO_DIR`.
    pub cwd: String,
}

/// A stdio server's program as an administrator enters it.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct StdioInput {
    /// The program's absolute path (it starts without `PATH`).
    pub command: String,
    /// Each one is one argument, kept exactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// The program's whole environment: names of letters, digits, and `_`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// A folder under `MCP_STDIO_DIR` (plain names joined by `/`); blank or absent: the server's
    /// id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

impl fmt::Debug for StdioInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StdioInput")
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// A change to a stdio server's program: only the given fields change.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct StdioPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Replaces every variable; `{}` removes them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    /// Blank: the server's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

impl fmt::Debug for StdioPatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StdioPatch")
            .field("command", &self.command)
            .field("args", &self.args)
            .field(
                "env",
                &self.env.as_ref().map(|env| env.keys().collect::<Vec<_>>()),
            )
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// `GET /capabilities`: what this Copilot server lets the caller do.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesResponse {
    /// The caller may add and run stdio tool servers: `MCP_STDIO_ENABLED` is on and the caller
    /// is an administrator.
    pub stdio_servers: bool,
}

/// How a server's requests authenticate.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// No credentials.
    #[default]
    None,
    /// The stored bearer token and custom headers.
    Headers,
    /// An OAuth sign-in (MCP authorization): `POST /tool-servers/{id}/oauth/start`.
    Oauth,
}

/// The state of an OAuth server's sign-in.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SignInState {
    /// The access token is good or can be refreshed.
    SignedIn,
    /// The access token expired and there is no refresh token: sign in again.
    Expired,
    /// Never signed in, signed out, or the refresh was rejected.
    NeedsSignIn,
}

/// An OAuth server's sign-in and its pre-registered client. Tokens never come out.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct OAuthStatus {
    pub state: SignInState,
    /// The pre-registered client id, when one was entered.
    pub client_id: Option<String>,
    /// Whether the pre-registered client's secret is stored.
    pub client_secret: KeyState,
    /// When the access token expires, when the server said.
    pub expires_at: Option<DateTime<Utc>>,
    /// The redirect URI to register for the pre-registered client, once its first sign-in found
    /// the authorization server it belongs to (`<PUBLIC_URL>/api/oauth/callback/<issuer tag>`).
    pub redirect_uri: Option<String>,
}

/// A pre-registered OAuth client (a client id the authorization server issued for Copilot).
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct OAuthClientInput {
    pub client_id: String,
    /// A confidential client's secret. Blank or absent keeps the stored one while the client id
    /// stays the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl fmt::Debug for OAuthClientInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthClientInput")
            .field("client_id", &self.client_id)
            .field("client_secret", &redacted(self.client_secret.as_ref()))
            .finish()
    }
}

/// `POST /tool-servers/{id}/oauth/confirm`: the code the browser showed after the sign-in came
/// back, which finishes it.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct OAuthConfirmRequest {
    pub code: String,
}

impl fmt::Debug for OAuthConfirmRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthConfirmRequest")
            .field("code", &"***")
            .finish()
    }
}

/// `POST /tool-servers/{id}/oauth/start`: the URL to open in the browser, where the user signs
/// in; the sign-in comes back to `/api/oauth/callback` and then to the web app's `/oauth/done`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct OAuthStartResponse {
    pub authorization_url: String,
    /// The sign-in must come back before this (10 minutes).
    pub expires_at: DateTime<Utc>,
}

/// A tool from the server's last listing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ServerTool {
    /// The original name; per-tool modes and approvals use it.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    /// The tool's JSON Schema, as the server sent it.
    pub input_schema: Value,
    /// The server's `readOnlyHint`: the UI suggests `auto`, nothing more (annotations are
    /// untrusted).
    pub read_only: bool,
}

/// A tool server (lists, details, and writes).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolServerResponse {
    pub id: Uuid,
    pub name: String,
    /// The prefix of the server's tool names sent to models (`{slug}__{tool}`); never changes.
    pub slug: String,
    pub transport: Transport,
    /// A Streamable HTTP server's URL; `null` for a stdio server.
    pub url: Option<String>,
    /// A stdio server's program; `null` for a Streamable HTTP server.
    pub stdio: Option<StdioProgram>,
    pub approval: ApprovalMode,
    /// Per-tool modes by original tool name.
    pub tool_modes: BTreeMap<String, ToolMode>,
    pub enabled: bool,
    /// Whether a bearer token or custom headers are stored. Lists report `set` or `missing`;
    /// details report `unreadable` after a `SECRET_KEY` change.
    pub credentials: KeyState,
    pub auth_mode: AuthMode,
    /// The sign-in of an `oauth` server; `null` in the other modes.
    pub oauth: Option<OAuthStatus>,
    /// The negotiated MCP revision.
    pub protocol_version: Option<String>,
    /// The cached tool list; `null` until a Test succeeds.
    pub tools: Option<Vec<ServerTool>>,
    pub tools_fetched_at: Option<DateTime<Utc>>,
    /// The tool list changed (or the URL did) since the last review: every call asks until
    /// `POST /tool-servers/{id}/review`.
    pub review_required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /tool-servers`.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct CreateToolServerRequest {
    pub name: String,
    /// 1 to 12 lowercase letters or digits; derived from the name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Default `http`; `stdio` needs `stdio` and an administrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<Transport>,
    /// Required for `http`; none for `stdio`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The program of a `stdio` server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdio: Option<StdioInput>,
    /// Sent as `Authorization: Bearer <token>`. Blank or absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer: Option<String>,
    /// Custom headers. `Authorization` is rejected when `bearer` is set, and MCP's own headers
    /// always.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Default `ask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalMode>,
    /// Default: `headers` when a bearer token or headers are given, else `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<AuthMode>,
    /// A pre-registered OAuth client (`oauth` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client: Option<OAuthClientInput>,
}

impl fmt::Debug for CreateToolServerRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateToolServerRequest")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("transport", &self.transport)
            .field("url", &self.url)
            .field("stdio", &self.stdio)
            .field("bearer", &redacted(self.bearer.as_ref()))
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("approval", &self.approval)
            .field("auth_mode", &self.auth_mode)
            .field("oauth_client", &self.oauth_client)
            .finish()
    }
}

/// New credentials for a server: they replace the stored bearer token and headers together.
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("bearer", &redacted(self.bearer.as_ref()))
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// `PATCH /tool-servers/{id}`: only the given fields change. A URL with another origin clears
/// the stored credentials unless `credentials` comes with it, any new URL ends an OAuth
/// sign-in, and every tool goes back to `ask` until the server is reviewed. Credentials without
/// `authMode` switch the server to `headers`; another `authMode` drops the old mode's
/// credentials. A stdio server takes `stdio` (administrators only), never a URL or
/// credentials; a changed program is reviewed again like a new URL.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct UpdateToolServerRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdio: Option<StdioPatch>,
    /// New credentials; `null` removes the stored ones.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub credentials: Option<Option<Credentials>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalMode>,
    /// Replaces every per-tool mode. Names must be on the server's tool list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_modes: Option<BTreeMap<String, ToolMode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// `headers` needs credentials, given now or stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<AuthMode>,
    /// A new pre-registered OAuth client (`oauth` only); `null` removes it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::patch::nullable"
    )]
    pub oauth_client: Option<Option<OAuthClientInput>>,
}

/// Tool names that changed between two listings of a server.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Same name, new description or input schema.
    pub changed: Vec<String>,
}

/// `POST /tool-servers/{id}/test`: connects, lists the tools, and refreshes the cache. Any
/// change sets `reviewRequired`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ToolTestResponse {
    pub server: ToolServerResponse,
    pub changes: ToolChanges,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_tells_keeping_clearing_and_replacing_credentials_apart() {
        let keep: UpdateToolServerRequest =
            serde_json::from_value(serde_json::json!({ "name": "Docs" })).expect("valid body");
        assert_eq!(keep.credentials, None);
        let clear: UpdateToolServerRequest =
            serde_json::from_value(serde_json::json!({ "credentials": null })).expect("valid body");
        assert_eq!(clear.credentials, Some(None));
        let replace: UpdateToolServerRequest = serde_json::from_value(serde_json::json!({
            "credentials": { "headers": { "X-Api-Key": "secret" } },
        }))
        .expect("valid body");
        let credentials = replace.credentials.flatten().expect("credentials");
        assert_eq!(credentials.bearer, None);
        assert_eq!(credentials.headers["X-Api-Key"], "secret");
    }

    #[test]
    fn modes_are_snake_case_and_keyed_by_tool_name() {
        let request: UpdateToolServerRequest = serde_json::from_value(serde_json::json!({
            "approval": "auto",
            "toolModes": { "search_docs": "off", "read_page": "auto" },
        }))
        .expect("valid body");
        assert_eq!(request.approval, Some(ApprovalMode::Auto));
        let modes = request.tool_modes.expect("modes");
        assert_eq!(modes["search_docs"], ToolMode::Off);
        assert_eq!(modes["read_page"], ToolMode::Auto);
    }

    #[test]
    fn stdio_inputs_never_print_environment_values() {
        let input = StdioInput {
            command: "/usr/local/bin/server".to_owned(),
            env: BTreeMap::from([("API_KEY".to_owned(), "sk-TOPSECRET".to_owned())]),
            ..StdioInput::default()
        };
        let patch = StdioPatch {
            env: Some(input.env.clone()),
            ..StdioPatch::default()
        };
        let create = CreateToolServerRequest {
            stdio: Some(input.clone()),
            ..CreateToolServerRequest::default()
        };
        for printed in [
            format!("{input:?}"),
            format!("{patch:?}"),
            format!("{create:?}"),
        ] {
            assert!(!printed.contains("TOPSECRET"), "{printed}");
            assert!(printed.contains("API_KEY"), "{printed}");
        }
    }

    #[test]
    fn create_takes_a_stdio_program_without_a_url() {
        let request: CreateToolServerRequest = serde_json::from_value(serde_json::json!({
            "name": "Files",
            "transport": "stdio",
            "stdio": { "command": "/usr/local/bin/files", "args": ["--root", "/srv"] },
        }))
        .expect("valid body");
        assert_eq!(request.transport, Some(Transport::Stdio));
        assert_eq!(request.url, None);
        let stdio = request.stdio.expect("a program");
        assert_eq!(stdio.args, ["--root", "/srv"]);
        assert!(stdio.env.is_empty());
        assert_eq!(stdio.cwd, None);
    }

    #[test]
    fn create_defaults_to_no_credentials() {
        let request: CreateToolServerRequest = serde_json::from_value(serde_json::json!({
            "name": "Docs",
            "url": "https://mcp.example.com/mcp",
        }))
        .expect("valid body");
        assert_eq!(request.bearer, None);
        assert!(request.headers.is_empty());
        assert_eq!(request.approval, None);
    }

    #[test]
    fn auth_modes_and_sign_in_states_are_snake_case() {
        assert_eq!(
            serde_json::to_value([AuthMode::None, AuthMode::Headers, AuthMode::Oauth]).unwrap(),
            serde_json::json!(["none", "headers", "oauth"])
        );
        assert_eq!(
            serde_json::to_value(SignInState::NeedsSignIn).unwrap(),
            "needs_sign_in"
        );
        let clear: UpdateToolServerRequest =
            serde_json::from_value(serde_json::json!({ "oauthClient": null })).unwrap();
        assert_eq!(clear.oauth_client, Some(None));
        let set: UpdateToolServerRequest = serde_json::from_value(serde_json::json!({
            "authMode": "oauth",
            "oauthClient": { "clientId": "abc" },
        }))
        .unwrap();
        assert_eq!(set.auth_mode, Some(AuthMode::Oauth));
        assert_eq!(set.oauth_client.flatten().unwrap().client_id, "abc");
    }

    #[test]
    fn debug_hides_tokens_and_header_values() {
        let headers = BTreeMap::from([("X-Api-Key".to_owned(), "header-secret".to_owned())]);
        let create = CreateToolServerRequest {
            name: "Docs".to_owned(),
            url: Some("https://mcp.example.com/mcp".to_owned()),
            bearer: Some("token-secret".to_owned()),
            headers: headers.clone(),
            ..CreateToolServerRequest::default()
        };
        let update = UpdateToolServerRequest {
            credentials: Some(Some(Credentials {
                bearer: Some("token-secret".to_owned()),
                headers,
            })),
            oauth_client: Some(Some(OAuthClientInput {
                client_id: "client".to_owned(),
                client_secret: Some("client-hidden".to_owned()),
            })),
            ..UpdateToolServerRequest::default()
        };
        let printed = format!("{create:?} {update:?}");
        for value in ["token-secret", "header-secret", "client-hidden"] {
            assert!(!printed.contains(value), "{printed}");
        }
        assert!(printed.contains("X-Api-Key"), "{printed}");
    }
}
