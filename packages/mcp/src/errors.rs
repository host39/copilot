//! Readable failures of tool server requests (`docs/design/copilot.md`, Outbound Requests and
//! Tool Execution and Approvals).
//!
//! A failure never echoes a response body, an address, or a credential to the client: the log
//! gets the server id, a short failure kind, and the HTTP status, and the client a message and a
//! stable code. The one exception is a JSON-RPC error that a server returns for `tools/call`:
//! its message goes into the call's result like any other tool output (untrusted text the model
//! may use to correct its arguments), cut to one short line.

use std::error::Error;

use copilot_core::shared::constants::MCP_TOOLS_MAX;
use copilot_core::{Blocked, DomainError, ToolEndpoint, ToolServerId};
use rmcp::ServiceError;
use rmcp::model::ErrorData;
use rmcp::service::ClientInitializeError;
use rmcp::transport::streamable_http_client::StreamableHttpError;

/// The URL may not be reached from this account (the outbound policy refused it).
pub const TOOL_SERVER_BLOCKED: &str = "TOOL_SERVER_BLOCKED";
/// The server rejected the credentials, or asked for some.
pub const TOOL_SERVER_AUTH: &str = "TOOL_SERVER_AUTH";
/// No connection to the server.
pub const TOOL_SERVER_UNREACHABLE: &str = "TOOL_SERVER_UNREACHABLE";
/// The server did not answer in time (connecting or listing; a call uses `TOOL_TIMEOUT`).
pub const TOOL_SERVER_TIMEOUT: &str = "TOOL_SERVER_TIMEOUT";
/// A response or a tool list over `MCP_RESPONSE_MAX_BYTES`.
pub const TOOL_SERVER_TOO_LARGE: &str = "TOOL_SERVER_TOO_LARGE";
/// An HTTP status other than success.
pub const TOOL_SERVER_HTTP: &str = "TOOL_SERVER_HTTP";
/// No common MCP revision, or a reply that is not MCP.
pub const TOOL_SERVER_PROTOCOL: &str = "TOOL_SERVER_PROTOCOL";
/// The stored credentials cannot be sent as headers.
pub const TOOL_SERVER_HEADERS: &str = "TOOL_SERVER_HEADERS";
/// The server answered a request with a JSON-RPC error.
pub const TOOL_SERVER_REFUSED: &str = "TOOL_SERVER_REFUSED";
/// Any other failure.
pub const TOOL_SERVER_FAILED: &str = "TOOL_SERVER_FAILED";
/// The server no longer lists the called tool, even after listing again (core's port constant).
pub use copilot_core::TOOL_UNKNOWN;

/// The longest part of a server's JSON-RPC error message that a call result keeps.
const REFUSAL_MAX_CHARS: usize = 300;

/// What a server's requests carry, for the message of a refused request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Credentials {
    None,
    /// A bearer token or custom headers.
    Headers,
    /// An OAuth sign-in's access token.
    SignIn,
}

impl Credentials {
    pub(crate) fn of(endpoint: &ToolEndpoint) -> Self {
        if endpoint.tokens.is_some() {
            Self::SignIn
        } else if endpoint.headers.is_some() {
            Self::Headers
        } else {
            Self::None
        }
    }
}

/// What went wrong, as far as the error chain tells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Blocked(Blocked),
    /// `401` with a challenge.
    Auth,
    /// `403` with an `insufficient_scope` challenge.
    Scope,
    /// The stored credentials cannot be sent as headers.
    Headers,
    Unreachable,
    Timeout,
    TooLarge,
    Status(u16),
    /// No common MCP revision.
    Version,
    /// A reply that is not the MCP the client expected.
    Protocol,
    /// The connection ended during a request: the server went away, or an SSE event passed
    /// the size cap (rmcp ends the whole connection for that and reports only the end).
    Closed,
    Other,
}

impl Failure {
    /// The failure of a connection attempt.
    pub(crate) fn connect(error: &ClientInitializeError) -> Self {
        match error {
            ClientInitializeError::NoCompatibleProtocolVersion { .. }
            | ClientInitializeError::NoPreferredProtocolVersion => Self::Version,
            ClientInitializeError::JsonRpcError(_)
            | ClientInitializeError::ExpectedInitResponse(_)
            | ClientInitializeError::ExpectedInitResult(_)
            | ClientInitializeError::ConflictInitResponseId(..)
            | ClientInitializeError::UncorrelatedErrorResponse { .. } => Self::Protocol,
            ClientInitializeError::LegacyFallbackFailed { fallback, .. } => Self::connect(fallback),
            ClientInitializeError::TransportError { error, .. } => classify(error.error.as_ref()),
            _ if error.is_authorization_required() => Self::Auth,
            _ => Self::Other,
        }
    }

