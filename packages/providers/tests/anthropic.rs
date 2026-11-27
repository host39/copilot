//! The Anthropic adapter against a wiremock provider, through the hardened outbound clients the
//! composition root injects (`copilot_security::clients(Purpose::Providers)`). The mock listens
//! on loopback over plain `http`, which only a private-scope connection may reach. Event shapes
//! follow the Messages API streaming reference (`docs/research/providers.md`, Anthropic).

use bytes::Bytes;
use copilot_core::{
    ANTHROPIC_VERSION, BaseUrl, CallId, Dialect, DomainResult, FinishReason, LlmEvent, LlmGateway,
    LlmMessage, LlmPart, LlmRequest, ModelTarget, NetworkScope, Preset, ProviderId, ProviderKind,
    ProviderTarget, ReasoningLevel, Refusal, Role, Secret, ToolAlias, ToolContent, ToolSpec, Usage,
};
use copilot_providers::{Gateway, UNSUPPORTED};
use copilot_security::{Purpose, clients};
use futures_util::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn gateway() -> Gateway {
    Gateway::new(clients(Purpose::Providers).expect("clients build"))
}

fn provider(base: &str, preset: Preset, key: Option<&str>) -> ProviderTarget {
    ProviderTarget {
        provider_id: ProviderId::generate(),
        kind: ProviderKind::Anthropic,
        preset,
        base_url: BaseUrl::parse(base, NetworkScope::Private).expect("a loopback URL"),
        api_key: key.map(Secret::new),
        network: NetworkScope::Private,
    }
}

fn target(server: &MockServer) -> ModelTarget {
    ModelTarget {
        provider: provider(&server.uri(), Preset::Anthropic, Some("sk-ant-test")),
        remote_id: "claude-example-1".to_owned(),
        dialect: Dialect::new(ProviderKind::Anthropic, Preset::Anthropic),
    }
}

fn request(text: &str) -> LlmRequest {
    LlmRequest {
        model: "claude-example-1".to_owned(),
        system: "Be brief.".to_owned(),
        messages: vec![message(
            Role::User,
            vec![LlmPart::Text(text.to_owned())],
            None,
        )],
        tools: Vec::new(),
        max_output_tokens: 1_024,
        reasoning: None,
        can_reason: false,
        audio_out: None,
        cache_breakpoints: true,
    }
}

fn message(role: Role, parts: Vec<LlmPart>, replay: Option<Value>) -> LlmMessage {
    LlmMessage {
        role,
        parts,
        replay,
    }
}

