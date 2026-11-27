//! Event decoding: text, reasoning summaries and raw reasoning text, function call argument
//! deltas, whole items from servers that skip deltas, refusals, replay state, usage, every
//! terminal event, and failures.

use serde_json::json;

use super::*;

/// Feeds `events` (each an event's data JSON; the SSE name is its `type`).
fn decode(events: &[Value]) -> Out {
    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    for event in events {
        let name = event.get("type").and_then(Value::as_str);
        if decoder.feed(name, &event.to_string(), &mut out) == Flow::Done {
            break;
        }
    }
    out
}

fn ok(out: Out) -> Vec<LlmEvent> {
    out.into_iter()
        .map(|event| event.expect("no failure"))
        .collect()
}

/// The last item, which must be a failure.
fn failure(mut out: Out) -> DomainError {
    out.pop().expect("an item").expect_err("a failure")
}

fn added(output: u64, item: Value) -> Value {
    json!({ "type": "response.output_item.added", "output_index": output, "item": item, "sequence_number": 1 })
}

fn done(output: u64, item: Value) -> Value {
    json!({ "type": "response.output_item.done", "output_index": output, "item": item, "sequence_number": 9 })
}

fn text_delta(output: u64, delta: &str) -> Value {
    json!({ "type": "response.output_text.delta", "item_id": "msg_1", "output_index": output, "content_index": 0, "delta": delta, "logprobs": [] })
}

fn summary_delta(output: u64, summary: u64, delta: &str) -> Value {
    json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": output, "summary_index": summary, "delta": delta })
}

fn args_delta(output: u64, delta: &str) -> Value {
    json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": output, "delta": delta })
}

fn message(text: &str) -> Value {
    json!({ "id": "msg_1", "type": "message", "role": "assistant", "status": "completed", "phase": "final_answer",
        "content": [{ "type": "output_text", "text": text, "annotations": [], "logprobs": [] }] })
}

fn reasoning(summaries: &[&str]) -> Value {
    let summary: Vec<Value> = summaries
        .iter()
        .map(|text| json!({ "type": "summary_text", "text": text }))
        .collect();
    json!({ "id": "rs_1", "type": "reasoning", "summary": summary, "encrypted_content": "gAAAAB" })
}

fn function_call(arguments: &str) -> Value {
    json!({ "id": "fc_1", "type": "function_call", "status": "completed", "call_id": "call_1", "name": "docs__search", "arguments": arguments })
}

fn completed(usage: Value) -> Value {
    json!({ "type": "response.completed", "response": {
        "id": "resp_1", "object": "response", "status": "completed", "error": null,
        "incomplete_details": null, "output": [], "usage": usage,
    } })
}

fn usage() -> Value {
    json!({
        "input_tokens": 120,
        "input_tokens_details": { "cached_tokens": 100, "cache_write_tokens": 0 },
        "output_tokens": 40,
        "output_tokens_details": { "reasoning_tokens": 25 },
        "total_tokens": 160,
    })
}

fn finished(reason: FinishReason) -> LlmEvent {
    LlmEvent::Finished {
        reason,
        refusal: None,
    }
}

