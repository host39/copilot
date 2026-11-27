//! Responses request bodies per part, reasoning setting, and replay state.

use bytes::Bytes;
use copilot_core::{AudioOut, CallId, ToolAlias};

use super::*;

fn message(role: Role, parts: Vec<LlmPart>) -> LlmMessage {
    LlmMessage {
        role,
        parts,
        replay: None,
    }
}

fn text(role: Role, text: &str) -> LlmMessage {
    message(role, vec![LlmPart::Text(text.to_owned())])
}

fn request(messages: Vec<LlmMessage>) -> LlmRequest {
    LlmRequest {
        model: "gpt-test".to_owned(),
        system: "Be brief.".to_owned(),
        messages,
        tools: Vec::new(),
        max_output_tokens: 1_024,
        reasoning: None,
        can_reason: false,
        audio_out: None,
        cache_breakpoints: false,
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

fn result(id: &str, content: Vec<ToolContent>) -> LlmPart {
    LlmPart::ToolResult {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: "docs__search".to_owned(),
        is_error: false,
        content,
    }
}

fn png() -> (String, Bytes) {
    ("image/png".to_owned(), Bytes::from_static(b"\x89PNG"))
}

#[test]
fn sends_a_stateless_streamed_request_with_instructions() {
    let body = body(&request(vec![text(Role::User, "Hi")])).expect("text only");
    assert_eq!(
        body,
        json!({
            "model": "gpt-test",
            "stream": true,
            "store": false,
            "instructions": "Be brief.",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Hi" }] },
            ],
            "max_output_tokens": 1_024,
        })
    );
    let mut tiny = request(vec![text(Role::User, "Hi")]);
    tiny.system = String::new();
    tiny.max_output_tokens = 1;
    let body = body_of(&tiny);
    assert!(body.get("instructions").is_none(), "no empty instructions");
    assert_eq!(body["max_output_tokens"], 16, "the API's minimum");
}

fn body_of(request: &LlmRequest) -> Value {
    body(request).expect("a body")
}

#[test]
fn asks_for_reasoning_summaries_and_encrypted_reasoning() {
    let mut reasoning = request(vec![text(Role::User, "Hi")]);
    reasoning.can_reason = true;
    let body = body_of(&reasoning);
    assert_eq!(body["reasoning"], json!({ "summary": "auto" }));
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));

    reasoning.reasoning = Some(ReasoningLevel::High);
    let body = body_of(&reasoning);
    assert_eq!(
        body["reasoning"],
        json!({ "effort": "high", "summary": "auto" })
    );

    reasoning.reasoning = Some(ReasoningLevel::Off);
    let body = body_of(&reasoning);
    assert_eq!(body["reasoning"], json!({ "effort": "none" }));
    assert!(body.get("include").is_none(), "no reasoning to keep");

    let plain = body_of(&request(vec![text(Role::User, "Hi")]));
    assert!(plain.get("reasoning").is_none(), "models without reasoning");
    assert!(plain.get("include").is_none());
}

#[test]
fn sends_function_tools_without_strict_mode() {
    let mut with_tools = request(vec![text(Role::User, "Search")]);
    with_tools.tools = vec![ToolSpec {
        alias: ToolAlias::restore("docs__search".to_owned()),
        description: "Search the docs.".to_owned(),
        input_schema: json!({ "properties": { "q": { "type": "string" } } }),
    }];
    let body = body_of(&with_tools);
    assert_eq!(
        body["tools"],
        json!([{
            "type": "function",
            "name": "docs__search",
            "description": "Search the docs.",
            "parameters": { "type": "object", "properties": { "q": { "type": "string" } } },
            "strict": false,
        }])
    );
    assert_eq!(body["tool_choice"], "auto");
}

#[test]
fn sends_images_and_pdfs_and_merges_user_messages() {
    let (media_type, data) = png();
    let body = body_of(&request(vec![
        text(Role::User, "Look"),
        message(
            Role::User,
            vec![
                LlmPart::Image { media_type, data },
                LlmPart::Pdf {
                    name: "report.pdf".to_owned(),
                    data: Bytes::from_static(b"%PDF"),
                },
            ],
        ),
    ]));
    assert_eq!(
        body["input"],
        json!([{
            "type": "message",
            "role": "user",
            "content": [
                { "type": "input_text", "text": "Look" },
                { "type": "input_image", "image_url": "data:image/png;base64,iVBORw==" },
                { "type": "input_file", "filename": "report.pdf", "file_data": "data:application/pdf;base64,JVBERg==" },
            ],
        }])
    );
}

