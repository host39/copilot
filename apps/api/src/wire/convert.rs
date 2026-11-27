//! Conversions between `copilot-core` values and their wire forms in `copilot-contracts`. Both
//! sides are foreign to this crate, so the conversions are local traits: [`ToDto`] for responses
//! and [`ToCore`] for request bodies. Route modules map whole entities with them.

use copilot_contracts as dto;
use copilot_core as core;

/// A core value as the API sends it.
pub trait ToDto {
    type Dto;
    fn to_dto(self) -> Self::Dto;
}

/// A request value as core reads it.
pub trait ToCore {
    type Core;
    fn to_core(self) -> Self::Core;
}

/// Implements both directions for an enum whose variants have the same names on both sides.
macro_rules! both_ways {
    ($core:ty, $dto:ty, [$($variant:ident),+ $(,)?]) => {
        impl ToDto for $core {
            type Dto = $dto;
            fn to_dto(self) -> $dto {
                match self {
                    $(<$core>::$variant => <$dto>::$variant,)+
                }
            }
        }

        impl ToCore for $dto {
            type Core = $core;
            fn to_core(self) -> $core {
                match self {
                    $(<$dto>::$variant => <$core>::$variant,)+
                }
            }
        }
    };
}

/// Implements [`ToDto`] for an enum whose variants have the same names on both sides.
macro_rules! out {
    ($core:ty, $dto:ty, [$($variant:ident),+ $(,)?]) => {
        impl ToDto for $core {
            type Dto = $dto;
            fn to_dto(self) -> $dto {
                match self {
                    $(<$core>::$variant => <$dto>::$variant,)+
                }
            }
        }
    };
}

both_ways!(
    core::ReasoningLevel,
    dto::ReasoningLevel,
    [Off, Low, Medium, High]
);
both_ways!(core::BuiltinTool, dto::BuiltinTool, [Schedules]);
both_ways!(core::ProviderKind, dto::ProviderKind, [OpenAi, Anthropic]);
both_ways!(
    core::OpenAiApi,
    dto::ProviderApi,
    [ChatCompletions, Responses]
);
both_ways!(
    core::Preset,
    dto::Preset,
    [OpenAi, Anthropic, Gemini, OpenRouter, Ollama, Custom]
);
both_ways!(core::ApprovalMode, dto::ApprovalMode, [Ask, Auto]);
both_ways!(core::ToolMode, dto::ToolMode, [Off, Ask, Auto]);
both_ways!(core::OnAsk, dto::OnAsk, [Deny, Pause]);
both_ways!(core::Decision, dto::Decision, [Approve, Deny]);
both_ways!(core::DecisionScope, dto::DecisionScope, [Once, Chat]);
out!(core::CreatedBy, dto::CreatedBy, [User, Assistant]);
out!(
    core::RunStatus,
    dto::RunStatus,
    [Ok, Error, AwaitingApproval]
);
out!(core::KeyState, dto::KeyState, [Missing, Set, Unreadable]);
out!(core::Role, dto::Role, [User, Assistant, Tool]);
out!(
    core::MessageStatus,
    dto::MessageStatus,
    [
        Streaming,
        Complete,
        AwaitingApproval,
        Cancelled,
        Error,
        Refused
    ]
);
out!(
    core::FinishReason,
    dto::FinishReason,
    [Stop, ToolCalls, Length, Refusal, Error, Other, ToolLimit]
);
out!(
    core::ToolStatus,
    dto::ToolStatus,
    [Ok, Error, Denied, Cancelled, Timeout]
);
out!(
    core::Approval,
    dto::Approval,
    [Auto, Pending, Approved, Denied]
);
out!(
    core::PartKind,
    dto::PartKind,
    [Text, Reasoning, ToolCall, Audio]
);
out!(
    core::Capability,
    dto::Capability,
    [Vision, Pdf, Tools, AudioIn, AudioOut, Reasoning]
);
out!(
    core::CapacityLevel,
    dto::CapacityLevel,
    [Unknown, Fits, Warn, Over]
);

impl ToDto for core::Capacity {
    type Dto = dto::Capacity;
    fn to_dto(self) -> dto::Capacity {
        dto::Capacity {
            used: self.used,
            window: self.window,
            level: self.level().to_dto(),
        }
    }
}

impl ToDto for core::Usage {
    type Dto = dto::Usage;
    fn to_dto(self) -> dto::Usage {
        dto::Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
        }
    }
}

