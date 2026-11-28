//! The OpenAI Responses format: `POST /v1/responses` (the model list is the OpenAI-compatible
//! `GET /v1/models`).
//!
//! - **Key:** `Authorization: Bearer <key>`, else `401` with OpenAI's `invalid_api_key` body.
//! - **Requests** need a listed `model` (`404 model_not_found` otherwise) and `input` (a string
//!   or a list of items). The fake stores nothing, as with `store: false`, so it refuses what the
//!   real API refuses then: `previous_response_id` (`404`), a reasoning item without
//!   `encrypted_content` (`404`, "Items are not persisted when `store` is set to false"), a
//!   reasoning item whose `encrypted_content` this server did not give for its summary (`400`),
//!   and a reasoning item another model gave (`400`; for a model without reasoning, OpenAI's
//!   "Reasoning input items can only be provided to a reasoning or computer use model").
//!   Every `function_call` needs a `function_call_output` in the same input, and every output a
//!   call before it (`400` otherwise, with OpenAI's messages).
//! - **Streams** are named events whose data repeats the name in `type`: `response.created` and
//!   `response.in_progress`; per output item `response.output_item.added`, its deltas
//!   (`response.reasoning_summary_text.delta`, `response.output_text.delta`,
//!   `response.refusal.delta`, `response.function_call_arguments.delta`) and their `.done`
//!   events, and `response.output_item.done`; then `response.completed` with the output and
//!   usage. `midstream error` ends with `response.failed` instead; `refuse` adds a `refusal`
//!   content part after a little text.
//! - Reasoning comes as a summary with `encrypted_content` (`think`, or `fake-reasoner` unless
//!   `reasoning.effort` is `none`). The Responses API takes no audio, so `fake-audio` answers in
//!   text here.

use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::http::{now, pieces, stream, with_id};
use crate::openai::{error, failure, refusal};
use crate::reply::{Block, Ending, Plan, Reply, Turn, chunks, newest, plan, signature};
use crate::{AppState, MODELS, model};

/// The header that carries a request id.
const REQUEST_ID: &str = "x-request-id";
/// The refusal text after the opening words of a `refuse` reply.
pub const REFUSAL: &str = "I can't help with that.";

/// `POST /v1/responses`.
pub async fn respond(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let sequence = state.next();
    let response = answer(&state, &headers, &body, sequence);
    with_id(response, REQUEST_ID, &format!("req_fake_{sequence}"))
}

fn answer(state: &AppState, headers: &HeaderMap, body: &[u8], sequence: u64) -> Response {
    if let Some(response) = refusal(state, headers) {
        return response;
    }
    let (name, turn) = match read(body) {
        Ok(read) => read,
        Err(response) => return *response,
    };
    let streamed = turn.streamed;
    match plan(&turn.turn, sequence) {
        Plan::Fail(status) => {
            tracing::info!(model = name, status, "fake-llm.responses.failed");
            failure(status)
        }
        Plan::Answer { reply, slow } => {
            tracing::info!(model = name, streamed, ending = ?reply.ending, "fake-llm.responses.answered");
            let items = Items::new(&reply, sequence, &name);
            if streamed {
                stream(
                    events(&reply, &items, sequence, &name),
                    state.config.delay(slow),
                )
            } else if reply.ending == Ending::Error {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The fake model broke off mid-answer.",
                    "server_error",
                    Some("fake_midstream_error"),
                )
            } else {
                axum::Json(response(&reply, &items, sequence, &name, "completed")).into_response()
            }
        }
    }
}

/// A readable request: its turn, and whether it asks for a stream.
struct Request {
    turn: Turn,
    streamed: bool,
}

/// The model name and the turn of a request body, or the error the real API answers.
fn read(body: &[u8]) -> Result<(String, Request), Box<Response>> {
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return Err(Box::new(error(
            StatusCode::BAD_REQUEST,
            "We could not parse the JSON body of your request.",
            "invalid_request_error",
            None,
        )));
    };
    let name = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if model(&name).is_none() {
        return Err(Box::new(error(
            StatusCode::NOT_FOUND,
            &format!("The model `{name}` does not exist or you do not have access to it."),
            "invalid_request_error",
            Some("model_not_found"),
        )));
    }
    if let Some(previous) = request.get("previous_response_id").and_then(Value::as_str) {
        return Err(Box::new(error(
            StatusCode::NOT_FOUND,
            &format!("Previous response with id '{previous}' not found."),
            "invalid_request_error",
            Some("previous_response_not_found"),
        )));
    }
    let turn = turn(&request).map_err(|(status, message)| {
        tracing::warn!(model = %name, reason = %message, "fake-llm.responses.invalid");
        Box::new(error(status, &message, "invalid_request_error", None))
    })?;
    let streamed = request.get("stream").and_then(Value::as_bool) == Some(true);
    Ok((name, Request { turn, streamed }))
}

