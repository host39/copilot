//! Logger implementations.
//!
//! Every crate logs through the `tracing` macros with a dotted key as the message and structured
//! fields for context:
//!
//! ```
//! tracing::info!(port = 3000, "api.server.listening");
//! ```
//!
//! [`init`] installs the global subscriber once per process: a compact human-readable format for
//! `ENVIRONMENT=local` and one JSON object per line everywhere else.

use copilot_core::{Environment, LogLevel};
use tracing::level_filters::LevelFilter;

#[derive(Clone, Copy, Debug)]
pub struct LoggerOptions {
    pub environment: Environment,
    pub level: LogLevel,
}

#[derive(Debug, thiserror::Error)]
pub enum LoggerError {
    #[error("logger.init.failed")]
    Init(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Installs the global subscriber. Fails if one is already installed.
pub fn init(options: LoggerOptions) -> Result<(), LoggerError> {
    let level = level_filter(options.level);
    let builder = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stdout);

    let result = if options.environment.is_local() {
        builder.compact().with_target(false).try_init()
    } else {
        builder
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .try_init()
    };

    result.map_err(LoggerError::Init)
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_levels() {
        assert_eq!(level_filter(LogLevel::Warn), LevelFilter::WARN);
    }

    #[test]
    fn refuses_a_second_subscriber() {
        let options = LoggerOptions {
            environment: Environment::Production,
            level: LogLevel::Error,
        };
        let _ = init(options);
        assert!(init(options).is_err());
    }
}