impl ToDto for core::Refusal {
    type Dto = dto::Refusal;
    fn to_dto(self) -> dto::Refusal {
        dto::Refusal {
            category: self.category,
            explanation: self.explanation,
        }
    }
}

impl ToDto for &core::ResultContent {
    type Dto = dto::ResultContent;
    fn to_dto(self) -> dto::ResultContent {
        match self {
            core::ResultContent::Text(text) => dto::ResultContent::Text { text: text.clone() },
            core::ResultContent::Image { file_id } => dto::ResultContent::Image {
                file_id: file_id.uuid(),
            },
        }
    }
}

impl ToDto for &core::Capabilities {
    type Dto = dto::Capabilities;
    fn to_dto(self) -> dto::Capabilities {
        dto::Capabilities {
            vision: self.vision,
            pdf: self.pdf,
            tools: self.tools,
            audio_in: self.audio_in,
            audio_out: self.audio_out,
            reasoning: self.reasoning,
            reasoning_levels: self
                .reasoning_levels
                .iter()
                .map(|level| level.to_dto())
                .collect(),
            default_reasoning: self.default_reasoning.map(ToDto::to_dto),
        }
    }
}

impl ToDto for &core::CapabilityOverrides {
    type Dto = dto::CapabilityOverrides;
    fn to_dto(self) -> dto::CapabilityOverrides {
        dto::CapabilityOverrides {
            vision: self.vision,
            pdf: self.pdf,
            tools: self.tools,
            audio_in: self.audio_in,
            audio_out: self.audio_out,
            reasoning: self.reasoning,
        }
    }
}

impl ToCore for dto::CapabilityOverrides {
    type Core = core::CapabilityOverrides;
    fn to_core(self) -> core::CapabilityOverrides {
        core::CapabilityOverrides {
            vision: self.vision,
            pdf: self.pdf,
            tools: self.tools,
            audio_in: self.audio_in,
            audio_out: self.audio_out,
            reasoning: self.reasoning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_apis_keep_their_names_on_the_wire() {
        for api in core::OpenAiApi::ALL {
            assert_eq!(api.to_dto().to_core(), api);
            assert_eq!(serde_json::to_value(api.to_dto()).unwrap(), api.as_str());
        }
    }

    #[test]
    fn enums_keep_their_meaning_on_the_wire() {
        for level in core::ReasoningLevel::ALL {
            assert_eq!(level.to_dto().to_core(), level);
            assert_eq!(
                serde_json::to_value(level.to_dto()).unwrap(),
                level.as_str()
            );
        }
        for preset in core::Preset::ALL {
            assert_eq!(preset.to_dto().to_core(), preset);
            assert_eq!(
                serde_json::to_value(preset.to_dto()).unwrap(),
                preset.as_str()
            );
        }
        assert_eq!(
            serde_json::to_value(core::MessageStatus::AwaitingApproval.to_dto()).unwrap(),
            core::MessageStatus::AwaitingApproval.as_str()
        );
        assert_eq!(
            serde_json::to_value(core::FinishReason::ToolLimit.to_dto()).unwrap(),
            core::FinishReason::ToolLimit.as_str()
        );
        assert_eq!(
            serde_json::to_value(core::Capability::AudioIn.to_dto()).unwrap(),
            "audioIn"
        );
        for mode in [
            core::ToolMode::Off,
            core::ToolMode::Ask,
            core::ToolMode::Auto,
        ] {
            assert_eq!(mode.to_dto().to_core(), mode);
            assert_eq!(serde_json::to_value(mode.to_dto()).unwrap(), mode.as_str());
        }
        for mode in [core::ApprovalMode::Ask, core::ApprovalMode::Auto] {
            assert_eq!(mode.to_dto().to_core(), mode);
            assert_eq!(serde_json::to_value(mode.to_dto()).unwrap(), mode.as_str());
        }
        for on_ask in [core::OnAsk::Deny, core::OnAsk::Pause] {
            assert_eq!(on_ask.to_dto().to_core(), on_ask);
            assert_eq!(
                serde_json::to_value(on_ask.to_dto()).unwrap(),
                on_ask.as_str()
            );
        }
        for by in [core::CreatedBy::User, core::CreatedBy::Assistant] {
            assert_eq!(serde_json::to_value(by.to_dto()).unwrap(), by.as_str());
        }
        for status in [
            core::RunStatus::Ok,
            core::RunStatus::Error,
            core::RunStatus::AwaitingApproval,
        ] {
            assert_eq!(
                serde_json::to_value(status.to_dto()).unwrap(),
                status.as_str()
            );
        }
    }
}
