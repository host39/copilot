//! `UsageRecord` (`messages.usage`) and the `provider_state` record (`messages.provider_state`).

use copilot_core::{ModelId, ProviderId, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::records::{RecordError, VERSION, check};

/// Token counts of one answer; every count may be missing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UsageRecord {
    pub v: u8,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
}

/// Provider-opaque output of an answer, replayed only to the connection (and, for `gemini`, the
/// model) that wrote it. `content` is what the adapter reported; core reads only that. The ids
/// record where it came from even after the connection or model is deleted (the message's own
/// columns then become null).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProviderStateRecord {
    pub v: u8,
    pub provider_id: Option<Uuid>,
    pub model_id: Option<Uuid>,
    pub content: Value,
}

impl From<Usage> for UsageRecord {
    fn from(usage: Usage) -> Self {
        Self {
            v: VERSION,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
        }
    }
}

impl TryFrom<UsageRecord> for Usage {
    type Error = RecordError;

    fn try_from(record: UsageRecord) -> Result<Self, Self::Error> {
        check(record.v)?;
        Ok(Self {
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            reasoning_tokens: record.reasoning_tokens,
            cache_read_tokens: record.cache_read_tokens,
            cache_write_tokens: record.cache_write_tokens,
        })
    }
}

impl ProviderStateRecord {
    /// The state of an answer from `model` on `provider`.
    pub fn new(content: Value, provider: Option<ProviderId>, model: Option<ModelId>) -> Self {
        Self {
            v: VERSION,
            provider_id: provider.map(ProviderId::uuid),
            model_id: model.map(ModelId::uuid),
            content,
        }
    }

    /// What core replays.
    pub fn content(self) -> Result<Value, RecordError> {
        check(self.v)?;
        Ok(self.content)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn usage_round_trips_with_missing_counts() {
        let usage = Usage {
            input_tokens: Some(12),
            output_tokens: Some(3),
            ..Usage::default()
        };
        let json = serde_json::to_value(UsageRecord::from(usage)).unwrap();
        assert_eq!(
            json,
            json!({
                "v": 1,
                "input_tokens": 12,
                "output_tokens": 3,
                "reasoning_tokens": null,
                "cache_read_tokens": null,
                "cache_write_tokens": null,
            })
        );
        let record: UsageRecord = serde_json::from_value(json).unwrap();
        assert_eq!(Usage::try_from(record).unwrap(), usage);
    }

    #[test]
    fn provider_state_keeps_its_origin_and_gives_core_the_content() {
        let provider = ProviderId::generate();
        let record = ProviderStateRecord::new(json!({"reasoning": "signed"}), Some(provider), None);
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["provider_id"], json!(provider.uuid()));
        assert_eq!(json["model_id"], Value::Null);
        let record: ProviderStateRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.content().unwrap(), json!({"reasoning": "signed"}));
    }
}
