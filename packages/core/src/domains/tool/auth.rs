//! How a tool server's requests authenticate: no credentials, a bearer token or custom headers
//! (`ToolHeaders`), or an OAuth sign-in (`docs/design/copilot.md`, Tool Server Sign-In).
//!
//! An OAuth sign-in is stored sealed and bound to its row: the pre-registered client's secret in
//! `oauth_client_secret_sealed` (bound to one authorization server, `oauth_client_issuer`), and
//! the tokens with everything a refresh needs in `oauth_tokens_sealed` ([`Session`]). A sign-in in
//! progress lives in `oauth_flows` ([`SignInFlow`]): the PKCE verifier and the issuer to check
//! sealed, the `state` only as its SHA-256, so a database read never yields a usable `state`.
//! After the callback, the flow holds the exchanged tokens ([`Held`]) until the owner confirms
//! them with the code the returning browser was shown, stored only as its SHA-256.

use std::fmt;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::ports::secret_cipher::SealContext;
use crate::ports::tool_auth::{ClientAuth, OAuthClient, TokenSet};
use crate::shared::constants::TOKEN_EXPIRY_SKEW_SECS;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ToolServerId, UserId};
use crate::shared::secret::Secret;

/// How a server's requests authenticate (`auth_mode`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AuthMode {
    /// No credentials.
    #[default]
    None,
    /// The stored bearer token and custom headers (`headers_sealed`).
    Headers,
    /// An OAuth sign-in: the access token of the stored sign-in, refreshed when it expires.
    OAuth,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Headers => "headers",
            Self::OAuth => "oauth",
        }
    }
}

impl FromStr for AuthMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "headers" => Ok(Self::Headers),
            "oauth" => Ok(Self::OAuth),
            other => Err(other.to_owned()),
        }
    }
}

/// A pre-registered OAuth client the user entered for a server (`oauth_client_id`, and the
/// secret of a confidential client sealed in `oauth_client_secret_sealed`). A client belongs to
/// one authorization server: `issuer` (`oauth_client_issuer`) is the one its first sign-in
/// discovered, and a sign-in that discovers another one is refused, so the secret never goes to
/// a token endpoint it was not entered for. Saving the client again forgets it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientConfig {
    pub id: String,
    pub secret_sealed: Option<Vec<u8>>,
    pub issuer: Option<String>,
}

/// A stored sign-in: the sealed [`Session`] (`oauth_tokens_sealed`), when its access token
/// expires (`oauth_expires_at`; `None` when the server did not say), and whether it holds a
/// refresh token (`oauth_refreshable`), so its state shows without opening it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignIn {
    pub sealed: Vec<u8>,
    pub expires_at: Option<DateTime<Utc>>,
    pub refreshable: bool,
}

impl SignIn {
    /// The access token has expired, or does within `TOKEN_EXPIRY_SKEW_SECS`.
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at
            .is_some_and(|at| at <= now + Duration::seconds(TOKEN_EXPIRY_SKEW_SECS))
    }

    /// Signed in while the access token is good or can be refreshed.
    pub fn state(&self, now: DateTime<Utc>) -> SignInState {
        if self.expired(now) && !self.refreshable {
            SignInState::Expired
        } else {
            SignInState::SignedIn
        }
    }
}

/// What the settings show for an OAuth server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignInState {
    SignedIn,
    /// The access token expired and there is no refresh token: sign in again.
    Expired,
    /// Never signed in, signed out, or the refresh was rejected.
    NeedsSignIn,
}

/// What `oauth_tokens_sealed` holds: the tokens and everything a refresh needs, so a refresh
/// never discovers again. `Debug` hides the tokens and the client secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub issuer: String,
    pub token_endpoint: Url,
    pub client: OAuthClient,
    pub resource: String,
    pub scope: Option<String>,
    pub access: Secret,
    pub refresh: Option<Secret>,
}

impl Session {
    /// The session after a token answer: a new access token, the new refresh token or the old
    /// one (no rotation), and the granted scope or the old one.
    #[must_use]
    pub fn renewed(&self, tokens: TokenSet) -> Self {
        Self {
            access: tokens.access,
            refresh: tokens.refresh.or_else(|| self.refresh.clone()),
            scope: tokens.scope.or_else(|| self.scope.clone()),
            ..self.clone()
        }
    }

