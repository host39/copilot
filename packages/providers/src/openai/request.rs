//! Chat Completions request bodies (`POST {base}/chat/completions`, streamed).
//!
//! The walking skeleton sends text only. Parts and features that need the full adapter (images,
//! PDFs, audio, tool calls and results, tools, spoken replies) fail with `PROVIDER_UNSUPPORTED`
//! before anything is sent, so a request never goes out with something silently dropped.

use copilot_core::{
    DomainError, DomainResult, LlmMessage, LlmPart, LlmRequest, ModelTarget, ReasoningLevel, Role,
};
use serde_json::{Map, Value, json};

use crate::gateway::UNSUPPORTED;

/// The request body for `request` in the target's dialect: the system text under the dialect's
/// role, the output limit under its field, and `stream_options` where the dialect reports usage
/// that way.
pub(crate) fn body(target: &ModelTarget, request: &LlmRequest) -> DomainResult<Value> {
    if !request.tools.is_empty() {
        return Err(unsupported("tools"));
    }
    if request.audio_out.is_some() {
        return Err(unsupported("spoken replies"));
    }
    let dialect = target.dialect;
    let mut messages = Vec::with_capacity(request.messages.len() + 1);
    if !request.system.is_empty() {
        let role = dialect.system_role().unwrap_or("system");
        messages.push(json!({ "role": role, "content": request.system }));
    }
    for item in &request.messages {
        messages.push(message(item)?);
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), Value::from(request.model.as_str()));
    body.insert("stream".to_owned(), Value::Bool(true));
    body.insert("messages".to_owned(), Value::Array(messages));
    body.insert(
        dialect.token_field().to_owned(),
        Value::from(request.max_output_tokens),
    );
    if dialect.stream_usage() {
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }
    if let Some(level) = request.reasoning {
        body.insert("reasoning_effort".to_owned(), Value::from(effort(level)));
    }
    Ok(Value::Object(body))
}

/// One message: a user message with one text part sends a string, with several an array of
/// `text` parts; an assistant message sends its text joined.
fn message(message: &LlmMessage) -> DomainResult<Value> {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => return Err(unsupported("tool results")),
    };
    let mut texts = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        match part {
            LlmPart::Text(text) => texts.push(text.as_str()),
            LlmPart::Image { .. } => return Err(unsupported("images")),
            LlmPart::Pdf { .. } => return Err(unsupported("PDF files")),
            LlmPart::Audio { .. } => return Err(unsupported("audio")),
            LlmPart::ToolCall { .. } => return Err(unsupported("tool calls")),
            LlmPart::ToolResult { .. } => return Err(unsupported("tool results")),
        }
    }
    let content = match (message.role, texts.as_slice()) {
        (Role::User, [text]) => Value::from(*text),
        (Role::User, _) => Value::Array(
            texts
                .iter()
                .map(|text| json!({ "type": "text", "text": text }))
                .collect(),
        ),
        _ => Value::from(texts.concat()),
    };
    Ok(json!({ "role": role, "content": content }))
}

/// `reasoning_effort`: `none` turns reasoning off.
fn effort(level: ReasoningLevel) -> &'static str {
    match level {
        ReasoningLevel::Off => "none",
        ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
    }
}