    /// The failure of a request on an open connection (a JSON-RPC error is handled before).
    pub(crate) fn request(error: &ServiceError) -> Self {
        match error {
            ServiceError::TransportSend(error) => classify(error.error.as_ref()),
            _ if error.is_authorization_required() => Self::Auth,
            ServiceError::Timeout { .. } => Self::Timeout,
            ServiceError::TransportClosed => Self::Closed,
            ServiceError::UnexpectedResponse | ServiceError::ListPaginationExceeded { .. } => {
                Self::Protocol
            }
            _ => Self::Other,
        }
    }

    /// The server refused the request's credentials with `401`: a refreshed access token may
    /// pass.
    pub(crate) fn unauthorized(self) -> bool {
        matches!(self, Self::Auth | Self::Status(401))
    }

    /// Whether another lifecycle might work: the server answered, but not as the remembered
    /// revision expects. Refusals, timeouts, and unreachable hosts never retry.
    pub(crate) fn era_mismatch(self) -> bool {
        match self {
            Self::Version | Self::Protocol | Self::Closed | Self::Other => true,
            Self::Status(status) => {
                (400..500).contains(&status) && !matches!(status, 401 | 403 | 429)
            }
            Self::Blocked(_)
            | Self::Auth
            | Self::Scope
            | Self::Headers
            | Self::Unreachable
            | Self::Timeout
            | Self::TooLarge => false,
        }
    }

    /// A short name for logs.
    fn kind(self) -> &'static str {
        match self {
            Self::Blocked(_) => "blocked",
            Self::Auth => "auth",
            Self::Scope => "scope",
            Self::Headers => "headers",
            Self::Unreachable => "unreachable",
            Self::Timeout => "timeout",
            Self::TooLarge => "too-large",
            Self::Status(_) => "status",
            Self::Version => "version",
            Self::Protocol => "protocol",
            Self::Closed => "closed",
            Self::Other => "other",
        }
    }

    /// Logs the failure (`mcp.http.blocked` or `mcp.http.failed`) and returns the client's error.
    /// `credentials` says what the request carried, for the sign-in message.
    pub(crate) fn report(self, server_id: ToolServerId, credentials: Credentials) -> DomainError {
        match self {
            Self::Blocked(reason) => {
                tracing::warn!(server_id = %server_id, reason = %reason, "mcp.http.blocked");
            }
            Self::Status(status) => {
                tracing::warn!(
                    server_id = %server_id,
                    failure = self.kind(),
                    status,
                    "mcp.http.failed"
                );
            }
            _ => {
                tracing::warn!(server_id = %server_id, failure = self.kind(), "mcp.http.failed");
            }
        }
        self.error(credentials)
    }

    fn error(self, credentials: Credentials) -> DomainError {
        match (self, credentials) {
            (Self::Blocked(_), _) => DomainError::upstream(
                "This account may not reach the tool server's address. Check its URL.",
                TOOL_SERVER_BLOCKED,
            ),
            (Self::Auth | Self::Status(401), Credentials::SignIn) => DomainError::upstream(
                "The tool server did not accept the sign-in. Sign in to it again in its settings.",
                TOOL_SERVER_AUTH,
            ),
            (Self::Scope, Credentials::SignIn) => DomainError::upstream(
                "The tool server needs permissions the sign-in did not grant. Sign in to it again in its settings.",
                TOOL_SERVER_AUTH,
            ),
            (Self::Auth | Self::Scope, Credentials::Headers) => DomainError::upstream(
                "The tool server rejected the credentials. Check its bearer token and headers.",
                TOOL_SERVER_AUTH,
            ),
            (Self::Auth | Self::Scope, Credentials::None) => DomainError::upstream(
                "The tool server requires sign-in. Add a bearer token or headers, or sign in with OAuth, in its settings.",
                TOOL_SERVER_AUTH,
            ),
            (Self::Headers, _) => DomainError::upstream(
                "The tool server's credentials cannot be sent. Enter them again in its settings.",
                TOOL_SERVER_HEADERS,
            ),
            (failure, _) => failure.plain(),
        }
    }

    /// The error of a failure that does not depend on the credentials.
    fn plain(self) -> DomainError {
        match self {
            Self::Unreachable => DomainError::upstream(
                "Could not connect to the tool server. Check its URL.",
                TOOL_SERVER_UNREACHABLE,
            ),
            Self::Timeout => DomainError::upstream(
                "The tool server did not answer in time.",
                TOOL_SERVER_TIMEOUT,
            ),
            Self::TooLarge => too_large(),
            Self::Status(429) => DomainError::upstream(
                "The tool server is rate limiting requests. Try again later.",
                TOOL_SERVER_HTTP,
            ),
            Self::Status(status) => DomainError::upstream(
                format!("The tool server answered with HTTP {status}."),
                TOOL_SERVER_HTTP,
            ),
            Self::Version => DomainError::upstream(
                "The tool server supports no MCP version that Copilot speaks.",
                TOOL_SERVER_PROTOCOL,
            ),
            Self::Protocol => DomainError::upstream(
                "The tool server sent a reply Copilot cannot read. Check that the URL is its MCP endpoint.",
                TOOL_SERVER_PROTOCOL,
            ),
            Self::Closed => DomainError::upstream(
                "The connection to the tool server ended. The server may have stopped, or sent more than Copilot accepts in one response (4 MiB).",
                TOOL_SERVER_FAILED,
            ),
            Self::Other | Self::Blocked(_) | Self::Auth | Self::Scope | Self::Headers => {
                DomainError::upstream("The request to the tool server failed.", TOOL_SERVER_FAILED)
            }
        }
    }
}