    /// The JSON that gets sealed.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(&Value::Object(self.object())).expect("a JSON object serializes")
    }

    /// An opened `oauth_tokens_sealed`.
    pub fn parse(secret: &Secret) -> DomainResult<Self> {
        let invalid = || internal("core.tools.sign-in.invalid");
        object(secret)
            .as_ref()
            .and_then(Self::read)
            .ok_or_else(invalid)
    }

    fn object(&self) -> Map<String, Value> {
        let mut object = client_json(&self.client);
        object.insert("v".to_owned(), json!(1));
        object.insert("issuer".to_owned(), json!(self.issuer));
        object.insert(
            "tokenEndpoint".to_owned(),
            json!(self.token_endpoint.as_str()),
        );
        object.insert("resource".to_owned(), json!(self.resource));
        object.insert("scope".to_owned(), json!(self.scope));
        object.insert("access".to_owned(), json!(text(&self.access)));
        object.insert("refresh".to_owned(), json!(self.refresh.as_ref().map(text)));
        object
    }

    fn read(object: &Map<String, Value>) -> Option<Self> {
        Some(Self {
            issuer: string(object, "issuer")?,
            token_endpoint: url(object, "tokenEndpoint")?,
            client: client(object)?,
            resource: string(object, "resource")?,
            scope: string(object, "scope"),
            access: string(object, "access").map(Secret::new)?,
            refresh: string(object, "refresh").map(Secret::new),
        })
    }
}

/// A sign-in in progress (`oauth_flows`): bound to one user's server, used at most once, and
/// gone after `SIGN_IN_FLOW_SECS`. Its row id is also the row the sealed [`Pending`] is bound to.
///
/// Before the callback it waits for the `state` (`confirm_hash` is `None`, `sealed` is a
/// [`Pending`]); after it, the callback replaced it with a held flow: a `state_hash` nobody knows,
/// `sealed` the exchanged tokens ([`Held`]), and `confirm_hash` the SHA-256 of the code the
/// returning browser was shown, which the owner must send back ([`confirmation_hash`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignInFlow {
    pub id: Uuid,
    pub user_id: UserId,
    pub server_id: ToolServerId,
    /// SHA-256 of the `state` parameter.
    pub state_hash: Vec<u8>,
    /// The sealed [`Pending`], or [`Held`] once `confirm_hash` is set.
    pub sealed: Vec<u8>,
    /// SHA-256 of the confirmation code of a held flow.
    pub confirm_hash: Option<Vec<u8>>,
    /// Wrong confirmation codes so far (`SIGN_IN_CONFIRM_ATTEMPTS` end the flow).
    pub attempts: u32,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl SignInFlow {
    /// The associated data that binds [`SignInFlow::sealed`] to this row.
    pub fn context(&self) -> SealContext {
        SealContext {
            table: "oauth_flows",
            column: "sealed",
            row_id: self.id,
            user_id: self.user_id,
        }
    }
}

/// What a sign-in in progress keeps sealed until its callback: the PKCE verifier, the issuer
/// the callback's `iss` must match, and the token request's other parts. `Debug` hides the
/// verifier and the client secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pending {
    pub verifier: Secret,
    pub redirect_uri: Url,
    pub issuer: String,
    /// The issuer advertises `iss` in authorization responses (RFC 9207): one without it is
    /// rejected.
    pub iss_parameter: bool,
    pub token_endpoint: Url,
    pub client: OAuthClient,
    pub resource: String,
    pub scope: Option<String>,
    /// The server's URL when the sign-in started: a callback after the URL changed is refused.
    pub url: String,
    /// The user's connections may reach private addresses (`NetworkScope::Private`), for the
    /// token request of the callback, which has no signed-in caller.
    pub private: bool,
}

impl Pending {
    pub fn to_json(&self) -> Vec<u8> {
        let mut object = client_json(&self.client);
        object.insert("v".to_owned(), json!(1));
        object.insert("verifier".to_owned(), json!(text(&self.verifier)));
        object.insert("redirectUri".to_owned(), json!(self.redirect_uri.as_str()));
        object.insert("issuer".to_owned(), json!(self.issuer));
        object.insert("issParameter".to_owned(), json!(self.iss_parameter));
        object.insert(
            "tokenEndpoint".to_owned(),
            json!(self.token_endpoint.as_str()),
        );
        object.insert("resource".to_owned(), json!(self.resource));
        object.insert("scope".to_owned(), json!(self.scope));
        object.insert("url".to_owned(), json!(self.url));
        object.insert("private".to_owned(), json!(self.private));
        serde_json::to_vec(&Value::Object(object)).expect("a JSON object serializes")
    }

