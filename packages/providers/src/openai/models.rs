//! The model list (`GET {base}/models`) and what the provider says about each model.
//!
//! Chat Completions servers list `{data: [{id, ...}]}` (Together lists a bare array). Most add
//! nothing beyond the id, so core applies the preset defaults (`Capabilities::defaults`). What a
//! server does report is passed on, read leniently from fields the known servers use:
//!
//! | Field | Servers | Reported as |
//! | --- | --- | --- |
//! | `name`, `display_name` | OpenRouter, Together | display name |
//! | `context_length`, `context_window`, `max_model_len`, `top_provider.context_length` | OpenRouter and Together, Groq, vLLM, OpenRouter | context window |
//! | `max_completion_tokens`, `top_provider.max_completion_tokens` | Groq, OpenRouter | max output tokens |
//! | `architecture.input_modalities` (`image`, `audio`, `file`) | OpenRouter | `vision`, `audio_in`, `pdf` (only when listed: OpenRouter reads PDFs for any model) |
//! | `architecture.output_modalities` (`audio`) | OpenRouter | `audio_out` |
//! | `supported_parameters` (`tools`, `reasoning`) | OpenRouter | `tools`, `reasoning` |
//!
//! The `gemini` preset lists ids as `models/<id>`; the prefix is dropped, and Google's native
//! model list adds each model's limits (`crate::openai::gemini`). On the `ollama` preset each
//! model's `POST /api/show` (next to `/v1`) adds its `capabilities` (`vision`, `tools`,
//! `thinking`, `audio`) and context length, for lists of up to [`SHOW_MAX_MODELS`] models, four
//! at a time; a model whose details fail keeps the defaults.

use copilot_core::{DomainResult, Outbound, Preset, ProviderTarget, RemoteModel};
use futures_util::stream::{self, StreamExt};
use reqwest::Client;
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};

use crate::http::{bad_response, bearer, body, client, send};
use crate::openai::gemini::add_native;

/// Ollama lists at most this many models before the adapter stops asking for each one's details.
pub(crate) const SHOW_MAX_MODELS: usize = 64;

/// Concurrent `POST /api/show` requests.
const SHOW_CONCURRENCY: usize = 4;

/// The connection's models with what the provider reports about them.
pub(crate) async fn list(
    clients: &Outbound<Client>,
    target: &ProviderTarget,
) -> DomainResult<Vec<RemoteModel>> {
    let client = client(clients, target)?;
    let mut request = client.get(format!("{}/models", target.base_url.as_str()));
    if let Some(value) = bearer(target)? {
        request = request.header(AUTHORIZATION, value);
    }
    let response = send(target, request).await?;
    let bytes = body(target, response).await?;
    let models = parse(target.preset, &bytes).ok_or_else(|| {
        tracing::warn!(provider_id = %target.provider_id, "provider.models.invalid");
        bad_response()
    })?;
    Ok(match target.preset {
        Preset::Ollama => show_all(client, target, models).await,
        Preset::Gemini => add_native(client, target, models).await,
        _ => models,
    })
}

/// The listed models, or `None` when the body is not a model list.
pub(crate) fn parse(preset: Preset, bytes: &[u8]) -> Option<Vec<RemoteModel>> {
    let list: Value = serde_json::from_slice(bytes).ok()?;
    let items = match list {
        Value::Array(items) => items,
        Value::Object(mut list) => match list.remove("data")? {
            Value::Array(items) => items,
            _ => return None,
        },
        _ => return None,
    };
    Some(
        items
            .iter()
            .filter_map(|item| model(preset, item))
            .collect(),
    )
}

/// One listed model; entries without a string id are skipped.
fn model(preset: Preset, item: &Value) -> Option<RemoteModel> {
    let listed = item.get("id")?.as_str()?;
    let short = listed.strip_prefix("models/").unwrap_or(listed);
    let remote_id = if preset == Preset::Gemini {
        short
    } else {
        listed
    };
    let display_name = ["name", "display_name"]
        .into_iter()
        .find_map(|field| item.get(field).and_then(Value::as_str))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(short);
    let top = item.get("top_provider");
    let mut model = RemoteModel {
        remote_id: remote_id.to_owned(),
        display_name: display_name.to_owned(),
        context_window: count(item, &["context_length", "context_window", "max_model_len"])
            .or_else(|| top.and_then(|top| count(top, &["context_length"]))),
        max_output_tokens: count(item, &["max_completion_tokens"])
            .or_else(|| top.and_then(|top| count(top, &["max_completion_tokens"]))),
        ..RemoteModel::default()
    };
    let reported = &mut model.reported;
    if let Some(architecture) = item.get("architecture") {
        if let Some(input) = names(architecture.get("input_modalities")) {
            reported.vision = Some(input.contains(&"image"));
            reported.audio_in = Some(input.contains(&"audio"));
            reported.pdf = input.contains(&"file").then_some(true);
        }
        if let Some(output) = names(architecture.get("output_modalities")) {
            reported.audio_out = Some(output.contains(&"audio"));
        }
    }
    if let Some(parameters) = names(item.get("supported_parameters")) {
        reported.tools = Some(parameters.contains(&"tools"));
        reported.reasoning =
            Some(parameters.contains(&"reasoning") || parameters.contains(&"include_reasoning"));
    }
    Some(model)
}

