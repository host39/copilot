//! API configuration parsed from environment variables. See `docs/env.md`.

use copilot_core::{Environment, LogLevel};
use copilot_database::{DbConfig, Env, EnvError, EnvParser};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    pub environment: Environment,
    pub host: String,
    pub port: u16,
    pub log_level: LogLevel,
    pub auth_disabled: bool,
    /// Hostnames (or `.suffix` patterns) browsers may call the API from, besides localhost.
    pub cors_allowed_origins: Vec<String>,
    pub db: DbConfig,
}

impl AppConfig {
    pub fn parse(env: &Env) -> Result<Self, EnvError> {
        let mut parser = EnvParser::new(env);
        let environment = parser.parse::<Environment>("ENVIRONMENT");
        let host = parser
            .optional("HOST")
            .unwrap_or_else(|| "127.0.0.1".to_owned());
        let port = parser.parse_or::<u16>("PORT", 3000);
        let log_level = parser.parse_or("LOG_LEVEL", LogLevel::Info);
        let auth_disabled = parser.parse_or("AUTH_DISABLED", environment.is_local());
        let cors_allowed_origins = parser
            .optional("CORS_ALLOWED_ORIGINS")
            .map(|value| comma_list(&value))
            .unwrap_or_default();

        if port == 0 {
            parser.reject("PORT");
        }
        // Production must verify tokens; a forgotten flag must not leave the API open.
        if auth_disabled && environment == Environment::Production {
            parser.reject("AUTH_DISABLED");
        }
        parser.finish("api.config.env.invalid")?;

        Ok(Self {
            environment,
            host,
            port,
            log_level,
            auth_disabled,
            cors_allowed_origins,
            db: DbConfig::parse(env)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    const DB: [(&str, &str); 5] = [
        ("DB_HOST", "localhost"),
        ("DB_PORT", "5432"),
        ("DB_NAME", "copilot"),
        ("DB_USER", "postgres"),
        ("DB_PASSWORD", "postgres"),
    ];

    fn env(extra: &[(&str, &str)]) -> Env {
        DB.iter()
            .chain(extra)
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn applies_defaults() {
        let config = AppConfig::parse(&env(&[("ENVIRONMENT", "local")])).expect("valid");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(config.auth_disabled);
        assert!(config.cors_allowed_origins.is_empty());
    }

    #[test]
    fn requires_auth_outside_local_by_default() {
        let config = AppConfig::parse(&env(&[("ENVIRONMENT", "staging")])).expect("valid");
        assert!(!config.auth_disabled);
    }

    #[test]
    fn refuses_disabled_auth_in_production() {
        let error = AppConfig::parse(&env(&[
            ("ENVIRONMENT", "production"),
            ("AUTH_DISABLED", "true"),
        ]))
        .expect_err("refused");
        assert_eq!(error.key, "api.config.env.invalid");
        assert_eq!(error.fields, vec!["AUTH_DISABLED"]);
    }

    #[test]
    fn reports_invalid_values() {
        let error = AppConfig::parse(&env(&[
            ("ENVIRONMENT", "qa"),
            ("PORT", "http"),
            ("LOG_LEVEL", "loud"),
        ]))
        .expect_err("invalid");
        assert_eq!(error.fields, vec!["ENVIRONMENT", "PORT", "LOG_LEVEL"]);
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
}