/// An SSE body of named events; each event's name is its data's `type`.
fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|event| {
            let name = event["type"].as_str().expect("a typed event");
            format!("event: {name}\ndata: {event}\n\n")
        })
        .collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn message_start(input_tokens: u32) -> Value {
    json!({ "type": "message_start", "message": {
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-example-1",
        "content": [], "stop_reason": null, "stop_sequence": null,
        "usage": { "input_tokens": input_tokens, "output_tokens": 1 },
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

fn text_delta(index: u64, text: &str) -> Value {
    delta(index, json!({ "type": "text_delta", "text": text }))
}

fn message_delta(reason: &str, output_tokens: u32) -> Value {
    json!({
        "type": "message_delta",
        "delta": { "stop_reason": reason, "stop_sequence": null },
        "usage": { "output_tokens": output_tokens },
    })
}

fn message_stop() -> Value {
    json!({ "type": "message_stop" })
}

/// A whole answer of one text block.
fn answer(text: &str) -> Vec<Value> {
    vec![
        message_start(10),
        start(0, json!({ "type": "text", "text": "" })),
        text_delta(0, text),
        stop(0),
        message_delta("end_turn", 4),
        message_stop(),
    ]
}

async fn stream_all(target: &ModelTarget, request: LlmRequest) -> Vec<DomainResult<LlmEvent>> {
    gateway()
        .stream(target, request)
        .await
        .expect("the stream opens")
        .collect()
        .await
}

fn ok(events: Vec<DomainResult<LlmEvent>>) -> Vec<LlmEvent> {
    events
        .into_iter()
        .map(|event| event.expect("no failure"))
        .collect()
}

fn replay_state(events: &[LlmEvent]) -> Value {
    events
        .iter()
        .find_map(|event| match event {
            LlmEvent::ReplayState(state) => Some(state.clone()),
            _ => None,
        })
        .expect("a replay state")
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.expect("recording")
}

fn weather() -> ToolSpec {
    ToolSpec {
        alias: ToolAlias::restore("weather__get".to_owned()),
        description: "Get the weather.".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
        }),
    }
}

fn call_part(id: &str, arguments: Value) -> LlmPart {
    LlmPart::ToolCall {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: "weather__get".to_owned(),
        arguments,
    }
}

fn result_part(id: &str, text: &str) -> LlmPart {
    LlmPart::ToolResult {
        call_id: CallId::generate().uuid(),
        provider_call_id: id.to_owned(),
        alias: "weather__get".to_owned(),
        is_error: false,
        content: vec![ToolContent::Text(text.to_owned())],
    }
}

fn finished(reason: FinishReason) -> LlmEvent {
    LlmEvent::Finished {
        reason,
        refusal: None,
    }
}

#[tokio::test]
async fn streams_text_with_the_version_and_key_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", ANTHROPIC_VERSION))
        .and(header("accept", "text/event-stream"))
        .respond_with(sse(&answer("Hello!")))
        .expect(1)
        .mount(&server)
        .await;

    let events = ok(stream_all(&target(&server), request("Hi")).await);
    assert_eq!(
        events,
        [
            LlmEvent::Usage(Usage {
                input_tokens: Some(10),
                output_tokens: Some(1),
                ..Usage::default()
            }),
            LlmEvent::TextDelta {
                index: 0,
                text: "Hello!".to_owned()
            },
            LlmEvent::Usage(Usage {
                input_tokens: Some(10),
                output_tokens: Some(4),
                ..Usage::default()
            }),
            LlmEvent::ReplayState(json!([{ "type": "text", "text": "Hello!" }])),
            finished(FinishReason::Stop),
        ]
    );

    let sent = &requests(&server).await[0];
    assert_eq!(ANTHROPIC_VERSION, "2026-10-21");
    assert!(
        sent.headers.get("authorization").is_none(),
        "the key goes in x-api-key only"
    );
    let body: Value = sent.body_json().expect("a JSON body");
    let marker = json!({ "type": "ephemeral" });
    assert_eq!(
        body,
        json!({
            "model": "claude-example-1",
            "max_tokens": 1_024,
            "stream": true,
            "system": [{ "type": "text", "text": "Be brief.", "cache_control": marker }],
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "Hi", "cache_control": marker },
            ] }],
        })
    );
}

#[tokio::test]
async fn a_gateway_without_a_key_and_with_a_v1_base_gets_neither_twice() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(&answer("Hi")))
        .expect(1)
        .mount(&server)
        .await;

    let mut gateway_target = target(&server);
    gateway_target.provider = provider(&format!("{}/v1", server.uri()), Preset::Custom, None);
    let events = ok(stream_all(&gateway_target, request("Hi")).await);
    assert_eq!(events.last(), Some(&finished(FinishReason::Stop)));
    let sent = &requests(&server).await[0];
    assert!(sent.headers.get("x-api-key").is_none());
    assert_eq!(
        sent.headers
            .get("anthropic-version")
            .and_then(|value| value.to_str().ok()),
        Some(ANTHROPIC_VERSION)
    );
}

