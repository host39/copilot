//! Connection kinds, presets, and request dialects.

use std::fmt;
use std::str::FromStr;

/// The wire protocol of a provider connection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProviderKind {
    /// Chat Completions, `POST {base}/chat/completions`.
    OpenAi,
    /// Messages API, `POST {base}/v1/messages`.
    Anthropic,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProviderKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            other => Err(other.to_owned()),
        }
    }
}

/// A known service (or `custom`) behind a connection. It fixes the default base URL, the request
/// dialect, capability defaults, and capability locks.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Preset {
    OpenAi,
    Anthropic,
    Gemini,
    OpenRouter,
    Ollama,
    /// Any other compatible server or gateway.
    Custom,
}

impl Preset {
    pub const ALL: [Self; 6] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::Gemini,
        Self::OpenRouter,
        Self::Ollama,
        Self::Custom,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }

    /// Whether the preset exists for a connection kind: `openai` takes `openai`, `gemini`,
    /// `openrouter`, `ollama`, and `custom`; `anthropic` takes `anthropic` and `custom`.
    pub fn fits(self, kind: ProviderKind) -> bool {
        match kind {
            ProviderKind::OpenAi => !matches!(self, Self::Anthropic),
            ProviderKind::Anthropic => matches!(self, Self::Anthropic | Self::Custom),
        }
    }

    /// The base URL a new connection gets when the user enters none. `custom` has none.
    pub fn base_url(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Gemini => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            Self::OpenRouter => Some("https://openrouter.ai/api/v1"),
            Self::Ollama => Some("http://localhost:11434/v1"),
            Self::Custom => None,
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Preset {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.as_str() == value)
            .ok_or_else(|| value.to_owned())
    }
}

/// How the provider adapter shapes requests. It comes from the kind and preset (no stored JSON);
/// see Adapter Rules in `docs/design/copilot.md`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Dialect {
    /// The Messages API (`anthropic` kind, any preset).
    Anthropic,
    OpenAi,
    OpenRouter,
    Ollama,
    /// Google's OpenAI-compatible endpoint.
    Gemini,
    /// Any other OpenAI-compatible server (`custom` preset).
    Compatible,
}

/// The `anthropic-version` header the Anthropic dialect sends.
pub const ANTHROPIC_VERSION: &str = "2026-10-21";

impl Dialect {
    pub fn new(kind: ProviderKind, preset: Preset) -> Self {
        match (kind, preset) {
            (ProviderKind::Anthropic, _) => Self::Anthropic,
            (ProviderKind::OpenAi, Preset::OpenAi) => Self::OpenAi,
            (ProviderKind::OpenAi, Preset::OpenRouter) => Self::OpenRouter,
            (ProviderKind::OpenAi, Preset::Ollama) => Self::Ollama,
            (ProviderKind::OpenAi, Preset::Gemini) => Self::Gemini,
            (ProviderKind::OpenAi, Preset::Anthropic | Preset::Custom) => Self::Compatible,
        }
    }