/// A response, or a whole tool list, over `MCP_RESPONSE_MAX_BYTES`.
pub(crate) fn too_large() -> DomainError {
    DomainError::upstream(
        "The tool server sent more than Copilot accepts in one response (4 MiB).",
        TOOL_SERVER_TOO_LARGE,
    )
}

pub(crate) fn too_many() -> DomainError {
    DomainError::upstream(
        format!(
            "The tool server lists more than {MCP_TOOLS_MAX} tools, more than Copilot accepts."
        ),
        TOOL_SERVER_TOO_LARGE,
    )
}

/// A `tools/call` the server answered with a JSON-RPC error: its message, on one short line.
pub(crate) fn refused(data: &ErrorData) -> DomainError {
    let message = line(&data.message);
    let text = if message.is_empty() {
        format!("The tool server refused the call (error {}).", data.code.0)
    } else {
        format!("The tool server refused the call: {message}")
    };
    DomainError::upstream(text, TOOL_SERVER_REFUSED)
}

/// A `tools/list` the server answered with a JSON-RPC error. Its message is not echoed.
pub(crate) fn list_refused(server_id: ToolServerId, data: &ErrorData) -> DomainError {
    tracing::warn!(server_id = %server_id, code = data.code.0, "mcp.tools.list-refused");
    DomainError::upstream(
        "The tool server refused to list its tools.",
        TOOL_SERVER_REFUSED,
    )
}

/// A call to a tool the server no longer lists.
pub(crate) fn unknown_tool() -> DomainError {
    DomainError::upstream(
        "The tool server no longer offers this tool. Test the server to refresh its tool list.",
        TOOL_UNKNOWN,
    )
}

/// Control characters become spaces, runs of white space one space, and the text is cut to
/// [`REFUSAL_MAX_CHARS`] characters.
fn line(text: &str) -> String {
    let joined = text
        .split(|character: char| character.is_whitespace() || character.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() <= REFUSAL_MAX_CHARS {
        return joined;
    }
    let mut cut: String = joined.chars().take(REFUSAL_MAX_CHARS).collect();
    cut.push('…');
    cut
}

/// Walks an error's source chain for the first thing that tells what happened. rmcp's transport
/// error does not expose the reqwest error as its source, so that variant is opened by hand.
fn classify(error: &(dyn Error + 'static)) -> Failure {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(blocked) = error.downcast_ref::<Blocked>() {
            return Failure::Blocked(*blocked);
        }
        if let Some(error) = error.downcast_ref::<StreamableHttpError<reqwest::Error>>()
            && let Some(failure) = transport(error)
        {
            return failure;
        }
        if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            return http(error);
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>()
            && error.kind() == std::io::ErrorKind::TimedOut
        {
            return Failure::Timeout;
        }
        // rmcp's SSE size limit is a private type; its message is the only handle.
        if error.to_string().contains("exceeded the maximum size") {
            return Failure::TooLarge;
        }
        current = error.source();
    }
    Failure::Other
}

/// What a transport error says, or `None` to keep walking its sources.
fn transport(error: &StreamableHttpError<reqwest::Error>) -> Option<Failure> {
    match error {
        StreamableHttpError::Client(error) => Some(http(error)),
        StreamableHttpError::HttpStatus(status) => Some(Failure::Status(status.status)),
        StreamableHttpError::ResponseBodyTooLarge { .. } => Some(Failure::TooLarge),
        StreamableHttpError::AuthRequired(_) => Some(Failure::Auth),
        StreamableHttpError::InsufficientScope(_) => Some(Failure::Scope),
        // "HTTP 404: <body>": keep the status, never the body.
        StreamableHttpError::UnexpectedServerResponse(text) => Some(
            status(text)
                .map(Failure::Status)
                .unwrap_or(Failure::Protocol),
        ),
        StreamableHttpError::UnexpectedContentType(_)
        | StreamableHttpError::Deserialize(_)
        | StreamableHttpError::MissingSessionIdInResponse
        | StreamableHttpError::ServerDoesNotSupportSse
        | StreamableHttpError::UnexpectedEndOfStream => Some(Failure::Protocol),
        _ => None,
    }
}

