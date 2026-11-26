//! Model capabilities: defaults, user overrides, and locks.
//!
//! Sources, in order (`docs/design/copilot.md`, Providers and Models): provider-reported
//! metadata; preset defaults (PDF on for the `openai` and `openrouter` presets); otherwise text
//! plus `tools`, everything else off until the user enables it. Locked flags are forced off and
//! cannot be overridden. There is no curated model-family table beyond the locks below.

use std::fmt;

use crate::domains::provider::values::{Preset, ProviderKind};
use crate::ports::llm_gateway::RemoteModel;
use crate::shared::reasoning::ReasoningLevel;

/// One capability flag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    Vision,
    Pdf,
    Tools,
    AudioIn,
    AudioOut,
    Reasoning,
}

impl Capability {
    pub const ALL: [Self; 6] = [
        Self::Vision,
        Self::Pdf,
        Self::Tools,
        Self::AudioIn,
        Self::AudioOut,
        Self::Reasoning,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vision => "vision",
            Self::Pdf => "pdf",
            Self::Tools => "tools",
            Self::AudioIn => "audio_in",
            Self::AudioOut => "audio_out",
            Self::Reasoning => "reasoning",
        }
    }

    /// A label for messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Vision => "Image input",
            Self::Pdf => "PDF input",
            Self::Tools => "Tool use",
            Self::AudioIn => "Audio input",
            Self::AudioOut => "Audio output",
            Self::Reasoning => "Reasoning",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a model can do (stored as the detected `CapabilitiesRecord`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    pub vision: bool,
    pub pdf: bool,
    pub tools: bool,
    pub audio_in: bool,
    pub audio_out: bool,
    pub reasoning: bool,
    /// Levels the picker offers; empty unless `reasoning`.
    pub reasoning_levels: Vec<ReasoningLevel>,
    /// The model's own default level when the provider reports one.
    pub default_reasoning: Option<ReasoningLevel>,
}

impl Capabilities {
    /// Detected capabilities of a listed model: reported values first, then preset defaults.
    pub fn defaults(preset: Preset, remote: &RemoteModel) -> Self {
        let reported = &remote.reported;
        let pdf_default = matches!(preset, Preset::OpenAi | Preset::OpenRouter);
        Self {
            vision: reported.vision.unwrap_or(false),
            pdf: reported.pdf.unwrap_or(pdf_default),
            tools: reported.tools.unwrap_or(true),
            audio_in: reported.audio_in.unwrap_or(false),
            audio_out: reported.audio_out.unwrap_or(false),
            reasoning: reported.reasoning.unwrap_or(false),
            reasoning_levels: reported.reasoning_levels.clone().unwrap_or_default(),
            default_reasoning: reported.default_reasoning,
        }
        .normalized()
    }

    /// Overrides on top of the detected capabilities, then the locks.
    pub fn effective(detected: &Self, overrides: &CapabilityOverrides, locks: &Locks) -> Self {
        let mut capabilities = detected.clone();
        for flag in Capability::ALL {
            if let Some(value) = overrides.get(flag) {
                capabilities.set(flag, value);
            }
        }
        capabilities.locked(locks)
    }

    /// A copy with every locked flag off.
    #[must_use]
    pub fn locked(mut self, locks: &Locks) -> Self {
        for lock in locks.iter() {
            self.set(lock.capability, false);
        }
        self.normalized()
    }

    pub fn get(&self, flag: Capability) -> bool {
        match flag {
            Capability::Vision => self.vision,
            Capability::Pdf => self.pdf,
            Capability::Tools => self.tools,
            Capability::AudioIn => self.audio_in,
            Capability::AudioOut => self.audio_out,
            Capability::Reasoning => self.reasoning,
        }
    }

    pub fn set(&mut self, flag: Capability, value: bool) {
        let slot = match flag {
            Capability::Vision => &mut self.vision,
            Capability::Pdf => &mut self.pdf,
            Capability::Tools => &mut self.tools,
            Capability::AudioIn => &mut self.audio_in,
            Capability::AudioOut => &mut self.audio_out,
            Capability::Reasoning => &mut self.reasoning,
        };
        *slot = value;
    }

    /// Reasoning levels follow the flag: sorted and unique while on (every level when none were
    /// reported), empty while off. A default outside the levels is dropped.
    fn normalized(mut self) -> Self {
        if self.reasoning {
            if self.reasoning_levels.is_empty() {
                self.reasoning_levels = ReasoningLevel::ALL.to_vec();
            }
            self.reasoning_levels.sort_unstable();
            self.reasoning_levels.dedup();
            if self
                .default_reasoning
                .is_some_and(|level| !self.reasoning_levels.contains(&level))
            {
                self.default_reasoning = None;
            }
        } else {
            self.reasoning_levels.clear();
            self.default_reasoning = None;
        }
        self
    }
}