#[tokio::test]
async fn thinking_tool_loop_replays_signed_blocks_on_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(&[
            message_start(472),
            start(0, json!({ "type": "thinking", "thinking": "", "signature": "" })),
            delta(0, json!({ "type": "thinking_delta", "thinking": "The user wants " })),
            delta(0, json!({ "type": "thinking_delta", "thinking": "the weather." })),
            delta(0, json!({ "type": "signature_delta", "signature": "RXVnZW5pYQ==" })),
            stop(0),
            start(1, json!({ "type": "redacted_thinking", "data": "ZW5jcnlwdGVk" })),
            stop(1),
            start(2, json!({ "type": "text", "text": "" })),
            json!({ "type": "ping" }),
            text_delta(2, "Okay"),
            stop(2),
            start(3, json!({ "type": "tool_use", "id": "toolu_01T1x1fJ34qAmk2tNTrN7Up6", "name": "weather__get", "input": {} })),
            delta(3, json!({ "type": "input_json_delta", "partial_json": "" })),
            delta(3, json!({ "type": "input_json_delta", "partial_json": "{\"location\":" })),
            delta(3, json!({ "type": "input_json_delta", "partial_json": " \"San" })),
            delta(3, json!({ "type": "input_json_delta", "partial_json": " Francisc" })),
            delta(3, json!({ "type": "input_json_delta", "partial_json": "o, CA\"}" })),
            stop(3),
            message_delta("tool_use", 89),
            message_stop(),
        ]))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse(&answer("15 degrees in San Francisco.")))
        .mount(&server)
        .await;

    let target = target(&server);
    let mut first = request("Weather in San Francisco?");
    first.tools = vec![weather()];
    first.reasoning = Some(ReasoningLevel::Medium);
    let events = ok(stream_all(&target, first.clone()).await);
    let shown: Vec<&LlmEvent> = events
        .iter()
        .filter(|event| !matches!(event, LlmEvent::Usage(_) | LlmEvent::ReplayState(_)))
        .collect();
    assert_eq!(
        shown,
        [
            &LlmEvent::ReasoningDelta {
                index: 0,
                text: "The user wants ".to_owned()
            },
            &LlmEvent::ReasoningDelta {
                index: 0,
                text: "the weather.".to_owned()
            },
            &LlmEvent::TextDelta {
                index: 1,
                text: "Okay".to_owned()
            },
            &LlmEvent::ToolCallStarted {
                index: 2,
                provider_call_id: Some("toolu_01T1x1fJ34qAmk2tNTrN7Up6".to_owned()),
                alias: "weather__get".to_owned(),
            },
            &LlmEvent::ToolCallArgs {
                index: 2,
                json: "{\"location\":".to_owned()
            },
            &LlmEvent::ToolCallArgs {
                index: 2,
                json: " \"San".to_owned()
            },
            &LlmEvent::ToolCallArgs {
                index: 2,
                json: " Francisc".to_owned()
            },
            &LlmEvent::ToolCallArgs {
                index: 2,
                json: "o, CA\"}".to_owned()
            },
            &LlmEvent::ToolCallEnded { index: 2 },
            &finished(FinishReason::ToolCalls),
        ]
    );
    let state = replay_state(&events);
    assert_eq!(
        state,
        json!([
            { "type": "thinking", "thinking": "The user wants the weather.", "signature": "RXVnZW5pYQ==" },
            { "type": "redacted_thinking", "data": "ZW5jcnlwdGVk" },
            { "type": "text", "text": "Okay" },
            { "type": "tool_use", "id": "toolu_01T1x1fJ34qAmk2tNTrN7Up6", "name": "weather__get", "input": { "location": "San Francisco, CA" } },
        ])
    );

    // Core replays the answer (same connection, current turn) with the provider's call id.
    let mut second = first;
    second.messages.extend([
        message(
            Role::Assistant,
            vec![
                LlmPart::Text("Okay".to_owned()),
                call_part(
                    "toolu_01T1x1fJ34qAmk2tNTrN7Up6",
                    json!({ "location": "San Francisco, CA" }),
                ),
            ],
            Some(state.clone()),
        ),
        message(
            Role::Tool,
            vec![result_part("toolu_01T1x1fJ34qAmk2tNTrN7Up6", "15 degrees")],
            None,
        ),
    ]);
    let events = ok(stream_all(&target, second).await);
    assert_eq!(events.last(), Some(&finished(FinishReason::Stop)));

    let sent = requests(&server).await;
    let first_body: Value = sent[0].body_json().expect("a JSON body");
    assert_eq!(
        first_body["thinking"],
        json!({ "type": "adaptive", "display": "summarized" })
    );
    assert_eq!(first_body["output_config"], json!({ "effort": "medium" }));
    assert_eq!(
        first_body["tools"],
        json!([{
            "name": "weather__get",
            "description": "Get the weather.",
            "input_schema": {
                "type": "object",
                "properties": { "location": { "type": "string" } },
                "required": ["location"],
            },
        }])
    );
    for field in ["temperature", "top_p", "top_k", "tool_choice"] {
        assert!(first_body.get(field).is_none(), "{field} is never sent");
    }

    let second_body: Value = sent[1].body_json().expect("a JSON body");
    let messages = second_body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"], state, "replayed verbatim");
    assert_eq!(
        messages[2],
        json!({ "role": "user", "content": [{
            "type": "tool_result",
            "tool_use_id": "toolu_01T1x1fJ34qAmk2tNTrN7Up6",
            "content": [{ "type": "text", "text": "15 degrees" }],
            "cache_control": { "type": "ephemeral" },
        }] })
    );
    assert_eq!(
        messages[0]["content"][0]["cache_control"],
        json!({ "type": "ephemeral" }),
        "the first user message keeps its breakpoint"
    );
    assert_eq!(
        second_body["thinking"], first_body["thinking"],
        "the turn keeps its reasoning level"
    );
}

