//! Event decoding: text, thinking (summarized and omitted), signatures, redacted thinking, tool
//! call argument deltas, replay state, usage, every stop reason, and failures.

use serde_json::json;

use super::*;

/// Feeds `events` (each an event's data JSON; the SSE name is its `type`) and ends the body
/// unless an event ended the answer.
fn decode(events: &[Value]) -> Out {
    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    let mut flow = Flow::Continue;
    for event in events {
        let name = event.get("type").and_then(Value::as_str);
        flow = decoder.feed(name, &event.to_string(), &mut out);
        if flow == Flow::Done {
            break;
        }
    }
    if flow == Flow::Continue {
        decoder.end(&mut out);
    }
    out
}

/// The last item, which must be a failure.
fn failure(mut out: Out) -> DomainError {
    out.pop().expect("an item").expect_err("a failure")
}

fn ok(out: Out) -> Vec<LlmEvent> {
    out.into_iter()
        .map(|event| event.expect("no failure"))
        .collect()
}

fn message_start(usage: Value) -> Value {
    json!({ "type": "message_start", "message": {
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-example-1",
        "content": [], "stop_reason": null, "stop_sequence": null, "usage": usage,
    } })
}

fn start(index: u64, block: Value) -> Value {
    json!({ "type": "content_block_start", "index": index, "content_block": block })
}

fn delta(index: u64, delta: Value) -> Value {
    json!({ "type": "content_block_delta", "index": index, "delta": delta })
}

fn stop(index: u64) -> Value {
    json!({ "type": "content_block_stop", "index": index })
}

fn message_delta(reason: &str, usage: Value) -> Value {
    json!({ "type": "message_delta", "delta": { "stop_reason": reason, "stop_sequence": null }, "usage": usage })
}

fn message_stop() -> Value {
    json!({ "type": "message_stop" })
}

fn text_block(index: u64, pieces: &[&str]) -> Vec<Value> {
    let mut events = vec![start(index, json!({ "type": "text", "text": "" }))];
    events.extend(
        pieces
            .iter()
            .map(|piece| delta(index, json!({ "type": "text_delta", "text": piece }))),
    );
    events.push(stop(index));
    events
}

fn finished(reason: FinishReason) -> LlmEvent {
    LlmEvent::Finished {
        reason,
        refusal: None,
    }
}

fn usage(input: u32, output: u32) -> LlmEvent {
    LlmEvent::Usage(Usage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        ..Usage::default()
    })
}

