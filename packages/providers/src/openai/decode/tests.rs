//! Chunk decoding: text, every reasoning variant, tool call reassembly, audio, finish reasons,
//! usage, replay state, and failures.

use serde_json::json;

use super::*;

fn decode_as(dialect: Dialect, events: &[&str]) -> (Out, Flow) {
    let mut decoder = Decoder::new(ProviderId::generate(), dialect);
    let mut out = Vec::new();
    let mut flow = Flow::Continue;
    for data in events {
        flow = decoder.feed(data, &mut out);
        if flow == Flow::Done {
            break;
        }
    }
    if flow == Flow::Continue {
        decoder.end(&mut out);
    }
    (out, flow)
}

fn decode(events: &[&str]) -> (Out, Flow) {
    decode_as(Dialect::Compatible, events)
}

fn ok(events: Out) -> Vec<LlmEvent> {
    events
        .into_iter()
        .map(|event| event.expect("no failure"))
        .collect()
}

fn text(index: u32, text: &str) -> LlmEvent {
    LlmEvent::TextDelta {
        index,
        text: text.to_owned(),
    }
}

fn reasoning(index: u32, text: &str) -> LlmEvent {
    LlmEvent::ReasoningDelta {
        index,
        text: text.to_owned(),
    }
}

fn finished(reason: FinishReason) -> LlmEvent {
    LlmEvent::Finished {
        reason,
        refusal: None,
    }
}

fn delta(delta: &Value) -> String {
    json!({ "choices": [{ "index": 0, "delta": delta, "finish_reason": null }] }).to_string()
}

fn finish_chunk(reason: &str) -> String {
    json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": reason }] }).to_string()
}

fn started(index: u32, id: Option<&str>, alias: &str) -> LlmEvent {
    LlmEvent::ToolCallStarted {
        index,
        provider_call_id: id.map(str::to_owned),
        alias: alias.to_owned(),
    }
}

fn args(index: u32, json: &str) -> LlmEvent {
    LlmEvent::ToolCallArgs {
        index,
        json: json.to_owned(),
    }
}