#[tokio::test]
async fn an_answer_rebuilt_without_replay_sends_text_and_tool_use() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&answer("Sure.")))
        .mount(&server)
        .await;

    let call_id = format!("call_{}", CallId::generate().uuid().simple());
    let mut later = request("Weather?");
    later.tools = vec![weather()];
    later.reasoning = Some(ReasoningLevel::Off);
    later.messages.extend([
        message(
            Role::Assistant,
            vec![call_part(&call_id, json!({ "location": "Paris" }))],
            None,
        ),
        message(Role::Tool, vec![result_part(&call_id, "15 degrees")], None),
        message(
            Role::User,
            vec![LlmPart::Text("And tomorrow?".to_owned())],
            None,
        ),
    ]);
    let _ = stream_all(&target(&server), later).await;

    let body: Value = requests(&server).await[0].body_json().expect("a JSON body");
    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
    assert!(body.get("output_config").is_none());
    assert_eq!(
        body["messages"][1]["content"],
        json!([{ "type": "tool_use", "id": call_id, "name": "weather__get", "input": { "location": "Paris" } }])
    );
    assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], call_id);
    assert_eq!(
        body["messages"][2]["content"][1],
        json!({ "type": "text", "text": "And tomorrow?", "cache_control": { "type": "ephemeral" } }),
        "the next user message merges after the results"
    );
}

#[tokio::test]
async fn sends_images_and_pdfs_and_never_audio() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&answer("A chart.")))
        .expect(1)
        .mount(&server)
        .await;

    let target = target(&server);
    let mut with_files = request("What is in these?");
    with_files.messages[0].parts.extend([
        LlmPart::Image {
            media_type: "image/jpeg".to_owned(),
            data: Bytes::from_static(b"\xff\xd8\xff"),
        },
        LlmPart::Pdf {
            name: "report.pdf".to_owned(),
            data: Bytes::from_static(b"%PDF-1.7"),
        },
    ]);
    let _ = stream_all(&target, with_files).await;
    let body: Value = requests(&server).await[0].body_json().expect("a JSON body");
    let content = &body["messages"][0]["content"];
    assert_eq!(
        content[1],
        json!({ "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": "/9j/" } })
    );
    assert_eq!(content[2]["type"], "document");
    assert_eq!(
        content[2]["source"],
        json!({ "type": "base64", "media_type": "application/pdf", "data": "JVBERi0xLjc=" })
    );
    assert_eq!(content[2]["title"], "report.pdf");

    let mut with_audio = request("Listen");
    with_audio.messages[0].parts.push(LlmPart::Audio {
        format: "wav".to_owned(),
        data: Bytes::from_static(b"RIFF"),
    });
    let error = gateway()
        .stream(&target, with_audio)
        .await
        .err()
        .expect("unsupported");
    assert_eq!(error.code(), UNSUPPORTED);
    assert_eq!(error.to_string(), "This connection cannot send audio.");
}

#[tokio::test]
async fn a_refusal_finishes_with_its_category_and_explanation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            message_start(5),
            start(0, json!({ "type": "text", "text": "" })),
            text_delta(0, "Here is how"),
            stop(0),
            json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": "refusal",
                    "stop_sequence": null,
                    "stop_details": { "type": "refusal", "category": "cyber", "explanation": "This request asks for help with an attack." },
                },
                "usage": { "output_tokens": 3 },
            }),
            message_stop(),
        ]))
        .mount(&server)
        .await;

    let events = ok(stream_all(&target(&server), request("Hi")).await);
    assert_eq!(
        events.last(),
        Some(&LlmEvent::Finished {
            reason: FinishReason::Refusal,
            refusal: Some(Refusal {
                category: Some("cyber".to_owned()),
                explanation: "This request asks for help with an attack.".to_owned(),
            }),
        })
    );
}

