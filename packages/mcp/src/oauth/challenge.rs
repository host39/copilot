//! The parameters of a `Bearer` challenge in `WWW-Authenticate` (RFC 9110 Section 11.6.1, RFC
//! 6750 Section 3, RFC 9728 Section 5.1).

use std::collections::HashMap;

/// What a `401` tells the client: where the protected resource metadata is, and which scopes
/// the request needs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Challenge {
    pub(crate) resource_metadata: Option<String>,
    pub(crate) scope: Option<String>,
}

impl Challenge {
    /// The `Bearer` challenge among `headers` (each a `WWW-Authenticate` value, which may hold
    /// several challenges); empty when there is none.
    pub(crate) fn parse<'a>(headers: impl IntoIterator<Item = &'a str>) -> Self {
        for header in headers {
            if let Some(params) = bearer(header) {
                return Self {
                    resource_metadata: params.get("resource_metadata").cloned(),
                    scope: params.get("scope").cloned(),
                };
            }
        }
        Self::default()
    }
}

/// The auth-params of the first `Bearer` challenge in `header`, names in lowercase.
fn bearer(header: &str) -> Option<HashMap<String, String>> {
    let mut cursor = Cursor {
        chars: header.chars().collect(),
        at: 0,
    };
    let mut scheme: Option<String> = None;
    let mut params = HashMap::new();
    loop {
        cursor.skip(|character| character.is_whitespace() || character == ',');
        let token = cursor.token();
        if token.is_empty() {
            // Nothing readable left (or a stray character): stop at what was read.
            if cursor.done() || cursor.peek().is_none() {
                break;
            }
            cursor.at += 1;
            continue;
        }
        cursor.skip(char::is_whitespace);
        if cursor.peek() == Some('=') {
            cursor.at += 1;
            cursor.skip(char::is_whitespace);
            let value = cursor.value();
            if scheme
                .as_deref()
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
            {
                params.entry(token.to_ascii_lowercase()).or_insert(value);
            }
        } else {
            if scheme
                .as_deref()
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
            {
                return Some(params);
            }
            scheme = Some(token);
        }
        if cursor.done() {
            break;
        }
    }
    scheme
        .filter(|scheme| scheme.eq_ignore_ascii_case("bearer"))
        .map(|_| params)
}

struct Cursor {
    chars: Vec<char>,
    at: usize,
}

impl Cursor {
    fn done(&self) -> bool {
        self.at >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip(&mut self, skipped: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&skipped) {
            self.at += 1;
        }
    }

    /// An RFC 9110 token.
    fn token(&mut self) -> String {
        let start = self.at;
        self.skip(|character| {
            character.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(character)
        });
        self.chars[start..self.at].iter().collect()
    }

    /// A quoted string (with `\` escapes) or a token.
    fn value(&mut self) -> String {
        if self.peek() != Some('"') {
            let start = self.at;
            self.skip(|character| character != ',' && !character.is_whitespace());
            return self.chars[start..self.at].iter().collect();
        }
        self.at += 1;
        let mut value = String::new();
        while let Some(character) = self.peek() {
            self.at += 1;
            match character {
                '"' => break,
                '\\' => {
                    if let Some(escaped) = self.peek() {
                        value.push(escaped);
                        self.at += 1;
                    }
                }
                other => value.push(other),
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_resource_metadata_and_scope() {
        let challenge = Challenge::parse([
            r#"Bearer resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="files:read files:write""#,
        ]);
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );
        assert_eq!(challenge.scope.as_deref(), Some("files:read files:write"));
    }

    #[test]
    fn finds_bearer_among_other_schemes_and_headers() {
        let challenge = Challenge::parse([
            r#"Basic realm="x""#,
            r#"Digest realm="a", nonce="b", Bearer error="invalid_token", Resource_Metadata="https://a.example/m", Newscheme x=y"#,
        ]);
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://a.example/m"),
            "names are case-insensitive"
        );
        assert_eq!(challenge.scope, None);
    }

    #[test]
    fn reads_tokens_escapes_and_bare_bearer() {
        let challenge =
            Challenge::parse([r#"bearer scope=read, resource_metadata="https://a.example/\"q\"""#]);
        assert_eq!(challenge.scope.as_deref(), Some("read"));
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://a.example/\"q\"")
        );
        assert_eq!(Challenge::parse(["Bearer"]), Challenge::default());
        assert_eq!(Challenge::parse(["Basic abc=="]), Challenge::default());
        assert_eq!(
            Challenge::parse(["", "=,,\"", "Bearer ="]),
            Challenge::default()
        );
    }
}
