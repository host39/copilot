//! The OpenAI-compatible format: `GET /v1/models` and `POST /v1/chat/completions`.
//!
//! - **Key:** `Authorization: Bearer <key>`, else `401` with OpenAI's `invalid_api_key` body.
//! - **Models** list OpenRouter's metadata fields (`context_length`, `architecture`
//!   modalities, `supported_parameters`), which Copilot reads for every preset.
//! - **Requests** need a listed `model` (`404 model_not_found` otherwise) and `messages`, and
//!   answer every call right away: the `tool` messages after an assistant message with
//!   `tool_calls` answer each of its calls, and a `tool` message answers a call of the assistant
//!   message before its run (`400` otherwise, naming the ids that got no answer).
//! - **Streams** are data-only SSE chunks: `reasoning_content`, `content`, `tool_calls` with
//!   argument fragments, and `audio` deltas, then the finish reason, a usage chunk when
//!   `stream_options.include_usage` asks for one, and `[DONE]`. `midstream error` ends with an
//!   `error` chunk instead.
//! - `fake-reasoner` reasons unless `reasoning_effort` (or `reasoning.effort`) is `none`;
//!   `fake-audio` speaks only when `modalities` holds `audio`.

use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};

use crate::http::{now, pieces, stream, with_id};
use crate::reply::{Block, Ending, Plan, Reply, Turn, chunks, newest, plan, wav};
use crate::{AppState, CONTEXT_WINDOW, MAX_OUTPUT_TOKENS, MODELS, model};

/// The header that carries a request id.
const REQUEST_ID: &str = "x-request-id";
/// Seconds a `429` asks the client to wait.
pub const RETRY_AFTER_SECS: u64 = 2;
/// Audio deltas per spoken reply.
const AUDIO_PIECES: usize = 10;

/// `GET /v1/models`.
pub(crate) fn models(state: &AppState, headers: &HeaderMap) -> Response {
    let id = format!("req_fake_{}", state.next());
    if let Some(response) = refusal(state, headers) {
        return with_id(response, REQUEST_ID, &id);
    }
    tracing::info!("fake-llm.openai.models-listed");
    let created = now();
    let data: Vec<Value> = MODELS
        .iter()
        .map(|model| {
            let mut input = vec!["text", "image", "file"];
            let mut output = vec!["text"];
            let mut parameters = vec!["tools", "max_tokens"];
            if model.audio {
                input = vec!["text", "audio"];
                output.push("audio");
            }
            if model.reasoning {
                parameters.push("reasoning");
            }
            json!({
                "id": model.id,
                "object": "model",
                "created": created,
                "owned_by": "fake-llm",
                "name": model.name,
                "context_length": CONTEXT_WINDOW,
                "max_completion_tokens": MAX_OUTPUT_TOKENS,
                "architecture": { "input_modalities": input, "output_modalities": output },
                "supported_parameters": parameters,
            })
        })
        .collect();
    let body = json!({ "object": "list", "data": data });
    with_id(axum::Json(body).into_response(), REQUEST_ID, &id)
}

/// `POST /v1/chat/completions`.
pub async fn complete(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let sequence = state.next();
    let response = answer(&state, &headers, &body, sequence);
    with_id(response, REQUEST_ID, &format!("req_fake_{sequence}"))
}

