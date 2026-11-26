//! Domain errors.
//!
//! Services fail with a [`DomainError`] variant. Messages are human-readable English because API
//! clients see them. Machine identifiers go in `code`, `field`, and `context`.
//!
//! [`DomainError::Internal`] is the one exception: it carries a dotted operational key (for example
//! `database.widgets.insert.failed`) for logs, and clients only ever see a generic message.

use std::collections::BTreeMap;

use serde_json::Value;

/// Extra structured data attached to an error (`details` in the HTTP error body).
pub type Context = BTreeMap<String, Value>;

/// A boxed error from infrastructure code (database driver, network client, and so on).
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// A domain failure that fits no narrower variant. Maps to 400.
    #[error("{message}")]
    Domain {
        message: String,
        code: String,
        context: Context,
    },

    /// A requested entity does not exist. Maps to 404.
    #[error("{entity_name} with id '{entity_id}' not found")]
    EntityNotFound {
        entity_name: String,
        entity_id: String,
    },

    /// Input failed validation. Maps to 400.
    #[error("{message}")]
    Validation {
        message: String,
        field: Option<String>,
        context: Context,
    },

    /// The caller may not perform the operation. Maps to 403.
    #[error("{message}")]
    Forbidden { message: String, context: Context },

    /// A business rule or invariant was violated. Maps to 422.
    #[error("{message}")]
    BusinessRule {
        message: String,
        rule_code: String,
        context: Context,
    },

    /// The caller is not signed in, the session ended, or the credentials are wrong. Maps to
    /// 401. `code` is `UNAUTHORIZED`, or lib-rustauth's code (`INVALID_CREDENTIALS`).
    #[error("{message}")]
    Unauthorized { message: String, code: String },

    /// The caller sent too many requests and may retry after `retry_after_secs`. Maps to 429
    /// with `Retry-After`. `code` is `RATE_LIMITED`, or lib-rustauth's code (`ACCOUNT_LOCKED`).
    #[error("{message}")]
    RateLimited {
        message: String,
        code: String,
        retry_after_secs: u64,
    },

    /// A provider or tool server failed during a call that does not stream (for example a model
    /// refresh or a tool server test). Maps to 502.
    #[error("{message}")]
    Upstream { message: String, code: String },

    /// Infrastructure failure. Maps to 500. `key` is a dotted log key, never shown to clients.
    #[error("{key}")]
    Internal {
        key: String,
        #[source]
        source: Option<BoxError>,
    },
}

impl DomainError {
    pub fn new(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self::Domain {
            message: message.into(),
            code: code.into(),
            context: Context::new(),
        }
    }

