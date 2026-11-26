//! `CapabilitiesRecord` (`models.capabilities`, what a refresh detected) and `OverridesRecord`
//! (`models.capability_overrides`, the user's flags).

use copilot_core::{Capabilities, CapabilityOverrides, ReasoningLevel};
use serde::{Deserialize, Serialize};

use crate::records::{RecordError, VERSION, check, parse, version};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilitiesRecord {
    pub v: u8,
    pub vision: bool,
    pub pdf: bool,
    pub tools: bool,
    pub audio_in: bool,
    pub audio_out: bool,
    pub reasoning: bool,
    /// `off`, `low`, `medium`, `high`.
    pub reasoning_levels: Vec<String>,
    pub default_reasoning: Option<String>,
}

/// Only the flags the user set; the column defaults to `{}`, which reads as no overrides.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverridesRecord {
    #[serde(default = "version")]
    pub v: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_in: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_out: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
}

impl From<&Capabilities> for CapabilitiesRecord {
    fn from(capabilities: &Capabilities) -> Self {
        Self {
            v: VERSION,
            vision: capabilities.vision,
            pdf: capabilities.pdf,
            tools: capabilities.tools,
            audio_in: capabilities.audio_in,
            audio_out: capabilities.audio_out,
            reasoning: capabilities.reasoning,
            reasoning_levels: capabilities
                .reasoning_levels
                .iter()
                .map(|level| level.as_str().to_owned())
                .collect(),
            default_reasoning: capabilities
                .default_reasoning
                .map(|level| level.as_str().to_owned()),
        }
    }
}

impl TryFrom<CapabilitiesRecord> for Capabilities {
    type Error = RecordError;

    fn try_from(record: CapabilitiesRecord) -> Result<Self, Self::Error> {
        check(record.v)?;
        Ok(Self {
            vision: record.vision,
            pdf: record.pdf,
            tools: record.tools,
            audio_in: record.audio_in,
            audio_out: record.audio_out,
            reasoning: record.reasoning,
            reasoning_levels: record
                .reasoning_levels
                .iter()
                .map(|level| parse::<ReasoningLevel>(level))
                .collect::<Result<_, _>>()?,
            default_reasoning: record
                .default_reasoning
                .as_deref()
                .map(parse::<ReasoningLevel>)
                .transpose()?,
        })
    }
}

impl From<&CapabilityOverrides> for OverridesRecord {
    fn from(overrides: &CapabilityOverrides) -> Self {
        Self {
            v: VERSION,
            vision: overrides.vision,
            pdf: overrides.pdf,
            tools: overrides.tools,
            audio_in: overrides.audio_in,
            audio_out: overrides.audio_out,
            reasoning: overrides.reasoning,
        }
    }
}

impl TryFrom<OverridesRecord> for CapabilityOverrides {
    type Error = RecordError;

    fn try_from(record: OverridesRecord) -> Result<Self, Self::Error> {
        check(record.v)?;
        Ok(Self {
            vision: record.vision,
            pdf: record.pdf,
            tools: record.tools,
            audio_in: record.audio_in,
            audio_out: record.audio_out,
            reasoning: record.reasoning,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn capabilities_round_trip_with_their_levels() {
        let capabilities = Capabilities {
            vision: true,
            pdf: false,
            tools: true,
            audio_in: false,
            audio_out: true,
            reasoning: true,
            reasoning_levels: vec![ReasoningLevel::Low, ReasoningLevel::High],
            default_reasoning: Some(ReasoningLevel::Low),
        };
        let json = serde_json::to_value(CapabilitiesRecord::from(&capabilities)).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["reasoning_levels"], json!(["low", "high"]));
        let record: CapabilitiesRecord = serde_json::from_value(json).unwrap();
        assert_eq!(Capabilities::try_from(record).unwrap(), capabilities);
    }

    #[test]
    fn overrides_store_only_the_flags_the_user_set() {
        let overrides = CapabilityOverrides {
            vision: Some(false),
            ..CapabilityOverrides::default()
        };
        let json = serde_json::to_value(OverridesRecord::from(&overrides)).unwrap();
        assert_eq!(json, json!({"v": 1, "vision": false}));
        let record: OverridesRecord = serde_json::from_value(json).unwrap();
        assert_eq!(CapabilityOverrides::try_from(record).unwrap(), overrides);

        let column_default: OverridesRecord = serde_json::from_value(json!({})).unwrap();
        assert!(
            CapabilityOverrides::try_from(column_default)
                .unwrap()
                .is_empty()
        );
    }
}