/// Reads the request the same way for every reply.
fn turn(request: &Value) -> Result<Turn, (StatusCode, String)> {
    let bad = |message: String| (StatusCode::BAD_REQUEST, message);
    let items: Vec<Value> = match request.get("input") {
        Some(Value::String(text)) => {
            vec![json!({ "type": "message", "role": "user", "content": text })]
        }
        Some(Value::Array(items)) if !items.is_empty() => items.clone(),
        Some(Value::Array(_)) => {
            return Err(bad(
                "Invalid 'input': empty array. Expected an array with minimum length 1.".to_owned(),
            ));
        }
        _ => return Err(bad("Missing required parameter: 'input'.".to_owned())),
    };
    let name = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut replayed = 0_usize;
    for item in &items {
        if reasoning(item, name)? {
            replayed += 1;
        }
    }
    if replayed > 0 {
        tracing::info!(items = replayed, "fake-llm.responses.reasoning-replayed");
    }
    answered(&items).map_err(bad)?;

    let mut turn = Turn {
        model: request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        input_chars: request
            .get("instructions")
            .and_then(Value::as_str)
            .map_or(0, str::len),
        ..Turn::default()
    };
    for item in &items {
        turn.input_chars += texts(item.get("content"))
            .iter()
            .map(String::len)
            .sum::<usize>();
    }
    let last_user = items.iter().rposition(is_user).unwrap_or(0);
    let results: Vec<String> = items[last_user..]
        .iter()
        .filter(|item| kind(item) == "function_call_output")
        .map(|item| output_text(item.get("output")))
        .collect();
    if results.is_empty() {
        if let Some(user) = items.get(last_user).filter(|item| is_user(item)) {
            (turn.text, turn.attachments) = match user.get("content") {
                Some(Value::String(text)) => newest([Some(text.as_str())]),
                Some(Value::Array(parts)) => newest(parts.iter().map(|part| {
                    (kind(part) == "input_text")
                        .then(|| part.get("text").and_then(Value::as_str).unwrap_or_default())
                })),
                _ => (String::new(), 0),
            };
        }
    } else {
        turn.results = Some(results);
    }
    turn.tools = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|tool| kind(tool) == "function")
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    turn.reasoning = request.pointer("/reasoning/effort").and_then(Value::as_str) != Some("none");
    Ok(turn)
}

/// A replayed reasoning item must carry the `encrypted_content` this server gave it, for the
/// model of this request: without one the real API looks the item up, and a stateless response
/// was never stored; a model without reasoning takes no reasoning items at all (only `think`
/// makes the fake's other models reason, so their own items still pass). `true` for a reasoning
/// item that passed.
fn reasoning(item: &Value, name: &str) -> Result<bool, (StatusCode, String)> {
    if kind(item) != "reasoning" {
        return Ok(false);
    }
    let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
    let Some(encrypted) = item
        .get("encrypted_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "Item with id '{id}' not found. Items are not persisted when `store` is set to false. Try again with `store` set to true, or remove this item from your input."
            ),
        ));
    };
    let summary = summary(item);
    if encrypted != encrypt(name, &summary) {
        if model(name).is_some_and(|model| !model.reasoning)
            && MODELS
                .iter()
                .any(|other| encrypted == encrypt(other.id, &summary))
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Reasoning input items can only be provided to a reasoning or computer use model. \
                 Remove reasoning items from your input and try again."
                    .to_owned(),
            ));
        }
        return Err((
            StatusCode::BAD_REQUEST,
            format!("The encrypted content for item {id} could not be verified."),
        ));
    }
    Ok(true)
}

