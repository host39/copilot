//! API configuration parsed from environment variables. See `docs/env.md`.

use std::path::{Path, PathBuf};

use copilot_core::shared::constants::{DEFAULT_MAX_STORAGE_MB_PER_USER, DEFAULT_MAX_UPLOAD_MB};
use copilot_core::{Environment, FileLimits, LogLevel, PrivateNetwork};
use copilot_database::{DbConfig, Env, EnvError, EnvParser};
use copilot_security::SecretKey;

use crate::helpers::auth::IdentityConfig;

/// Largest `MAX_UPLOAD_MB`: file bytes live in one Postgres `bytea` value, which holds at most
/// 1 GB.
pub const UPLOAD_MB_MAX: u64 = 1_000;
/// Largest `MAX_STORAGE_MB_PER_USER` (1 TiB), so the byte count never overflows.
pub const STORAGE_MB_MAX: u64 = 1_048_576;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub environment: Environment,
    pub host: String,
    pub port: u16,
    pub log_level: LogLevel,
    /// Hostnames (or `.suffix` patterns) browsers may call the API from, besides localhost.
    pub cors_allowed_origins: Vec<String>,
    pub db: DbConfig,
    /// `SECRET_KEY`: seals provider keys and tool server headers. Required everywhere.
    pub secret_key: SecretKey,
    /// `ALLOW_PRIVATE_NETWORK`: whose connections may reach private and loopback addresses.
    pub private_network: PrivateNetwork,
    /// `MAX_UPLOAD_MB` and `MAX_STORAGE_MB_PER_USER`.
    pub files: FileLimits,
    /// `WEB_DIR`: the built web bundle the server serves, with `index.html` for client routes.
    pub web_dir: Option<PathBuf>,
    /// lib-rustauth's `AUTH_*` variables.
    pub identity: IdentityConfig,
}

impl AppConfig {
    /// Reads every variable and fails with the names of all invalid ones at once
    /// (`api.config.env.invalid SECRET_KEY, AUTH_SETUP_TOKEN`), then checks the `DB_*` ones.
    pub fn parse(env: &Env) -> Result<Self, EnvError> {
        let mut parser = EnvParser::new(env);
        let environment = parser.parse::<Environment>("ENVIRONMENT");
        let host = parser
            .optional("HOST")
            .unwrap_or_else(|| "127.0.0.1".to_owned());
        let port = parser.parse_or::<u16>("PORT", 3000);
        let log_level = parser.parse_or("LOG_LEVEL", LogLevel::Info);
        let cors_allowed_origins = parser
            .optional("CORS_ALLOWED_ORIGINS")
            .map(|value| comma_list(&value))
            .unwrap_or_default();
        let secret_key = match parser
            .optional("SECRET_KEY")
            .map(|value| SecretKey::parse(&value))
        {
            Some(Ok(key)) => Some(key),
            _ => {
                parser.reject("SECRET_KEY");
                None
            }
        };
        let private_network = parser.parse_or("ALLOW_PRIVATE_NETWORK", PrivateNetwork::Admins);
        let max_upload_mb = parser.parse_or("MAX_UPLOAD_MB", DEFAULT_MAX_UPLOAD_MB);
        let max_storage_mb =
            parser.parse_or("MAX_STORAGE_MB_PER_USER", DEFAULT_MAX_STORAGE_MB_PER_USER);
        let web_dir = parser.optional("WEB_DIR").map(PathBuf::from);
        let identity = IdentityConfig::parse(env).map_err(|fields| {
            for field in fields {
                parser.reject(field);
            }
        });

        if port == 0 {
            parser.reject("PORT");
        }
        if !(1..=UPLOAD_MB_MAX).contains(&max_upload_mb) {
            parser.reject("MAX_UPLOAD_MB");
        }
        if !(1..=STORAGE_MB_MAX).contains(&max_storage_mb) {
            parser.reject("MAX_STORAGE_MB_PER_USER");
        }
        if web_dir.as_deref().is_some_and(|dir| !is_bundle(dir)) {
            parser.reject("WEB_DIR");
        }
        parser.finish("api.config.env.invalid")?;

        Ok(Self {
            environment,
            host,
            port,
            log_level,
            cors_allowed_origins,
            db: DbConfig::parse(env)?,
            secret_key: secret_key.expect("finish() fails when SECRET_KEY is invalid"),
            private_network,
            files: FileLimits {
                max_upload_mb,
                max_storage_mb,
            },
            web_dir,
            identity: identity.expect("finish() fails when an AUTH_* variable is invalid"),
        })
    }
}

fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A directory with the bundle's `index.html`, which client routes fall back to.
fn is_bundle(dir: &Path) -> bool {
    dir.join("index.html").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `openssl rand -base64 32` of 32 zero bytes, for tests only.
    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    const BASE: [(&str, &str); 6] = [
        ("DB_HOST", "localhost"),
        ("DB_PORT", "5432"),
        ("DB_NAME", "copilot"),
        ("DB_USER", "postgres"),
        ("DB_PASSWORD", "postgres"),
        ("SECRET_KEY", KEY),
    ];

    fn env(extra: &[(&str, &str)]) -> Env {
        let mut pairs: Vec<(&str, &str)> = BASE.to_vec();
        for (name, value) in extra {
            pairs.retain(|(existing, _)| existing != name);
            pairs.push((name, value));
        }
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    fn invalid(extra: &[(&str, &str)]) -> Vec<&'static str> {
        let error = AppConfig::parse(&env(extra)).expect_err("invalid");
        assert_eq!(error.key, "api.config.env.invalid");
        error.fields
    }

    #[test]
    fn applies_defaults() {
        let config = AppConfig::parse(&env(&[("ENVIRONMENT", "local")])).expect("valid");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(config.cors_allowed_origins.is_empty());
        assert_eq!(config.private_network, PrivateNetwork::Admins);
        assert_eq!(config.files.max_upload_mb, 25);
        assert_eq!(config.files.max_storage_mb, 1_024);
        assert_eq!(config.web_dir, None);
    }

    #[test]
    fn rejects_port_zero() {
        assert_eq!(
            invalid(&[("ENVIRONMENT", "production"), ("PORT", "0")]),
            ["PORT"]
        );
    }

    #[test]
    fn reports_invalid_values() {
        assert_eq!(
            invalid(&[
                ("ENVIRONMENT", "qa"),
                ("PORT", "http"),
                ("LOG_LEVEL", "loud")
            ]),
            ["ENVIRONMENT", "PORT", "LOG_LEVEL"]
        );
    }

    #[test]
    fn splits_cors_origins() {
        let config = AppConfig::parse(&env(&[
            ("ENVIRONMENT", "local"),
            ("CORS_ALLOWED_ORIGINS", "app.example.com, .example.org,"),
        ]))
        .expect("valid");
        assert_eq!(
            config.cors_allowed_origins,
            vec!["app.example.com", ".example.org"]
        );
    }

    #[test]
    fn requires_a_32_byte_base64_secret_key() {
        let local = ("ENVIRONMENT", "local");
        assert_eq!(invalid(&[local, ("SECRET_KEY", "")]), ["SECRET_KEY"]);
        assert_eq!(
            invalid(&[local, ("SECRET_KEY", "not base64!")]),
            ["SECRET_KEY"]
        );
        assert_eq!(
            invalid(&[local, ("SECRET_KEY", "AAAAAAAAAAAAAAAAAAAAAA==")]),
            ["SECRET_KEY"],
            "16 bytes"
        );
        let mut missing = env(&[local]).vars().clone();
        missing.remove("SECRET_KEY");
        let error = AppConfig::parse(&missing.into_iter().collect()).expect_err("required");
        assert_eq!(error.fields, ["SECRET_KEY"]);

        let padded = format!("  {KEY}\n");
        let config = AppConfig::parse(&env(&[local, ("SECRET_KEY", &padded)])).expect("valid");
        assert_eq!(
            config.secret_key.id(),
            SecretKey::parse(KEY).expect("valid").id()
        );
    }

    #[test]
    fn reads_the_private_network_setting() {
        for (value, expected) in [
            ("none", PrivateNetwork::None),
            ("admins", PrivateNetwork::Admins),
            ("all", PrivateNetwork::All),
        ] {
            let config = AppConfig::parse(&env(&[
                ("ENVIRONMENT", "local"),
                ("ALLOW_PRIVATE_NETWORK", value),
            ]))
            .expect("valid");
            assert_eq!(config.private_network, expected);
        }
        assert_eq!(
            invalid(&[
                ("ENVIRONMENT", "local"),
                ("ALLOW_PRIVATE_NETWORK", "everyone")
            ]),
            ["ALLOW_PRIVATE_NETWORK"]
        );
    }

    #[test]
    fn bounds_the_file_limits() {
        let local = ("ENVIRONMENT", "local");
        let config = AppConfig::parse(&env(&[
            local,
            ("MAX_UPLOAD_MB", "1000"),
            ("MAX_STORAGE_MB_PER_USER", "1"),
        ]))
        .expect("valid");
        assert_eq!(config.files.max_upload_mb, 1_000);
        assert_eq!(config.files.max_storage_mb, 1);
        assert_eq!(
            invalid(&[
                local,
                ("MAX_UPLOAD_MB", "0"),
                ("MAX_STORAGE_MB_PER_USER", "0")
            ]),
            ["MAX_UPLOAD_MB", "MAX_STORAGE_MB_PER_USER"]
        );
        assert_eq!(
            invalid(&[
                local,
                ("MAX_UPLOAD_MB", "1001"),
                ("MAX_STORAGE_MB_PER_USER", "1048577")
            ]),
            ["MAX_UPLOAD_MB", "MAX_STORAGE_MB_PER_USER"]
        );
        assert_eq!(
            invalid(&[local, ("MAX_UPLOAD_MB", "-1")]),
            ["MAX_UPLOAD_MB"]
        );
    }

    #[test]
    fn web_dir_must_hold_the_bundle() {
        let dir = std::env::temp_dir().join(format!("copilot-web-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.to_str().expect("utf-8 path");
        let local = ("ENVIRONMENT", "local");
        assert_eq!(
            invalid(&[local, ("WEB_DIR", path)]),
            ["WEB_DIR"],
            "no index.html"
        );

        std::fs::write(dir.join("index.html"), "<!doctype html>").expect("index");
        let config = AppConfig::parse(&env(&[local, ("WEB_DIR", path)])).expect("valid");
        assert_eq!(config.web_dir.as_deref(), Some(dir.as_path()));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn reports_invalid_auth_variables_with_the_others() {
        let fields = invalid(&[
            ("ENVIRONMENT", "local"),
            ("SECRET_KEY", ""),
            ("AUTH_SETUP_TOKEN", "short"),
            ("AUTH_REGISTRATION_OPEN", "maybe"),
        ]);
        assert!(fields.contains(&"SECRET_KEY"));
        assert!(fields.contains(&"AUTH_SETUP_TOKEN"));
        assert!(fields.contains(&"AUTH_REGISTRATION_OPEN"));
    }
}