    pub fn not_found(entity_name: impl Into<String>, entity_id: impl Into<String>) -> Self {
        Self::EntityNotFound {
            entity_name: entity_name.into(),
            entity_id: entity_id.into(),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            field: None,
            context: Context::new(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden {
            message: message.into(),
            context: Context::new(),
        }
    }

    pub fn business_rule(message: impl Into<String>, rule_code: impl Into<String>) -> Self {
        Self::BusinessRule {
            message: message.into(),
            rule_code: rule_code.into(),
            context: Context::new(),
        }
    }

    /// Not signed in (`UNAUTHORIZED`).
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized {
            message: message.into(),
            code: "UNAUTHORIZED".to_owned(),
        }
    }

    /// Too many requests (`RATE_LIMITED`); the client may retry after `retry_after_secs`.
    pub fn rate_limited(message: impl Into<String>, retry_after_secs: u64) -> Self {
        Self::RateLimited {
            message: message.into(),
            code: "RATE_LIMITED".to_owned(),
            retry_after_secs,
        }
    }

    /// A provider or tool server failure the user should read, with a stable code.
    pub fn upstream(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self::Upstream {
            message: message.into(),
            code: code.into(),
        }
    }

    /// Wraps an infrastructure failure under a dotted log key.
    pub fn internal(key: impl Into<String>, source: impl Into<BoxError>) -> Self {
        Self::Internal {
            key: key.into(),
            source: Some(source.into()),
        }
    }

    /// Sets the failing field on a [`DomainError::Validation`]; other variants are unchanged.
    #[must_use]
    pub fn with_field(mut self, name: impl Into<String>) -> Self {
        if let Self::Validation { field, .. } = &mut self {
            *field = Some(name.into());
        }
        self
    }

    /// Adds a context entry. [`DomainError::EntityNotFound`], [`DomainError::Unauthorized`],
    /// [`DomainError::RateLimited`], [`DomainError::Upstream`], and [`DomainError::Internal`]
    /// build their context from their own fields and ignore this.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        match &mut self {
            Self::Domain { context, .. }
            | Self::Validation { context, .. }
            | Self::Forbidden { context, .. }
            | Self::BusinessRule { context, .. } => {
                context.insert(key.into(), value.into());
            }
            Self::EntityNotFound { .. }
            | Self::Unauthorized { .. }
            | Self::RateLimited { .. }
            | Self::Upstream { .. }
            | Self::Internal { .. } => {}
        }
        self
    }

    /// Machine-readable error code for clients and log filters.
    pub fn code(&self) -> &str {
        match self {
            Self::Domain { code, .. } => code,
            Self::EntityNotFound { .. } => "ENTITY_NOT_FOUND",
            Self::Validation { .. } => "VALIDATION_ERROR",
            Self::Forbidden { .. } => "FORBIDDEN",
            Self::BusinessRule { rule_code, .. } => rule_code,
            Self::Unauthorized { code, .. }
            | Self::RateLimited { code, .. }
            | Self::Upstream { code, .. } => code,
            Self::Internal { .. } => "INTERNAL_ERROR",
        }
    }

    /// Structured details for the HTTP error body: `retryAfterSecs` for
    /// [`DomainError::RateLimited`], empty for internal errors.
    pub fn context(&self) -> Context {
        match self {
            Self::Domain { context, .. }
            | Self::Validation { context, .. }
            | Self::Forbidden { context, .. }
            | Self::BusinessRule { context, .. } => context.clone(),
            Self::EntityNotFound {
                entity_name,
                entity_id,
            } => Context::from([
                ("entityName".to_owned(), Value::from(entity_name.as_str())),
                ("entityId".to_owned(), Value::from(entity_id.as_str())),
            ]),
            Self::RateLimited {
                retry_after_secs, ..
            } => Context::from([("retryAfterSecs".to_owned(), Value::from(*retry_after_secs))]),
            Self::Unauthorized { .. } | Self::Upstream { .. } | Self::Internal { .. } => {
                Context::new()
            }
        }
    }

    /// Seconds the client should wait before retrying (`Retry-After`), for
    /// [`DomainError::RateLimited`].
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_the_error_code() {
        let error = DomainError::new("something failed", "DOMAIN_ERROR");
        assert_eq!(error.code(), "DOMAIN_ERROR");
        assert_eq!(error.to_string(), "something failed");
    }

    #[test]
    fn keeps_context() {
        let error = DomainError::new("boom", "BOOM").with_context("id", "1");
        assert_eq!(error.code(), "BOOM");
        assert_eq!(error.context().get("id"), Some(&Value::from("1")));
    }

    #[test]
    fn builds_a_helpful_not_found_message() {
        let error = DomainError::not_found("User", "123");
        assert_eq!(error.to_string(), "User with id '123' not found");
        assert_eq!(error.code(), "ENTITY_NOT_FOUND");
        assert_eq!(error.context().get("entityId"), Some(&Value::from("123")));
    }

    #[test]
    fn captures_the_failing_field() {
        let error = DomainError::validation("Email is invalid.").with_field("email");
        assert!(
            matches!(&error, DomainError::Validation { field: Some(field), .. } if field == "email")
        );
        assert_eq!(error.code(), "VALIDATION_ERROR");
    }

    #[test]
    fn keeps_the_codes_of_the_identity_errors() {
        let signed_out = DomainError::unauthorized("Sign in to continue.");
        assert_eq!(signed_out.code(), "UNAUTHORIZED");
        assert_eq!(signed_out.to_string(), "Sign in to continue.");
        assert!(signed_out.context().is_empty());
        assert_eq!(signed_out.retry_after_secs(), None);

        let wrong = DomainError::Unauthorized {
            message: "Invalid username or password.".to_owned(),
            code: "INVALID_CREDENTIALS".to_owned(),
        };
        assert_eq!(wrong.code(), "INVALID_CREDENTIALS");
    }

    #[test]
    fn rate_limits_carry_the_wait() {
        let error = DomainError::rate_limited("Too many requests.", 42).with_context("ignored", 1);
        assert_eq!(error.code(), "RATE_LIMITED");
        assert_eq!(error.retry_after_secs(), Some(42));
        assert_eq!(
            error.context(),
            Context::from([("retryAfterSecs".to_owned(), Value::from(42))])
        );
    }

    #[test]
    fn upstream_failures_keep_their_code() {
        let error = DomainError::upstream("Key rejected: open provider settings.", "PROVIDER_AUTH");
        assert_eq!(error.code(), "PROVIDER_AUTH");
        assert_eq!(error.to_string(), "Key rejected: open provider settings.");
        assert!(error.context().is_empty());
    }

    #[test]
    fn hides_internal_details_from_context() {
        let error = DomainError::internal("database.users.insert.failed", "connection reset");
        assert_eq!(error.code(), "INTERNAL_ERROR");
        assert!(error.context().is_empty());
        assert_eq!(error.to_string(), "database.users.insert.failed");
    }
}
