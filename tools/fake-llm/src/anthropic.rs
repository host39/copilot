//! The Anthropic format: `GET /v1/models` and `POST /v1/messages`.
//!
//! - **Key:** `x-api-key: <key>`, else `401 authentication_error`; every request also needs
//!   `anthropic-version` (`400` without it), as the real API does.
//! - **Models** report the Models API `capabilities` (images, PDFs, adaptive thinking and
//!   effort levels on `fake-reasoner`). Claude models have no audio, so neither do these.
//! - **Requests** need a listed `model` (`404`), `max_tokens`, and messages that end with a
//!   user message (no prefill). A replayed `thinking` block must carry the signature this server
//!   gave it, a `tool_result` must answer a `tool_use` of the message before it, and every
//!   `tool_use` must be answered by a `tool_result` at the start of the next message (`400`
//!   otherwise), so a client that gets any of them wrong while replaying fails here as it would
//!   there.
//! - **Streams** are named events: `message_start`, `ping`, per block `content_block_start`,
//!   deltas (`thinking_delta` then `signature_delta`, `text_delta`, `input_json_delta`), and
//!   `content_block_stop`, then `message_delta` with the stop reason (`refusal` with
//!   `stop_details`) and `message_stop`. `midstream error` ends with an `error` event instead.
//! - `fake-reasoner` thinks when `thinking.type` is `adaptive` or `enabled`.

use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::http::{pieces, stream, with_id};
use crate::reply::{Block, Ending, Plan, Reply, Turn, chunks, newest, plan, signature};
use crate::{AppState, CONTEXT_WINDOW, MAX_OUTPUT_TOKENS, MODELS, model};

/// The header that carries the key.
pub const API_KEY: &str = "x-api-key";
/// The header that names the API version.
pub const VERSION: &str = "anthropic-version";
/// The header that carries a request id.
const REQUEST_ID: &str = "request-id";
/// Seconds a `429` asks the client to wait.
pub const RETRY_AFTER_SECS: u64 = 2;
/// When every model was "created".
const CREATED_AT: &str = "2026-01-01T00:00:00Z";
/// The explanation a refusal carries.
pub const REFUSAL: &str = "The fake model was asked to refuse.";

/// `GET /v1/models` (any `limit` or `after_id`: every model fits one page).
pub(crate) fn models(state: &AppState, headers: &HeaderMap) -> Response {
    let id = format!("req_fake_{}", state.next());
    if let Some(response) = refusal(state, headers) {
        return with_id(response, REQUEST_ID, &id);
    }
    tracing::info!("fake-llm.anthropic.models-listed");
    let supported = |flag: bool| json!({ "supported": flag });
    let data: Vec<Value> = MODELS
        .iter()
        .map(|model| {
            let thinking = model.reasoning;
            json!({
                "type": "model",
                "id": model.id,
                "display_name": model.name,
                "created_at": CREATED_AT,
                "max_input_tokens": CONTEXT_WINDOW,
                "max_tokens": MAX_OUTPUT_TOKENS,
                "capabilities": {
                    "image_input": supported(true),
                    "pdf_input": supported(true),
                    "thinking": {
                        "supported": thinking,
                        "types": {
                            "adaptive": supported(thinking),
                            "enabled": supported(false),
                            "disabled": supported(true),
                        },
                    },
                    "effort": {
                        "supported": thinking,
                        "low": supported(thinking),
                        "medium": supported(thinking),
                        "high": supported(thinking),
                    },
                },
            })
        })
        .collect();
    let body = json!({
        "data": data,
        "has_more": false,
        "first_id": MODELS[0].id,
        "last_id": MODELS[MODELS.len() - 1].id,
    });
    with_id(axum::Json(body).into_response(), REQUEST_ID, &id)
}

/// `POST /v1/messages`.
pub async fn messages(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
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
            "invalid_request_error",
            "The request body is not valid JSON.",
        );
    };
    let name = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if model(name).is_none() {
        return error(
            StatusCode::NOT_FOUND,
            "not_found_error",
            &format!("model: {name}"),
        );
    }
    let turn = match turn(&request) {
        Ok(turn) => turn,
        Err(message) => {
            tracing::warn!(model = name, reason = %message, "fake-llm.anthropic.invalid");
            return error(StatusCode::BAD_REQUEST, "invalid_request_error", &message);
        }
    };
    let streamed = request.get("stream").and_then(Value::as_bool) == Some(true);
    match plan(&turn, sequence) {
        Plan::Fail(status) => {
            tracing::info!(model = name, status, "fake-llm.anthropic.failed");
            failure(status)
        }
        Plan::Answer { reply, slow } => {
            tracing::info!(model = name, streamed, ending = ?reply.ending, "fake-llm.anthropic.answered");
            let id = format!("msg_fake_{sequence}");
            if streamed {
                stream(events(&reply, &id, name), state.config.delay(slow))
            } else {
                document(&reply, &id, name)
            }
        }
    }
}

