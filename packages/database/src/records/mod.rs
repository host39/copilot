//! The versioned JSON records of the `jsonb` columns (`docs/design/copilot.md`, Stored JSON
//! Shapes). Repositories map them to and from `copilot-core` entities; nothing outside this crate
//! sees them.
//!
//! Every record carries `"v": 1`; tagged records (message parts, result content) also carry
//! `"type"`. The shapes are frozen with `p1-frozen`: a change adds a new version that readers
//! accept next to the old one, because stored rows are never rewritten.

mod capabilities;
mod message;
mod part;
mod snapshot;
mod tool;

use std::str::FromStr;

pub use crate::records::capabilities::{CapabilitiesRecord, OverridesRecord};
pub use crate::records::message::{ProviderStateRecord, UsageRecord};
pub use crate::records::part::{ContentRecord, PartRecord};
pub use crate::records::snapshot::{SnapshotFileRecord, SnapshotToolRecord};
pub use crate::records::tool::{RemoteToolRecord, ToolCacheRecord, modes, to_modes};

/// The version every record is written with.
pub const VERSION: u8 = 1;

/// A stored record this version cannot read.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("database.records.version.unknown {0}")]
    Version(u8),
    #[error("database.records.value.invalid {0}")]
    Value(String),
}

/// The default of a `v` field that older rows may lack (the column defaults).
fn version() -> u8 {
    VERSION
}

/// Accepts the versions this code reads.
fn check(v: u8) -> Result<(), RecordError> {
    if v == VERSION {
        Ok(())
    } else {
        Err(RecordError::Version(v))
    }
}

/// A stored enum value (`text_enum` spelling), or the value it did not recognize.
fn parse<T: FromStr>(value: &str) -> Result<T, RecordError> {
    value
        .parse()
        .map_err(|_| RecordError::Value(value.to_owned()))
}

/// Lowercase hex of a digest.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A 32-byte digest from its lowercase hex.
fn unhex(text: &str) -> Result<[u8; 32], RecordError> {
    let invalid = || RecordError::Value("sha256".to_owned());
    if text.len() != 64 || !text.is_ascii() {
        return Err(invalid());
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let pair = &text[index * 2..index * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).map_err(|_| invalid())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_round_trip_through_hex() {
        let digest = [0xab_u8; 32];
        let text = hex(&digest);
        assert_eq!(text.len(), 64);
        assert_eq!(unhex(&text).unwrap(), digest);
        assert!(unhex("ab").is_err());
        assert!(unhex(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn only_the_current_version_reads() {
        assert!(check(VERSION).is_ok());
        assert_eq!(
            check(2).unwrap_err().to_string(),
            "database.records.version.unknown 2"
        );
    }
}
