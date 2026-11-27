//! The lib-rustauth adapter: the only module that names `rustauth` (a test below checks this),
//! so a library upgrade touches one place.
//!
//! - [`IdentityConfig`] reads rustauth's `AUTH_*` variables.
//! - [`Identity`] starts rustauth on the database, mounts its routes, and puts its guard in front
//!   of Copilot's `/api` routes.
//! - [`Caller`] turns rustauth's `AuthUser` into core's [`Actor`].
//! - [`Directory`] is core's `Accounts` port: the owners of scheduled runs, through rustauth's
//!   `AuthService::users`, never by reading the `auth` tables. It also tells a long-lived stream
//!   when its [`Session`] ended ([`Directory::ended`]).
//! - [`to_domain`] maps `AuthError` into [`DomainError`] for the rustauth calls Copilot code makes
//!   (the extractor and the composition root). rustauth's own routes and guard answer with
//!   rustauth's mapping and the same error envelope.
//!
//! See "Identity (lib-rustauth)" in `docs/design/copilot.md` and lib-rustauth's
//! `docs/integration.md`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use copilot_contracts::ErrorResponse;
use copilot_core::shared::constants::SESSION_RECHECK_SECS;
use copilot_core::{Accounts, Actor, DomainError, DomainResult, UserId};
use copilot_database::{DbPool, Env};
use copilot_middleware::HttpError;
use rustauth::{
    AuthConfig, AuthError, AuthService, AuthUser, BuildError, Principal, Role, RustAuth, User,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use utoipa_axum::router::OpenApiRouter;

/// How often expired sessions and stale sign-in failure records are deleted.
const PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// lib-rustauth's settings, from its `AUTH_*` variables (documented in `docs/env.md`).
#[derive(Clone, Debug)]
pub struct IdentityConfig(AuthConfig);

impl IdentityConfig {
    /// Reads the `AUTH_*` variables. Fails with the name of every invalid one.
    pub fn parse(env: &Env) -> Result<Self, Vec<&'static str>> {
        AuthConfig::parse(env.vars())
            .map(Self)
            .map_err(|error| error.fields)
    }
}

/// lib-rustauth, started on Copilot's pool. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Identity(RustAuth);

impl Identity {
    /// Starts rustauth: checks that its `auth` migrations are applied (`make migrate` runs them
    /// before Copilot's) and, while no account exists and `AUTH_SETUP_TOKEN` is unset, logs a
    /// setup token once (`rustauth.setup.token-issued`). Needs a reachable database.
    pub async fn start(pool: DbPool, config: &IdentityConfig) -> DomainResult<Self> {
        RustAuth::builder()
            .pool(pool)
            .config(config.0.clone())
            .build()
            .await
            .map(Self)
            .map_err(build_error)
    }

    /// rustauth's routes (`/auth/*`, `/admin/users*`, with their OpenAPI documentation) plus
    /// `routes` behind its guard, which answers `401` without a valid bearer token. Mount the
    /// result under `/api`.
    pub fn api<S>(&self, routes: OpenApiRouter<S>) -> OpenApiRouter<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        self.0.router().merge(routes.route_layer(self.0.guard()))
    }
}

impl Identity {
    /// Core's `Accounts` port on rustauth's service.
    pub fn directory(&self) -> Directory {
        Directory(Arc::clone(self.0.service()))
    }

    /// Deletes expired sessions and stale sign-in failure records now and every hour, until
    /// `shutdown` fires. A failed pass is logged (`api.auth.purge.failed`) and retried next hour.
    pub fn purge(&self, shutdown: CancellationToken) -> JoinHandle<()> {
        let service = self.0.service().clone();
        tokio::spawn(async move {
            let mut hourly = tokio::time::interval(PURGE_INTERVAL);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = hourly.tick() => match service.purge().await {
                        Ok(removed) => tracing::debug!(removed, "api.auth.purged"),
                        Err(error) => tracing::warn!(error = %error, "api.auth.purge.failed"),
                    },
                }
            }
        })
    }
}

