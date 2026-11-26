//! Tool server records: the cached tool list (`tool_servers.tools_cached`) and the per-tool
//! modes (`tool_servers.tool_modes`, a plain map of tool name to `off`, `ask`, or `auto`).

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use copilot_core::{RemoteTool, ToolCache, ToolMode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::records::{RecordError, VERSION, check, parse};

/// The last `tools/list` result with the server's `ttlMs`. The time it was fetched is the
/// `tools_fetched_at` column.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolCacheRecord {
    pub v: u8,
    pub ttl_ms: Option<u64>,
    pub tools: Vec<RemoteToolRecord>,
}

/// A tool as the server listed it (untrusted server input).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RemoteToolRecord {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
}

impl From<&ToolCache> for ToolCacheRecord {
    fn from(cache: &ToolCache) -> Self {
        Self {
            v: VERSION,
            ttl_ms: cache
                .ttl
                .map(|ttl| u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX)),
            tools: cache
                .tools
                .iter()
                .map(|tool| RemoteToolRecord {
                    name: tool.name.clone(),
                    title: tool.title.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                    read_only: tool.read_only,
                })
                .collect(),
        }
    }
}

impl ToolCacheRecord {
    /// The cache as core reads it, fetched at `fetched_at`.
    pub fn to_cache(self, fetched_at: DateTime<Utc>) -> Result<ToolCache, RecordError> {
        check(self.v)?;
        Ok(ToolCache {
            tools: self
                .tools
                .into_iter()
                .map(|tool| RemoteTool {
                    name: tool.name,
                    title: tool.title,
                    description: tool.description,
                    input_schema: tool.input_schema,
                    read_only: tool.read_only,
                })
                .collect(),
            ttl: self.ttl_ms.map(Duration::from_millis),
            fetched_at,
        })
    }
}

/// The stored form of the per-tool modes.
pub fn modes(modes: &BTreeMap<String, ToolMode>) -> BTreeMap<String, String> {
    modes
        .iter()
        .map(|(name, mode)| (name.clone(), mode.as_str().to_owned()))
        .collect()
}

/// The per-tool modes as core reads them.
pub fn to_modes(
    stored: BTreeMap<String, String>,
) -> Result<BTreeMap<String, ToolMode>, RecordError> {
    stored
        .into_iter()
        .map(|(name, mode)| Ok((name, parse(&mode)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_cache_round_trips_with_its_ttl() {
        let fetched_at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let cache = ToolCache {
            tools: vec![RemoteTool {
                name: "search".to_owned(),
                title: Some("Search".to_owned()),
                description: "Search the docs.".to_owned(),
                input_schema: json!({"type": "object"}),
                read_only: true,
            }],
            ttl: Some(Duration::from_millis(60_000)),
            fetched_at,
        };
        let json = serde_json::to_value(ToolCacheRecord::from(&cache)).unwrap();
        assert_eq!(json["ttl_ms"], 60_000);
        let record: ToolCacheRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.to_cache(fetched_at).unwrap(), cache);
    }

    #[test]
    fn modes_round_trip_and_reject_unknown_values() {
        let stored = BTreeMap::from([("search".to_owned(), ToolMode::Off)]);
        assert_eq!(to_modes(modes(&stored)).unwrap(), stored);
        let unknown = BTreeMap::from([("search".to_owned(), "never".to_owned())]);
        assert!(to_modes(unknown).is_err());
    }
}