fn answer(state: &AppState, headers: &HeaderMap, body: &[u8], sequence: u64) -> Response {
    if let Some(response) = refusal(state, headers) {
        return response;
    }
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "We could not parse the JSON body of your request.",
            "invalid_request_error",
            None,
        );
    };
    let name = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if model(name).is_none() {
        return error(
            StatusCode::NOT_FOUND,
            &format!("The model `{name}` does not exist or you do not have access to it."),
            "invalid_request_error",
            Some("model_not_found"),
        );
    }
    let turn = match turn(&request) {
        Ok(turn) => turn,
        Err(message) => {
            tracing::warn!(model = name, reason = %message, "fake-llm.openai.invalid");
            return error(
                StatusCode::BAD_REQUEST,
                &message,
                "invalid_request_error",
                None,
            );
        }
    };
    let streamed = request.get("stream").and_then(Value::as_bool) == Some(true);
    let format = request
        .pointer("/audio/format")
        .and_then(Value::as_str)
        .unwrap_or("pcm16");
    match plan(&turn, sequence) {
        Plan::Fail(status) => {
            tracing::info!(model = name, status, "fake-llm.openai.failed");
            failure(status)
        }
        Plan::Answer { reply, slow } => {
            tracing::info!(model = name, streamed, ending = ?reply.ending, "fake-llm.openai.answered");
            let id = format!("chatcmpl-fake-{sequence}");
            if streamed {
                let delay = state.config.delay(slow);
                let usage = request
                    .pointer("/stream_options/include_usage")
                    .and_then(Value::as_bool)
                    == Some(true);
                stream(events(&reply, &id, name, usage), delay)
            } else {
                document(&reply, &id, name, format)
            }
        }
    }
}

/// The `401` for a request without the bearer key; `None` lets it through.
pub(crate) fn refusal(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let expected = format!("Bearer {}", state.config.key);
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    (given != Some(expected.as_str())).then(|| {
        tracing::warn!("fake-llm.openai.key-rejected");
        error(
            StatusCode::UNAUTHORIZED,
            "Incorrect API key provided. The fake server takes the key set in FAKE_LLM_KEY.",
            "invalid_request_error",
            Some("invalid_api_key"),
        )
    })
}

/// Reads the request the same way for every reply.
fn turn(request: &Value) -> Result<Turn, String> {
    let Some(messages) = request.get("messages").and_then(Value::as_array) else {
        return Err("Missing required parameter: 'messages'.".to_owned());
    };
    if messages.is_empty() {
        return Err(
            "Invalid 'messages': empty array. Expected an array with minimum length 1.".to_owned(),
        );
    }
    let mut input_chars = 0;
    for message in messages {
        input_chars += texts(message.get("content"))
            .iter()
            .map(String::len)
            .sum::<usize>();
    }
    answered(messages)?;
    let mut turn = Turn {
        model: request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_chars,
        ..Turn::default()
    };
    let trailing: Vec<&Value> = messages
        .iter()
        .rev()
        .take_while(|message| role(message) == "tool")
        .collect();
    if trailing.is_empty() {
        if let Some(user) = messages
            .iter()
            .rev()
            .find(|message| role(message) == "user")
        {
            (turn.text, turn.attachments) = match user.get("content") {
                Some(Value::String(text)) => newest([Some(text.as_str())]),
                Some(Value::Array(parts)) => newest(parts.iter().map(|part| {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        Some(part.get("text").and_then(Value::as_str).unwrap_or_default())
                    } else {
                        None
                    }
                })),
                _ => (String::new(), 0),
            };
        }
    } else {
        turn.results = Some(
            trailing
                .iter()
                .rev()
                .map(|message| texts(message.get("content")).join("\n"))
                .collect(),
        );
    }
    turn.tools = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    let effort = request
        .get("reasoning_effort")
        .or_else(|| request.pointer("/reasoning/effort"))
        .and_then(Value::as_str);
    turn.reasoning = effort != Some("none");
    turn.audio = request
        .get("modalities")
        .and_then(Value::as_array)
        .is_some_and(|modalities| modalities.iter().any(|modality| modality == "audio"));
    Ok(turn)
}

