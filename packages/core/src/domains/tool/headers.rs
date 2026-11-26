//! Tool server credentials: a bearer token and custom headers, sealed together.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::secret::Secret;

/// Headers the MCP client sets itself, compared in lowercase. Names starting with `mcp-` are
/// reserved too (`Mcp-Session-Id`, `Mcp-Method`, `Mcp-Name`, `Mcp-Param-*`).
const RESERVED: [&str; 8] = [
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-protocol-version",
    "transfer-encoding",
];
const AUTHORIZATION: &str = "Authorization";

/// The credentials a tool server request carries: an optional bearer token and custom headers.
///
/// They are sealed as one JSON object of header name to value (`headers_sealed`), with the
/// bearer token stored as `Authorization: Bearer <token>`, so the MCP adapter only copies
/// [`ToolHeaders::entries`] onto each request. A custom `Authorization` header is rejected when
/// the bearer field is set.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ToolHeaders {
    entries: BTreeMap<String, String>,
}

impl ToolHeaders {
    /// Validates the form input. A blank bearer token means none; header values are trimmed.
    pub fn new(bearer: Option<&str>, custom: &BTreeMap<String, String>) -> DomainResult<Self> {
        let bearer = bearer.map(str::trim).filter(|token| !token.is_empty());
        let mut entries = BTreeMap::new();
        let mut seen = Vec::new();
        for (name, value) in custom {
            let name = name.trim();
            check_name(name)?;
            let lower = name.to_ascii_lowercase();
            if lower == "authorization" && bearer.is_some() {
                return Err(DomainError::validation(
                    "Set either a bearer token or a custom Authorization header, not both.",
                )
                .with_field("headers"));
            }
            if seen.contains(&lower) {
                return Err(
                    DomainError::validation(format!("The {name} header is listed twice."))
                        .with_field("headers"),
                );
            }
            let value = value.trim();
            check_value(name, value)?;
            seen.push(lower);
            entries.insert(name.to_owned(), value.to_owned());
        }
        if let Some(token) = bearer {
            if token
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
            {
                return Err(DomainError::validation(
                    "The bearer token must not contain spaces or line breaks.",
                )
                .with_field("bearer"));
            }
            entries.insert(AUTHORIZATION.to_owned(), format!("Bearer {token}"));
        }
        Ok(Self { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The JSON object that gets sealed.
    pub fn to_json(&self) -> Vec<u8> {
        let object: Map<String, Value> = self
            .entries
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect();
        serde_json::to_vec(&Value::Object(object)).expect("a string map serializes")
    }

    /// The header pairs of an opened secret, for the MCP adapter.
    pub fn entries(secret: &Secret) -> DomainResult<Vec<(String, String)>> {
        let invalid = || DomainError::Internal {
            key: "core.tool-headers.invalid".to_owned(),
            source: None,
        };
        let value: Value = serde_json::from_slice(secret.expose()).map_err(|_| invalid())?;
        let Value::Object(object) = value else {
            return Err(invalid());
        };
        object
            .into_iter()
            .map(|(name, value)| match value {
                Value::String(text) => Ok((name, text)),
                _ => Err(invalid()),
            })
            .collect()
    }
}

/// Never prints values.
impl std::fmt::Debug for ToolHeaders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_set().entries(self.entries.keys()).finish()
    }
}

/// An HTTP token (RFC 9110): letters, digits, and ``!#$%&'*+-.^_`|~``; not a reserved name.
fn check_name(name: &str) -> DomainResult<()> {
    let valid = !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(character)
        });
    if !valid {
        return Err(DomainError::validation(
            "Header names may contain only letters, digits, and !#$%&'*+-.^_`|~.",
        )
        .with_field("headers"));
    }
    let lower = name.to_ascii_lowercase();
    if RESERVED.contains(&lower.as_str()) || lower.starts_with("mcp-") {
        return Err(
            DomainError::validation(format!("The {name} header is set by Copilot."))
                .with_field("headers"),
        );
    }
    Ok(())
}

fn check_value(name: &str, value: &str) -> DomainResult<()> {
    if value.is_empty() {
        return Err(
            DomainError::validation(format!("Enter a value for the {name} header."))
                .with_field("headers"),
        );
    }
    if value
        .chars()
        .any(|character| character.is_control() && character != '\t')
    {
        return Err(
            DomainError::validation(format!("The {name} header must be a single line."))
                .with_field("headers"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn round_trip(headers: &ToolHeaders) -> Vec<(String, String)> {
        ToolHeaders::entries(&Secret::new(headers.to_json())).unwrap()
    }

    #[test]
    fn stores_the_bearer_token_as_an_authorization_header() {
        let headers = ToolHeaders::new(Some("  tok-1 "), &custom(&[("X-Team", " core ")])).unwrap();
        assert_eq!(
            round_trip(&headers),
            [
                ("Authorization".to_owned(), "Bearer tok-1".to_owned()),
                ("X-Team".to_owned(), "core".to_owned()),
            ]
        );
        assert!(
            ToolHeaders::new(Some("  "), &BTreeMap::new())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn allows_a_custom_authorization_header_without_a_bearer_token() {
        let headers = ToolHeaders::new(None, &custom(&[("authorization", "Token abc")])).unwrap();
        assert_eq!(
            round_trip(&headers),
            [("authorization".to_owned(), "Token abc".to_owned())]
        );
    }

    #[test]
    fn rejects_a_custom_authorization_header_with_a_bearer_token() {
        let error =
            ToolHeaders::new(Some("tok"), &custom(&[("AUTHORIZATION", "Token abc")])).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Set either a bearer token or a custom Authorization header, not both."
        );
    }

    #[test]
    fn rejects_reserved_invalid_and_duplicate_headers() {
        let cases = [
            (
                custom(&[("Host", "x")]),
                "The Host header is set by Copilot.",
            ),
            (
                custom(&[("Mcp-Session-Id", "x")]),
                "The Mcp-Session-Id header is set by Copilot.",
            ),
            (
                custom(&[("X Team", "x")]),
                "Header names may contain only letters, digits, and !#$%&'*+-.^_`|~.",
            ),
            (
                custom(&[("X-Team", " ")]),
                "Enter a value for the X-Team header.",
            ),
            (
                custom(&[("X-Team", "a\r\nInjected: 1")]),
                "The X-Team header must be a single line.",
            ),
            (
                custom(&[("X-Team", "a"), ("x-team", "b")]),
                "The x-team header is listed twice.",
            ),
        ];
        for (input, message) in cases {
            let error = ToolHeaders::new(None, &input).unwrap_err();
            assert_eq!(error.to_string(), message);
        }
        let error = ToolHeaders::new(Some("a b"), &BTreeMap::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "The bearer token must not contain spaces or line breaks."
        );
    }

    #[test]
    fn never_prints_values() {
        let headers = ToolHeaders::new(Some("secret-token"), &BTreeMap::new()).unwrap();
        assert_eq!(format!("{headers:?}"), "{\"Authorization\"}");
    }

    #[test]
    fn rejects_a_secret_that_is_not_a_string_map() {
        assert!(ToolHeaders::entries(&Secret::new("[1]")).is_err());
        assert!(ToolHeaders::entries(&Secret::new("{\"a\": 1}")).is_err());
        assert!(ToolHeaders::entries(&Secret::new("not json")).is_err());
    }
}