/// Checks that every call has its output and every output a call before it, as the real API
/// does for a stateless input.
fn answered(items: &[Value]) -> Result<(), String> {
    let mut calls: HashSet<&str> = HashSet::new();
    let mut outputs: HashSet<&str> = HashSet::new();
    for item in items {
        let id = item
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind(item) {
            "function_call" => {
                calls.insert(id);
            }
            "function_call_output" => {
                if !calls.contains(id) {
                    return Err(format!(
                        "No tool call found for function call output with call_id {id}."
                    ));
                }
                outputs.insert(id);
            }
            _ => {}
        }
    }
    let mut missing: Vec<&&str> = calls.difference(&outputs).collect();
    missing.sort();
    match missing.first() {
        Some(id) => Err(format!("No tool output found for function call {id}.")),
        None => Ok(()),
    }
}

fn kind(item: &Value) -> &str {
    item.get("type").and_then(Value::as_str).unwrap_or_else(|| {
        // An easy input message may leave out its type.
        if item.get("role").is_some() {
            "message"
        } else {
            ""
        }
    })
}

fn is_user(item: &Value) -> bool {
    kind(item) == "message" && item.get("role").and_then(Value::as_str) == Some("user")
}

/// The text of a `content` field: a string, or the text of its text parts.
fn texts(content: Option<&Value>) -> Vec<String> {
    match content {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| matches!(kind(part), "input_text" | "output_text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// A function call output's text: a string, or the text of its `input_text` items.
fn output_text(output: Option<&Value>) -> String {
    texts(output).join("\n")
}

/// The summary text of a reasoning item.
fn summary(item: &Value) -> String {
    item.get("summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The fake's `encrypted_content` for a reasoning summary of `model`: a hash of both, so a
/// replayed item that was changed, lost its content, or went to another model can be told apart
/// from one sent back verbatim to the model that wrote it.
pub fn encrypt(model: &str, summary: &str) -> String {
    format!(
        "fake-encrypted-{}",
        signature(&format!("{model}\n{summary}"))
    )
}

/// The finished output items of a reply, in order.
struct Items {
    items: Vec<Value>,
}

impl Items {
    fn new(reply: &Reply, sequence: u64, model: &str) -> Self {
        let mut items = Vec::new();
        for (index, block) in reply.blocks.iter().enumerate() {
            items.push(match block {
                Block::Reasoning(text) => json!({
                    "id": format!("rs_fake_{sequence}_{index}"),
                    "type": "reasoning",
                    "status": "completed",
                    "summary": [{ "type": "summary_text", "text": text }],
                    "encrypted_content": encrypt(model, text),
                }),
                Block::Text(text)
                | Block::Audio {
                    transcript: text, ..
                } => {
                    let mut content = vec![json!({
                        "type": "output_text", "text": text, "annotations": [], "logprobs": [],
                    })];
                    if reply.ending == Ending::Refusal {
                        content.push(json!({ "type": "refusal", "refusal": REFUSAL }));
                    }
                    json!({
                        "id": format!("msg_fake_{sequence}_{index}"),
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "phase": "final_answer",
                        "content": content,
                    })
                }
                Block::Call {
                    id,
                    name,
                    arguments,
                } => json!({
                    "id": format!("fc_fake_{sequence}_{index}"),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": id,
                    "name": name,
                    "arguments": arguments,
                }),
            });
        }
        Self { items }
    }
}

/// The streamed events of `reply`.
fn events(reply: &Reply, items: &Items, sequence: u64, model: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let mut number = 0_u64;
    let mut push = |mut data: Value| {
        data["sequence_number"] = json!(number);
        number += 1;
        let name = data["type"].as_str().unwrap_or_default().to_owned();
        out.push(Event::default().event(name).data(data.to_string()));
    };
    let shell =
        |status: &str| response(reply, &Items { items: Vec::new() }, sequence, model, status);
    push(json!({ "type": "response.created", "response": shell("in_progress") }));
    push(json!({ "type": "response.in_progress", "response": shell("in_progress") }));
    for (output, (block, item)) in reply.blocks.iter().zip(&items.items).enumerate() {
        let id = item["id"].clone();
        let mut started = item.clone();
        started["status"] = json!("in_progress");
        match block {
            Block::Reasoning(text) => {
                started["summary"] = json!([]);
                if let Some(fields) = started.as_object_mut() {
                    fields.remove("encrypted_content");
                }
                push(
                    json!({ "type": "response.output_item.added", "output_index": output, "item": started }),
                );
                let part = |text: &str| json!({ "type": "summary_text", "text": text });
                push(
                    json!({ "type": "response.reasoning_summary_part.added", "item_id": id, "output_index": output, "summary_index": 0, "part": part("") }),
                );
                for piece in chunks(text) {
                    push(
                        json!({ "type": "response.reasoning_summary_text.delta", "item_id": id, "output_index": output, "summary_index": 0, "delta": piece }),
                    );
                }
                push(
                    json!({ "type": "response.reasoning_summary_text.done", "item_id": id, "output_index": output, "summary_index": 0, "text": text }),
                );
                push(
                    json!({ "type": "response.reasoning_summary_part.done", "item_id": id, "output_index": output, "summary_index": 0, "part": part(text) }),
                );
            }
            Block::Text(text)
            | Block::Audio {
                transcript: text, ..
            } => {
                started["content"] = json!([]);
                push(
                    json!({ "type": "response.output_item.added", "output_index": output, "item": started }),
                );
                let part =
                    |text: &str| json!({ "type": "output_text", "text": text, "annotations": [] });
                push(
                    json!({ "type": "response.content_part.added", "item_id": id, "output_index": output, "content_index": 0, "part": part("") }),
                );
                for piece in chunks(text) {
                    push(
                        json!({ "type": "response.output_text.delta", "item_id": id, "output_index": output, "content_index": 0, "delta": piece, "logprobs": [] }),
                    );
                }
                push(
                    json!({ "type": "response.output_text.done", "item_id": id, "output_index": output, "content_index": 0, "text": text, "logprobs": [] }),
                );
                push(
                    json!({ "type": "response.content_part.done", "item_id": id, "output_index": output, "content_index": 0, "part": part(text) }),
                );
                if reply.ending == Ending::Refusal {
                    let part = |text: &str| json!({ "type": "refusal", "refusal": text });
                    push(
                        json!({ "type": "response.content_part.added", "item_id": id, "output_index": output, "content_index": 1, "part": part("") }),
                    );
                    for piece in chunks(REFUSAL) {
                        push(
                            json!({ "type": "response.refusal.delta", "item_id": id, "output_index": output, "content_index": 1, "delta": piece }),
                        );
                    }
                    push(
                        json!({ "type": "response.refusal.done", "item_id": id, "output_index": output, "content_index": 1, "refusal": REFUSAL }),
                    );
                    push(
                        json!({ "type": "response.content_part.done", "item_id": id, "output_index": output, "content_index": 1, "part": part(REFUSAL) }),
                    );
                }
                if reply.ending == Ending::Error {
                    break;
                }
            }
            Block::Call { arguments, .. } => {
                started["arguments"] = json!("");
                push(
                    json!({ "type": "response.output_item.added", "output_index": output, "item": started }),
                );
                for piece in pieces(arguments, 8) {
                    push(
                        json!({ "type": "response.function_call_arguments.delta", "item_id": id, "output_index": output, "delta": piece }),
                    );
                }
                push(
                    json!({ "type": "response.function_call_arguments.done", "item_id": id, "output_index": output, "arguments": arguments }),
                );
            }
        }
        push(json!({ "type": "response.output_item.done", "output_index": output, "item": item }));
    }
    if reply.ending == Ending::Error {
        let mut failed = shell("failed");
        failed["error"] =
            json!({ "code": "server_error", "message": "The fake model broke off mid-answer." });
        failed["usage"] = Value::Null;
        push(json!({ "type": "response.failed", "response": failed }));
    } else {
        push(
            json!({ "type": "response.completed", "response": response(reply, items, sequence, model, "completed") }),
        );
    }
    out
}

/// The response object of `reply` with `items` as its output.
fn response(reply: &Reply, items: &Items, sequence: u64, model: &str, status: &str) -> Value {
    let reasoning: usize = reply
        .blocks
        .iter()
        .map(|block| match block {
            Block::Reasoning(text) => text.len().div_ceil(4),
            _ => 0,
        })
        .sum();
    let usage = (status == "completed").then(|| {
        json!({
            "input_tokens": reply.input_tokens,
            "input_tokens_details": { "cached_tokens": 0 },
            "output_tokens": reply.output_tokens,
            "output_tokens_details": { "reasoning_tokens": reasoning },
            "total_tokens": reply.input_tokens.saturating_add(reply.output_tokens),
        })
    });
    json!({
        "id": format!("resp_fake_{sequence}"),
        "object": "response",
        "created_at": now(),
        "status": status,
        "error": null,
        "incomplete_details": null,
        "model": model,
        "output": items.items,
        "store": false,
        "usage": usage,
    })
}