/// The user's per-flag overrides (`capability_overrides`). `None` keeps the detected value; a
/// refresh never touches overrides.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilityOverrides {
    pub vision: Option<bool>,
    pub pdf: Option<bool>,
    pub tools: Option<bool>,
    pub audio_in: Option<bool>,
    pub audio_out: Option<bool>,
    pub reasoning: Option<bool>,
}

impl CapabilityOverrides {
    pub fn get(&self, flag: Capability) -> Option<bool> {
        match flag {
            Capability::Vision => self.vision,
            Capability::Pdf => self.pdf,
            Capability::Tools => self.tools,
            Capability::AudioIn => self.audio_in,
            Capability::AudioOut => self.audio_out,
            Capability::Reasoning => self.reasoning,
        }
    }

    /// The flags that have an override.
    pub fn flags(&self) -> impl Iterator<Item = Capability> + '_ {
        Capability::ALL
            .into_iter()
            .filter(|flag| self.get(*flag).is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.flags().next().is_none()
    }
}

/// A flag the UI hides because Copilot cannot use it with this connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lock {
    pub capability: Capability,
    pub reason: &'static str,
}

/// The locked flags of one model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Locks(Vec<Lock>);

impl Locks {
    /// `audio_in` and `audio_out` on `anthropic` connections; `tools` on `gpt-6*` models under
    /// the `openai` preset (tool calling there needs the Responses API, a later adapter).
    pub fn new(kind: ProviderKind, preset: Preset, remote_id: &str) -> Self {
        let mut locks = Vec::new();
        if kind == ProviderKind::Anthropic {
            let reason = "Claude models do not take or return audio.";
            locks.push(Lock {
                capability: Capability::AudioIn,
                reason,
            });
            locks.push(Lock {
                capability: Capability::AudioOut,
                reason,
            });
        }
        if preset == Preset::OpenAi && remote_id.to_ascii_lowercase().starts_with("gpt-6") {
            locks.push(Lock {
                capability: Capability::Tools,
                reason: "Tool use on this model needs the Responses adapter.",
            });
        }
        Self(locks)
    }

    pub fn get(&self, flag: Capability) -> Option<&Lock> {
        self.0.iter().find(|lock| lock.capability == flag)
    }