#[test]
fn streams_text_then_finishes_at_done() {
    let (events, flow) = decode(&[
        r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(flow, Flow::Done);
    assert_eq!(
        ok(events),
        [text(0, "Hel"), text(0, "lo"), finished(FinishReason::Stop)]
    );
}

#[test]
fn keeps_the_finish_for_the_usage_chunk_after_it() {
    let (events, _) = decode(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"length"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":82,"completion_tokens":18,"total_tokens":100,"completion_tokens_details":{"reasoning_tokens":4},"prompt_tokens_details":{"cached_tokens":64,"cache_write_tokens":8}}}"#,
        "[DONE]",
    ]);
    assert_eq!(
        ok(events),
        [
            text(0, "Hi"),
            LlmEvent::Usage(Usage {
                input_tokens: Some(82),
                output_tokens: Some(18),
                reasoning_tokens: Some(4),
                cache_read_tokens: Some(64),
                cache_write_tokens: Some(8),
            }),
            finished(FinishReason::Length),
        ]
    );
}

#[test]
fn reads_flat_cached_tokens_and_ignores_counts_out_of_range() {
    let (events, _) = decode(&[
        r#"{"choices":[],"usage":{"prompt_tokens":99999999999,"completion_tokens":3,"cached_tokens":2}}"#,
    ]);
    assert_eq!(
        ok(events),
        [LlmEvent::Usage(Usage {
            input_tokens: None,
            output_tokens: Some(3),
            cache_read_tokens: Some(2),
            ..Usage::default()
        })]
    );
}

#[test]
fn openrouter_usage_frames_repeat_the_finish_without_a_second_end() {
    let (events, _) = decode_as(
        Dialect::OpenRouter,
        &[
            &delta(&json!({ "content": "Hi" })),
            &finish_chunk("stop"),
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop","native_finish_reason":"end_turn"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"cost":0.0001}}"#,
            "[DONE]",
        ],
    );
    assert_eq!(
        ok(events),
        [
            text(0, "Hi"),
            LlmEvent::Usage(Usage {
                input_tokens: Some(5),
                output_tokens: Some(1),
                ..Usage::default()
            }),
            finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn maps_every_finish_reason() {
    for (wire, reason) in [
        ("stop", FinishReason::Stop),
        ("length", FinishReason::Length),
        ("tool_calls", FinishReason::ToolCalls),
        ("function_call", FinishReason::ToolCalls),
        ("error", FinishReason::Error),
        ("something_new", FinishReason::Other),
    ] {
        let (events, _) = decode(&[&finish_chunk(wire), "[DONE]"]);
        assert_eq!(ok(events), [finished(reason)], "{wire}");
    }
}

#[test]
fn a_content_filter_or_refusal_text_is_a_refusal() {
    let (events, _) = decode(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Partial"},"finish_reason":"content_filter"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(
        ok(events),
        [
            text(0, "Partial"),
            LlmEvent::Finished {
                reason: FinishReason::Refusal,
                refusal: None,
            }
        ]
    );

    let (events, _) = decode(&[
        r#"{"choices":[{"index":0,"delta":{"refusal":"I can't "},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{"refusal":"help with that."},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(
        ok(events),
        [LlmEvent::Finished {
            reason: FinishReason::Refusal,
            refusal: Some(Refusal {
                category: None,
                explanation: "I can't help with that.".to_owned(),
            }),
        }]
    );
}

#[test]
fn a_body_that_ends_after_a_finish_still_finishes() {
    let (events, flow) =
        decode(&[r#"{"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#]);
    assert_eq!(flow, Flow::Continue);
    assert_eq!(ok(events), [text(0, "Hi"), finished(FinishReason::Stop)]);
}

#[test]
fn a_body_that_ends_without_a_finish_reports_no_finish() {
    let (events, _) = decode(&[r#"{"choices":[{"index":0,"delta":{"content":"Hi"}}]}"#]);
    assert_eq!(ok(events), [text(0, "Hi")]);
}

#[test]
fn done_without_a_finish_reason_finishes_as_other() {
    let (events, _) = decode(&["[DONE]"]);
    assert_eq!(ok(events), [finished(FinishReason::Other)]);
}

#[test]
fn an_error_chunk_fails_the_answer() {
    let (events, flow) = decode(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#,
        r#"{"error":{"code":502,"message":"Provider disconnected unexpectedly"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}"#,
        r#"{"choices":[{"index":0,"delta":{"content":"never read"}}]}"#,
    ]);
    assert_eq!(flow, Flow::Done);
    assert_eq!(events.len(), 2);
    let error = events[1].as_ref().expect_err("the error chunk");
    assert_eq!(error.code(), PROVIDER_ERROR);
    assert!(
        !error.to_string().contains("disconnected"),
        "the provider's text is not echoed"
    );
}

#[test]
fn a_vllm_error_object_fails_the_answer() {
    let (events, flow) = decode(&[
        r#"{"object":"error","message":"The model ran out of memory","type":"InternalServerError","code":500}"#,
    ]);
    assert_eq!(flow, Flow::Done);
    let error = events[0].as_ref().expect_err("the error object");
    assert_eq!(error.code(), PROVIDER_ERROR);
    assert!(!error.to_string().contains("memory"));
}

#[test]
fn unreadable_json_fails_the_answer() {
    let (events, flow) = decode(&["{not json"]);
    assert_eq!(flow, Flow::Done);
    assert_eq!(
        events[0].as_ref().expect_err("invalid").code(),
        "PROVIDER_BAD_RESPONSE"
    );
}

#[test]
fn reads_only_the_first_choice_and_skips_empty_data() {
    let (events, _) = decode(&[
        "",
        r#"{"choices":[{"index":1,"delta":{"content":"other"}},{"index":0,"delta":{"content":"mine"}}]}"#,
        r#"{"id":"x","object":"chat.completion.chunk","obfuscation":"abc","choices":[{"index":0,"delta":{"content":null},"logprobs":null,"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(ok(events), [text(0, "mine"), finished(FinishReason::Stop)]);
}

#[test]
fn fields_of_unexpected_types_are_skipped() {
    let (events, _) = decode(&[
        r#"{"choices":[{"index":0,"delta":{"content":["odd"],"reasoning":{"odd":true},"tool_calls":"odd","audio":7,"reasoning_details":"odd"}}]}"#,
        r#"{"choices":[{"index":null,"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]);
    assert_eq!(ok(events), [text(0, "Hi"), finished(FinishReason::Stop)]);
}

#[test]
fn reads_reasoning_under_either_key_and_keeps_it_for_replay() {
    for key in ["reasoning", "reasoning_content"] {
        let (events, _) = decode(&[
            &delta(&json!({ key: "Let me " })),
            &delta(&json!({ key: "think." })),
            &delta(&json!({ "content": "Answer." })),
            &finish_chunk("stop"),
            "[DONE]",
        ]);
        assert_eq!(
            ok(events),
            [
                reasoning(0, "Let me "),
                reasoning(0, "think."),
                text(1, "Answer."),
                LlmEvent::ReplayState(
                    json!({ "reasoning_key": key, "reasoning": "Let me think." })
                ),
                finished(FinishReason::Stop),
            ],
            "{key}"
        );
    }
}

#[test]
fn prefers_reasoning_when_a_server_sends_both_keys() {
    let (events, _) = decode(&[
        &delta(&json!({ "reasoning": "Once.", "reasoning_content": "Once." })),
        "[DONE]",
    ]);
    assert_eq!(
        ok(events),
        [
            reasoning(0, "Once."),
            LlmEvent::ReplayState(json!({ "reasoning_key": "reasoning", "reasoning": "Once." })),
            finished(FinishReason::Other),
        ]
    );
}

#[test]
fn interleaved_reasoning_and_text_start_new_parts() {
    let (events, _) = decode(&[
        &delta(&json!({ "reasoning": "A" })),
        &delta(&json!({ "content": "B" })),
        &delta(&json!({ "reasoning": "C" })),
        &delta(&json!({ "content": "D" })),
        &delta(&json!({ "content": "E" })),
        "[DONE]",
    ]);
    let deltas: Vec<_> = ok(events)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                LlmEvent::TextDelta { .. } | LlmEvent::ReasoningDelta { .. }
            )
        })
        .collect();
    assert_eq!(
        deltas,
        [
            reasoning(0, "A"),
            text(1, "B"),
            reasoning(2, "C"),
            text(3, "D"),
            text(3, "E"),
        ]
    );
}

#[test]
fn openrouter_reasoning_details_are_kept_whole_and_shown_once() {
    let (events, _) = decode_as(
        Dialect::OpenRouter,
        &[
            // OpenRouter sends the plain text next to the detail: it is shown once.
            &delta(&json!({
                "reasoning": "Plan ",
                "reasoning_details": [{ "type": "reasoning.text", "text": "Plan ", "index": 0, "format": "anthropic-claude-v1" }],
            })),
            &delta(&json!({
                "reasoning": "it.",
                "reasoning_details": [{ "type": "reasoning.text", "text": "it.", "index": 0 }],
            })),
            &delta(&json!({
                "reasoning_details": [{ "type": "reasoning.text", "text": "", "signature": "sig-1", "index": 0 }],
            })),
            // Details alone: summaries are shown, encrypted data is not.
            &delta(&json!({
                "reasoning_details": [
                    { "type": "reasoning.summary", "summary": "Summary.", "index": 1 },
                    { "type": "reasoning.encrypted", "data": "opaque", "index": 2 },
                ],
            })),
            &delta(&json!({ "content": "Done." })),
            &finish_chunk("stop"),
            "[DONE]",
        ],
    );
    assert_eq!(
        ok(events),
        [
            reasoning(0, "Plan "),
            reasoning(0, "it."),
            reasoning(0, "Summary."),
            text(1, "Done."),
            LlmEvent::ReplayState(json!({
                "reasoning_key": "reasoning",
                "reasoning": "Plan it.",
                "reasoning_details": [
                    { "type": "reasoning.text", "text": "Plan it.", "index": 0, "format": "anthropic-claude-v1", "signature": "sig-1" },
                    { "type": "reasoning.summary", "summary": "Summary.", "index": 1 },
                    { "type": "reasoning.encrypted", "data": "opaque", "index": 2 },
                ],
            })),
            finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn reassembles_streamed_tool_call_arguments_by_index() {
    let (events, _) = decode(&[
        &delta(&json!({ "content": "Checking." })),
        &delta(&json!({ "tool_calls": [
            { "index": 0, "id": "call_a", "type": "function", "function": { "name": "weather__get", "arguments": "" } },
        ] })),
        &delta(&json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{\"locat" } }] })),
        &delta(&json!({ "tool_calls": [
            { "index": 1, "id": "call_b", "type": "function", "function": { "name": "clock__now", "arguments": "{}" } },
        ] })),
        &delta(
            &json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "ion\":\"Paris\"}" } }] }),
        ),
        &finish_chunk("tool_calls"),
        r#"{"choices":[],"usage":{"prompt_tokens":82,"completion_tokens":18}}"#,
        "[DONE]",
    ]);
    assert_eq!(
        ok(events),
        [
            text(0, "Checking."),
            started(1, Some("call_a"), "weather__get"),
            args(1, "{\"locat"),
            started(2, Some("call_b"), "clock__now"),
            args(2, "{}"),
            args(1, "ion\":\"Paris\"}"),
            LlmEvent::ToolCallEnded { index: 1 },
            LlmEvent::ToolCallEnded { index: 2 },
            LlmEvent::Usage(Usage {
                input_tokens: Some(82),
                output_tokens: Some(18),
                ..Usage::default()
            }),
            finished(FinishReason::ToolCalls),
        ]
    );
}

#[test]
fn whole_calls_without_index_or_with_index_zero_each_start_a_call() {
    // Some servers number every whole call 0; others leave the index out.
    for calls in [
        json!([
            { "index": 0, "id": "call_a", "function": { "name": "a__x", "arguments": "{\"n\":1}" } },
            { "index": 0, "id": "call_b", "function": { "name": "b__y", "arguments": "{\"n\":2}" } },
        ]),
        json!([
            { "id": "call_a", "function": { "name": "a__x", "arguments": "{\"n\":1}" } },
            { "id": "call_b", "function": { "name": "b__y", "arguments": "{\"n\":2}" } },
        ]),
    ] {
        let (events, _) = decode(&[&delta(&json!({ "tool_calls": calls })), "[DONE]"]);
        assert_eq!(
            ok(events),
            [
                started(0, Some("call_a"), "a__x"),
                args(0, "{\"n\":1}"),
                started(1, Some("call_b"), "b__y"),
                args(1, "{\"n\":2}"),
                LlmEvent::ToolCallEnded { index: 0 },
                LlmEvent::ToolCallEnded { index: 1 },
                finished(FinishReason::ToolCalls),
            ]
        );
    }
}

#[test]
fn a_stop_finish_with_calls_reports_tool_calls() {
    let (events, _) = decode(&[
        &delta(
            &json!({ "tool_calls": [{ "index": 0, "id": "call_a", "function": { "name": "a__x", "arguments": "{}" } }] }),
        ),
        &finish_chunk("stop"),
        "[DONE]",
    ]);
    assert_eq!(ok(events).last(), Some(&finished(FinishReason::ToolCalls)));

    let (events, _) = decode(&[
        &delta(
            &json!({ "tool_calls": [{ "index": 0, "id": "call_a", "function": { "name": "a__x", "arguments": "{\"a\":" } }] }),
        ),
        &finish_chunk("length"),
        "[DONE]",
    ]);
    assert_eq!(
        ok(events).last(),
        Some(&finished(FinishReason::Length)),
        "a cut-off call keeps the length finish"
    );
}

#[test]
fn text_after_a_tool_call_starts_a_new_part() {
    let (events, _) = decode(&[
        &delta(&json!({ "content": "Before." })),
        &delta(
            &json!({ "tool_calls": [{ "index": 0, "id": "call_a", "function": { "name": "a__x" } }] }),
        ),
        &delta(&json!({ "content": "After." })),
        "[DONE]",
    ]);
    let events = ok(events);
    assert_eq!(events[0], text(0, "Before."));
    assert_eq!(events[1], started(1, Some("call_a"), "a__x"));
    assert_eq!(events[2], text(2, "After."));
}

#[test]
fn arguments_after_the_finish_are_not_reported() {
    let (events, _) = decode(&[
        &delta(
            &json!({ "tool_calls": [{ "index": 0, "id": "call_a", "function": { "name": "a__x", "arguments": "{}" } }] }),
        ),
        &finish_chunk("tool_calls"),
        &delta(&json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "late" } }] })),
        "[DONE]",
    ]);
    assert!(
        !ok(events).contains(&args(0, "late")),
        "an ended call takes no more arguments"
    );
}