/// The signed-in caller, for handlers behind the guard ([`Identity::api`]). Anonymous requests
/// never reach a handler (the guard answers `401`); on a route without the guard the extractor
/// answers `500` and rustauth logs `rustauth.guard.missing`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Caller(pub Actor);

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state)
            .await
            .map_err(rejected)?;
        Ok(Self(actor(&user)))
    }
}

/// Core's view of a rustauth caller: `is_admin` only for the role `administrator` (`manager`
/// acts as a plain user in v1).
fn actor(principal: &Principal) -> Actor {
    Actor::new(
        UserId::new(principal.user_id.as_uuid()),
        principal.has_role(Role::Administrator),
    )
}

/// Core's view of an account read for background work, with the same admin rule as [`actor`].
fn account(user: &User) -> Actor {
    Actor::new(
        UserId::new(user.id.as_uuid()),
        user.role == Role::Administrator,
    )
}

/// The `Accounts` port: accounts many at once through rustauth's `AuthService::users` (one
/// query), so Copilot never reads lib-rustauth's tables beyond `auth.users (id)`.
#[derive(Clone)]
pub struct Directory(Arc<AuthService>);

impl Directory {
    /// Resolves once `session` has ended: signed out, revoked, expired, or its account disabled
    /// or deleted. Checks every `SESSION_RECHECK_SECS` (the guard has just checked it); a check
    /// that fails for another reason (the database is down) is not an end.
    pub async fn ended(self, session: Session) {
        let mut every = tokio::time::interval(Duration::from_secs(SESSION_RECHECK_SECS));
        every.tick().await;
        loop {
            every.tick().await;
            if let Err(AuthError::Unauthorized) = self.0.verify(&session.0).await {
                tracing::info!("api.auth.stream.session-ended");
                return;
            }
        }
    }
}

/// The caller's session token (`Authorization: Bearer`), for a stream that stays open and checks
/// it again ([`Directory::ended`]). Behind the guard the header is always there and valid.
pub struct Session(String);

impl<S: Send + Sync> FromRequestParts<S> for Session {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                let (scheme, token) = value.trim().split_once(' ')?;
                scheme
                    .eq_ignore_ascii_case("bearer")
                    .then(|| token.trim().to_owned())
            })
            .unwrap_or_default();
        Ok(Self(token))
    }
}

#[async_trait]
impl Accounts for Directory {
    async fn active(&self, ids: &[UserId]) -> DomainResult<Vec<Actor>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<rustauth::UserId> = ids
            .iter()
            .map(|id| rustauth::UserId::from(id.uuid()))
            .collect();
        let users = self.0.users(&ids).await.map_err(to_domain)?;
        Ok(users
            .iter()
            .filter(|user| !user.is_disabled())
            .map(account)
            .collect())
    }
}

/// Maps an `AuthError` into a [`DomainError`]; codes and waits survive the mapping. Variants
/// added in a later rustauth release fall back on their code.
pub fn to_domain(error: AuthError) -> DomainError {
    let message = error.to_string();
    match error {
        AuthError::InvalidCredentials | AuthError::Unauthorized => DomainError::Unauthorized {
            code: error.code().to_owned(),
            message,
        },
        AuthError::Forbidden { message } => DomainError::forbidden(message),
        AuthError::SetupRequired | AuthError::RegistrationClosed => DomainError::forbidden(message),
        AuthError::Validation { field, message } => {
            DomainError::validation(message).with_field(field)
        }
        AuthError::NotFound { entity } => DomainError::not_found(entity, "unknown"),
        AuthError::UsernameTaken | AuthError::LastAdministrator | AuthError::UserInUse => {
            DomainError::business_rule(message, error.code())
        }
        AuthError::Internal { key, source } => DomainError::Internal { key, source },
        other => match other.retry_after_secs() {
            // `RateLimited` and `AccountLocked`, and any later variant that limits retries.
            Some(retry_after_secs) => DomainError::RateLimited {
                code: other.code().to_owned(),
                message,
                retry_after_secs,
            },
            None => DomainError::new(message, other.code()),
        },
    }
}