    pub fn contains(&self, flag: Capability) -> bool {
        self.get(flag).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Lock> {
        self.0.iter()
    }
}

/// Whether the `openai` preset must send `reasoning_effort: none` whenever tools are in the
/// request (GPT-5.4 and later 5.x models on Chat Completions). The picker says so.
pub fn reasoning_off_with_tools(preset: Preset, remote_id: &str) -> bool {
    if preset != Preset::OpenAi {
        return false;
    }
    let id = remote_id.to_ascii_lowercase();
    let Some(minor) = id.strip_prefix("gpt-5.") else {
        return false;
    };
    let digits: String = minor.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u32>().is_ok_and(|minor| minor >= 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::llm_gateway::ReportedCapabilities;

    fn remote(reported: ReportedCapabilities) -> RemoteModel {
        RemoteModel {
            remote_id: "model-1".to_owned(),
            display_name: "Model 1".to_owned(),
            reported,
            ..RemoteModel::default()
        }
    }

    #[test]
    fn defaults_to_text_plus_tools() {
        let capabilities =
            Capabilities::defaults(Preset::Ollama, &remote(ReportedCapabilities::default()));
        assert_eq!(
            capabilities,
            Capabilities {
                tools: true,
                ..Capabilities::default()
            }
        );
    }

    #[test]
    fn turns_pdf_on_for_the_openai_and_openrouter_presets() {
        for preset in Preset::ALL {
            let capabilities =
                Capabilities::defaults(preset, &remote(ReportedCapabilities::default()));
            let expected = matches!(preset, Preset::OpenAi | Preset::OpenRouter);
            assert_eq!(capabilities.pdf, expected, "{preset}");
        }
    }

    #[test]
    fn prefers_reported_values_over_preset_defaults() {
        let reported = ReportedCapabilities {
            vision: Some(true),
            pdf: Some(false),
            tools: Some(false),
            audio_in: Some(true),
            reasoning: Some(true),
            reasoning_levels: Some(vec![
                ReasoningLevel::High,
                ReasoningLevel::Low,
                ReasoningLevel::High,
            ]),
            default_reasoning: Some(ReasoningLevel::Low),
            ..ReportedCapabilities::default()
        };
        let capabilities = Capabilities::defaults(Preset::OpenAi, &remote(reported));
        assert!(capabilities.vision);
        assert!(!capabilities.pdf, "reported false beats the preset default");
        assert!(!capabilities.tools);
        assert!(capabilities.audio_in);
        assert!(!capabilities.audio_out);
        assert_eq!(
            capabilities.reasoning_levels,
            [ReasoningLevel::Low, ReasoningLevel::High]
        );
        assert_eq!(capabilities.default_reasoning, Some(ReasoningLevel::Low));
    }

    #[test]
    fn offers_every_level_when_reasoning_has_none_reported() {
        let reported = ReportedCapabilities {
            reasoning: Some(true),
            ..ReportedCapabilities::default()
        };
        let capabilities = Capabilities::defaults(Preset::Custom, &remote(reported));
        assert_eq!(capabilities.reasoning_levels, ReasoningLevel::ALL);
    }

    #[test]
    fn drops_levels_and_a_foreign_default_without_reasoning() {
        let reported = ReportedCapabilities {
            reasoning: Some(false),
            reasoning_levels: Some(vec![ReasoningLevel::Low]),
            default_reasoning: Some(ReasoningLevel::Low),
            ..ReportedCapabilities::default()
        };
        let capabilities = Capabilities::defaults(Preset::Custom, &remote(reported));
        assert!(capabilities.reasoning_levels.is_empty());
        assert_eq!(capabilities.default_reasoning, None);

        let reported = ReportedCapabilities {
            reasoning: Some(true),
            reasoning_levels: Some(vec![ReasoningLevel::Low]),
            default_reasoning: Some(ReasoningLevel::High),
            ..ReportedCapabilities::default()
        };
        let capabilities = Capabilities::defaults(Preset::Custom, &remote(reported));
        assert_eq!(capabilities.default_reasoning, None);
    }

    #[test]
    fn locks_audio_on_anthropic_connections() {
        let locks = Locks::new(
            ProviderKind::Anthropic,
            Preset::Anthropic,
            "claude-opus-5-5",
        );
        assert!(locks.contains(Capability::AudioIn));
        assert!(locks.contains(Capability::AudioOut));
        assert!(!locks.contains(Capability::Tools));
        let custom = Locks::new(ProviderKind::Anthropic, Preset::Custom, "anything");
        assert!(custom.contains(Capability::AudioIn));
        let openai = Locks::new(ProviderKind::OpenAi, Preset::OpenAi, "gpt-audio-2");
        assert!(!openai.contains(Capability::AudioIn));
    }

    #[test]
    fn locks_tools_on_gpt_6_under_the_openai_preset_only() {
        let locks = Locks::new(ProviderKind::OpenAi, Preset::OpenAi, "GPT-6.1-sol");
        assert_eq!(
            locks.get(Capability::Tools).map(|lock| lock.reason),
            Some("Tool use on this model needs the Responses adapter.")
        );
        assert!(
            !Locks::new(ProviderKind::OpenAi, Preset::OpenRouter, "gpt-6-astra")
                .contains(Capability::Tools)
        );
        assert!(
            !Locks::new(ProviderKind::OpenAi, Preset::OpenAi, "gpt-5.6-terra")
                .contains(Capability::Tools)
        );
    }

    #[test]
    fn applies_overrides_then_locks() {
        let detected = Capabilities {
            tools: true,
            ..Capabilities::default()
        };
        let overrides = CapabilityOverrides {
            vision: Some(true),
            tools: Some(false),
            audio_in: Some(true),
            reasoning: Some(true),
            ..CapabilityOverrides::default()
        };
        let locks = Locks::new(ProviderKind::Anthropic, Preset::Anthropic, "claude");
        let effective = Capabilities::effective(&detected, &overrides, &locks);
        assert!(effective.vision);
        assert!(!effective.tools);
        assert!(!effective.audio_in, "the lock wins over the override");
        assert!(effective.reasoning);
        assert_eq!(effective.reasoning_levels, ReasoningLevel::ALL);
    }

    #[test]
    fn lists_overridden_flags() {
        let overrides = CapabilityOverrides {
            pdf: Some(false),
            audio_out: Some(true),
            ..CapabilityOverrides::default()
        };
        assert_eq!(
            overrides.flags().collect::<Vec<_>>(),
            [Capability::Pdf, Capability::AudioOut]
        );
        assert!(!overrides.is_empty());
        assert!(CapabilityOverrides::default().is_empty());
    }

    #[test]
    fn forces_reasoning_off_with_tools_from_gpt_5_4() {
        let cases = [
            (Preset::OpenAi, "gpt-5.4", true),
            (Preset::OpenAi, "gpt-5.4-mini", true),
            (Preset::OpenAi, "gpt-5.6-terra", true),
            (Preset::OpenAi, "gpt-5.10", true),
            (Preset::OpenAi, "gpt-5.3", false),
            (Preset::OpenAi, "gpt-5", false),
            (Preset::OpenAi, "gpt-5-mini", false),
            (Preset::OpenAi, "gpt-4.1", false),
            (Preset::OpenRouter, "gpt-5.6-terra", false),
        ];
        for (preset, id, expected) in cases {
            assert_eq!(
                reasoning_off_with_tools(preset, id),
                expected,
                "{preset} {id}"
            );
        }
    }
}
