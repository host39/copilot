//! Messages API request bodies: system, tools, thinking per level, parts, tool rounds, replay,
//! merging, and cache breakpoints.

use bytes::Bytes;
use copilot_core::{CallId, ToolAlias};

use super::*;

fn message(role: Role, parts: Vec<LlmPart>) -> LlmMessage {
    LlmMessage {
        role,
        parts,
        replay: None,
    }
}

fn text(text: &str) -> LlmPart {
    LlmPart::Text(text.to_owned())
}

fn request(messages: Vec<LlmMessage>) -> LlmRequest {
    LlmRequest {
        model: "claude-example-1".to_owned(),
        system: "Be brief.".to_owned(),
        messages,
        tools: Vec::new(),
        max_output_tokens: 4_096,
        reasoning: None,
        audio_out: None,
        cache_breakpoints: false,
    }
}

fn call(id: &str) -> LlmPart {
    LlmPart::ToolCall {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: "weather__get".to_owned(),
        arguments: json!({ "location": "Paris" }),
    }
}

fn result(id: &str, is_error: bool, content: Vec<ToolContent>) -> LlmPart {
    LlmPart::ToolResult {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: "weather__get".to_owned(),
        is_error,
        content,
    }
}

#[test]
fn sends_the_required_fields_and_no_sampling_parameters() {
    let body = body(&request(vec![message(Role::User, vec![text("Hi")])])).expect("a body");
    assert_eq!(
        body,
        json!({
            "model": "claude-example-1",
            "max_tokens": 4_096,
            "stream": true,
            "system": "Be brief.",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Hi" }] }],
        })
    );
    for field in [
        "temperature",
        "top_p",
        "top_k",
        "tool_choice",
        "thinking",
        "output_config",
    ] {
        assert!(body.get(field).is_none(), "{field} is not sent");
    }

    let mut silent = request(vec![message(Role::User, vec![text("Hi")])]);
    silent.system.clear();
    assert!(
        body_without_system(&silent),
        "an empty system text is left out"
    );
}

fn body_without_system(request: &LlmRequest) -> bool {
    body(request).expect("a body").get("system").is_none()
}

#[test]
fn maps_each_reasoning_level_to_thinking() {
    let mut low = request(vec![message(Role::User, vec![text("Hi")])]);
    low.reasoning = Some(ReasoningLevel::Low);
    let body_low = body(&low).expect("a body");
    assert_eq!(
        body_low["thinking"],
        json!({ "type": "adaptive", "display": "summarized" })
    );
    assert_eq!(body_low["output_config"], json!({ "effort": "low" }));
    assert!(body_low["thinking"].get("budget_tokens").is_none());

    for (level, effort) in [
        (ReasoningLevel::Medium, "medium"),
        (ReasoningLevel::High, "high"),
    ] {
        let mut leveled = low.clone();
        leveled.reasoning = Some(level);
        assert_eq!(
            body(&leveled).expect("a body")["output_config"]["effort"],
            effort
        );
    }

    let mut off = low.clone();
    off.reasoning = Some(ReasoningLevel::Off);
    let body_off = body(&off).expect("a body");
    assert_eq!(body_off["thinking"], json!({ "type": "disabled" }));
    assert!(body_off.get("output_config").is_none());
}

#[test]
fn sends_tools_with_object_schemas() {
    let mut with_tools = request(vec![message(Role::User, vec![text("Weather?")])]);
    with_tools.tools = vec![
        ToolSpec {
            alias: ToolAlias::restore("weather__get".to_owned()),
            description: "Current weather.".to_owned(),
            input_schema: json!({ "type": "object", "properties": { "location": { "type": "string" } } }),
        },
        ToolSpec {
            alias: ToolAlias::restore("clock__now".to_owned()),
            description: " ".to_owned(),
            input_schema: json!(null),
        },
    ];
    let body = body(&with_tools).expect("a body");
    assert_eq!(
        body["tools"],
        json!([
            {
                "name": "weather__get",
                "description": "Current weather.",
                "input_schema": { "type": "object", "properties": { "location": { "type": "string" } } },
            },
            { "name": "clock__now", "input_schema": { "type": "object", "properties": {} } },
        ])
    );
    assert!(body.get("tool_choice").is_none());
}

#[test]
fn sends_images_and_pdfs_as_base64_blocks() {
    let body = body(&request(vec![message(
        Role::User,
        vec![
            LlmPart::Image {
                media_type: "image/png".to_owned(),
                data: Bytes::from_static(b"png"),
            },
            LlmPart::Pdf {
                name: "report.pdf".to_owned(),
                data: Bytes::from_static(b"%PDF"),
            },
            text(""),
            text("Summarize."),
        ],
    )]))
    .expect("a body");
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "cG5n" } },
            {
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": "JVBERg==" },
                "title": "report.pdf",
            },
            { "type": "text", "text": "Summarize." },
        ])
    );
}