fn unsupported(what: &str) -> DomainError {
    DomainError::upstream(
        format!("This connection cannot send {what} yet."),
        UNSUPPORTED,
    )
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use copilot_core::{
        AudioOut, BaseUrl, Dialect, NetworkScope, Preset, ProviderId, ProviderKind, ProviderTarget,
    };

    use super::*;

    fn target(preset: Preset) -> ModelTarget {
        ModelTarget {
            provider: ProviderTarget {
                provider_id: ProviderId::generate(),
                kind: ProviderKind::OpenAi,
                preset,
                base_url: BaseUrl::parse("https://api.example.com/v1", NetworkScope::Public)
                    .expect("valid"),
                api_key: None,
                network: NetworkScope::Public,
            },
            remote_id: "model-1".to_owned(),
            dialect: Dialect::new(ProviderKind::OpenAi, preset),
        }
    }

    fn text(role: Role, parts: &[&str]) -> LlmMessage {
        LlmMessage {
            role,
            parts: parts
                .iter()
                .map(|text| LlmPart::Text((*text).to_owned()))
                .collect(),
            replay: None,
        }
    }

    fn request(messages: Vec<LlmMessage>) -> LlmRequest {
        LlmRequest {
            model: "model-1".to_owned(),
            system: "Be brief.".to_owned(),
            messages,
            tools: Vec::new(),
            max_output_tokens: 1_024,
            reasoning: None,
            audio_out: None,
            cache_breakpoints: false,
        }
    }

    #[test]
    fn follows_the_openai_dialect() {
        let body = body(
            &target(Preset::OpenAi),
            &request(vec![text(Role::User, &["Hi"])]),
        )
        .expect("text only");
        assert_eq!(
            body,
            json!({
                "model": "model-1",
                "stream": true,
                "messages": [
                    { "role": "developer", "content": "Be brief." },
                    { "role": "user", "content": "Hi" },
                ],
                "max_completion_tokens": 1_024,
                "stream_options": { "include_usage": true },
            })
        );
    }

    #[test]
    fn other_presets_use_system_and_max_tokens() {
        for preset in [
            Preset::Custom,
            Preset::Ollama,
            Preset::OpenRouter,
            Preset::Gemini,
        ] {
            let body = body(&target(preset), &request(vec![text(Role::User, &["Hi"])]))
                .expect("text only");
            assert_eq!(body["messages"][0]["role"], "system", "{preset:?}");
            assert_eq!(body["max_tokens"], 1_024, "{preset:?}");
            assert!(body.get("max_completion_tokens").is_none());
            assert!(body.get("stream_options").is_none());
        }
    }

    #[test]
    fn leaves_out_an_empty_system_text() {
        let mut request = request(vec![text(Role::User, &["Hi"])]);
        request.system.clear();
        let body = body(&target(Preset::Custom), &request).expect("text only");
        assert_eq!(body["messages"].as_array().expect("messages").len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn sends_several_user_parts_as_an_array_and_assistant_text_joined() {
        let body = body(
            &target(Preset::Custom),
            &request(vec![
                text(Role::User, &["<file name=\"a.txt\">a</file>", "Summarize."]),
                text(Role::Assistant, &["Sure", "."]),
                text(Role::User, &["Thanks"]),
            ]),
        )
        .expect("text only");
        assert_eq!(
            body["messages"][1]["content"],
            json!([
                { "type": "text", "text": "<file name=\"a.txt\">a</file>" },
                { "type": "text", "text": "Summarize." },
            ])
        );
        assert_eq!(
            body["messages"][2],
            json!({ "role": "assistant", "content": "Sure." })
        );
        assert_eq!(body["messages"][3]["content"], "Thanks");
    }

    #[test]
    fn maps_reasoning_levels_to_reasoning_effort() {
        for (level, effort) in [
            (ReasoningLevel::Off, "none"),
            (ReasoningLevel::Low, "low"),
            (ReasoningLevel::Medium, "medium"),
            (ReasoningLevel::High, "high"),
        ] {
            let mut request = request(vec![text(Role::User, &["Hi"])]);
            request.reasoning = Some(level);
            let body = body(&target(Preset::OpenAi), &request).expect("text only");
            assert_eq!(body["reasoning_effort"], effort);
        }
    }

    #[test]
    fn refuses_what_the_skeleton_cannot_send() {
        let image = LlmMessage {
            role: Role::User,
            parts: vec![LlmPart::Image {
                media_type: "image/png".to_owned(),
                data: Bytes::from_static(b"png"),
            }],
            replay: None,
        };
        let error = body(&target(Preset::OpenAi), &request(vec![image])).expect_err("image");
        assert_eq!(error.code(), UNSUPPORTED);
        assert_eq!(error.to_string(), "This connection cannot send images yet.");

        let tool = text(Role::Tool, &["result"]);
        let error = body(&target(Preset::OpenAi), &request(vec![tool])).expect_err("tool");
        assert_eq!(error.code(), UNSUPPORTED);

        let mut spoken = request(vec![text(Role::User, &["Hi"])]);
        spoken.audio_out = Some(AudioOut {
            voice: "alloy".to_owned(),
            format: "pcm16".to_owned(),
        });
        assert_eq!(
            body(&target(Preset::OpenAi), &spoken)
                .expect_err("audio out")
                .code(),
            UNSUPPORTED
        );
    }
}
