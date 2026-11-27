//! Request bodies per dialect, part, and replay state.

use bytes::Bytes;
use copilot_core::{
    AudioOut, BaseUrl, CallId, NetworkScope, Preset, ProviderId, ProviderKind, ProviderTarget,
    ToolAlias,
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

fn message(role: Role, parts: Vec<LlmPart>) -> LlmMessage {
    LlmMessage {
        role,
        parts,
        replay: None,
    }
}

fn text(role: Role, parts: &[&str]) -> LlmMessage {
    message(
        role,
        parts
            .iter()
            .map(|text| LlmPart::Text((*text).to_owned()))
            .collect(),
    )
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

fn spec(alias: &str, schema: Value) -> ToolSpec {
    ToolSpec {
        alias: ToolAlias::restore(alias.to_owned()),
        description: format!("The {alias} tool."),
        input_schema: schema,
    }
}

fn call(id: &str, alias: &str, arguments: Value) -> LlmPart {
    LlmPart::ToolCall {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: alias.to_owned(),
        arguments,
    }
}

fn result(id: &str, content: Vec<ToolContent>, is_error: bool) -> LlmPart {
    LlmPart::ToolResult {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: "files__read".to_owned(),
        is_error,
        content,
    }
}

fn png() -> LlmPart {
    LlmPart::Image {
        media_type: "image/png".to_owned(),
        data: Bytes::from_static(b"\x89PNG"),
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
        let body =
            body(&target(preset), &request(vec![text(Role::User, &["Hi"])])).expect("text only");
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
            text(Role::Assistant, &["First.", "Second."]),
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
        json!({ "role": "assistant", "content": "First.\n\nSecond." })
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
        for preset in [
            Preset::OpenAi,
            Preset::Ollama,
            Preset::Gemini,
            Preset::Custom,
        ] {
            let body = body(&target(preset), &request).expect("text only");
            assert_eq!(body["reasoning_effort"], effort, "{preset:?}");
            assert!(body.get("reasoning").is_none());
        }
        let body = body(&target(Preset::OpenRouter), &request).expect("text only");
        assert_eq!(body["reasoning"], json!({ "effort": effort }));
        assert!(body.get("reasoning_effort").is_none());
    }
    let body = body(
        &target(Preset::OpenAi),
        &request(vec![text(Role::User, &["Hi"])]),
    )
    .expect("text only");
    assert!(
        body.get("reasoning_effort").is_none(),
        "the model default sends nothing"
    );
}

#[test]
fn sends_tools_as_functions_with_object_schemas() {
    let mut request = request(vec![text(Role::User, &["Read a.txt"])]);
    request.tools = vec![
        spec(
            "files__read",
            json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }),
        ),
        spec("clock__now", json!({})),
        spec("odd__tool", json!("not a schema")),
    ];
    let body = body(&target(Preset::OpenAi), &request).expect("tools");
    assert_eq!(
        body["tools"],
        json!([
            {
                "type": "function",
                "function": {
                    "name": "files__read",
                    "description": "The files__read tool.",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                    },
                    "strict": false,
                },
            },
            {
                "type": "function",
                "function": {
                    "name": "clock__now",
                    "description": "The clock__now tool.",
                    "parameters": { "type": "object", "properties": {} },
                    "strict": false,
                },
            },
            {
                "type": "function",
                "function": {
                    "name": "odd__tool",
                    "description": "The odd__tool tool.",
                    "parameters": { "type": "object", "properties": {} },
                    "strict": false,
                },
            },
        ])
    );
    assert_eq!(body["tool_choice"], "auto");
}

#[test]
fn ollama_never_gets_tool_choice_and_no_tools_send_neither() {
    let mut with_tools = request(vec![text(Role::User, &["Hi"])]);
    with_tools.tools = vec![spec("clock__now", json!({ "type": "object" }))];
    let body = body(&target(Preset::Ollama), &with_tools).expect("tools");
    assert!(body["tools"].is_array());
    assert!(body.get("tool_choice").is_none());

    let plain = super::body(
        &target(Preset::OpenAi),
        &request(vec![text(Role::User, &["Hi"])]),
    )
    .expect("text");
    assert!(plain.get("tools").is_none());
    assert!(plain.get("tool_choice").is_none());
}

