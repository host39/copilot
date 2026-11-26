//! Operational tasks run by the `migrate` and `seed` binaries. The API does not use these.

pub mod describe;
pub mod migrate;
pub mod readonly;

use copilot_core::{BoxError, Environment, LogLevel};
use copilot_logger::LoggerOptions;

use crate::env::{Env, EnvParser};

/// Parses `ENVIRONMENT` and `LOG_LEVEL` and installs the logger for a one-shot binary.
pub fn start(env: &Env) -> Result<Environment, BoxError> {
    let mut parser = EnvParser::new(env);
    let environment = parser.parse::<Environment>("ENVIRONMENT");
    let level = parser.parse_or("LOG_LEVEL", LogLevel::Info);
    parser.finish("database.env.invalid")?;
    copilot_logger::init(LoggerOptions { environment, level })?;
    Ok(environment)
}