#[test]
fn gemini_keeps_extra_call_fields_for_replay_and_others_drop_them() {
    let call = json!({ "tool_calls": [{
        "id": "function-call-1",
        "type": "function",
        "function": { "name": "files__read", "arguments": "{\"path\":\"a\"}" },
        "extra_content": { "google": { "thought_signature": "sig-1" } },
    }] });
    let (events, _) = decode_as(
        Dialect::Gemini,
        &[&delta(&call), &finish_chunk("stop"), "[DONE]"],
    );
    assert!(ok(events).contains(&LlmEvent::ReplayState(json!({
        "tool_call_extra": {
            "function-call-1": { "extra_content": { "google": { "thought_signature": "sig-1" } } },
        },
    }))));

    let (events, _) = decode(&[&delta(&call), &finish_chunk("stop"), "[DONE]"]);
    assert!(
        !ok(events)
            .iter()
            .any(|event| matches!(event, LlmEvent::ReplayState(_))),
        "nothing to replay outside the gemini dialect"
    );
}

#[test]
fn spoken_replies_report_pcm_bytes_and_transcript_pieces() {
    let (events, _) = decode(&[
        &delta(
            &json!({ "role": "assistant", "content": null, "audio": { "id": "audio_1", "transcript": "Hel" } }),
        ),
        &delta(&json!({ "audio": { "data": "AAEC" } })),
        &delta(&json!({ "audio": { "transcript": "lo", "data": "AwQ=" } })),
        &delta(&json!({ "audio": { "expires_at": 1_700_000_000 } })),
        &finish_chunk("stop"),
        "[DONE]",
    ]);
    assert_eq!(
        ok(events),
        [
            LlmEvent::AudioDelta {
                data: Bytes::new(),
                transcript: Some("Hel".to_owned()),
            },
            LlmEvent::AudioDelta {
                data: Bytes::from_static(&[0, 1, 2]),
                transcript: None,
            },
            LlmEvent::AudioDelta {
                data: Bytes::from_static(&[3, 4]),
                transcript: Some("lo".to_owned()),
            },
            finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn audio_that_is_not_base64_fails_the_answer() {
    let (events, flow) = decode(&[&delta(&json!({ "audio": { "data": "not base64!" } }))]);
    assert_eq!(flow, Flow::Done);
    assert_eq!(
        events[0].as_ref().expect_err("invalid").code(),
        "PROVIDER_BAD_RESPONSE"
    );
}

#[test]
fn replay_state_past_its_limit_fails_the_answer() {
    let big = "x".repeat(REPLAY_MAX_BYTES / 2);
    let fragment =
        delta(&json!({ "reasoning_details": [{ "type": "reasoning.encrypted", "data": big }] }));
    let (events, flow) = decode_as(Dialect::OpenRouter, &[&fragment, &fragment, &fragment]);
    assert_eq!(flow, Flow::Done);
    let error = events
        .last()
        .expect("events")
        .as_ref()
        .expect_err("too large");
    assert_eq!(error.code(), "PROVIDER_BAD_RESPONSE");
    assert_eq!(
        error.to_string(),
        "The provider sent a reply over the size limit."
    );
}