#[tokio::test]
async fn maps_every_stop_reason_over_the_wire() {
    for (reason, expected) in [
        ("end_turn", FinishReason::Stop),
        ("stop_sequence", FinishReason::Stop),
        ("tool_use", FinishReason::ToolCalls),
        ("max_tokens", FinishReason::Length),
        ("model_context_window_exceeded", FinishReason::Length),
        ("pause_turn", FinishReason::Other),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(sse(&[
                message_start(1),
                message_delta(reason, 1),
                message_stop(),
            ]))
            .mount(&server)
            .await;
        let events = ok(stream_all(&target(&server), request("Hi")).await);
        assert_eq!(events.last(), Some(&finished(expected)), "{reason}");
    }
}

#[tokio::test]
async fn reports_cache_reads_and_writes_in_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            json!({ "type": "message_start", "message": {
                "id": "msg_1", "type": "message", "role": "assistant", "content": [],
                "usage": { "input_tokens": 50, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 2_048, "output_tokens": 1 },
            } }),
            start(0, json!({ "type": "text", "text": "" })),
            text_delta(0, "Cached."),
            stop(0),
            message_delta("end_turn", 12),
            message_stop(),
        ]))
        .mount(&server)
        .await;

    let events = ok(stream_all(&target(&server), request("Hi")).await);
    let last_usage = events
        .iter()
        .rev()
        .find_map(|event| match event {
            LlmEvent::Usage(usage) => Some(*usage),
            _ => None,
        })
        .expect("usage");
    assert_eq!(
        last_usage,
        Usage {
            input_tokens: Some(2_098),
            output_tokens: Some(12),
            reasoning_tokens: None,
            cache_read_tokens: Some(2_048),
            cache_write_tokens: Some(0),
        }
    );
}

#[tokio::test]
async fn http_errors_fail_before_the_stream_without_echoing_the_body() {
    let server = MockServer::start().await;
    let error_body = |kind: &str| json!({ "type": "error", "error": { "type": kind, "message": "secret detail sk-ant-test" } });
    for (status, kind) in [
        (401, "authentication_error"),
        (403, "permission_error"),
        (400, "invalid_request_error"),
        (413, "request_too_large"),
        (529, "overloaded_error"),
    ] {
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body(kind)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "20")
                .set_body_json(error_body("rate_limit_error")),
        )
        .mount(&server)
        .await;

    let gateway = gateway();
    let target = target(&server);
    let mut errors = Vec::new();
    for _ in 0..6 {
        errors.push(
            gateway
                .stream(&target, request("Hi"))
                .await
                .err()
                .expect("an error status fails"),
        );
    }
    let seen: Vec<(&str, String)> = errors
        .iter()
        .map(|error| (error.code(), error.to_string()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                "PROVIDER_AUTH",
                "Key rejected: open provider settings.".to_owned()
            ),
            (
                "PROVIDER_AUTH",
                "Key rejected: open provider settings.".to_owned()
            ),
            (
                "PROVIDER_HTTP",
                "The provider answered with HTTP 400.".to_owned()
            ),
            (
                "PROVIDER_HTTP",
                "The request is too large for this provider.".to_owned()
            ),
            (
                "PROVIDER_HTTP",
                "The provider is overloaded. Try again in a moment.".to_owned()
            ),
            (
                "PROVIDER_RATE_LIMITED",
                "The provider is rate limiting requests. Try again in 20 seconds.".to_owned()
            ),
        ]
    );
    for (_, text) in &seen {
        assert!(!text.contains("secret") && !text.contains("sk-ant"));
    }
}

