//! Model provider port.
//!
//! `copilot-providers` implements [`LlmGateway`] for the OpenAI-compatible and Anthropic wire
//! protocols. Core builds every target and every request (`conversation::context`), so adapters
//! never read the database or the cipher, and never decide what a model sees. The port is frozen
//! from P1.3 on (`docs/design/roadmap.md`, Freeze List).

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::BoxStream;
use serde_json::Value;
use uuid::Uuid;

use crate::domains::chat::{FinishReason, Role, Usage};
use crate::domains::provider::{Dialect, Preset, ProviderKind};
use crate::domains::tool::ToolSpec;
use crate::ports::tool_host::ToolContent;
use crate::shared::base_url::BaseUrl;
use crate::shared::errors::DomainResult;
use crate::shared::ids::ProviderId;
use crate::shared::net::NetworkScope;
use crate::shared::reasoning::ReasoningLevel;
use crate::shared::secret::Secret;

#[async_trait]
pub trait LlmGateway: Send + Sync {
    /// The provider's models. Adapters map HTTP failures to human-readable `DomainError`s
    /// (for example `PROVIDER_AUTH` "Key rejected: open provider settings.").
    async fn models(&self, target: &ProviderTarget) -> DomainResult<Vec<RemoteModel>>;

    /// Streams one answer. Failing before the first event (connection, HTTP status) returns
    /// `Err`; a failure after that is an `Err` item, and the stream ends after it. A finished
    /// answer ends with [`LlmEvent::Finished`]. Dropping the stream cancels the request.
    async fn stream(
        &self,
        target: &ModelTarget,
        request: LlmRequest,
    ) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>>;
}

/// Everything an adapter needs to reach a provider connection. `network` picks the outbound
/// client (public-only or private-allowed); a plain `http` `base_url` (`!is_https()`) takes the
/// plaintext client, whose resolver admits only `net::is_allowed_plaintext` addresses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderTarget {
    pub provider_id: ProviderId,
    pub kind: ProviderKind,
    pub preset: Preset,
    pub base_url: BaseUrl,
    pub api_key: Option<Secret>,
    pub network: NetworkScope,
}

/// A model on a provider connection, with the dialect for its requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelTarget {
    pub provider: ProviderTarget,
    pub remote_id: String,
    pub dialect: Dialect,
}

/// A model as the provider lists it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RemoteModel {
    pub remote_id: String,
    pub display_name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reported: ReportedCapabilities,
}

/// Capabilities the provider reported (Anthropic Models API `capabilities`, OpenRouter
/// `architecture` modalities). `None` means "not reported"; core fills the gaps with preset
/// defaults (`Capabilities::defaults`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReportedCapabilities {
    pub vision: Option<bool>,
    pub pdf: Option<bool>,
    pub tools: Option<bool>,
    pub audio_in: Option<bool>,
    pub audio_out: Option<bool>,
    pub reasoning: Option<bool>,
    pub reasoning_levels: Option<Vec<ReasoningLevel>>,
    pub default_reasoning: Option<ReasoningLevel>,
}

/// One model request, built by core from the turn snapshot (`conversation::context`).
#[derive(Clone, Debug, PartialEq)]
pub struct LlmRequest {
    /// The model's remote id.
    pub model: String,
    /// Empty means no system prompt.
    pub system: String,
    /// Never ends with an assistant message.
    pub messages: Vec<LlmMessage>,
    /// Sorted by alias; empty when the model has no tool use or the turn offers no tools.
    pub tools: Vec<ToolSpec>,
    pub max_output_tokens: u32,
    /// `None` is the model default. `Some(Off)` turns reasoning off (`reasoning_effort: none`
    /// where the dialect needs it, for example on the `openai` preset when tools are sent).
    pub reasoning: Option<ReasoningLevel>,
    /// The model has the `reasoning` capability, so the adapter may ask to see its reasoning
    /// without choosing a level (Anthropic `display: "summarized"` with the model's default
    /// effort when `reasoning` is `None`).
    pub can_reason: bool,
    /// Spoken replies for `audio_out` models with Speak replies on.
    pub audio_out: Option<AudioOut>,
    /// Mark the end of the file preamble and the last message as cache breakpoints (Anthropic
    /// `cache_control`).
    pub cache_breakpoints: bool,
}

/// A message of a request. `replay` is the stored `provider_state` content of an assistant
/// message, sent verbatim when the replay rules allow it (`docs/design/copilot.md`, Replay
/// Rules); the adapter then prefers it over the neutral parts.
#[derive(Clone, Debug, PartialEq)]
pub struct LlmMessage {
    pub role: Role,
    /// Never empty. A `Tool` message holds `ToolResult` parts and, after them, text parts that
    /// the adapter sends as user text (results lowered to text).
    pub parts: Vec<LlmPart>,
    pub replay: Option<Value>,
}

/// A provider-neutral part of a request message.
#[derive(Clone, Debug, PartialEq)]
pub enum LlmPart {
    Text(String),
    Image {
        media_type: String,
        data: Bytes,
    },
    Pdf {
        name: String,
        data: Bytes,
    },
    Audio {
        /// `wav` or `mp3` (`input_audio` names).
        format: String,
        data: Bytes,
    },
    ToolCall {
        /// Copilot's id of the call.
        call_id: Uuid,
        /// The id to send: the provider's own id when the message is replayed, else
        /// `call_<call_id without dashes>`. Results of the call use the same id.
        provider_call_id: String,
        alias: String,
        /// Always a JSON object.
        arguments: Value,
    },
    ToolResult {
        call_id: Uuid,
        provider_call_id: String,
        alias: String,
        is_error: bool,
        /// `Text` and `Image` only.
        content: Vec<ToolContent>,
    },
}

/// The audio a request asks for (`modalities: ["text", "audio"]`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioOut {
    pub voice: String,
    /// `pcm16`.
    pub format: String,
}

/// What a streamed part is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PartKind {
    Text,
    Reasoning,
    ToolCall,
    Audio,
}

/// A refusal reported by the provider (Anthropic `stop_reason: refusal`, OpenAI `refusal`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub category: Option<String>,
    /// Shown to the user instead of the partial answer.
    pub explanation: String,
}

/// One event of a streamed answer. `index` is the adapter's part index within the answer;
/// a delta for an index that was not started starts that part.
#[derive(Clone, Debug, PartialEq)]
pub enum LlmEvent {
    PartStarted {
        index: u32,
        kind: PartKind,
    },
    TextDelta {
        index: u32,
        text: String,
    },
    ReasoningDelta {
        index: u32,
        text: String,
    },
    AudioDelta {
        data: Bytes,
        transcript: Option<String>,
    },
    ToolCallStarted {
        index: u32,
        provider_call_id: Option<String>,
        alias: String,
    },
    /// A piece of the arguments JSON, in order.
    ToolCallArgs {
        index: u32,
        json: String,
    },
    ToolCallEnded {
        index: u32,
    },
    /// Provider-opaque output to store as `provider_state`; the last one wins.
    ReplayState(Value),
    /// Token counts; values that are reported replace earlier ones.
    Usage(Usage),
    Finished {
        reason: FinishReason,
        refusal: Option<Refusal>,
    },
}