/// The first of `fields` that holds a token count that fits a `u32`.
pub(crate) fn count(item: &Value, fields: &[&str]) -> Option<u32> {
    fields
        .iter()
        .find_map(|field| item.get(*field).and_then(Value::as_u64))
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

/// The strings of a string array.
fn names(value: Option<&Value>) -> Option<Vec<&str>> {
    Some(
        value?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect(),
    )
}

/// Ollama's native API root: the base URL without its `/v1`, or `None` for another path.
fn ollama_root(target: &ProviderTarget) -> Option<String> {
    target
        .base_url
        .as_str()
        .strip_suffix("/v1")
        .map(str::to_owned)
}

/// Adds each model's `POST /api/show` details, when the list is small enough to ask.
async fn show_all(
    client: &Client,
    target: &ProviderTarget,
    models: Vec<RemoteModel>,
) -> Vec<RemoteModel> {
    let Some(root) = ollama_root(target) else {
        return models;
    };
    if models.len() > SHOW_MAX_MODELS {
        return models;
    }
    stream::iter(models)
        .map(|model| show(client, target, &root, model))
        .buffered(SHOW_CONCURRENCY)
        .collect()
        .await
}

/// One model's details; a failure keeps the model as listed.
async fn show(
    client: &Client,
    target: &ProviderTarget,
    root: &str,
    mut model: RemoteModel,
) -> RemoteModel {
    let details = async {
        let mut request = client
            .post(format!("{root}/api/show"))
            .json(&json!({ "model": model.remote_id }));
        if let Some(value) = bearer(target)? {
            request = request.header(AUTHORIZATION, value);
        }
        let response = send(target, request).await?;
        let bytes = body(target, response).await?;
        serde_json::from_slice::<Value>(&bytes).map_err(|_| bad_response())
    }
    .await;
    match details {
        Ok(details) => apply_show(&mut model, &details),
        Err(error) => tracing::warn!(
            provider_id = %target.provider_id,
            code = error.code(),
            "provider.models.details-failed"
        ),
    }
    model
}

/// Ollama's `capabilities` and `model_info.<architecture>.context_length`.
pub(crate) fn apply_show(model: &mut RemoteModel, details: &Value) {
    if let Some(capabilities) = names(details.get("capabilities")) {
        let reported = &mut model.reported;
        reported.vision = Some(capabilities.contains(&"vision"));
        reported.tools = Some(capabilities.contains(&"tools"));
        reported.reasoning = Some(capabilities.contains(&"thinking"));
        reported.audio_in = Some(capabilities.contains(&"audio"));
    }
    let context = details
        .get("model_info")
        .and_then(Value::as_object)
        .and_then(|info| {
            info.iter()
                .find(|(key, _)| key.ends_with(".context_length"))
                .and_then(|(_, value)| value.as_u64())
        })
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0);
    if context.is_some() {
        model.context_window = context;
    }
}

#[cfg(test)]
mod tests {
    use copilot_core::ReportedCapabilities;

    use super::*;

    fn parsed(preset: Preset, list: &Value) -> Vec<RemoteModel> {
        parse(preset, list.to_string().as_bytes()).expect("a model list")
    }

    #[test]
    fn plain_lists_report_ids_only() {
        let models = parsed(
            Preset::OpenAi,
            &json!({ "object": "list", "data": [
                { "id": "gpt-5.6-terra", "object": "model", "created": 1, "owned_by": "openai" },
                { "object": "model" },
                { "id": 42 },
            ] }),
        );
        assert_eq!(
            models,
            [RemoteModel {
                remote_id: "gpt-5.6-terra".to_owned(),
                display_name: "gpt-5.6-terra".to_owned(),
                ..RemoteModel::default()
            }],
            "entries without a string id are skipped"
        );
    }