    pub fn parse(secret: &Secret) -> DomainResult<Self> {
        let invalid = || internal("core.tools.sign-in-flow.invalid");
        let object = object(secret).ok_or_else(invalid)?;
        Ok(Self {
            verifier: string(&object, "verifier")
                .map(Secret::new)
                .ok_or_else(invalid)?,
            redirect_uri: url(&object, "redirectUri").ok_or_else(invalid)?,
            issuer: string(&object, "issuer").ok_or_else(invalid)?,
            iss_parameter: object
                .get("issParameter")
                .and_then(Value::as_bool)
                .ok_or_else(invalid)?,
            token_endpoint: url(&object, "tokenEndpoint").ok_or_else(invalid)?,
            client: client(&object).ok_or_else(invalid)?,
            resource: string(&object, "resource").ok_or_else(invalid)?,
            scope: string(&object, "scope"),
            url: string(&object, "url").ok_or_else(invalid)?,
            private: object
                .get("private")
                .and_then(Value::as_bool)
                .ok_or_else(invalid)?,
        })
    }
}

/// What a held flow keeps sealed until its owner confirms it: the session the exchange gave
/// (tokens and what a refresh needs), when its access token expires, and the server's URL when
/// the sign-in started (a confirmation after the URL changed is refused). `Debug` hides the
/// tokens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Held {
    pub session: Session,
    pub expires_at: Option<DateTime<Utc>>,
    pub url: String,
}

impl Held {
    pub fn to_json(&self) -> Vec<u8> {
        let mut object = self.session.object();
        object.insert(
            "expiresAt".to_owned(),
            json!(self.expires_at.map(|at| at.to_rfc3339())),
        );
        object.insert("url".to_owned(), json!(self.url));
        serde_json::to_vec(&Value::Object(object)).expect("a JSON object serializes")
    }

    pub fn parse(secret: &Secret) -> DomainResult<Self> {
        let invalid = || internal("core.tools.sign-in-held.invalid");
        let object = object(secret).ok_or_else(invalid)?;
        let expires_at = match object.get("expiresAt") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                    .ok_or_else(invalid)?
                    .with_timezone(&Utc),
            ),
        };
        Ok(Self {
            session: Session::read(&object).ok_or_else(invalid)?,
            expires_at,
            url: string(&object, "url").ok_or_else(invalid)?,
        })
    }
}

/// The letters of a confirmation code: no vowels (no words) and none that look alike.
const CONFIRMATION_LETTERS: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";
/// Letters in a confirmation code (two groups of four): about 34.6 bits.
const CONFIRMATION_LENGTH: usize = 8;

/// A new confirmation code, `ABCD-EFGH`: shown once, in the browser that came back from the
/// authorization server, and typed or sent back by the flow's owner.
pub fn confirmation() -> DomainResult<Secret> {
    let mut bytes = [0_u8; CONFIRMATION_LENGTH];
    getrandom::fill(&mut bytes)
        .map_err(|error| DomainError::internal("core.tools.sign-in.random-failed", error))?;
    let letters = u8::try_from(CONFIRMATION_LETTERS.len()).expect("a short alphabet");
    // 256 is not a multiple of 20; the slight bias costs a fraction of a bit.
    let mut code = String::with_capacity(CONFIRMATION_LENGTH + 1);
    for (index, byte) in bytes.iter().enumerate() {
        if index == CONFIRMATION_LENGTH / 2 {
            code.push('-');
        }
        code.push(char::from(
            CONFIRMATION_LETTERS[usize::from(byte % letters)],
        ));
    }
    Ok(Secret::new(code))
}

/// The SHA-256 a confirmation code is stored and compared under: its letters and digits only,
/// upper case, so `abcd efgh` matches `ABCD-EFGH`.
pub fn confirmation_hash(code: &str) -> Vec<u8> {
    let normalized: String = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_uppercase())
        .collect();
    Sha256::digest(normalized.as_bytes()).to_vec()
}

/// The path segment that names `issuer` in its redirect URI (`<PUBLIC_URL>/api/oauth/callback/
/// <tag>`): base64url of the first 16 bytes of its SHA-256. A distinct redirect URI per
/// authorization server lets the callback tell which one sent the browser back (RFC 9700, 4.4).
pub fn issuer_tag(issuer: &str) -> String {
    URL_SAFE_NO_PAD.encode(&Sha256::digest(issuer.as_bytes())[..16])
}

/// The values a sign-in starts with: the PKCE verifier and the `state`, 32 random bytes each in
/// base64url (43 characters).
pub struct Secrets {
    pub verifier: Secret,
    pub state: Secret,
}

impl Secrets {
    pub fn generate() -> DomainResult<Self> {
        Ok(Self {
            verifier: random()?,
            state: random()?,
        })
    }
}

