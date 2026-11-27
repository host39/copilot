//! Settings from the environment (`tools/fake-llm/README.md`, Settings).

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

/// The key every model request must carry unless `FAKE_LLM_KEY` names another.
pub const DEFAULT_KEY: &str = "fake-key";
/// The port `make fake-llm` serves on.
pub const DEFAULT_PORT: u16 = 4010;

#[derive(Clone, Debug)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    /// The one API key the model routes accept (`Authorization: Bearer` or `x-api-key`).
    pub key: String,
    /// The pause between streamed chunks.
    pub chunk_ms: u64,
    /// The pause between streamed chunks of a reply that asked for `slow`.
    pub slow_ms: u64,
    /// When set, `/mcp` answers `401` without `Authorization: Bearer <token>`.
    pub mcp_token: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConfigError {
    /// Every variable that did not parse, by name.
    Invalid(Vec<&'static str>),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(names) => {
                write!(formatter, "fake-llm.config.invalid {}", names.join(", "))
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: IpAddr::from([127, 0, 0, 1]),
            port: DEFAULT_PORT,
            key: DEFAULT_KEY.to_owned(),
            chunk_ms: 30,
            slow_ms: 500,
            mcp_token: None,
        }
    }
}

impl Config {
    /// Reads the process environment.
    pub fn load() -> Result<Self, ConfigError> {
        Self::parse(|name| std::env::var(name).ok())
    }

    /// Reads the variables through `lookup`; blank values count as unset.
    pub fn parse(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let read = |name: &str| {
            lookup(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let defaults = Self::default();
        let mut invalid = Vec::new();
        let mut number = |name: &'static str, default: u64| match read(name) {
            Some(value) => value.parse().unwrap_or_else(|_| {
                invalid.push(name);
                default
            }),
            None => default,
        };
        let port = number("FAKE_LLM_PORT", u64::from(defaults.port));
        let chunk_ms = number("FAKE_LLM_CHUNK_MS", defaults.chunk_ms);
        let slow_ms = number("FAKE_LLM_SLOW_MS", defaults.slow_ms);
        let port = u16::try_from(port).unwrap_or_else(|_| {
            invalid.push("FAKE_LLM_PORT");
            defaults.port
        });
        let host = match read("FAKE_LLM_HOST") {
            Some(value) => value.parse().unwrap_or_else(|_| {
                invalid.push("FAKE_LLM_HOST");
                defaults.host
            }),
            None => defaults.host,
        };
        let key = read("FAKE_LLM_KEY").unwrap_or(defaults.key);
        if key.chars().any(char::is_whitespace) {
            invalid.push("FAKE_LLM_KEY");
        }
        if !invalid.is_empty() {
            return Err(ConfigError::Invalid(invalid));
        }
        Ok(Self {
            host,
            port,
            key,
            chunk_ms,
            slow_ms,
            mcp_token: read("FAKE_LLM_MCP_TOKEN"),
        })
    }

    /// The pause between streamed chunks.
    pub fn delay(&self, slow: bool) -> Duration {
        Duration::from_millis(if slow { self.slow_ms } else { self.chunk_ms })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Config::parse(|name| map.get(name).cloned())
    }

    #[test]
    fn defaults_to_fake_key_on_loopback_port_4010() {
        let config = parse(&[]).expect("valid");
        assert_eq!(config.key, "fake-key");
        assert_eq!(config.port, 4010);
        assert_eq!(config.host, IpAddr::from([127, 0, 0, 1]));
        assert_eq!(config.mcp_token, None);
        assert_eq!(config.delay(false), Duration::from_millis(30));
        assert_eq!(config.delay(true), Duration::from_millis(500));
    }

    #[test]
    fn reads_every_variable_and_ignores_blank_ones() {
        let config = parse(&[
            ("FAKE_LLM_HOST", "0.0.0.0"),
            ("FAKE_LLM_PORT", "4019"),
            ("FAKE_LLM_KEY", "smoke-key"),
            ("FAKE_LLM_CHUNK_MS", "0"),
            ("FAKE_LLM_SLOW_MS", " 50 "),
            ("FAKE_LLM_MCP_TOKEN", "  "),
        ])
        .expect("valid");
        assert_eq!(config.host, IpAddr::from([0, 0, 0, 0]));
        assert_eq!(config.port, 4019);
        assert_eq!(config.key, "smoke-key");
        assert_eq!(config.delay(false), Duration::ZERO);
        assert_eq!(config.delay(true), Duration::from_millis(50));
        assert_eq!(config.mcp_token, None);
    }

    #[test]
    fn names_every_invalid_variable() {
        let error = parse(&[
            ("FAKE_LLM_PORT", "70000"),
            ("FAKE_LLM_HOST", "nowhere"),
            ("FAKE_LLM_SLOW_MS", "soon"),
            ("FAKE_LLM_KEY", "two words"),
        ])
        .expect_err("invalid");
        assert_eq!(
            error.to_string(),
            "fake-llm.config.invalid FAKE_LLM_SLOW_MS, FAKE_LLM_PORT, FAKE_LLM_HOST, FAKE_LLM_KEY"
        );
    }
}