/// Checks that every call gets its answer right away, as the real API does: the `tool`
/// messages after an assistant message with `tool_calls` answer each of its calls, and a `tool`
/// message only answers a call of the assistant message just before its run of `tool` messages.
fn answered(messages: &[Value]) -> Result<(), String> {
    // The calls of the assistant message just before the current run of `tool` messages, and
    // the ones that have no answer yet (in call order).
    let mut calls: HashSet<&str> = HashSet::new();
    let mut waiting: Vec<&str> = Vec::new();
    for message in messages {
        if role(message) == "tool" {
            let id = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if calls.is_empty() {
                return Err("Invalid parameter: messages with role 'tool' must be a response to a preceeding message with 'tool_calls'.".to_owned());
            }
            if !calls.contains(id) {
                return Err(format!(
                    "Invalid parameter: 'tool_call_id' of '{id}' not found in 'tool_calls' of previous message."
                ));
            }
            waiting.retain(|call| *call != id);
            continue;
        }
        unanswered(&waiting)?;
        waiting = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|_| role(message) == "assistant")
            .filter_map(|call| call.get("id").and_then(Value::as_str))
            .collect();
        calls = waiting.iter().copied().collect();
    }
    unanswered(&waiting)
}

/// OpenAI's `400` for calls that no `tool` message answered.
fn unanswered(waiting: &[&str]) -> Result<(), String> {
    if waiting.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "An assistant message with 'tool_calls' must be followed by tool messages responding to each 'tool_call_id'. The following tool_call_ids did not have response messages: {}",
            waiting.join(", ")
        ))
    }
}

fn role(message: &Value) -> &str {
    message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The text of a `content` field: a string, or the text of its text parts.
fn texts(content: Option<&Value>) -> Vec<String> {
    match content {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// The streamed chunks of `reply`, with a usage chunk before `[DONE]` only when the request
/// asked for one (`stream_options.include_usage`), as OpenAI and vLLM do.
fn events(reply: &Reply, id: &str, model: &str, usage: bool) -> Vec<Event> {
    let created = now();
    let chunk = |delta: Value, finish: Option<&str>| {
        let body = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{ "index": 0, "delta": delta, "logprobs": null, "finish_reason": finish }],
        });
        Event::default().data(body.to_string())
    };
    let mut out = vec![chunk(json!({ "role": "assistant", "content": "" }), None)];
    for block in &reply.blocks {
        match block {
            Block::Reasoning(text) => {
                for piece in chunks(text) {
                    out.push(chunk(json!({ "reasoning_content": piece }), None));
                }
            }
            Block::Text(text) => {
                for piece in chunks(text) {
                    out.push(chunk(json!({ "content": piece }), None));
                }
            }
            Block::Call {
                id,
                name,
                arguments,
            } => {
                let start = json!({
                    "index": 0,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": "" },
                });
                out.push(chunk(json!({ "tool_calls": [start] }), None));
                for piece in pieces(arguments, 8) {
                    let fragment = json!({ "index": 0, "function": { "arguments": piece } });
                    out.push(chunk(json!({ "tool_calls": [fragment] }), None));
                }
            }
            Block::Audio { pcm, transcript } => {
                let words = chunks(transcript);
                let size = pcm.len().div_ceil(AUDIO_PIECES).next_multiple_of(2).max(2);
                for (index, data) in pcm.chunks(size).enumerate() {
                    let mut audio = Map::new();
                    if index == 0 {
                        audio.insert("id".to_owned(), json!(format!("audio_{id}")));
                    }
                    audio.insert("data".to_owned(), json!(STANDARD.encode(data)));
                    if let Some(word) = words.get(index) {
                        audio.insert("transcript".to_owned(), json!(word));
                    }
                    out.push(chunk(json!({ "audio": audio }), None));
                }
                for word in words.iter().skip(pcm.chunks(size).count()) {
                    out.push(chunk(json!({ "audio": { "transcript": word } }), None));
                }
                out.push(chunk(
                    json!({ "audio": { "expires_at": created + 3_600 } }),
                    None,
                ));
            }
        }
    }
    let finish = match reply.ending {
        Ending::Stop => "stop",
        Ending::ToolUse => "tool_calls",
        Ending::Refusal => "content_filter",
        Ending::Error => {
            let body = json!({
                "error": {
                    "message": "The fake model broke off mid-answer.",
                    "type": "server_error",
                    "param": null,
                    "code": "fake_midstream_error",
                },
            });
            out.push(Event::default().data(body.to_string()));
            return out;
        }
    };
    out.push(chunk(json!({}), Some(finish)));
    if usage {
        let body = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [],
            "usage": counts(reply),
        });
        out.push(Event::default().data(body.to_string()));
    }
    out.push(Event::default().data("[DONE]"));
    out
}