#[test]
fn refuses_audio_before_sending() {
    let audio = message(
        Role::User,
        vec![LlmPart::Audio {
            format: "wav".to_owned(),
            data: Bytes::from_static(b"RIFF"),
        }],
    );
    let error = body(&request(vec![audio])).unwrap_err();
    assert_eq!(error.code(), UNSUPPORTED);
    assert_eq!(
        error.to_string(),
        "This connection cannot send audio through the Responses API."
    );
    let mut spoken = request(vec![text(Role::User, "Say hi")]);
    spoken.audio_out = Some(AudioOut {
        voice: "alloy".to_owned(),
        format: "pcm16".to_owned(),
    });
    assert_eq!(body(&spoken).unwrap_err().code(), UNSUPPORTED);
}

#[test]
fn rebuilds_an_answer_without_replay_from_its_parts() {
    let answer = message(
        Role::Assistant,
        vec![
            LlmPart::Text("Let me look.".to_owned()),
            LlmPart::Text("Searching.".to_owned()),
            call("call_abc", "docs__search", json!({ "q": "x" })),
        ],
    );
    let (media_type, data) = png();
    let results = message(
        Role::Tool,
        vec![
            result("call_abc", vec![ToolContent::Text("Found it".to_owned())]),
            LlmPart::Text("[file \"a.txt\" was deleted]".to_owned()),
        ],
    );
    let image = message(
        Role::Tool,
        vec![result(
            "call_img",
            vec![
                ToolContent::Text("A chart".to_owned()),
                ToolContent::Image { media_type, data },
            ],
        )],
    );
    let body = body_of(&request(vec![
        text(Role::User, "Find x"),
        answer,
        results,
        image,
    ]));
    assert_eq!(
        body["input"],
        json!([
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Find x" }] },
            { "type": "message", "role": "assistant", "content": "Let me look.\n\nSearching." },
            { "type": "function_call", "call_id": "call_abc", "name": "docs__search", "arguments": "{\"q\":\"x\"}" },
            { "type": "function_call_output", "call_id": "call_abc", "output": "Found it" },
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "[file \"a.txt\" was deleted]" }] },
            { "type": "function_call_output", "call_id": "call_img", "output": [
                { "type": "input_text", "text": "A chart" },
                { "type": "input_image", "image_url": "data:image/png;base64,iVBORw==" },
            ] },
        ])
    );
}

fn stored() -> Value {
    json!({
        "api": "responses",
        "items": [
            { "id": "rs_1", "type": "reasoning", "summary": [], "encrypted_content": "gAAAA" },
            { "id": "fc_1", "type": "function_call", "call_id": "call_abc", "name": "docs__search", "arguments": "{\"q\":\"x\"}", "status": "completed" },
        ],
    })
}

#[test]
fn replays_the_stored_items_of_an_answer() {
    let answer = LlmMessage {
        role: Role::Assistant,
        parts: vec![call("call_abc", "docs__search", json!({ "q": "x" }))],
        replay: Some(stored()),
    };
    let results = message(
        Role::Tool,
        vec![result(
            "call_abc",
            vec![ToolContent::Text("Found".to_owned())],
        )],
    );
    let body = body_of(&request(vec![text(Role::User, "Find x"), answer, results]));
    let input = body["input"].as_array().unwrap();
    assert_eq!(input[1], stored()["items"][0], "encrypted reasoning first");
    assert_eq!(input[2], stored()["items"][1], "the call with its item id");
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input.len(), 4);
}

#[test]
fn rebuilds_when_the_stored_items_do_not_hold_the_calls() {
    let other_call = LlmMessage {
        role: Role::Assistant,
        parts: vec![call("call_other", "docs__search", json!({}))],
        replay: Some(stored()),
    };
    let body = body_of(&request(vec![text(Role::User, "Find x"), other_call]));
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 2);
    assert_eq!(input[1]["call_id"], "call_other");
    assert!(input[1].get("id").is_none());

    let chat_state = LlmMessage {
        role: Role::Assistant,
        parts: vec![LlmPart::Text("Hi".to_owned())],
        replay: Some(json!({ "reasoning_key": "reasoning", "reasoning": "Hmm" })),
    };
    let body = body_of(&request(vec![
        text(Role::User, "Hi"),
        chat_state,
        text(Role::User, "Again"),
    ]));
    assert_eq!(
        body["input"][1],
        json!({ "type": "message", "role": "assistant", "content": "Hi" }),
        "a Chat Completions state rebuilds from the parts"
    );
}