    #[test]
    fn reads_openrouter_metadata() {
        let models = parsed(
            Preset::OpenRouter,
            &json!({ "data": [
                {
                    "id": "openai/gpt-audio",
                    "name": "OpenAI: GPT Audio",
                    "context_length": 128_000,
                    "architecture": {
                        "input_modalities": ["text", "image", "audio", "file"],
                        "output_modalities": ["text", "audio"],
                    },
                    "top_provider": { "context_length": 128_000, "max_completion_tokens": 16_384 },
                    "supported_parameters": ["tools", "tool_choice", "reasoning", "max_tokens"],
                },
                {
                    "id": "meta/llama-text",
                    "name": "Llama Text",
                    "architecture": { "input_modalities": ["text"], "output_modalities": ["text"] },
                    "top_provider": { "context_length": 8_192, "max_completion_tokens": null },
                    "supported_parameters": ["max_tokens"],
                },
            ] }),
        );
        assert_eq!(models[0].display_name, "OpenAI: GPT Audio");
        assert_eq!(models[0].context_window, Some(128_000));
        assert_eq!(models[0].max_output_tokens, Some(16_384));
        assert_eq!(
            models[0].reported,
            ReportedCapabilities {
                vision: Some(true),
                pdf: Some(true),
                tools: Some(true),
                audio_in: Some(true),
                audio_out: Some(true),
                reasoning: Some(true),
                ..ReportedCapabilities::default()
            }
        );
        assert_eq!(models[1].context_window, Some(8_192));
        assert_eq!(models[1].max_output_tokens, None);
        assert_eq!(
            models[1].reported,
            ReportedCapabilities {
                vision: Some(false),
                pdf: None,
                tools: Some(false),
                audio_in: Some(false),
                audio_out: Some(false),
                reasoning: Some(false),
                ..ReportedCapabilities::default()
            },
            "a model without `file` input leaves PDF to the preset default"
        );
    }

    #[test]
    fn reads_context_windows_from_groq_vllm_and_together() {
        let groq = parsed(
            Preset::Custom,
            &json!({ "data": [{ "id": "llama-3.3-70b-versatile", "context_window": 131_072, "max_completion_tokens": 32_768 }] }),
        );
        assert_eq!(groq[0].context_window, Some(131_072));
        assert_eq!(groq[0].max_output_tokens, Some(32_768));

        let vllm = parsed(
            Preset::Custom,
            &json!({ "data": [{ "id": "Qwen/Qwen3-8B", "max_model_len": 32_768 }] }),
        );
        assert_eq!(vllm[0].context_window, Some(32_768));

        let together = parsed(
            Preset::Custom,
            &json!([{ "id": "meta-llama/Llama-3-8b-chat-hf", "display_name": "Llama 3 8B", "context_length": 8_192, "type": "chat" }]),
        );
        assert_eq!(together[0].display_name, "Llama 3 8B");
        assert_eq!(together[0].context_window, Some(8_192));

        let odd = parsed(
            Preset::Custom,
            &json!({ "data": [{ "id": "m", "context_length": 99_999_999_999_u64, "max_completion_tokens": 0, "name": " " }] }),
        );
        assert_eq!(odd[0].context_window, None, "out of range");
        assert_eq!(odd[0].max_output_tokens, None, "zero is no limit");
        assert_eq!(
            odd[0].display_name, "m",
            "a blank name falls back to the id"
        );
    }

    #[test]
    fn gemini_ids_drop_their_models_prefix() {
        let models = parsed(
            Preset::Gemini,
            &json!({ "object": "list", "data": [{ "id": "models/gemini-4-hydrogen", "object": "model", "owned_by": "google" }] }),
        );
        assert_eq!(models[0].remote_id, "gemini-4-hydrogen");
        assert_eq!(models[0].display_name, "gemini-4-hydrogen");

        let custom = parsed(
            Preset::Custom,
            &json!({ "data": [{ "id": "models/gemini-4-hydrogen" }] }),
        );
        assert_eq!(custom[0].remote_id, "models/gemini-4-hydrogen");
        assert_eq!(custom[0].display_name, "gemini-4-hydrogen");
    }

    #[test]
    fn refuses_bodies_that_are_not_lists() {
        for body in ["<html>", "{}", r#"{"data":{}}"#, "42"] {
            assert!(parse(Preset::Custom, body.as_bytes()).is_none(), "{body}");
        }
        assert_eq!(parse(Preset::Custom, b"[]"), Some(Vec::new()));
    }

    #[test]
    fn ollama_details_add_capabilities_and_context() {
        let mut model = RemoteModel {
            remote_id: "gemma4".to_owned(),
            display_name: "gemma4".to_owned(),
            ..RemoteModel::default()
        };
        apply_show(
            &mut model,
            &json!({
                "capabilities": ["completion", "vision", "tools", "thinking"],
                "model_info": { "general.architecture": "gemma4", "gemma4.context_length": 131_072 },
            }),
        );
        assert_eq!(model.context_window, Some(131_072));
        assert_eq!(
            model.reported,
            ReportedCapabilities {
                vision: Some(true),
                tools: Some(true),
                audio_in: Some(false),
                reasoning: Some(true),
                ..ReportedCapabilities::default()
            }
        );

        let mut bare = model.clone();
        apply_show(&mut bare, &json!({ "license": "..." }));
        assert_eq!(bare, model, "details without capabilities change nothing");
    }
}
