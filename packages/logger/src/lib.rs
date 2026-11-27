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
//! `ENVIRONMENT=local` and one JSON object per line everywhere else. [`targets`] decides what
//! reaches it: the configured level for every crate, and nothing at all from the crates in
//! [`SILENCED`].

use copilot_core::{Environment, LogLevel};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Crates whose own events never reach the log, at any level. rmcp writes untrusted text into
/// free-form messages: a failed connection's tool server URL (which may carry a credential in
/// its path) and peer address at `error`, every server notification with its contents and the
/// server's `instructions` at `info`, and every message it receives at `trace`. copilot-mcp logs
/// what operators need as `mcp.*` events with the server id.
pub const SILENCED: &[&str] = &["rmcp"];

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
    let filter = targets(options.level);
    let builder = tracing_subscriber::fmt()
        .with_max_level(level_filter(options.level))
        .with_writer(std::io::stdout);

    let result = if options.environment.is_local() {
        builder
            .compact()
            .with_target(false)
            .finish()
            .with(filter)
            .try_init()
    } else {
        builder
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .finish()
            .with(filter)
            .try_init()
    };

    result.map_err(|error| LoggerError::Init(Box::new(error)))
}

/// What reaches the log: `level` and above from every crate, nothing from [`SILENCED`].
pub fn targets(level: LogLevel) -> Targets {
    SILENCED.iter().fold(
        Targets::new().with_default(level_filter(level)),
        |targets, crate_name| targets.with_target(*crate_name, LevelFilter::OFF),
    )
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
    use tracing::Level;

    use super::*;

    #[test]
    fn maps_levels() {
        assert_eq!(level_filter(LogLevel::Warn), LevelFilter::WARN);
    }

    #[test]
    fn silences_rmcp_at_every_level() {
        for level in [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ] {
            let filter = targets(level);
            for target in ["rmcp", "rmcp::service", "rmcp::transport::worker"] {
                for event in [Level::ERROR, Level::WARN, Level::INFO, Level::TRACE] {
                    assert!(
                        !filter.would_enable(target, &event),
                        "{target} at {event} with LOG_LEVEL {level:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn keeps_the_configured_level_for_other_crates() {
        let filter = targets(LogLevel::Info);
        assert!(filter.would_enable("copilot_mcp::errors", &Level::WARN));
        assert!(filter.would_enable("copilot_api", &Level::INFO));
        assert!(!filter.would_enable("copilot_api", &Level::DEBUG));
        assert!(targets(LogLevel::Debug).would_enable("copilot_mcp::connection", &Level::DEBUG));
        assert!(!targets(LogLevel::Error).would_enable("copilot_api", &Level::WARN));
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