#[test]
fn sends_tool_calls_and_one_tool_message_per_result() {
    let body = body(
        &target(Preset::Custom),
        &request(vec![
            text(Role::User, &["Read both"]),
            message(
                Role::Assistant,
                vec![
                    call("call_a", "files__read", json!({ "path": "a.txt" })),
                    call("call_b", "files__read", json!({})),
                ],
            ),
            message(
                Role::Tool,
                vec![
                    result(
                        "call_a",
                        vec![
                            ToolContent::Text("line 1".to_owned()),
                            ToolContent::Text("line 2".to_owned()),
                        ],
                        false,
                    ),
                    result(
                        "call_b",
                        vec![ToolContent::Text("No such file.".to_owned())],
                        true,
                    ),
                    LlmPart::Text("[result: lowered to text]".to_owned()),
                ],
            ),
            text(Role::User, &["And now?"]),
        ]),
    )
    .expect("tool round");
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(
        messages[2],
        json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [
                { "id": "call_a", "type": "function", "function": { "name": "files__read", "arguments": "{\"path\":\"a.txt\"}" } },
                { "id": "call_b", "type": "function", "function": { "name": "files__read", "arguments": "{}" } },
            ],
        })
    );
    assert_eq!(
        messages[3],
        json!({ "role": "tool", "tool_call_id": "call_a", "content": "line 1\nline 2" })
    );
    assert_eq!(
        messages[4],
        json!({ "role": "tool", "tool_call_id": "call_b", "content": "No such file." })
    );
    assert_eq!(
        messages[5],
        json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "[result: lowered to text]" },
                { "type": "text", "text": "And now?" },
            ],
        }),
        "lowered results and the next user message merge into one user message"
    );
    assert_eq!(messages.len(), 6);
}

#[test]
fn an_answer_with_text_and_calls_keeps_both() {
    let body = body(
        &target(Preset::Custom),
        &request(vec![
            text(Role::User, &["Hi"]),
            message(
                Role::Assistant,
                vec![
                    LlmPart::Text("Let me look.".to_owned()),
                    call("call_a", "files__read", json!({ "path": "a" })),
                ],
            ),
            message(
                Role::Tool,
                vec![result(
                    "call_a",
                    vec![ToolContent::Text("a".to_owned())],
                    false,
                )],
            ),
        ]),
    )
    .expect("tool round");
    assert_eq!(body["messages"][2]["content"], "Let me look.");
    assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call_a");
}

#[test]
fn tool_results_carry_text_only() {
    let body = body(
        &target(Preset::OpenAi),
        &request(vec![
            text(Role::User, &["Shoot"]),
            message(
                Role::Assistant,
                vec![call("call_a", "cam__shoot", json!({}))],
            ),
            message(
                Role::Tool,
                vec![result(
                    "call_a",
                    vec![
                        ToolContent::Text("Saved.".to_owned()),
                        ToolContent::Image {
                            media_type: "image/png".to_owned(),
                            data: Bytes::from_static(b"\x89PNG"),
                        },
                        ToolContent::Link {
                            uri: "https://example.com/a".to_owned(),
                            name: Some("a".to_owned()),
                        },
                        ToolContent::Binary { media_type: None },
                    ],
                    false,
                )],
            ),
        ]),
    )
    .expect("tool round");
    assert_eq!(
        body["messages"][3]["content"],
        "Saved.\n[image omitted: tool results reach this model as text only]\na: https://example.com/a\n[binary content omitted: tool results reach this model as text only]"
    );
}