#[test]
fn streams_text_with_usage_and_the_message_for_replay() {
    let events = ok(decode(&[
        json!({ "type": "response.created", "response": { "id": "resp_1", "status": "in_progress" } }),
        added(
            0,
            json!({ "id": "msg_1", "type": "message", "role": "assistant", "status": "in_progress", "content": [] }),
        ),
        json!({ "type": "response.content_part.added", "output_index": 0, "content_index": 0, "part": { "type": "output_text", "text": "" } }),
        text_delta(0, "Hel"),
        text_delta(0, "lo"),
        json!({ "type": "response.output_text.done", "output_index": 0, "content_index": 0, "text": "Hello" }),
        done(0, message("Hello")),
        completed(usage()),
    ]));
    assert_eq!(
        events,
        [
            LlmEvent::TextDelta {
                index: 0,
                text: "Hel".to_owned()
            },
            LlmEvent::TextDelta {
                index: 0,
                text: "lo".to_owned()
            },
            LlmEvent::Usage(Usage {
                input_tokens: Some(120),
                output_tokens: Some(40),
                reasoning_tokens: Some(25),
                cache_read_tokens: Some(100),
                cache_write_tokens: Some(0),
            }),
            LlmEvent::ReplayState(json!({ "api": "responses", "items": [message("Hello")] })),
            finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn shows_reasoning_summaries_as_paragraphs_and_keeps_encrypted_reasoning() {
    let events = ok(decode(&[
        added(
            0,
            json!({ "id": "rs_1", "type": "reasoning", "summary": [] }),
        ),
        json!({ "type": "response.reasoning_summary_part.added", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "part": { "type": "summary_text", "text": "" } }),
        summary_delta(0, 0, "**Plan**"),
        json!({ "type": "response.reasoning_summary_text.done", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "text": "**Plan**" }),
        summary_delta(0, 1, "Then answer."),
        done(0, reasoning(&["**Plan**", "Then answer."])),
        added(
            1,
            json!({ "id": "msg_1", "type": "message", "role": "assistant", "content": [] }),
        ),
        text_delta(1, "Done."),
        done(1, message("Done.")),
        completed(usage()),
    ]));
    assert_eq!(
        events[..4],
        [
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "**Plan**".to_owned()
            },
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "\n\nThen answer.".to_owned()
            },
            LlmEvent::TextDelta {
                index: 1,
                text: "Done.".to_owned()
            },
            LlmEvent::Usage(read_usage(&usage())),
        ]
    );
    assert_eq!(
        events[4],
        LlmEvent::ReplayState(json!({
            "api": "responses",
            "items": [reasoning(&["**Plan**", "Then answer."]), message("Done.")],
        }))
    );
}

#[test]
fn shows_raw_reasoning_text_from_open_models() {
    let events = ok(decode(&[
        added(
            0,
            json!({ "id": "rs_1", "type": "reasoning", "summary": [] }),
        ),
        json!({ "type": "response.reasoning_text.delta", "item_id": "rs_1", "output_index": 0, "content_index": 0, "delta": "The user" }),
        json!({ "type": "response.reasoning_text.delta", "item_id": "rs_1", "output_index": 0, "content_index": 0, "delta": " asks." }),
        done(
            0,
            json!({ "id": "rs_1", "type": "reasoning", "summary": [], "content": [{ "type": "reasoning_text", "text": "The user asks." }] }),
        ),
        text_delta(1, "Hi"),
        completed(Value::Null),
    ]));
    assert_eq!(
        events,
        [
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "The user".to_owned()
            },
            LlmEvent::ReasoningDelta {
                index: 0,
                text: " asks.".to_owned()
            },
            LlmEvent::TextDelta {
                index: 1,
                text: "Hi".to_owned()
            },
            finished(FinishReason::Stop),
        ],
        "reasoning without encrypted content is not kept, and no usage was reported"
    );
}

#[test]
fn streams_function_calls_and_finishes_with_tool_calls() {
    let events = ok(decode(&[
        added(
            0,
            json!({ "id": "fc_1", "type": "function_call", "status": "in_progress", "call_id": "call_1", "name": "docs__search", "arguments": "" }),
        ),
        args_delta(0, "{\"q\":"),
        args_delta(0, "\"x\"}"),
        json!({ "type": "response.function_call_arguments.done", "item_id": "fc_1", "output_index": 0, "arguments": "{\"q\":\"x\"}" }),
        done(0, function_call("{\"q\":\"x\"}")),
        added(
            1,
            json!({ "id": "fc_2", "type": "function_call", "call_id": "call_2", "name": "docs__read", "arguments": "" }),
        ),
        json!({ "type": "response.function_call_arguments.done", "item_id": "fc_2", "output_index": 1, "arguments": "{}" }),
        completed(usage()),
    ]));
    assert_eq!(
        events[..7],
        [
            LlmEvent::ToolCallStarted {
                index: 0,
                provider_call_id: Some("call_1".to_owned()),
                alias: "docs__search".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 0,
                json: "{\"q\":".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 0,
                json: "\"x\"}".to_owned()
            },
            LlmEvent::ToolCallEnded { index: 0 },
            LlmEvent::ToolCallStarted {
                index: 1,
                provider_call_id: Some("call_2".to_owned()),
                alias: "docs__read".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 1,
                json: "{}".to_owned()
            },
            LlmEvent::ToolCallEnded { index: 1 },
        ],
        "arguments from `.done` when no delta came; a call the item never finished ends at the end"
    );
    assert_eq!(
        events[8],
        LlmEvent::ReplayState(
            json!({ "api": "responses", "items": [function_call("{\"q\":\"x\"}")] })
        )
    );
    assert_eq!(events[9], finished(FinishReason::ToolCalls));
}

#[test]
fn shows_whole_items_from_servers_that_skip_deltas() {
    let events = ok(decode(&[
        done(0, reasoning(&["Short plan."])),
        done(1, function_call("{\"q\":\"y\"}")),
        json!({ "type": "response.completed", "response": {
            "status": "completed",
            "output": [reasoning(&["Short plan."]), function_call("{\"q\":\"y\"}"), message("And text.")],
        } }),
    ]));
    assert_eq!(
        events[..6],
        [
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "Short plan.".to_owned()
            },
            LlmEvent::ToolCallStarted {
                index: 1,
                provider_call_id: Some("call_1".to_owned()),
                alias: "docs__search".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 1,
                json: "{\"q\":\"y\"}".to_owned()
            },
            LlmEvent::ToolCallEnded { index: 1 },
            LlmEvent::TextDelta {
                index: 2,
                text: "And text.".to_owned()
            },
            LlmEvent::ReplayState(json!({
                "api": "responses",
                "items": [reasoning(&["Short plan."]), function_call("{\"q\":\"y\"}"), message("And text.")],
            })),
        ]
    );
    assert_eq!(events[6], finished(FinishReason::ToolCalls));
}