/// A reqwest failure: refused by the outbound policy, too slow, or no connection.
fn http(error: &reqwest::Error) -> Failure {
    if let Some(blocked) = Blocked::find(error) {
        Failure::Blocked(blocked)
    } else if error.is_timeout() {
        Failure::Timeout
    } else if error.is_connect() {
        Failure::Unreachable
    } else if let Some(status) = error.status() {
        Failure::Status(status.as_u16())
    } else {
        Failure::Other
    }
}

/// The status of rmcp's "HTTP 404: ..." message.
fn status(text: &str) -> Option<u16> {
    let rest = text.strip_prefix("HTTP ")?;
    rest.get(..3)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use rmcp::model::ErrorCode;

    use super::*;

    #[test]
    fn keeps_the_status_of_an_unexpected_response_but_never_its_body() {
        let error: StreamableHttpError<reqwest::Error> =
            StreamableHttpError::UnexpectedServerResponse("HTTP 404: <html>secret</html>".into());
        assert_eq!(classify(&error), Failure::Status(404));
        let failure = Failure::Status(404).error(Credentials::None);
        assert_eq!(
            failure.to_string(),
            "The tool server answered with HTTP 404."
        );
        assert_eq!(failure.code(), TOOL_SERVER_HTTP);

        let error: StreamableHttpError<reqwest::Error> =
            StreamableHttpError::UnexpectedServerResponse("could not parse".into());
        assert_eq!(classify(&error), Failure::Protocol);
    }

    #[test]
    fn finds_a_refusal_of_the_outbound_policy() {
        assert_eq!(
            classify(&Blocked::Address),
            Failure::Blocked(Blocked::Address)
        );
        let error = Failure::Blocked(Blocked::Address).error(Credentials::Headers);
        assert_eq!(error.code(), TOOL_SERVER_BLOCKED);
    }

    #[test]
    fn tells_a_rejected_credential_from_a_missing_one_and_a_sign_in() {
        assert_eq!(
            Failure::Auth.error(Credentials::Headers).to_string(),
            "The tool server rejected the credentials. Check its bearer token and headers."
        );
        assert_eq!(
            Failure::Auth.error(Credentials::None).to_string(),
            "The tool server requires sign-in. Add a bearer token or headers, or sign in with OAuth, in its settings."
        );
        assert_eq!(
            Failure::Status(401).error(Credentials::SignIn).to_string(),
            "The tool server did not accept the sign-in. Sign in to it again in its settings."
        );
        assert_eq!(
            Failure::Scope.error(Credentials::SignIn).to_string(),
            "The tool server needs permissions the sign-in did not grant. Sign in to it again in its settings."
        );
        assert_eq!(
            Failure::Headers.error(Credentials::Headers).code(),
            TOOL_SERVER_HEADERS
        );
        assert!(Failure::Auth.unauthorized() && Failure::Status(401).unauthorized());
        assert!(!Failure::Scope.unauthorized() && !Failure::Status(403).unauthorized());
    }

    #[test]
    fn retries_another_lifecycle_only_when_the_server_answered() {
        for failure in [
            Failure::Version,
            Failure::Protocol,
            Failure::Other,
            Failure::Status(400),
            Failure::Status(404),
            Failure::Status(405),
        ] {
            assert!(failure.era_mismatch(), "{failure:?}");
        }
        for failure in [
            Failure::Blocked(Blocked::Scheme),
            Failure::Auth,
            Failure::Scope,
            Failure::Headers,
            Failure::Unreachable,
            Failure::Timeout,
            Failure::TooLarge,
            Failure::Status(401),
            Failure::Status(403),
            Failure::Status(429),
            Failure::Status(500),
        ] {
            assert!(!failure.era_mismatch(), "{failure:?}");
        }
    }

    #[test]
    fn puts_a_server_refusal_on_one_short_line() {
        let data = ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Missing\r\n  field\t`query`\u{7}",
            None,
        );
        let error = refused(&data);
        assert_eq!(error.code(), TOOL_SERVER_REFUSED);
        assert_eq!(
            error.to_string(),
            "The tool server refused the call: Missing field `query`"
        );

        let long = ErrorData::new(ErrorCode::INVALID_PARAMS, "é".repeat(500), None);
        let text = refused(&long).to_string();
        let message = text
            .strip_prefix("The tool server refused the call: ")
            .unwrap();
        assert_eq!(message.chars().count(), REFUSAL_MAX_CHARS + 1);
        assert!(message.ends_with('…'));

        let blank = ErrorData::new(ErrorCode::INTERNAL_ERROR, " \n ", None);
        assert_eq!(
            refused(&blank).to_string(),
            "The tool server refused the call (error -32603)."
        );
    }
}