#[test]
fn sends_images_pdfs_and_audio_as_user_parts() {
    let body = body(
        &target(Preset::OpenAi),
        &request(vec![message(
            Role::User,
            vec![
                LlmPart::Text("Describe these.".to_owned()),
                png(),
                LlmPart::Pdf {
                    name: "report.pdf".to_owned(),
                    data: Bytes::from_static(b"%PDF-1.7"),
                },
                LlmPart::Audio {
                    format: "wav".to_owned(),
                    data: Bytes::from_static(b"RIFF"),
                },
                LlmPart::Audio {
                    format: "mp3".to_owned(),
                    data: Bytes::from_static(b"ID3"),
                },
            ],
        )]),
    )
    .expect("binary parts");
    assert_eq!(
        body["messages"][1]["content"],
        json!([
            { "type": "text", "text": "Describe these." },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw==" } },
            { "type": "file", "file": { "filename": "report.pdf", "file_data": "data:application/pdf;base64,JVBERi0xLjc=" } },
            { "type": "input_audio", "input_audio": { "data": "UklGRg==", "format": "wav" } },
            { "type": "input_audio", "input_audio": { "data": "SUQz", "format": "mp3" } },
        ])
    );
}

#[test]
fn a_single_image_still_sends_an_array() {
    let body = body(
        &target(Preset::Ollama),
        &request(vec![message(Role::User, vec![png()])]),
    )
    .expect("image");
    assert!(body["messages"][1]["content"].is_array());
}

#[test]
fn asks_for_spoken_replies() {
    let mut request = request(vec![text(Role::User, &["Say hi"])]);
    request.audio_out = Some(AudioOut {
        voice: "alloy".to_owned(),
        format: "pcm16".to_owned(),
    });
    let body = body(&target(Preset::OpenAi), &request).expect("audio out");
    assert_eq!(body["modalities"], json!(["text", "audio"]));
    assert_eq!(
        body["audio"],
        json!({ "voice": "alloy", "format": "pcm16" })
    );
}

#[test]
fn replays_reasoning_under_the_key_it_came_under() {
    for key in ["reasoning", "reasoning_content"] {
        let mut answer = text(Role::Assistant, &["Answer."]);
        answer.replay = Some(json!({ "reasoning_key": key, "reasoning": "Thought." }));
        let body = body(
            &target(Preset::Custom),
            &request(vec![
                text(Role::User, &["Q"]),
                answer,
                text(Role::User, &["Q2"]),
            ]),
        )
        .expect("replay");
        assert_eq!(
            body["messages"][2],
            json!({ "role": "assistant", "content": "Answer.", key: "Thought." })
        );
    }
}

#[test]
fn openrouter_replays_reasoning_details_unchanged() {
    let details = json!([
        { "type": "reasoning.text", "text": "Thought.", "signature": "sig", "index": 0, "format": "anthropic-claude-v1" },
        { "type": "reasoning.encrypted", "data": "opaque", "index": 1 },
    ]);
    let mut answer = message(
        Role::Assistant,
        vec![call("toolu_1", "files__read", json!({ "path": "a" }))],
    );
    answer.replay = Some(json!({
        "reasoning_key": "reasoning",
        "reasoning": "Thought.",
        "reasoning_details": details,
    }));
    let messages = vec![
        text(Role::User, &["Q"]),
        answer,
        message(
            Role::Tool,
            vec![result(
                "toolu_1",
                vec![ToolContent::Text("a".to_owned())],
                false,
            )],
        ),
    ];
    let body = body(&target(Preset::OpenRouter), &request(messages.clone())).expect("replay");
    assert_eq!(body["messages"][2]["reasoning_details"], details);
    assert!(
        body["messages"][2].get("reasoning").is_none(),
        "the details replace the plain text"
    );

    // Other dialects never send the details, only the text.
    let body = super::body(&target(Preset::Custom), &request(messages)).expect("replay");
    assert!(body["messages"][2].get("reasoning_details").is_none());
    assert_eq!(body["messages"][2]["reasoning"], "Thought.");
}