#[tokio::test]
async fn mid_stream_errors_end_the_answer_without_echoing_the_provider() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            message_start(5),
            start(0, json!({ "type": "text", "text": "" })),
            text_delta(0, "Partial"),
            json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded secret" } }),
            message_stop(),
        ]))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({ "type": "error", "error": { "type": "api_error", "message": "secret" } })),
        )
        .mount(&server)
        .await;

    let target = target(&server);
    let events = stream_all(&target, request("Hi")).await;
    let last = events.last().expect("an item");
    let error = last.as_ref().expect_err("the stream ends with a failure");
    assert_eq!(error.code(), "PROVIDER_ERROR");
    assert_eq!(
        error.to_string(),
        "The provider is overloaded. Try again in a moment."
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(LlmEvent::Finished { .. }))),
        "an answer cut by an error never finishes"
    );

    let error = gateway()
        .stream(&target, request("Hi"))
        .await
        .err()
        .expect("a JSON error reply fails before the stream");
    assert_eq!(error.code(), "PROVIDER_ERROR");
    assert!(!error.to_string().contains("secret"));
}

#[tokio::test]
async fn a_cut_off_stream_ends_without_a_finish() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            message_start(5),
            start(0, json!({ "type": "text", "text": "" })),
            text_delta(0, "Cut"),
        ]))
        .mount(&server)
        .await;

    let events = ok(stream_all(&target(&server), request("Hi")).await);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, LlmEvent::Finished { .. }))
    );
}

#[tokio::test]
async fn lists_models_page_by_page_with_reported_capabilities() {
    let server = MockServer::start().await;
    let model = |id: &str, adaptive: bool| {
        json!({
            "type": "model",
            "id": id,
            "display_name": format!("Display {id}"),
            "created_at": "2026-01-01T00:00:00Z",
            "lifecycle": "active",
            "max_input_tokens": 1_000_000,
            "max_tokens": 128_000,
            "capabilities": {
                "image_input": { "supported": true },
                "pdf_input": { "supported": true },
                "thinking": { "supported": true, "types": {
                    "adaptive": { "supported": adaptive },
                    "enabled": { "supported": !adaptive },
                    "disabled": { "supported": true },
                } },
                "effort": {
                    "supported": adaptive,
                    "low": { "supported": adaptive },
                    "medium": { "supported": adaptive },
                    "high": { "supported": adaptive },
                },
            },
        })
    };
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1000"))
        .and(query_param("after_id", "claude-a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [model("claude-b", false)],
            "has_more": false,
            "first_id": "claude-b",
            "last_id": "claude-b",
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", ANTHROPIC_VERSION))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [model("claude-a", true)],
            "has_more": true,
            "first_id": "claude-a",
            "last_id": "claude-a",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let models = gateway()
        .models(&provider(
            &server.uri(),
            Preset::Anthropic,
            Some("sk-ant-test"),
        ))
        .await
        .expect("listed");
    assert_eq!(models.len(), 2);
    let first = &models[0];
    assert_eq!(first.remote_id, "claude-a");
    assert_eq!(first.display_name, "Display claude-a");
    assert_eq!(first.context_window, Some(1_000_000));
    assert_eq!(first.max_output_tokens, Some(128_000));
    assert_eq!(first.reported.vision, Some(true));
    assert_eq!(first.reported.pdf, Some(true));
    assert_eq!(first.reported.reasoning, Some(true));
    assert_eq!(
        first.reported.reasoning_levels,
        Some(ReasoningLevel::ALL.to_vec())
    );
    assert_eq!(first.reported.audio_in, Some(false));
    assert_eq!(first.reported.audio_out, Some(false));
    assert_eq!(models[1].remote_id, "claude-b");
    assert_eq!(models[1].reported.reasoning, Some(false));

    for sent in requests(&server).await {
        let query = sent.url.query().unwrap_or_default();
        assert!(
            !query.contains("sk-ant"),
            "the key never travels in the URL"
        );
    }
}

#[tokio::test]
async fn model_list_failures_are_readable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" },
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;

    let gateway = gateway();
    let target = provider(&server.uri(), Preset::Anthropic, Some("sk-ant-test"));
    let auth = gateway.models(&target).await.expect_err("unauthorized");
    assert_eq!(auth.code(), "PROVIDER_AUTH");
    let invalid = gateway.models(&target).await.expect_err("not a list");
    assert_eq!(invalid.code(), "PROVIDER_BAD_RESPONSE");
}