/// A rejection of rustauth's `AuthUser` extractor, answered with Copilot's error type.
fn rejected(error: rustauth::HttpError) -> HttpError {
    let status = error.status();
    let body = error.body();
    match error.into_error() {
        Some(error) => HttpError::from(to_domain(error)),
        None => HttpError::Status {
            status,
            body: Box::new(ErrorResponse {
                error: body.error,
                code: body.code,
                field: body.field,
                details: body.details,
                request_id: body.request_id,
            }),
        },
    }
}

fn build_error(error: BuildError) -> DomainError {
    match error {
        BuildError::Auth(error) => to_domain(error),
        // The schema is missing, older, newer, or changed: the message says which.
        other => DomainError::internal("api.auth.start.failed", other),
    }
}

/// rustauth on in-memory fakes with a known setup token, for route tests without Postgres.
#[cfg(test)]
pub(crate) mod testing {
    use rustauth::testing::Fakes;
    use rustauth::{AuthConfig, RustAuth};

    use crate::helpers::auth::{Identity, IdentityConfig};

    /// The setup token of [`identity`].
    pub const SETUP_TOKEN: &str = "test-setup-token-0123456789";

    pub fn config() -> IdentityConfig {
        IdentityConfig(
            AuthConfig::builder()
                .setup_token(SETUP_TOKEN)
                .build()
                .expect("valid auth config"),
        )
    }