/// The non-streamed `chat.completion` of `reply`; `format` is the requested audio format.
fn document(reply: &Reply, id: &str, model: &str, format: &str) -> Response {
    if reply.ending == Ending::Error {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The fake model broke off mid-answer.",
            "server_error",
            Some("fake_midstream_error"),
        );
    }
    let created = now();
    let mut message = Map::new();
    message.insert("role".to_owned(), json!("assistant"));
    let text: String = reply
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    message.insert(
        "content".to_owned(),
        if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    message.insert("refusal".to_owned(), Value::Null);
    let mut calls = Vec::new();
    for block in &reply.blocks {
        match block {
            Block::Reasoning(text) => {
                message.insert("reasoning_content".to_owned(), json!(text));
            }
            Block::Call {
                id,
                name,
                arguments,
            } => calls.push(json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": arguments },
            })),
            Block::Audio { pcm, transcript } => {
                let data = if format == "wav" {
                    wav(pcm)
                } else {
                    pcm.clone()
                };
                message.insert(
                    "audio".to_owned(),
                    json!({
                        "id": format!("audio_{id}"),
                        "data": STANDARD.encode(data),
                        "transcript": transcript,
                        "expires_at": created + 3_600,
                    }),
                );
            }
            Block::Text(_) => {}
        }
    }
    if !calls.is_empty() {
        message.insert("tool_calls".to_owned(), Value::Array(calls));
    }
    let finish = match reply.ending {
        Ending::ToolUse => "tool_calls",
        Ending::Refusal => "content_filter",
        Ending::Stop | Ending::Error => "stop",
    };
    axum::Json(json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{ "index": 0, "message": message, "logprobs": null, "finish_reason": finish }],
        "usage": counts(reply),
    }))
    .into_response()
}

/// The `usage` object of `reply`.
fn counts(reply: &Reply) -> Value {
    let reasoning: usize = reply
        .blocks
        .iter()
        .map(|block| match block {
            Block::Reasoning(text) => text.len().div_ceil(4),
            _ => 0,
        })
        .sum();
    json!({
        "prompt_tokens": reply.input_tokens,
        "completion_tokens": reply.output_tokens,
        "total_tokens": reply.input_tokens.saturating_add(reply.output_tokens),
        "completion_tokens_details": { "reasoning_tokens": reasoning },
    })
}

/// The answer to `failed <status>`: OpenAI's error body, and `Retry-After` on a `429`.
pub(crate) fn failure(status: u16) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let message = format!("The fake model was asked to fail with HTTP {status}.");
    let (kind, reason) = match status {
        401 => ("invalid_request_error", Some("invalid_api_key")),
        403 => ("invalid_request_error", Some("permission_denied")),
        429 => ("requests", Some("rate_limit_exceeded")),
        500..=599 => ("server_error", None),
        _ => ("invalid_request_error", None),
    };
    let mut response = error(code, &message, kind, reason);
    if status == 429 {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, RETRY_AFTER_SECS.into());
    }
    response
}

/// OpenAI's error body.
pub(crate) fn error(status: StatusCode, message: &str, kind: &str, code: Option<&str>) -> Response {
    let body = json!({
        "error": { "message": message, "type": kind, "param": null, "code": code },
    });
    (status, axum::Json(body)).into_response()
}