#[test]
fn streams_text_with_usage_and_ends_at_message_stop() {
    let mut events = vec![
        message_start(json!({ "input_tokens": 12, "output_tokens": 1 })),
        json!({ "type": "ping" }),
    ];
    events.extend(text_block(0, &["Hello", "", " there."]));
    events.push(message_delta("end_turn", json!({ "output_tokens": 5 })));
    events.push(message_stop());
    events.push(json!({ "type": "content_block_start", "index": 9 }));
    assert_eq!(
        ok(decode(&events)),
        [
            usage(12, 1),
            LlmEvent::TextDelta {
                index: 0,
                text: "Hello".to_owned()
            },
            LlmEvent::TextDelta {
                index: 0,
                text: " there.".to_owned()
            },
            usage(12, 5),
            LlmEvent::ReplayState(json!([{ "type": "text", "text": "Hello there." }])),
            finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn summarized_thinking_streams_as_reasoning_and_keeps_its_signature() {
    let events = vec![
        message_start(json!({ "input_tokens": 3 })),
        start(
            0,
            json!({ "type": "thinking", "thinking": "", "signature": "" }),
        ),
        delta(
            0,
            json!({ "type": "thinking_delta", "thinking": "Let me " }),
        ),
        delta(0, json!({ "type": "thinking_delta", "thinking": "think." })),
        delta(
            0,
            json!({ "type": "signature_delta", "signature": "c2lnbmF0dXJl" }),
        ),
        stop(0),
        start(1, json!({ "type": "text", "text": "" })),
        delta(1, json!({ "type": "text_delta", "text": "Done." })),
        stop(1),
        message_delta("end_turn", json!({ "output_tokens": 30 })),
        message_stop(),
    ];
    let events = ok(decode(&events));
    assert_eq!(
        events[1..4],
        [
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "Let me ".to_owned()
            },
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "think.".to_owned()
            },
            LlmEvent::TextDelta {
                index: 1,
                text: "Done.".to_owned()
            },
        ]
    );
    assert_eq!(
        events[5],
        LlmEvent::ReplayState(json!([
            { "type": "thinking", "thinking": "Let me think.", "signature": "c2lnbmF0dXJl" },
            { "type": "text", "text": "Done." },
        ]))
    );
}

#[test]
fn omitted_thinking_shows_nothing_but_is_kept_for_replay() {
    let mut events = vec![
        start(0, json!({ "type": "text", "text": "" })),
        delta(0, json!({ "type": "text_delta", "text": "Before." })),
        stop(0),
        start(1, json!({ "type": "thinking", "thinking": "" })),
        delta(1, json!({ "type": "thinking_delta", "thinking": "" })),
        delta(1, json!({ "type": "signature_delta", "signature": "c2ln" })),
        stop(1),
        start(
            2,
            json!({ "type": "redacted_thinking", "data": "b3BhcXVl" }),
        ),
        stop(2),
    ];
    events.extend(text_block(3, &["After."]));
    events.push(message_delta("end_turn", json!({})));
    events.push(message_stop());
    let events = ok(decode(&events));
    assert_eq!(
        events,
        [
            LlmEvent::TextDelta {
                index: 0,
                text: "Before.".to_owned()
            },
            // The thinking in between ends the text run: the later text is its own part.
            LlmEvent::TextDelta {
                index: 1,
                text: "After.".to_owned()
            },
            LlmEvent::ReplayState(json!([
                { "type": "text", "text": "Before." },
                { "type": "thinking", "thinking": "", "signature": "c2ln" },
                { "type": "redacted_thinking", "data": "b3BhcXVl" },
                { "type": "text", "text": "After." },
            ])),
            finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn consecutive_text_blocks_continue_one_part() {
    let mut events = text_block(0, &["Cited "]);
    events.extend(text_block(1, &["text."]));
    events.push(message_stop());
    let events = ok(decode(&events));
    assert!(matches!(events[1], LlmEvent::TextDelta { index: 0, .. }));
}

#[test]
fn reassembles_tool_call_argument_deltas() {
    let mut events = text_block(0, &["Okay"]);
    events.extend([
        start(
            1,
            json!({ "type": "tool_use", "id": "toolu_1", "name": "weather__get", "input": {} }),
        ),
        delta(1, json!({ "type": "input_json_delta", "partial_json": "" })),
        delta(
            1,
            json!({ "type": "input_json_delta", "partial_json": "{\"location\":" }),
        ),
        delta(
            1,
            json!({ "type": "input_json_delta", "partial_json": " \"Paris\"}" }),
        ),
        stop(1),
        delta(
            1,
            json!({ "type": "input_json_delta", "partial_json": "late" }),
        ),
        start(
            2,
            json!({ "type": "tool_use", "id": "toolu_2", "name": "clock__now", "input": {} }),
        ),
        stop(2),
        message_delta("tool_use", json!({ "output_tokens": 89 })),
        message_stop(),
    ]);
    let events = ok(decode(&events));
    assert_eq!(
        events,
        [
            LlmEvent::TextDelta {
                index: 0,
                text: "Okay".to_owned()
            },
            LlmEvent::ToolCallStarted {
                index: 1,
                provider_call_id: Some("toolu_1".to_owned()),
                alias: "weather__get".to_owned(),
            },
            LlmEvent::ToolCallArgs {
                index: 1,
                json: "{\"location\":".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 1,
                json: " \"Paris\"}".to_owned()
            },
            LlmEvent::ToolCallEnded { index: 1 },
            LlmEvent::ToolCallStarted {
                index: 2,
                provider_call_id: Some("toolu_2".to_owned()),
                alias: "clock__now".to_owned(),
            },
            LlmEvent::ToolCallEnded { index: 2 },
            LlmEvent::Usage(Usage {
                output_tokens: Some(89),
                ..Usage::default()
            }),
            LlmEvent::ReplayState(json!([
                { "type": "text", "text": "Okay" },
                { "type": "tool_use", "id": "toolu_1", "name": "weather__get", "input": { "location": "Paris" } },
                { "type": "tool_use", "id": "toolu_2", "name": "clock__now", "input": {} },
            ])),
            finished(FinishReason::ToolCalls),
        ]
    );
}

#[test]
fn a_whole_tool_input_in_the_start_block_becomes_the_arguments() {
    let events = vec![
        start(
            0,
            json!({ "type": "tool_use", "id": "toolu_1", "name": "weather__get", "input": { "location": "Rome" } }),
        ),
        message_delta("tool_use", json!({})),
    ];
    // The body ends without `message_stop`, after a stop reason: still finished.
    let events = ok(decode(&events));
    assert_eq!(
        events[1..3],
        [
            LlmEvent::ToolCallArgs {
                index: 0,
                json: "{\"location\":\"Rome\"}".to_owned()
            },
            LlmEvent::ToolCallEnded { index: 0 },
        ]
    );
    assert_eq!(events.last(), Some(&finished(FinishReason::ToolCalls)));
}

#[test]
fn maps_every_stop_reason() {
    for (reason, expected) in [
        ("end_turn", FinishReason::Stop),
        ("stop_sequence", FinishReason::Stop),
        ("tool_use", FinishReason::ToolCalls),
        ("max_tokens", FinishReason::Length),
        ("model_context_window_exceeded", FinishReason::Length),
        ("pause_turn", FinishReason::Other),
        ("something_new", FinishReason::Other),
    ] {
        let events = ok(decode(&[message_delta(reason, json!({})), message_stop()]));
        assert_eq!(events.last(), Some(&finished(expected)), "{reason}");
    }
    let events = ok(decode(&[message_stop()]));
    assert_eq!(events, [finished(FinishReason::Other)]);
}

#[test]
fn a_refusal_carries_its_stop_details() {
    let mut events = text_block(0, &["Partial"]);
    events.push(json!({ "type": "message_delta", "delta": {
        "stop_reason": "refusal",
        "stop_sequence": null,
        "stop_details": { "type": "refusal", "category": "cyber", "explanation": "I can't help with that." },
    }, "usage": { "output_tokens": 4 } }));
    events.push(message_stop());
    assert_eq!(
        ok(decode(&events)).last(),
        Some(&LlmEvent::Finished {
            reason: FinishReason::Refusal,
            refusal: Some(Refusal {
                category: Some("cyber".to_owned()),
                explanation: "I can't help with that.".to_owned(),
            }),
        })
    );

    let unexplained = [
        json!({ "type": "message_delta", "delta": {
            "stop_reason": "refusal",
            "stop_details": { "type": "refusal", "category": null, "explanation": null },
        } }),
        message_stop(),
    ];
    assert_eq!(
        ok(decode(&unexplained)).last(),
        Some(&LlmEvent::Finished {
            reason: FinishReason::Refusal,
            refusal: Some(Refusal {
                category: None,
                explanation: DECLINED.to_owned(),
            }),
        })
    );
}

#[test]
fn usage_counts_the_whole_prompt_with_cache_tokens() {
    let events = ok(decode(&[
        message_start(json!({
            "input_tokens": 20,
            "cache_creation_input_tokens": 1_000,
            "cache_read_input_tokens": 3_000,
            "output_tokens": 1,
        })),
        message_delta(
            "end_turn",
            json!({ "output_tokens": 40, "cache_read_input_tokens": 3_000 }),
        ),
        message_stop(),
    ]));
    let expected = Usage {
        input_tokens: Some(4_020),
        output_tokens: Some(40),
        reasoning_tokens: None,
        cache_read_tokens: Some(3_000),
        cache_write_tokens: Some(1_000),
    };
    assert_eq!(events[1], LlmEvent::Usage(expected));
}

#[test]
fn a_body_that_ends_before_a_stop_reason_reports_no_finish() {
    let mut events = vec![message_start(json!({ "input_tokens": 1 }))];
    events.extend(text_block(0, &["Cut"]));
    let events = ok(decode(&events));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, LlmEvent::Finished { .. } | LlmEvent::ReplayState(_)))
    );
}

#[test]
fn error_events_fail_without_echoing_the_provider() {
    let overloaded = decode(&[
        message_start(json!({})),
        json!({ "type": "error", "error": { "type": "overloaded_error", "message": "secret detail" } }),
        message_stop(),
    ]);
    assert!(
        !overloaded
            .iter()
            .any(|item| matches!(item, Ok(LlmEvent::Finished { .. }))),
        "nothing after the error"
    );
    let error = failure(overloaded);
    assert_eq!(error.code(), PROVIDER_ERROR);
    assert_eq!(
        error.to_string(),
        "The provider is overloaded. Try again in a moment."
    );

    let limited = decode(&[
        json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "x" } }),
    ]);
    let error = failure(limited);
    assert_eq!(error.code(), PROVIDER_RATE_LIMITED);

    let other = decode(&[
        json!({ "type": "error", "error": { "type": "api_error", "message": "secret detail" } }),
    ]);
    let error = failure(other);
    assert_eq!(error.code(), PROVIDER_ERROR);
    assert!(!error.to_string().contains("secret"));

    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    assert_eq!(decoder.feed(Some("error"), "", &mut out), Flow::Done);
    assert_eq!(failure(out).code(), PROVIDER_ERROR);
}