    pub async fn identity() -> Identity {
        let auth = RustAuth::builder()
            .ports(Fakes::new().ports())
            .config(config().0)
            .build()
            .await
            .expect("rustauth on fakes");
        Identity(auth)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error as _;
    use std::path::Path;

    use axum::http::{Method, Request, StatusCode};
    use copilot_middleware::error_handler::{format_error, status_code};
    use rustauth::{SessionId, Username};
    use serde_json::Value;
    use uuid::Uuid;

    use super::*;
    use crate::helpers::testing::{request, send};

    fn principal(role: Role) -> Principal {
        Principal::new(
            rustauth::UserId::from(Uuid::now_v7()),
            Username::new("alice").expect("valid username"),
            role,
            SessionId::generate(),
        )
    }

    #[tokio::test]
    async fn the_purge_task_stops_on_shutdown() {
        let shutdown = CancellationToken::new();
        let task = testing::identity().await.purge(shutdown.clone());
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("stops promptly")
            .expect("no panic");
    }

    #[test]
    fn only_administrators_act_as_admins() {
        let admin = principal(Role::Administrator);
        let caller = actor(&admin);
        assert!(caller.is_admin);
        assert_eq!(caller.user_id, UserId::new(admin.user_id.as_uuid()));
        assert!(
            !actor(&principal(Role::Manager)).is_admin,
            "manager acts as a user"
        );
        assert!(!actor(&principal(Role::User)).is_admin);
    }

    #[tokio::test]
    async fn caller_reads_the_user_the_guard_inserted() {
        let admin = principal(Role::Administrator);
        let request = Request::builder()
            .extension(AuthUser::new(admin.clone()))
            .body(())
            .expect("request");
        let (mut parts, ()) = request.into_parts();
        let Caller(caller) = Caller::from_request_parts(&mut parts, &())
            .await
            .expect("signed in");
        assert_eq!(caller, Actor::admin(UserId::new(admin.user_id.as_uuid())));
    }

    #[tokio::test]
    async fn caller_without_a_guard_is_a_wiring_mistake() {
        let (mut parts, ()) = Request::new(()).into_parts();
        let error = Caller::from_request_parts(&mut parts, &())
            .await
            .expect_err("no guard ran");
        let HttpError::Domain(error) = error else {
            panic!("a domain error");
        };
        assert_eq!(error.to_string(), "rustauth.guard.missing");
        assert_eq!(status_code(&error), StatusCode::INTERNAL_SERVER_ERROR);
    }

    fn mapped(error: AuthError) -> (StatusCode, ErrorResponse) {
        let error = to_domain(error);
        (status_code(&error), format_error(&error))
    }

    #[test]
    fn maps_sign_in_failures_to_401_with_their_code() {
        let (status, body) = mapped(AuthError::InvalidCredentials);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body.code.as_deref(), Some("INVALID_CREDENTIALS"));
        assert_eq!(body.error, "Invalid username or password.");

        let (status, body) = mapped(AuthError::Unauthorized);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body.code.as_deref(), Some("UNAUTHORIZED"));
    }

    #[test]
    fn maps_refusals_to_403() {
        for error in [
            AuthError::forbidden(AuthError::FORBIDDEN),
            AuthError::SetupRequired,
            AuthError::RegistrationClosed,
        ] {
            let message = error.to_string();
            let (status, body) = mapped(error);
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body.error, message);
        }
    }

    #[test]
    fn keeps_the_invalid_field() {
        let (status, body) = mapped(AuthError::validation("username", "Username is too short."));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.field.as_deref(), Some("username"));
        assert_eq!(body.code.as_deref(), Some("VALIDATION_ERROR"));
        assert_eq!(body.error, "Username is too short.");
    }

    #[test]
    fn maps_not_found_to_404_with_the_entity() {
        let (status, body) = mapped(AuthError::not_found("User"));
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.code.as_deref(), Some("ENTITY_NOT_FOUND"));
        assert_eq!(
            body.details
                .as_ref()
                .and_then(|details| details.get("entityName")),
            Some(&Value::from("User"))
        );
    }

    #[test]
    fn keeps_the_code_of_account_rules() {
        for error in [
            AuthError::UsernameTaken,
            AuthError::LastAdministrator,
            AuthError::UserInUse,
        ] {
            let code = error.code();
            let message = error.to_string();
            let (status, body) = mapped(error);
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(body.code.as_deref(), Some(code));
            assert_eq!(body.error, message);
        }
    }

    #[test]
    fn keeps_the_wait_of_rate_limits_and_lockouts() {
        for error in [
            AuthError::RateLimited {
                retry_after_secs: 9,
            },
            AuthError::AccountLocked {
                retry_after_secs: 9,
            },
        ] {
            let code = error.code();
            let message = error.to_string();
            let domain = to_domain(error);
            assert_eq!(domain.code(), code);
            assert_eq!(domain.to_string(), message);
            assert_eq!(domain.retry_after_secs(), Some(9));
            assert_eq!(status_code(&domain), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(
                format_error(&domain).details,
                Some(BTreeMap::from([(
                    "retryAfterSecs".to_owned(),
                    Value::from(9)
                )]))
            );
        }
    }

    #[test]
    fn keeps_the_dotted_key_and_source_of_internal_errors() {
        let error = to_domain(AuthError::internal(
            "rustauth.sessions.insert.failed",
            "boom",
        ));
        assert_eq!(error.to_string(), "rustauth.sessions.insert.failed");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("boom")
        );
        let (status, body) = mapped(AuthError::internal(
            "rustauth.sessions.insert.failed",
            "boom",
        ));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.code.as_deref(), Some("INTERNAL_ERROR"));
    }

    #[test]
    fn a_missing_auth_schema_fails_startup_with_its_reason() {
        let error = build_error(BuildError::SchemaOutdated { pending: vec![1] });
        assert_eq!(error.to_string(), "api.auth.start.failed");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("Auth schema is out of date: run migrations")
        );
        let error = build_error(BuildError::Auth(AuthError::internal(
            "rustauth.users.count.failed",
            "down",
        )));
        assert_eq!(error.to_string(), "rustauth.users.count.failed");
    }

    #[test]
    fn rejects_invalid_auth_variables_by_name() {
        let env = Env::from([
            ("AUTH_SESSION_TTL_DAYS", "0"),
            ("AUTH_SETUP_TOKEN", "short"),
            ("AUTH_REGISTRATION_OPEN", "true"),
        ]);
        let mut fields = IdentityConfig::parse(&env).expect_err("invalid");
        fields.sort_unstable();
        assert_eq!(fields, ["AUTH_SESSION_TTL_DAYS", "AUTH_SETUP_TOKEN"]);
        assert!(IdentityConfig::parse(&Env::from([("AUTH_REGISTRATION_OPEN", "true")])).is_ok());
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run `make test-db`"]
    async fn starts_on_postgres_once_the_auth_schema_is_migrated(pool: DbPool) {
        let error = Identity::start(pool.clone(), &testing::config())
            .await
            .expect_err("no auth schema yet");
        assert_eq!(error.to_string(), "api.auth.start.failed");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("Auth schema is out of date: run migrations")
        );

        copilot_database::operations::migrate::run(&pool)
            .await
            .expect("migrated");
        let identity = Identity::start(pool.clone(), &testing::config())
            .await
            .expect("started on Postgres");
        let app = crate::helpers::testing::app_on_identity(
            crate::helpers::testing::config(),
            pool,
            identity,
        );

        let token = crate::helpers::testing::register(&app, "admin").await;
        let me = send(
            app.clone(),
            request(Method::GET, "/api/auth/me", Some(&token), None),
        )
        .await;
        assert_eq!(me.status, StatusCode::OK);
        assert_eq!(me.body["data"]["role"], "administrator");

        let signed_in = send(
            app.clone(),
            request(Method::GET, "/api/settings", Some(&token), None),
        )
        .await;
        assert_eq!(signed_in.status, StatusCode::OK, "past the guard");
        let anonymous = send(app, request(Method::GET, "/api/settings", None, None)).await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn the_directory_resolves_active_accounts_with_the_admin_rule(pool: DbPool) {
        copilot_database::operations::migrate::run(&pool)
            .await
            .expect("migrated");
        let identity = Identity::start(pool.clone(), &testing::config())
            .await
            .expect("started on Postgres");
        let directory = identity.directory();
        let app = crate::helpers::testing::app_on_identity(
            crate::helpers::testing::config(),
            pool.clone(),
            identity,
        );
        let token = crate::helpers::testing::register(&app, "admin").await;
        crate::helpers::testing::other_user(&app, &token, "plain").await;
        crate::helpers::testing::other_user(&app, &token, "gone").await;
        let ids: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, username FROM auth.users")
            .fetch_all(&pool)
            .await
            .unwrap();
        let id = |name: &str| {
            let found = ids.iter().find(|(_, username)| username == name).unwrap();
            UserId::new(found.0)
        };
        sqlx::query("UPDATE auth.users SET disabled_at = now() WHERE username = 'gone'")
            .execute(&pool)
            .await
            .unwrap();

        let missing = UserId::new(Uuid::now_v7());
        let mut active = directory
            .active(&[id("admin"), id("plain"), id("gone"), missing])
            .await
            .unwrap();
        active.sort_by_key(|actor| actor.is_admin);
        assert_eq!(
            active,
            [Actor::user(id("plain")), Actor::admin(id("admin"))],
            "disabled and missing accounts are left out"
        );
        assert!(directory.active(&[]).await.unwrap().is_empty());
    }

    /// Every rustauth use stays in this module, so a library upgrade touches one place.
    #[test]
    fn no_other_module_names_rustauth() {
        fn visit(dir: &Path, offenders: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("source directory") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    visit(&path, offenders);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && !path.ends_with("helpers/auth.rs")
                {
                    let source = std::fs::read_to_string(&path).expect("source file");
                    if source.contains("rustauth::") || source.contains("use rustauth") {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        let mut offenders = Vec::new();
        visit(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut offenders,
        );
        assert!(
            offenders.is_empty(),
            "rustauth used outside helpers/auth.rs: {offenders:?}"
        );
    }
}