impl fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secrets")
    }
}

/// The PKCE `S256` challenge of `verifier`: base64url of its SHA-256 (RFC 7636).
pub fn challenge(verifier: &Secret) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose()))
}

/// The SHA-256 under which a flow's `state` is stored and looked up.
pub fn state_hash(state: &str) -> Vec<u8> {
    Sha256::digest(state.as_bytes()).to_vec()
}

fn random() -> DomainResult<Secret> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| DomainError::internal("core.tools.sign-in.random-failed", error))?;
    Ok(Secret::new(URL_SAFE_NO_PAD.encode(bytes)))
}

fn internal(key: &str) -> DomainError {
    DomainError::Internal {
        key: key.to_owned(),
        source: None,
    }
}

/// A secret's text; sealed values are always written from strings.
fn text(secret: &Secret) -> &str {
    secret.text().unwrap_or_default()
}

fn client_json(client: &OAuthClient) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("clientId".to_owned(), json!(client.id));
    object.insert(
        "clientSecret".to_owned(),
        json!(client.secret.as_ref().map(text)),
    );
    object.insert("clientAuth".to_owned(), json!(client.auth.as_str()));
    object
}

fn client(object: &Map<String, Value>) -> Option<OAuthClient> {
    Some(OAuthClient {
        id: string(object, "clientId")?,
        secret: string(object, "clientSecret").map(Secret::new),
        auth: ClientAuth::parse(object.get("clientAuth")?.as_str()?)?,
    })
}

fn object(secret: &Secret) -> Option<Map<String, Value>> {
    match serde_json::from_slice(secret.expose()).ok()? {
        Value::Object(object) if object.get("v") == Some(&json!(1)) => Some(object),
        _ => None,
    }
}

fn string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToOwned::to_owned)
}