    /// The request field that carries the output token limit (`max_tokens` is required on the
    /// Anthropic dialect).
    pub fn token_field(self) -> &'static str {
        match self {
            Self::OpenAi => "max_completion_tokens",
            _ => "max_tokens",
        }
    }

    /// The role of the system message, or `None` for the top-level `system` field (Anthropic).
    pub fn system_role(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => None,
            Self::OpenAi => Some("developer"),
            _ => Some("system"),
        }
    }

    /// Whether to send `stream_options: {include_usage: true}`: OpenAI and Ollama send no usage
    /// without it. Other servers vary, so `custom` connections do not get it.
    pub fn stream_usage(self) -> bool {
        matches!(self, Self::OpenAi | Self::Ollama)
    }

    /// Whether `tool_choice` may be sent (Ollama never gets it).
    pub fn sends_tool_choice(self) -> bool {
        self != Self::Ollama
    }

    /// Whether `reasoning_details` is replayed unchanged on assistant messages (OpenRouter).
    pub fn replays_reasoning_details(self) -> bool {
        self == Self::OpenRouter
    }

    /// Whether extra per-tool-call fields (Gemini thought signatures) are kept as provider state.
    pub fn keeps_tool_call_extra(self) -> bool {
        self == Self::Gemini
    }

    /// The `anthropic-version` header value, for the Anthropic dialect only.
    pub fn api_version(self) -> Option<&'static str> {
        (self == Self::Anthropic).then_some(ANTHROPIC_VERSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_kinds_and_presets() {
        for kind in [ProviderKind::OpenAi, ProviderKind::Anthropic] {
            assert_eq!(kind.as_str().parse::<ProviderKind>(), Ok(kind));
        }
        for preset in Preset::ALL {
            assert_eq!(preset.as_str().parse::<Preset>(), Ok(preset));
        }
        assert!("google".parse::<Preset>().is_err());
        assert!("gemini".parse::<ProviderKind>().is_err());
    }

    #[test]
    fn pairs_presets_with_kinds() {
        let openai: Vec<_> = Preset::ALL
            .into_iter()
            .filter(|preset| preset.fits(ProviderKind::OpenAi))
            .collect();
        assert_eq!(
            openai,
            [
                Preset::OpenAi,
                Preset::Gemini,
                Preset::OpenRouter,
                Preset::Ollama,
                Preset::Custom
            ]
        );
        let anthropic: Vec<_> = Preset::ALL
            .into_iter()
            .filter(|preset| preset.fits(ProviderKind::Anthropic))
            .collect();
        assert_eq!(anthropic, [Preset::Anthropic, Preset::Custom]);
    }

    #[test]
    fn has_default_base_urls_except_custom() {
        assert_eq!(
            Preset::Gemini.base_url(),
            Some("https://generativelanguage.googleapis.com/v1beta/openai")
        );
        assert_eq!(
            Preset::Anthropic.base_url(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(Preset::Custom.base_url(), None);
    }

    #[test]
    fn picks_the_dialect_from_kind_and_preset() {
        let cases = [
            (
                ProviderKind::Anthropic,
                Preset::Anthropic,
                Dialect::Anthropic,
            ),
            (ProviderKind::Anthropic, Preset::Custom, Dialect::Anthropic),
            (ProviderKind::OpenAi, Preset::OpenAi, Dialect::OpenAi),
            (
                ProviderKind::OpenAi,
                Preset::OpenRouter,
                Dialect::OpenRouter,
            ),
            (ProviderKind::OpenAi, Preset::Ollama, Dialect::Ollama),
            (ProviderKind::OpenAi, Preset::Gemini, Dialect::Gemini),
            (ProviderKind::OpenAi, Preset::Custom, Dialect::Compatible),
        ];
        for (kind, preset, dialect) in cases {
            assert_eq!(Dialect::new(kind, preset), dialect, "{kind} {preset}");
        }
    }

    #[test]
    fn follows_the_adapter_rules_table() {
        assert_eq!(Dialect::Anthropic.token_field(), "max_tokens");
        assert_eq!(Dialect::OpenAi.token_field(), "max_completion_tokens");
        for dialect in [
            Dialect::OpenRouter,
            Dialect::Ollama,
            Dialect::Gemini,
            Dialect::Compatible,
        ] {
            assert_eq!(dialect.token_field(), "max_tokens");
            assert_eq!(dialect.system_role(), Some("system"));
        }
        for dialect in [Dialect::OpenRouter, Dialect::Gemini, Dialect::Compatible] {
            assert!(!dialect.stream_usage(), "{dialect:?}");
        }
        assert_eq!(Dialect::Anthropic.system_role(), None);
        assert_eq!(Dialect::OpenAi.system_role(), Some("developer"));
        assert!(Dialect::OpenAi.stream_usage());
        assert!(
            Dialect::Ollama.stream_usage(),
            "Ollama reports usage only when asked"
        );
        assert!(!Dialect::Ollama.sends_tool_choice());
        assert!(Dialect::OpenAi.sends_tool_choice());
        assert!(Dialect::OpenRouter.replays_reasoning_details());
        assert!(!Dialect::Compatible.replays_reasoning_details());
        assert!(Dialect::Gemini.keeps_tool_call_extra());
        assert!(!Dialect::OpenAi.keeps_tool_call_extra());
        assert_eq!(Dialect::Anthropic.api_version(), Some("2026-10-21"));
        assert_eq!(Dialect::OpenAi.api_version(), None);
    }
}