#[test]
fn unreadable_data_fails_the_answer() {
    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    assert_eq!(
        decoder.feed(Some("message_start"), "{not json", &mut out),
        Flow::Done
    );
    assert_eq!(failure(out).code(), PROVIDER_BAD_RESPONSE);
}

#[test]
fn replay_state_past_the_cap_fails_the_answer() {
    let mut decoder = Decoder::new(ProviderId::generate());
    let mut out = Vec::new();
    let huge = "x".repeat(REPLAY_MAX_BYTES + 1);
    let event = start(0, json!({ "type": "redacted_thinking", "data": huge }));
    assert_eq!(
        decoder.feed(Some("content_block_start"), &event.to_string(), &mut out),
        Flow::Done
    );
    assert_eq!(failure(out).code(), PROVIDER_BAD_RESPONSE);
}

#[test]
fn unknown_events_and_deltas_are_skipped() {
    let mut events = vec![json!({ "type": "something_new", "detail": 1 })];
    events.push(start(0, json!({ "type": "text", "text": "" })));
    events.push(delta(
        0,
        json!({ "type": "citations_delta", "citation": {} }),
    ));
    events.push(delta(
        7,
        json!({ "type": "text_delta", "text": "no block" }),
    ));
    events.push(delta(0, json!({ "type": "text_delta", "text": "Kept." })));
    events.push(stop(0));
    events.push(message_stop());
    let events = ok(decode(&events));
    assert_eq!(
        events[0],
        LlmEvent::TextDelta {
            index: 0,
            text: "Kept.".to_owned()
        }
    );
    assert_eq!(events.len(), 3);
}