/// The `401` for a request without the key, or the `400` for one without a version; `None`
/// lets it through.
fn refusal(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let key = headers.get(API_KEY).and_then(|value| value.to_str().ok());
    if key != Some(state.config.key.as_str()) {
        tracing::warn!("fake-llm.anthropic.key-rejected");
        return Some(error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "invalid x-api-key",
        ));
    }
    (!headers.contains_key(VERSION)).then(|| {
        tracing::warn!("fake-llm.anthropic.version-missing");
        error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "anthropic-version: header is required",
        )
    })
}

/// Reads the request the same way for every reply, checking what the real API checks.
fn turn(request: &Value) -> Result<Turn, String> {
    if !request
        .get("max_tokens")
        .and_then(Value::as_u64)
        .is_some_and(|tokens| tokens > 0)
    {
        return Err("max_tokens: Field required".to_owned());
    }
    let Some(messages) = request.get("messages").and_then(Value::as_array) else {
        return Err("messages: Field required".to_owned());
    };
    let Some(last) = messages.last() else {
        return Err("messages: at least one message is required".to_owned());
    };
    if role(last) != "user" {
        return Err("This model does not support assistant message prefill. The conversation must end with a user message.".to_owned());
    }
    let mut input_chars = system_chars(request.get("system"));
    let mut previous: Vec<String> = Vec::new();
    for (position, message) in messages.iter().enumerate() {
        let blocks = blocks(message);
        answered(&previous, position, message, &blocks)?;
        let mut calls = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            input_chars += block_text(block).len();
            match kind(block) {
                "thinking" if role(message) == "assistant" => {
                    let text = block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let given = block.get("signature").and_then(Value::as_str);
                    if given != Some(signature(text).as_str()) {
                        return Err(format!(
                            "messages.{position}.content.{index}: Invalid `signature` in `thinking` block"
                        ));
                    }
                }
                "tool_use" => {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        calls.push(id.to_owned());
                    }
                }
                "tool_result" => {
                    let id = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !previous.iter().any(|call| call == id) {
                        return Err(format!(
                            "messages.{position}.content.{index}: unexpected `tool_use_id` found in `tool_result` blocks: {id}. Each `tool_result` block must have a corresponding `tool_use` block in the previous message."
                        ));
                    }
                }
                _ => {}
            }
        }
        previous = calls;
    }

    let mut turn = Turn {
        model: request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_chars,
        ..Turn::default()
    };
    let blocks = blocks(last);
    let after = blocks
        .iter()
        .rposition(|block| kind(block) == "tool_result")
        .map_or(0, |position| position + 1);
    if after > 0 && after == blocks.len() {
        turn.results = Some(
            blocks
                .iter()
                .filter(|block| kind(block) == "tool_result")
                .map(|block| result_text(block.get("content")))
                .collect(),
        );
    } else {
        let texts: Vec<Option<String>> = blocks[after..]
            .iter()
            .map(|block| (kind(block) == "text").then(|| block_text(block)))
            .collect();
        (turn.text, turn.attachments) = newest(texts.iter().map(Option::as_deref));
    }
    turn.tools = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    turn.reasoning = matches!(
        request.pointer("/thinking/type").and_then(Value::as_str),
        Some("adaptive" | "enabled")
    );
    Ok(turn)
}

/// Anthropic's check that the `tool_use` blocks of the message before `message` (`calls`) are
/// answered right away: `message` is a user message that starts with a `tool_result` block for
/// each of them.
fn answered(
    calls: &[String],
    position: usize,
    message: &Value,
    blocks: &[Value],
) -> Result<(), String> {
    let leading: HashSet<&str> = if role(message) == "user" {
        blocks
            .iter()
            .take_while(|block| kind(block) == "tool_result")
            .filter_map(|block| block.get("tool_use_id").and_then(Value::as_str))
            .collect()
    } else {
        HashSet::new()
    };
    let missing: Vec<&str> = calls
        .iter()
        .map(String::as_str)
        .filter(|id| !leading.contains(id))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "messages.{}: `tool_use` ids were found without `tool_result` blocks immediately after: {}. Each `tool_use` block must have a corresponding `tool_result` block in the next message.",
        position.saturating_sub(1),
        missing.join(", ")
    ))
}

