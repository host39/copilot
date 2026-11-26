//! Environment parsing for the database (`DB_*`) and shared helpers other crates reuse.
//!
//! Parsing takes an [`Env`] map rather than reading the process environment directly so tests
//! can pass fixed values. Every invalid or missing variable is collected before failing, and the
//! error message is a dotted key followed by the variable names.

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use copilot_core::Environment;

use crate::types::{DbConfig, DbSsl, ReadonlyConfig};

pub const DB_POOL_MAX: u32 = 4;
pub const DB_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
pub const READONLY_USER: &str = "readonly";

/// A snapshot of environment variables. Blank values count as unset.
#[derive(Clone, Debug, Default)]
pub struct Env(HashMap<String, String>);

impl Env {
    /// Reads the process environment.
    pub fn load() -> Self {
        std::env::vars().collect()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    }
}

impl FromIterator<(String, String)> for Env {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<const N: usize> From<[(&str, &str); N]> for Env {
    fn from(pairs: [(&str, &str); N]) -> Self {
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{key} {}", fields.join(", "))]
pub struct EnvError {
    pub key: &'static str,
    pub fields: Vec<&'static str>,
}

/// Collects values and the names of variables that are missing or invalid.
#[derive(Debug)]
pub struct EnvParser<'a> {
    env: &'a Env,
    invalid: Vec<&'static str>,
}

impl<'a> EnvParser<'a> {
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            invalid: Vec::new(),
        }
    }

    /// A required string. Records the variable as invalid when unset.
    pub fn required(&mut self, name: &'static str) -> String {
        match self.env.get(name) {
            Some(value) => value.to_owned(),
            None => {
                self.reject(name);
                String::new()
            }
        }
    }

    pub fn optional(&self, name: &'static str) -> Option<String> {
        self.env.get(name).map(str::to_owned)
    }

    /// A required value parsed with `FromStr`.
    pub fn parse<T: FromStr + Default>(&mut self, name: &'static str) -> T {
        match self.env.get(name).map(str::parse) {
            Some(Ok(value)) => value,
            _ => {
                self.reject(name);
                T::default()
            }
        }
    }

    /// An optional value parsed with `FromStr`, falling back to `default` when unset.
    pub fn parse_or<T: FromStr>(&mut self, name: &'static str, default: T) -> T {
        match self.env.get(name).map(str::parse) {
            None => default,
            Some(Ok(value)) => value,
            Some(Err(_)) => {
                self.reject(name);
                default
            }
        }
    }

    /// An optional value parsed with `FromStr`. `None` when unset.
    pub fn optional_parse<T: FromStr>(&mut self, name: &'static str) -> Option<T> {
        match self.env.get(name).map(str::parse) {
            None => None,
            Some(Ok(value)) => Some(value),
            Some(Err(_)) => {
                self.reject(name);
                None
            }
        }
    }

    /// Records a variable as invalid for a rule checked by the caller.
    pub fn reject(&mut self, name: &'static str) {
        if !self.invalid.contains(&name) {
            self.invalid.push(name);
        }
    }

    /// Fails with `key` and every invalid variable name, if any.
    pub fn finish(self, key: &'static str) -> Result<(), EnvError> {
        if self.invalid.is_empty() {
            Ok(())
        } else {
            Err(EnvError {
                key,
                fields: self.invalid,
            })
        }
    }
}

impl DbConfig {
    /// Parses `DB_HOST`, `DB_PORT`, `DB_NAME`, `DB_USER`, `DB_PASSWORD`, `DB_SSL`,
    /// `DB_SSL_ROOT_CERT`, and `ENVIRONMENT`.
    pub fn parse(env: &Env) -> Result<Self, EnvError> {
        let mut parser = EnvParser::new(env);
        let host = parser.required("DB_HOST");
        let port = parser.parse::<u16>("DB_PORT");
        let database = parser.required("DB_NAME");
        let username = parser.required("DB_USER");
        let password = parser.required("DB_PASSWORD");
        let environment = parser.parse::<Environment>("ENVIRONMENT");
        let ssl = parser.optional_parse::<bool>("DB_SSL");
        let root_cert = parser.optional("DB_SSL_ROOT_CERT").map(PathBuf::from);
        if port == 0 {
            parser.reject("DB_PORT");
        }
        parser.finish("database.env.invalid")?;

        Ok(Self {
            host,
            port,
            database,
            username,
            password,
            ssl: ssl_mode(environment, ssl, root_cert),
            pool_max: DB_POOL_MAX,
            connection_timeout: DB_CONNECTION_TIMEOUT,
        })
    }
}