fn url(object: &Map<String, Value>, key: &str) -> Option<Url> {
    Url::parse(object.get(key)?.as_str()?).ok()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn client(secret: Option<&str>) -> OAuthClient {
        OAuthClient {
            id: "client-1".to_owned(),
            secret: secret.map(Secret::new),
            auth: if secret.is_some() {
                ClientAuth::Basic
            } else {
                ClientAuth::None
            },
        }
    }

    fn session() -> Session {
        Session {
            issuer: "https://auth.example.com".to_owned(),
            token_endpoint: Url::parse("https://auth.example.com/token").unwrap(),
            client: client(Some("client-secret")),
            resource: "https://mcp.example.com/mcp".to_owned(),
            scope: Some("files:read".to_owned()),
            access: Secret::new("access-1"),
            refresh: Some(Secret::new("refresh-1")),
        }
    }

    #[test]
    fn round_trips_a_session_through_json() {
        let session = session();
        let parsed = Session::parse(&Secret::new(session.to_json())).unwrap();
        assert_eq!(parsed, session);

        let public = Session {
            client: client(None),
            refresh: None,
            scope: None,
            ..session
        };
        let parsed = Session::parse(&Secret::new(public.to_json())).unwrap();
        assert_eq!(parsed, public);
    }

    #[test]
    fn rejects_a_session_it_cannot_read() {
        for text in [
            "not json",
            "[]",
            "{\"v\": 2}",
            "{\"v\": 1, \"issuer\": \"x\"}",
        ] {
            let error = Session::parse(&Secret::new(text)).unwrap_err();
            assert_eq!(error.code(), "INTERNAL_ERROR", "{text}");
        }
    }

    #[test]
    fn keeps_the_refresh_token_and_scope_the_answer_leaves_out() {
        let session = session();
        let renewed = session.renewed(TokenSet {
            access: Secret::new("access-2"),
            refresh: None,
            expires_in: None,
            scope: None,
        });
        assert_eq!(renewed.access, Secret::new("access-2"));
        assert_eq!(renewed.refresh, Some(Secret::new("refresh-1")));
        assert_eq!(renewed.scope.as_deref(), Some("files:read"));

        let rotated = session.renewed(TokenSet {
            access: Secret::new("access-3"),
            refresh: Some(Secret::new("refresh-2")),
            expires_in: None,
            scope: Some("files:read files:write".to_owned()),
        });
        assert_eq!(rotated.refresh, Some(Secret::new("refresh-2")));
        assert_eq!(rotated.scope.as_deref(), Some("files:read files:write"));
    }

    #[test]
    fn round_trips_a_pending_sign_in() {
        let pending = Pending {
            verifier: Secret::new("verifier"),
            redirect_uri: Url::parse("https://copilot.example.com/api/oauth/callback").unwrap(),
            issuer: "https://auth.example.com".to_owned(),
            iss_parameter: true,
            token_endpoint: Url::parse("https://auth.example.com/token").unwrap(),
            client: client(None),
            resource: "https://mcp.example.com/mcp".to_owned(),
            scope: None,
            url: "https://mcp.example.com/mcp".to_owned(),
            private: false,
        };
        let parsed = Pending::parse(&Secret::new(pending.to_json())).unwrap();
        assert_eq!(parsed, pending);
    }

    #[test]
    fn round_trips_a_held_sign_in() {
        let held = Held {
            session: session(),
            expires_at: Some(now() + Duration::hours(1)),
            url: "https://mcp.example.com/mcp".to_owned(),
        };
        let parsed = Held::parse(&Secret::new(held.to_json())).unwrap();
        assert_eq!(parsed, held);
        let open = Held {
            expires_at: None,
            ..held
        };
        assert_eq!(Held::parse(&Secret::new(open.to_json())).unwrap(), open);
        assert!(!format!("{open:?}").contains("access-1"));
        assert!(
            Held::parse(&Secret::new(session().to_json())).is_err(),
            "no URL"
        );
    }

    #[test]
    fn confirmation_codes_are_random_readable_and_forgiving() {
        let first = confirmation().unwrap();
        let second = confirmation().unwrap();
        assert_ne!(first, second);
        let code = first.text().unwrap();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        assert!(
            code.chars()
                .filter(|character| *character != '-')
                .all(|character| CONFIRMATION_LETTERS.contains(&(character as u8)))
        );
        assert_eq!(
            confirmation_hash("BCDF-GHJK"),
            confirmation_hash(" bcdf ghjk")
        );
        assert_ne!(
            confirmation_hash("BCDF-GHJK"),
            confirmation_hash("BCDF-GHJL")
        );
        assert_eq!(confirmation_hash("x").len(), 32);
    }

    #[test]
    fn tags_each_issuer_with_its_own_path_segment() {
        let tag = issuer_tag("https://auth.example.com");
        assert_eq!(tag.len(), 22);
        assert!(
            tag.chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
        );
        assert_eq!(tag, issuer_tag("https://auth.example.com"));
        assert_ne!(tag, issuer_tag("https://auth.example.com/"));
        assert_ne!(tag, issuer_tag("https://evil.example.com"));
    }

    #[test]
    fn debug_never_prints_tokens_or_secrets() {
        let printed = format!("{:?}", session());
        for secret in ["access-1", "refresh-1", "client-secret"] {
            assert!(!printed.contains(secret), "{printed}");
        }
        assert!(printed.contains("client-1"));
    }

    #[test]
    fn computes_the_pkce_challenge_of_rfc_7636() {
        // RFC 7636, Appendix B.
        let verifier = Secret::new("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            challenge(&verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generates_distinct_url_safe_values() {
        let first = Secrets::generate().unwrap();
        let second = Secrets::generate().unwrap();
        let text = first.verifier.text().unwrap();
        assert_eq!(text.len(), 43);
        assert!(
            text.chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
        );
        assert_ne!(first.verifier, second.verifier);
        assert_ne!(first.state, first.verifier);
        assert_eq!(format!("{first:?}"), "Secrets");
        assert_eq!(state_hash("abc").len(), 32);
        assert_ne!(state_hash("abc"), state_hash("abd"));
    }

    #[test]
    fn tells_the_sign_in_state() {
        let fresh = SignIn {
            sealed: vec![1],
            expires_at: Some(now() + Duration::hours(1)),
            refreshable: false,
        };
        assert_eq!(fresh.state(now()), SignInState::SignedIn);
        assert!(!fresh.expired(now()));
        assert!(
            fresh.expired(now() + Duration::minutes(59)),
            "expires within the skew"
        );
        assert_eq!(
            fresh.state(now() + Duration::hours(2)),
            SignInState::Expired
        );
        let refreshable = SignIn {
            refreshable: true,
            ..fresh.clone()
        };
        assert_eq!(
            refreshable.state(now() + Duration::hours(2)),
            SignInState::SignedIn
        );
        let unknown = SignIn {
            expires_at: None,
            ..fresh
        };
        assert!(!unknown.expired(now() + Duration::days(365)));
    }

    #[test]
    fn parses_auth_modes() {
        for mode in [AuthMode::None, AuthMode::Headers, AuthMode::OAuth] {
            assert_eq!(mode.as_str().parse::<AuthMode>(), Ok(mode));
        }
        assert!("bearer".parse::<AuthMode>().is_err());
    }
}