fn role(message: &Value) -> &str {
    message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn kind(block: &Value) -> &str {
    block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// A message's content as blocks (a string is one text block).
fn blocks(message: &Value) -> Vec<Value> {
    match message.get("content") {
        Some(Value::String(text)) => vec![json!({ "type": "text", "text": text })],
        Some(Value::Array(blocks)) => blocks.clone(),
        _ => Vec::new(),
    }
}

/// The readable text of a block (text, thinking, or a tool result's text).
fn block_text(block: &Value) -> String {
    match kind(block) {
        "text" => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        "thinking" => block
            .get("thinking")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        "tool_result" => result_text(block.get("content")),
        _ => String::new(),
    }
}

/// A `tool_result`'s content: a string, or its text blocks joined by line breaks.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| kind(block) == "text")
            .map(block_text)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Characters of the system prompt (a string or text blocks).
fn system_chars(system: Option<&Value>) -> usize {
    match system {
        Some(Value::String(text)) => text.len(),
        Some(Value::Array(blocks)) => blocks.iter().map(|block| block_text(block).len()).sum(),
        _ => 0,
    }
}

/// The call id as Anthropic shapes it.
fn call_id(id: &str) -> String {
    id.replacen("call_", "toolu_", 1)
}

fn stop_reason(ending: Ending) -> &'static str {
    match ending {
        Ending::Stop | Ending::Error => "end_turn",
        Ending::ToolUse => "tool_use",
        Ending::Refusal => "refusal",
    }
}

fn stop_details(ending: Ending) -> Value {
    if ending == Ending::Refusal {
        json!({ "type": "refusal", "category": null, "explanation": REFUSAL })
    } else {
        Value::Null
    }
}

/// The streamed events of `reply`.
fn events(reply: &Reply, id: &str, model: &str) -> Vec<Event> {
    let event = |name: &str, body: Value| Event::default().event(name).data(body.to_string());
    let mut out = vec![
        event(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": reply.input_tokens,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "output_tokens": 1,
                    },
                },
            }),
        ),
        event("ping", json!({ "type": "ping" })),
    ];
    for (index, block) in reply.blocks.iter().enumerate() {
        let start = |content: Value| {
            event(
                "content_block_start",
                json!({ "type": "content_block_start", "index": index, "content_block": content }),
            )
        };
        let delta = |delta: Value| {
            event(
                "content_block_delta",
                json!({ "type": "content_block_delta", "index": index, "delta": delta }),
            )
        };
        match block {
            Block::Reasoning(text) => {
                out.push(start(
                    json!({ "type": "thinking", "thinking": "", "signature": "" }),
                ));
                for piece in chunks(text) {
                    out.push(delta(
                        json!({ "type": "thinking_delta", "thinking": piece }),
                    ));
                }
                out.push(delta(
                    json!({ "type": "signature_delta", "signature": signature(text) }),
                ));
            }
            Block::Text(text)
            | Block::Audio {
                transcript: text, ..
            } => {
                out.push(start(json!({ "type": "text", "text": "" })));
                for piece in chunks(text) {
                    out.push(delta(json!({ "type": "text_delta", "text": piece })));
                }
            }
            Block::Call {
                id,
                name,
                arguments,
            } => {
                out.push(start(json!({
                    "type": "tool_use",
                    "id": call_id(id),
                    "name": name,
                    "input": {},
                })));
                for piece in pieces(arguments, 8) {
                    out.push(delta(
                        json!({ "type": "input_json_delta", "partial_json": piece }),
                    ));
                }
            }
        }
        out.push(event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": index }),
        ));
    }
    if reply.ending == Ending::Error {
        out.push(event(
            "error",
            json!({
                "type": "error",
                "error": { "type": "api_error", "message": "The fake model broke off mid-answer." },
            }),
        ));
        return out;
    }
    out.push(event(
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason(reply.ending),
                "stop_sequence": null,
                "stop_details": stop_details(reply.ending),
            },
            "usage": { "output_tokens": reply.output_tokens },
        }),
    ));
    out.push(event("message_stop", json!({ "type": "message_stop" })));
    out
}

/// The non-streamed message of `reply`.
fn document(reply: &Reply, id: &str, model: &str) -> Response {
    if reply.ending == Ending::Error {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_error",
            "The fake model broke off mid-answer.",
        );
    }
    let content: Vec<Value> = reply
        .blocks
        .iter()
        .map(|block| match block {
            Block::Reasoning(text) => json!({
                "type": "thinking",
                "thinking": text,
                "signature": signature(text),
            }),
            Block::Text(text)
            | Block::Audio {
                transcript: text, ..
            } => {
                json!({ "type": "text", "text": text })
            }
            Block::Call {
                id,
                name,
                arguments,
            } => json!({
                "type": "tool_use",
                "id": call_id(id),
                "name": name,
                "input": serde_json::from_str::<Value>(arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({})),
            }),
        })
        .collect();
    axum::Json(json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason(reply.ending),
        "stop_sequence": null,
        "stop_details": stop_details(reply.ending),
        "usage": {
            "input_tokens": reply.input_tokens,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": reply.output_tokens,
        },
    }))
    .into_response()
}

/// The answer to `failed <status>`: Anthropic's error body for the status, and `Retry-After`
/// on a `429`.
fn failure(status: u16) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let kind = match status {
        401 => "authentication_error",
        402 => "billing_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        504 => "timeout_error",
        529 => "overloaded_error",
        500..=599 => "api_error",
        _ => "invalid_request_error",
    };
    let message = format!("The fake model was asked to fail with HTTP {status}.");
    let mut response = error(code, kind, &message);
    if status == 429 {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, RETRY_AFTER_SECS.into());
    }
    response
}

/// Anthropic's error body.
fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    let body = json!({ "type": "error", "error": { "type": kind, "message": message } });
    (status, axum::Json(body)).into_response()
}