#[test]
fn gemini_replays_extra_call_fields_but_never_over_the_call_itself() {
    let mut answer = message(
        Role::Assistant,
        vec![
            call("function-call-1", "files__read", json!({ "path": "a" })),
            call("function-call-2", "files__read", json!({ "path": "b" })),
        ],
    );
    answer.replay = Some(json!({
        "tool_call_extra": {
            "function-call-1": {
                "extra_content": { "google": { "thought_signature": "sig-1" } },
                "id": "forged",
                "function": { "name": "forged" },
            },
        },
    }));
    let messages = vec![
        text(Role::User, &["Q"]),
        answer,
        message(
            Role::Tool,
            vec![
                result(
                    "function-call-1",
                    vec![ToolContent::Text("a".to_owned())],
                    false,
                ),
                result(
                    "function-call-2",
                    vec![ToolContent::Text("b".to_owned())],
                    false,
                ),
            ],
        ),
    ];
    let body = body(&target(Preset::Gemini), &request(messages.clone())).expect("replay");
    assert_eq!(
        body["messages"][2]["tool_calls"],
        json!([
            {
                "extra_content": { "google": { "thought_signature": "sig-1" } },
                "id": "function-call-1",
                "type": "function",
                "function": { "name": "files__read", "arguments": "{\"path\":\"a\"}" },
            },
            {
                "id": "function-call-2",
                "type": "function",
                "function": { "name": "files__read", "arguments": "{\"path\":\"b\"}" },
            },
        ])
    );

    let body = super::body(&target(Preset::Custom), &request(messages)).expect("replay");
    assert!(
        body["messages"][2]["tool_calls"][0]
            .get("extra_content")
            .is_none(),
        "only the gemini dialect keeps extra call fields"
    );
}

#[test]
fn gemini_signs_unsigned_steps_with_the_documented_stand_in() {
    let rebuilt = message(
        Role::Assistant,
        vec![
            call("call_1", "files__read", json!({ "path": "a" })),
            call("call_2", "files__read", json!({ "path": "b" })),
        ],
    );
    let messages = vec![
        text(Role::User, &["Q"]),
        rebuilt,
        message(
            Role::Tool,
            vec![
                result("call_1", vec![ToolContent::Text("a".to_owned())], false),
                result("call_2", vec![ToolContent::Text("b".to_owned())], false),
            ],
        ),
    ];
    let body = body(&target(Preset::Gemini), &request(messages.clone())).expect("a body");
    let calls = &body["messages"][2]["tool_calls"];
    assert_eq!(
        calls[0]["extra_content"],
        json!({ "google": { "thought_signature": "skip_thought_signature_validator" } })
    );
    assert!(
        calls[1].get("extra_content").is_none(),
        "only the first call of a step"
    );

    let body = super::body(&target(Preset::OpenRouter), &request(messages)).expect("a body");
    assert!(
        body["messages"][2]["tool_calls"][0]
            .get("extra_content")
            .is_none()
    );
}

#[test]
fn an_unreadable_replay_state_replays_nothing_extra() {
    let mut answer = text(Role::Assistant, &["Answer."]);
    answer.replay = Some(json!("garbage"));
    let body = body(
        &target(Preset::OpenRouter),
        &request(vec![
            text(Role::User, &["Q"]),
            answer,
            text(Role::User, &["Q2"]),
        ]),
    )
    .expect("replay");
    assert_eq!(
        body["messages"][2],
        json!({ "role": "assistant", "content": "Answer." })
    );
}

#[test]
fn refuses_what_the_protocol_cannot_carry() {
    let cases = [
        (
            message(
                Role::User,
                vec![LlmPart::Audio {
                    format: "ogg".to_owned(),
                    data: Bytes::from_static(b"OggS"),
                }],
            ),
            "This connection cannot send this audio format.",
        ),
        (
            message(Role::Assistant, vec![png()]),
            "This connection cannot send files in an answer.",
        ),
        (
            message(Role::Tool, vec![png()]),
            "This connection cannot send files in a tool result.",
        ),
        (
            message(Role::User, vec![call("call_a", "x", json!({}))]),
            "This connection cannot send tool parts in a user message.",
        ),
    ];
    for (item, expected) in cases {
        let error = body(&target(Preset::OpenAi), &request(vec![item])).expect_err(expected);
        assert_eq!(error.code(), UNSUPPORTED);
        assert_eq!(error.to_string(), expected);
    }
}