#[test]
fn a_refusal_ends_the_answer_with_its_text() {
    let events = ok(decode(&[
        added(
            0,
            json!({ "id": "msg_1", "type": "message", "role": "assistant", "content": [] }),
        ),
        text_delta(0, "I"),
        json!({ "type": "response.refusal.delta", "item_id": "msg_1", "output_index": 0, "content_index": 1, "delta": "I can't help " }),
        json!({ "type": "response.refusal.delta", "item_id": "msg_1", "output_index": 0, "content_index": 1, "delta": "with that." }),
        json!({ "type": "response.refusal.done", "item_id": "msg_1", "output_index": 0, "content_index": 1, "refusal": "I can't help with that." }),
        completed(usage()),
    ]));
    assert_eq!(
        events.last(),
        Some(&LlmEvent::Finished {
            reason: FinishReason::Refusal,
            refusal: Some(Refusal {
                category: None,
                explanation: "I can't help with that.".to_owned(),
            }),
        })
    );
    let whole = ok(decode(&[
        done(
            0,
            json!({ "id": "msg_1", "type": "message", "role": "assistant", "content": [{ "type": "refusal", "refusal": "No." }] }),
        ),
        completed(Value::Null),
    ]));
    assert!(matches!(
        whole.last(),
        Some(LlmEvent::Finished { reason: FinishReason::Refusal, refusal: Some(refusal) })
            if refusal.explanation == "No."
    ));
}

#[test]
fn incomplete_answers_finish_by_their_reason() {
    let incomplete = |reason: &str| {
        json!({ "type": "response.incomplete", "response": {
            "status": "incomplete", "incomplete_details": { "reason": reason }, "output": [], "usage": null,
        } })
    };
    for (reason, expected) in [
        ("max_output_tokens", FinishReason::Length),
        ("content_filter", FinishReason::Refusal),
        ("max_messages", FinishReason::Other),
    ] {
        let events = ok(decode(&[text_delta(0, "Partial"), incomplete(reason)]));
        assert_eq!(events.last(), Some(&finished(expected)), "{reason}");
    }
}

#[test]
fn failures_end_the_answer_without_echoing_the_provider() {
    let failed = |code: &str| {
        json!({ "type": "response.failed", "response": {
            "status": "failed", "error": { "code": code, "message": "secret details" }, "output": [],
        } })
    };
    let error = failure(decode(&[text_delta(0, "Hi"), failed("server_error")]));
    assert_eq!(error.code(), crate::sse::PROVIDER_ERROR);
    assert!(!error.to_string().contains("secret"));
    let limited = failure(decode(&[failed("rate_limit_exceeded")]));
    assert_eq!(limited.code(), PROVIDER_RATE_LIMITED);

    let event =
        json!({ "type": "error", "code": "ERR_SOMETHING", "message": "secret", "param": null });
    let out = decode(&[text_delta(0, "Hi"), event]);
    assert!(out[0].is_ok(), "text before the error stays");
    assert_eq!(failure(out).code(), crate::sse::PROVIDER_ERROR);

    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    assert_eq!(decoder.feed(Some("error"), "", &mut out), Flow::Done);
    assert_eq!(failure(out).code(), crate::sse::PROVIDER_ERROR);
}

#[test]
fn unreadable_events_fail_and_unknown_ones_are_skipped() {
    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    assert_eq!(decoder.feed(None, "{not json", &mut out), Flow::Done);
    assert_eq!(failure(out).code(), PROVIDER_BAD_RESPONSE);

    let events = ok(decode(&[
        json!({ "type": "response.web_search_call.searching", "output_index": 0 }),
        json!({ "type": "response.queued" }),
        text_delta(1, "Hi"),
        completed(Value::Null),
    ]));
    assert_eq!(events.len(), 2);

    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    let named = json!({ "output_index": 0, "delta": "Hi" }).to_string();
    decoder.feed(Some("response.output_text.delta"), &named, &mut out);
    assert_eq!(
        ok(out),
        [LlmEvent::TextDelta {
            index: 0,
            text: "Hi".to_owned()
        }],
        "the SSE name stands in for a missing type"
    );
    assert_eq!(decoder.feed(None, "[DONE]", &mut Vec::new()), Flow::Done);
}

#[test]
fn replay_state_over_the_limit_fails() {
    let huge = json!({ "id": "rs_1", "type": "reasoning", "summary": [], "encrypted_content": "a".repeat(REPLAY_MAX_BYTES) });
    let error = failure(decode(&[done(0, huge)]));
    assert_eq!(error.code(), PROVIDER_BAD_RESPONSE);
}