#[test]
fn refuses_what_the_messages_api_cannot_carry() {
    let cases = [
        (
            message(
                Role::User,
                vec![LlmPart::Audio {
                    format: "wav".to_owned(),
                    data: Bytes::from_static(b"RIFF"),
                }],
            ),
            "This connection cannot send audio.",
        ),
        (
            message(
                Role::User,
                vec![LlmPart::Image {
                    media_type: "image/bmp".to_owned(),
                    data: Bytes::from_static(b"BM"),
                }],
            ),
            "This connection cannot send this image type.",
        ),
        (
            message(Role::User, vec![call("toolu_1")]),
            "This connection cannot send tool parts in a user message.",
        ),
        (
            message(
                Role::Assistant,
                vec![LlmPart::Pdf {
                    name: "a.pdf".to_owned(),
                    data: Bytes::from_static(b"%PDF"),
                }],
            ),
            "This connection cannot send files in an answer.",
        ),
        (
            message(Role::Tool, vec![call("toolu_1")]),
            "This connection cannot send files in a tool result.",
        ),
    ];
    for (item, expected) in cases {
        let error = body(&request(vec![item])).expect_err("unsupported");
        assert_eq!(error.code(), UNSUPPORTED);
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn never_sends_an_assistant_prefill() {
    let error = body(&request(vec![
        message(Role::User, vec![text("Hi")]),
        message(Role::Assistant, vec![text("Partial answer")]),
    ]))
    .expect_err("a prefill");
    assert_eq!(error.code(), UNSUPPORTED);
    assert_eq!(
        error.to_string(),
        "This connection cannot send a request that ends with an answer."
    );
}

#[test]
fn sends_a_tool_round_with_results_first_and_lowered_text_after() {
    let body = body(&request(vec![
        message(Role::User, vec![text("Weather in Paris and Rome?")]),
        message(
            Role::Assistant,
            vec![text("Checking."), call("toolu_1"), call("call_2")],
        ),
        message(
            Role::Tool,
            vec![
                result(
                    "toolu_1",
                    false,
                    vec![ToolContent::Text("15 degrees".to_owned())],
                ),
                result(
                    "call_2",
                    true,
                    vec![
                        ToolContent::Text("Timed out.".to_owned()),
                        ToolContent::Image {
                            media_type: "image/webp".to_owned(),
                            data: Bytes::from_static(b"webp"),
                        },
                        ToolContent::Image {
                            media_type: "image/tiff".to_owned(),
                            data: Bytes::from_static(b"tiff"),
                        },
                        ToolContent::Audio {
                            media_type: "audio/wav".to_owned(),
                            data: Bytes::from_static(b"RIFF"),
                        },
                        ToolContent::Link {
                            uri: "https://example.com/a".to_owned(),
                            name: Some("a".to_owned()),
                        },
                        ToolContent::Link {
                            uri: "https://example.com/b".to_owned(),
                            name: None,
                        },
                        ToolContent::Binary { media_type: None },
                    ],
                ),
                text("[result: lowered]"),
            ],
        ),
        message(Role::User, vec![text("Thanks.")]),
    ]))
    .expect("a body");
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(
        messages.len(),
        3,
        "the tool message and the next user message merge"
    );
    assert_eq!(
        messages[1],
        json!({ "role": "assistant", "content": [
            { "type": "text", "text": "Checking." },
            { "type": "tool_use", "id": "toolu_1", "name": "weather__get", "input": { "location": "Paris" } },
            { "type": "tool_use", "id": "call_2", "name": "weather__get", "input": { "location": "Paris" } },
        ] })
    );
    assert_eq!(
        messages[2],
        json!({ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": [{ "type": "text", "text": "15 degrees" }] },
            { "type": "tool_result", "tool_use_id": "call_2", "is_error": true, "content": [
                { "type": "text", "text": "Timed out." },
                { "type": "image", "source": { "type": "base64", "media_type": "image/webp", "data": "d2VicA==" } },
                { "type": "text", "text": "[image omitted: this model cannot read this image type]" },
                { "type": "text", "text": "[audio omitted: this model cannot read audio]" },
                { "type": "text", "text": "a: https://example.com/a" },
                { "type": "text", "text": "https://example.com/b" },
                { "type": "text", "text": "[binary content omitted: this model cannot read it]" },
            ] },
            { "type": "text", "text": "[result: lowered]" },
            { "type": "text", "text": "Thanks." },
        ] })
    );
}

#[test]
fn a_result_without_content_sends_no_content_field() {
    let body = body(&request(vec![
        message(Role::User, vec![text("Run it.")]),
        message(Role::Assistant, vec![call("toolu_1")]),
        message(
            Role::Tool,
            vec![result(
                "toolu_1",
                false,
                vec![ToolContent::Text(String::new())],
            )],
        ),
    ]))
    .expect("a body");
    assert_eq!(
        body["messages"][2]["content"][0],
        json!({ "type": "tool_result", "tool_use_id": "toolu_1" })
    );
}

#[test]
fn rebuilt_call_ids_fit_the_tool_use_id_pattern() {
    // Core rebuilds ids as `call_<uuid without dashes>`; the API takes `^[a-zA-Z0-9_-]+$`.
    let id = format!("call_{}", CallId::generate().uuid().simple());
    assert!(
        id.chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-')
    );
}

#[test]
fn replays_stored_content_blocks_verbatim() {
    let stored = json!([
        { "type": "thinking", "thinking": "Let me check.", "signature": "c2ln" },
        { "type": "redacted_thinking", "data": "b3BhcXVl" },
        { "type": "text", "text": "Checking." },
        { "type": "tool_use", "id": "toolu_1", "name": "weather__get", "input": { "location": "Paris" } },
    ]);
    let mut answer = message(Role::Assistant, vec![text("Checking."), call("toolu_1")]);
    answer.replay = Some(stored.clone());
    let body = body(&request(vec![
        message(Role::User, vec![text("Weather?")]),
        answer,
        message(
            Role::Tool,
            vec![result(
                "toolu_1",
                false,
                vec![ToolContent::Text("15".to_owned())],
            )],
        ),
    ]))
    .expect("a body");
    assert_eq!(body["messages"][1]["content"], stored);
}

#[test]
fn an_unreadable_replay_state_rebuilds_the_answer() {
    for state in [
        json!({ "reasoning": "openai-shaped" }),
        json!([]),
        json!([{ "text": "no type" }]),
        json!("text"),
    ] {
        let mut answer = message(Role::Assistant, vec![text("Hello.")]);
        answer.replay = Some(state);
        let body = body(&request(vec![
            message(Role::User, vec![text("Hi")]),
            answer,
            message(Role::User, vec![text("Again")]),
        ]))
        .expect("a body");
        assert_eq!(
            body["messages"][1]["content"],
            json!([{ "type": "text", "text": "Hello." }])
        );
    }
}

#[test]
fn marks_the_system_the_first_user_message_and_the_last_block() {
    let mut cached = request(vec![
        message(
            Role::User,
            vec![
                text("<file name=\"notes.md\">\nNotes\n</file>"),
                text("First question"),
            ],
        ),
        message(Role::Assistant, vec![text("First answer")]),
        message(Role::User, vec![text("Second question")]),
    ]);
    cached.cache_breakpoints = true;
    let body = body(&cached).expect("a body");
    let marker = json!({ "type": "ephemeral" });
    assert_eq!(
        body["system"],
        json!([{ "type": "text", "text": "Be brief.", "cache_control": marker }])
    );
    let messages = &body["messages"];
    assert!(messages[0]["content"][0].get("cache_control").is_none());
    assert_eq!(messages[0]["content"][1]["cache_control"], marker);
    assert!(messages[1]["content"][0].get("cache_control").is_none());
    assert_eq!(messages[2]["content"][0]["cache_control"], marker);

    let markers = body.to_string().matches("cache_control").count();
    assert!(markers <= 4, "at most four breakpoints");
}

#[test]
fn a_single_message_gets_one_breakpoint_and_none_without_caching() {
    let mut cached = request(vec![message(Role::User, vec![text("Only question")])]);
    cached.cache_breakpoints = true;
    let body_cached = body(&cached).expect("a body");
    assert_eq!(body_cached.to_string().matches("cache_control").count(), 2);
    assert_eq!(
        body_cached["messages"][0]["content"][0]["cache_control"],
        json!({ "type": "ephemeral" })
    );

    cached.cache_breakpoints = false;
    assert!(
        !body(&cached)
            .expect("a body")
            .to_string()
            .contains("cache_control")
    );
}

#[test]
fn the_last_breakpoint_lands_on_a_tool_result() {
    let mut cached = request(vec![
        message(Role::User, vec![text("Run it.")]),
        message(Role::Assistant, vec![call("toolu_1")]),
        message(
            Role::Tool,
            vec![result(
                "toolu_1",
                false,
                vec![ToolContent::Text("done".to_owned())],
            )],
        ),
    ]);
    cached.cache_breakpoints = true;
    let body = body(&cached).expect("a body");
    assert_eq!(
        body["messages"][2]["content"][0]["cache_control"],
        json!({ "type": "ephemeral" })
    );
    assert!(
        body["messages"][1]["content"][0]
            .get("cache_control")
            .is_none()
    );
}

#[test]
fn never_marks_thinking_blocks() {
    let mut block = Map::new();
    block.insert("type".to_owned(), Value::from("thinking"));
    assert!(!cacheable(&block));
    block.insert("type".to_owned(), Value::from("redacted_thinking"));
    assert!(!cacheable(&block));
    block.insert("type".to_owned(), Value::from("tool_result"));
    assert!(cacheable(&block));
}

#[test]
fn the_first_user_breakpoint_never_lands_on_an_answer() {
    let mut cached = request(vec![
        message(Role::User, vec![text("")]),
        message(Role::Assistant, vec![text("An answer")]),
        message(Role::User, vec![text("A question")]),
    ]);
    cached.cache_breakpoints = true;
    let body = body(&cached).expect("a body");
    assert!(
        body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none()
    );
    assert_eq!(
        body["messages"][1]["content"][0]["cache_control"],
        json!({ "type": "ephemeral" })
    );
}