impl ReadonlyConfig {
    /// Parses `DB_READONLY_USER` and `DB_READONLY_PASSWORD`. `None` when both are unset.
    pub fn parse(env: &Env) -> Result<Option<Self>, EnvError> {
        let mut parser = EnvParser::new(env);
        let username = parser.optional("DB_READONLY_USER");
        let password = parser.optional("DB_READONLY_PASSWORD");

        match (username, password) {
            (None, None) => Ok(None),
            (Some(username), Some(password)) if username == READONLY_USER => {
                Ok(Some(Self { username, password }))
            }
            (username, password) => {
                if username.as_deref() != Some(READONLY_USER) {
                    parser.reject("DB_READONLY_USER");
                }
                if password.is_none() {
                    parser.reject("DB_READONLY_PASSWORD");
                }
                parser.finish("database.env.readonly-invalid")?;
                Ok(None)
            }
        }
    }
}

/// SSL is off for `local` and on everywhere else, unless `DB_SSL` says otherwise.
fn ssl_mode(environment: Environment, ssl: Option<bool>, root_cert: Option<PathBuf>) -> DbSsl {
    let enabled = ssl.unwrap_or(!environment.is_local());
    if enabled {
        DbSsl::Enabled { root_cert }
    } else {
        DbSsl::Disabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(environment: &str, ssl: Option<&str>) -> Env {
        let mut pairs: Vec<(String, String)> = [
            ("ENVIRONMENT", environment),
            ("DB_HOST", "localhost"),
            ("DB_PORT", "5432"),
            ("DB_NAME", "copilot"),
            ("DB_USER", "postgres"),
            ("DB_PASSWORD", "postgres"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
        if let Some(ssl) = ssl {
            pairs.push(("DB_SSL".to_owned(), ssl.to_owned()));
        }
        pairs.into_iter().collect()
    }

    #[test]
    fn disables_ssl_for_local_by_default() {
        let config = DbConfig::parse(&env("local", None)).expect("valid");
        assert_eq!(config.ssl, DbSsl::Disabled);
        assert_eq!(config.port, 5432);
        assert_eq!(config.pool_max, DB_POOL_MAX);
    }

    #[test]
    fn enables_ssl_for_deployed_environments() {
        for environment in ["development", "staging", "production"] {
            let config = DbConfig::parse(&env(environment, None)).expect("valid");
            assert_eq!(config.ssl, DbSsl::Enabled { root_cert: None });
        }
    }

    #[test]
    fn lets_db_ssl_override_the_default() {
        let config = DbConfig::parse(&env("production", Some("false"))).expect("valid");
        assert_eq!(config.ssl, DbSsl::Disabled);
        let config = DbConfig::parse(&env("local", Some("true"))).expect("valid");
        assert_eq!(config.ssl, DbSsl::Enabled { root_cert: None });
    }

    #[test]
    fn lists_every_invalid_variable() {
        let error = DbConfig::parse(&Env::from([("DB_PORT", "not-a-port")])).expect_err("invalid");
        assert_eq!(error.key, "database.env.invalid");
        assert_eq!(
            error.fields,
            vec![
                "DB_HOST",
                "DB_PORT",
                "DB_NAME",
                "DB_USER",
                "DB_PASSWORD",
                "ENVIRONMENT"
            ]
        );
        assert!(
            error
                .to_string()
                .starts_with("database.env.invalid DB_HOST, DB_PORT")
        );
    }

    #[test]
    fn readonly_is_optional() {
        assert_eq!(ReadonlyConfig::parse(&Env::default()), Ok(None));
    }

    #[test]
    fn readonly_user_must_be_named_readonly() {
        let error = ReadonlyConfig::parse(&Env::from([
            ("DB_READONLY_USER", "reporting"),
            ("DB_READONLY_PASSWORD", "secret"),
        ]))
        .expect_err("invalid");
        assert_eq!(error.fields, vec!["DB_READONLY_USER"]);
    }

    #[test]
    fn readonly_parses_valid_credentials() {
        let config = ReadonlyConfig::parse(&Env::from([
            ("DB_READONLY_USER", "readonly"),
            ("DB_READONLY_PASSWORD", "secret"),
        ]))
        .expect("valid");
        assert_eq!(
            config.map(|readonly| readonly.password),
            Some("secret".to_owned())
        );
    }
}
