//! The OpenAI-compatible adapter against a wiremock provider, through the hardened outbound
//! clients the composition root injects (`copilot_security::clients(Purpose::Providers)`). The
//! mock listens on loopback over plain `http`, which only a private-scope connection may reach.

use bytes::Bytes;
use copilot_core::{
    AudioOut, BaseUrl, CallId, Dialect, DomainResult, FinishReason, LlmEvent, LlmGateway,
    LlmMessage, LlmPart, LlmRequest, ModelTarget, NetworkScope, Preset, ProviderId, ProviderKind,
    ProviderTarget, ReasoningLevel, Refusal, RemoteModel, ReportedCapabilities, Role, Secret,
    ToolAlias, ToolContent, ToolSpec, Usage,
};
use copilot_providers::{Gateway, UNSUPPORTED};
use copilot_security::{Purpose, clients};
use futures_util::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{
    body_partial_json, header, method, path, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn gateway() -> Gateway {
    Gateway::new(clients(Purpose::Providers).expect("clients build"))
}

fn provider(server: &MockServer, preset: Preset, key: Option<&str>) -> ProviderTarget {
    ProviderTarget {
        provider_id: ProviderId::generate(),
        kind: ProviderKind::OpenAi,
        preset,
        base_url: BaseUrl::parse(&format!("{}/v1", server.uri()), NetworkScope::Private)
            .expect("a loopback URL"),
        api_key: key.map(Secret::new),
        network: NetworkScope::Private,
    }
}

fn target(server: &MockServer, preset: Preset) -> ModelTarget {
    ModelTarget {
        provider: provider(server, preset, Some("sk-test")),
        remote_id: "model-1".to_owned(),
        dialect: Dialect::new(ProviderKind::OpenAi, preset),
    }
}

fn request(text: &str) -> LlmRequest {
    LlmRequest {
        model: "model-1".to_owned(),
        system: "Be brief.".to_owned(),
        messages: vec![LlmMessage {
            role: Role::User,
            parts: vec![LlmPart::Text(text.to_owned())],
            replay: None,
        }],
        tools: Vec::new(),
        max_output_tokens: 256,
        reasoning: None,
        can_reason: false,
        audio_out: None,
        cache_breakpoints: false,
    }
}

/// An SSE body of data-only events, one per line of `chunks`.
fn sse(chunks: &[&str]) -> ResponseTemplate {
    let body: String = chunks
        .iter()
        .map(|data| format!("data: {data}\n\n"))
        .collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn chunk(content: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "choices": [{ "index": 0, "delta": { "content": content }, "finish_reason": null }],
    })
    .to_string()
}

fn finish(reason: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "choices": [{ "index": 0, "delta": {}, "finish_reason": reason }],
    })
    .to_string()
}

async fn collect(
    gateway: &Gateway,
    target: &ModelTarget,
    text: &str,
) -> Vec<DomainResult<LlmEvent>> {
    gateway
        .stream(target, request(text))
        .await
        .expect("the stream opens")
        .collect()
        .await
}

fn text(text: &str) -> LlmEvent {
    LlmEvent::TextDelta {
        index: 0,
        text: text.to_owned(),
    }
}

fn ok(events: Vec<DomainResult<LlmEvent>>) -> Vec<LlmEvent> {
    events
        .into_iter()
        .map(|event| event.expect("no failure"))
        .collect()
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.expect("recording")
}

#[tokio::test]
async fn streams_text_and_usage_in_the_openai_dialect() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(header("accept", "text/event-stream"))
        .respond_with(sse(&[
            &chunk(""),
            &chunk("Hello"),
            &chunk(", world"),
            &finish("stop"),
            r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}"#,
            "[DONE]",
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::OpenAi), "Hi").await;
    assert_eq!(
        ok(events),
        [
            text("Hello"),
            text(", world"),
            LlmEvent::Usage(Usage {
                input_tokens: Some(12),
                output_tokens: Some(3),
                ..Usage::default()
            }),
            LlmEvent::Finished {
                reason: FinishReason::Stop,
                refusal: None,
            },
        ]
    );

    let sent: Value = requests(&server).await[0].body_json().expect("a JSON body");
    assert_eq!(
        sent,
        json!({
            "model": "model-1",
            "stream": true,
            "messages": [
                { "role": "developer", "content": "Be brief." },
                { "role": "user", "content": "Hi" },
            ],
            "max_completion_tokens": 256,
            "stream_options": { "include_usage": true },
        })
    );
}

#[tokio::test]
async fn compatible_servers_get_system_and_max_tokens_and_no_key_header_without_a_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "messages": [{ "role": "system", "content": "Be brief." }, { "role": "user" }],
            "max_tokens": 256,
        })))
        .respond_with(sse(&[&chunk("Hi"), &finish("length"), "[DONE]"]))
        .expect(1)
        .mount(&server)
        .await;

    let mut target = target(&server, Preset::Custom);
    target.provider.api_key = None;
    let events = collect(&gateway(), &target, "Hi").await;
    assert_eq!(
        ok(events),
        [
            text("Hi"),
            LlmEvent::Finished {
                reason: FinishReason::Length,
                refusal: None,
            },
        ]
    );
    let sent = &requests(&server).await[0];
    assert!(sent.headers.get("authorization").is_none());
}

#[tokio::test]
async fn parses_crlf_endings_comments_and_multi_line_events() {
    let server = MockServer::start().await;
    let body = format!(
        ": OPENROUTER PROCESSING\r\n\r\ndata: {}\r\n\r\ndata: {}\r\n\r\n: keep-alive\r\n\r\n{}data: [DONE]\r\n\r\n",
        chunk("One"),
        chunk(" two"),
        // One event in two `data:` lines, joined with a line feed.
        "data: {\"choices\":[{\"index\":0,\r\ndata: \"delta\":{},\"finish_reason\":\"stop\"}]}\r\n\r\n",
    );
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::OpenRouter), "Hi").await;
    assert_eq!(
        ok(events),
        [
            text("One"),
            text(" two"),
            LlmEvent::Finished {
                reason: FinishReason::Stop,
                refusal: None,
            },
        ]
    );
}

#[tokio::test]
async fn a_refusal_finishes_with_its_explanation() {
    let server = MockServer::start().await;
    let refusal = json!({
        "choices": [{ "index": 0, "delta": { "refusal": "I can't help with that." }, "finish_reason": "stop" }],
    })
    .to_string();
    Mock::given(method("POST"))
        .respond_with(sse(&[&refusal, "[DONE]"]))
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::OpenAi), "Hi").await;
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

#[tokio::test]
async fn a_mid_stream_error_chunk_ends_the_stream_with_an_error() {
    let server = MockServer::start().await;
    let error = json!({
        "error": { "code": 502, "message": "Provider disconnected unexpectedly" },
        "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "error" }],
    })
    .to_string();
    Mock::given(method("POST"))
        .respond_with(sse(&[
            &chunk("Partial"),
            &error,
            &chunk("ignored"),
            "[DONE]",
        ]))
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::OpenRouter), "Hi").await;
    assert_eq!(events.len(), 2, "nothing after the error");
    assert_eq!(events[0].as_ref().expect("text"), &text("Partial"));
    let error = events[1].as_ref().expect_err("the error chunk");
    assert_eq!(error.code(), "PROVIDER_ERROR");
    assert_eq!(
        error.to_string(),
        "The provider ended the answer with an error."
    );
}

#[tokio::test]
async fn a_cut_off_stream_ends_without_a_finish() {
    let server = MockServer::start().await;
    // The last event has no blank line after it, so it never completes.
    let body = format!("data: {}\n\ndata: {}", chunk("Hel"), chunk("lo"));
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::Custom), "Hi").await;
    assert_eq!(ok(events), [text("Hel")]);
}

#[tokio::test]
async fn an_unreadable_chunk_fails_the_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[&chunk("Hi"), "{not json", "[DONE]"]))
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::Custom), "Hi").await;
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1].as_ref().expect_err("invalid").code(),
        "PROVIDER_BAD_RESPONSE"
    );
}

#[tokio::test]
async fn an_event_over_the_size_cap_fails_the_answer() {
    let server = MockServer::start().await;
    let huge = chunk(&"x".repeat(copilot_core::shared::constants::MCP_RESPONSE_MAX_BYTES));
    Mock::given(method("POST"))
        .respond_with(sse(&[&chunk("Hi"), &huge, "[DONE]"]))
        .mount(&server)
        .await;

    let events = collect(&gateway(), &target(&server, Preset::Custom), "Hi").await;
    assert_eq!(events.len(), 2);
    let error = events[1].as_ref().expect_err("too large");
    assert_eq!(error.code(), "PROVIDER_BAD_RESPONSE");
    assert_eq!(
        error.to_string(),
        "The provider sent a reply over the size limit."
    );
}

#[tokio::test]
async fn http_errors_fail_before_the_stream_with_readable_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({ "error": { "message": "Incorrect API key provided: sk-t***" } }),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
        .mount(&server)
        .await;

    let gateway = gateway();
    let target = target(&server, Preset::OpenAi);
    let errors = [
        gateway.stream(&target, request("Hi")).await.err(),
        gateway.stream(&target, request("Hi")).await.err(),
        gateway.stream(&target, request("Hi")).await.err(),
    ]
    .map(|error| error.expect("an error status fails"));

    assert_eq!(errors[0].code(), "PROVIDER_AUTH");
    assert_eq!(
        errors[0].to_string(),
        "Key rejected: open provider settings."
    );
    assert_eq!(errors[1].code(), "PROVIDER_RATE_LIMITED");
    assert_eq!(
        errors[1].to_string(),
        "The provider is rate limiting requests. Try again in 7 seconds."
    );
    assert_eq!(errors[2].code(), "PROVIDER_HTTP");
    assert_eq!(
        errors[2].to_string(),
        "The provider answered with HTTP 500."
    );
    for error in &errors {
        assert!(
            !error.to_string().contains("sk-t"),
            "no response body is echoed"
        );
        assert!(!error.to_string().contains("exploded"));
    }
}

#[tokio::test]
async fn a_public_connection_never_reaches_a_loopback_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&["[DONE]"]))
        .expect(0)
        .mount(&server)
        .await;

    let mut target = target(&server, Preset::Custom);
    target.provider.network = NetworkScope::Public;
    let error = gateway()
        .stream(&target, request("Hi"))
        .await
        .err()
        .expect("refused");
    assert_eq!(error.code(), "PROVIDER_BLOCKED");
}

#[tokio::test]
async fn an_unreachable_provider_fails_with_a_readable_message() {
    // A port nothing listens on: bound, then released.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    drop(listener);
    let server = MockServer::start().await;
    let mut target = target(&server, Preset::Custom);
    target.provider.base_url = BaseUrl::parse(
        &format!("http://127.0.0.1:{port}/v1"),
        NetworkScope::Private,
    )
    .expect("a loopback URL");
    let error = gateway()
        .stream(&target, request("Hi"))
        .await
        .err()
        .expect("nothing listens");
    assert_eq!(error.code(), "PROVIDER_UNREACHABLE");
    assert!(
        !error.to_string().contains("127.0.0.1"),
        "no address echoed"
    );
}

#[tokio::test]
async fn requests_the_protocol_cannot_carry_never_reach_the_provider() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&["[DONE]"]))
        .expect(0)
        .mount(&server)
        .await;

    let mut with_ogg = request("Listen");
    with_ogg.messages[0].parts.push(LlmPart::Audio {
        format: "ogg".to_owned(),
        data: Bytes::from_static(b"OggS"),
    });
    let error = gateway()
        .stream(&target(&server, Preset::OpenAi), with_ogg)
        .await
        .err()
        .expect("unsupported");
    assert_eq!(error.code(), UNSUPPORTED);
    assert_eq!(
        error.to_string(),
        "This connection cannot send this audio format."
    );
}

#[tokio::test]
async fn lists_models_with_the_key_in_the_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                { "id": "model-1", "object": "model", "created": 1_686_935_002, "owned_by": "org" },
                { "id": "model-2", "object": "model", "created": 1_686_935_003, "owned_by": "org" },
            ],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let models = gateway()
        .models(&provider(&server, Preset::OpenAi, Some("sk-test")))
        .await
        .expect("listed");
    let ids: Vec<_> = models
        .iter()
        .map(|model| model.remote_id.as_str())
        .collect();
    assert_eq!(ids, ["model-1", "model-2"]);
    assert_eq!(models[0].display_name, "model-1");
    assert_eq!(models[0].context_window, None);
    let sent = &requests(&server).await[0];
    assert_eq!(sent.url.query(), None, "the key never travels in the URL");
}

#[tokio::test]
async fn model_list_failures_are_readable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(403))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;

    let gateway = gateway();
    let target = provider(&server, Preset::Custom, Some("sk-test"));
    let auth = gateway.models(&target).await.expect_err("forbidden");
    assert_eq!(auth.code(), "PROVIDER_AUTH");
    let invalid = gateway.models(&target).await.expect_err("not a list");
    assert_eq!(invalid.code(), "PROVIDER_BAD_RESPONSE");
}

// Beyond text: tools, reasoning, dialects, audio, and model metadata.

async fn stream_all(target: &ModelTarget, request: LlmRequest) -> Vec<DomainResult<LlmEvent>> {
    gateway()
        .stream(target, request)
        .await
        .expect("the stream opens")
        .collect()
        .await
}

fn data(delta: &Value) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "choices": [{ "index": 0, "delta": delta, "finish_reason": null }],
    })
    .to_string()
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

fn message(role: Role, parts: Vec<LlmPart>, replay: Option<Value>) -> LlmMessage {
    LlmMessage {
        role,
        parts,
        replay,
    }
}

fn finished(reason: FinishReason) -> LlmEvent {
    LlmEvent::Finished {
        reason,
        refusal: None,
    }
}

#[tokio::test]
async fn streams_tool_call_argument_deltas_and_sends_the_tool_round_back() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            &data(&json!({ "role": "assistant", "content": null, "tool_calls": [
                { "index": 0, "id": "call_abc", "type": "function", "function": { "name": "weather__get", "arguments": "" } },
            ] })),
            &data(&json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{\"locat" } }] })),
            &data(&json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "ion\":\"Paris\"}" } }] })),
            &finish("tool_calls"),
            r#"{"choices":[],"usage":{"prompt_tokens":82,"completion_tokens":18,"total_tokens":100}}"#,
            "[DONE]",
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut request = request("Weather in Paris and Rome?");
    request.tools = vec![weather()];
    request.messages.extend([
        message(
            Role::Assistant,
            vec![
                LlmPart::Text("Rome first.".to_owned()),
                call_part("call_rome", json!({ "location": "Rome" })),
            ],
            None,
        ),
        message(
            Role::Tool,
            vec![result_part("call_rome", "21 degrees")],
            None,
        ),
    ]);
    let events = stream_all(&target(&server, Preset::OpenAi), request).await;
    assert_eq!(
        ok(events),
        [
            LlmEvent::ToolCallStarted {
                index: 0,
                provider_call_id: Some("call_abc".to_owned()),
                alias: "weather__get".to_owned(),
            },
            LlmEvent::ToolCallArgs {
                index: 0,
                json: "{\"locat".to_owned(),
            },
            LlmEvent::ToolCallArgs {
                index: 0,
                json: "ion\":\"Paris\"}".to_owned(),
            },
            LlmEvent::ToolCallEnded { index: 0 },
            LlmEvent::Usage(Usage {
                input_tokens: Some(82),
                output_tokens: Some(18),
                ..Usage::default()
            }),
            finished(FinishReason::ToolCalls),
        ]
    );

    let sent: Value = requests(&server).await[0].body_json().expect("a JSON body");
    assert_eq!(
        sent["tools"],
        json!([{
            "type": "function",
            "function": {
                "name": "weather__get",
                "description": "Get the weather.",
                "parameters": {
                    "type": "object",
                    "properties": { "location": { "type": "string" } },
                    "required": ["location"],
                },
                "strict": false,
            },
        }])
    );
    assert_eq!(sent["tool_choice"], "auto");
    assert_eq!(
        sent["messages"][2],
        json!({
            "role": "assistant",
            "content": "Rome first.",
            "tool_calls": [{
                "id": "call_rome",
                "type": "function",
                "function": { "name": "weather__get", "arguments": "{\"location\":\"Rome\"}" },
            }],
        })
    );
    assert_eq!(
        sent["messages"][3],
        json!({ "role": "tool", "tool_call_id": "call_rome", "content": "21 degrees" })
    );
}

#[tokio::test]
async fn ollama_gets_tools_without_tool_choice_and_whole_calls_work() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            &data(&json!({ "role": "assistant", "content": "", "tool_calls": [
                { "id": "call_1", "index": 0, "type": "function", "function": { "name": "weather__get", "arguments": "{\"location\":\"Oslo\"}" } },
            ] })),
            // Ollama finishes calls with `stop`.
            &finish("stop"),
            "[DONE]",
        ]))
        .mount(&server)
        .await;

    let mut request = request("Weather in Oslo?");
    request.tools = vec![weather()];
    let events = ok(stream_all(&target(&server, Preset::Ollama), request).await);
    assert_eq!(
        events[1],
        LlmEvent::ToolCallArgs {
            index: 0,
            json: "{\"location\":\"Oslo\"}".to_owned(),
        }
    );
    assert_eq!(events.last(), Some(&finished(FinishReason::ToolCalls)));
    let sent: Value = requests(&server).await[0].body_json().expect("a JSON body");
    assert!(sent["tools"].is_array());
    assert!(sent.get("tool_choice").is_none());
}

#[tokio::test]
async fn every_reasoning_field_becomes_reasoning_deltas_and_replay_state() {
    let cases = [
        (
            Preset::Ollama,
            json!({ "reasoning": "Hmm." }),
            json!({ "reasoning_key": "reasoning", "reasoning": "Hmm." }),
        ),
        (
            Preset::Custom,
            json!({ "reasoning_content": "Hmm." }),
            json!({ "reasoning_key": "reasoning_content", "reasoning": "Hmm." }),
        ),
        (
            Preset::OpenRouter,
            json!({ "reasoning": "Hmm.", "reasoning_details": [{ "type": "reasoning.text", "text": "Hmm.", "signature": "sig", "index": 0 }] }),
            json!({
                "reasoning_key": "reasoning",
                "reasoning": "Hmm.",
                "reasoning_details": [{ "type": "reasoning.text", "text": "Hmm.", "signature": "sig", "index": 0 }],
            }),
        ),
    ];
    for (preset, delta, state) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(sse(&[
                &data(&delta),
                &chunk("Yes."),
                &finish("stop"),
                "[DONE]",
            ]))
            .mount(&server)
            .await;
        let events = ok(stream_all(&target(&server, preset), request("Hi")).await);
        assert_eq!(
            events,
            [
                LlmEvent::ReasoningDelta {
                    index: 0,
                    text: "Hmm.".to_owned(),
                },
                LlmEvent::TextDelta {
                    index: 1,
                    text: "Yes.".to_owned(),
                },
                LlmEvent::ReplayState(state),
                finished(FinishReason::Stop),
            ],
            "{preset}"
        );
    }
}

#[tokio::test]
async fn replays_reasoning_to_the_same_dialect_on_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[&chunk("Ok."), &finish("stop"), "[DONE]"]))
        .mount(&server)
        .await;

    let details = json!([{ "type": "reasoning.encrypted", "data": "opaque", "index": 0 }]);
    let mut request = request("Use it.");
    request.tools = vec![weather()];
    request.messages = vec![
        message(Role::User, vec![LlmPart::Text("Weather?".to_owned())], None),
        message(
            Role::Assistant,
            vec![call_part("toolu_1", json!({ "location": "Paris" }))],
            Some(
                json!({ "reasoning_key": "reasoning", "reasoning": "Hmm.", "reasoning_details": details }),
            ),
        ),
        message(Role::Tool, vec![result_part("toolu_1", "15 degrees")], None),
    ];
    let _ = stream_all(&target(&server, Preset::OpenRouter), request.clone()).await;
    let _ = stream_all(&target(&server, Preset::Custom), request).await;

    let sent = requests(&server).await;
    let openrouter: Value = sent[0].body_json().expect("a JSON body");
    assert_eq!(openrouter["messages"][2]["reasoning_details"], details);
    assert!(openrouter["messages"][2].get("reasoning").is_none());
    let custom: Value = sent[1].body_json().expect("a JSON body");
    assert_eq!(custom["messages"][2]["reasoning"], "Hmm.");
    assert!(custom["messages"][2].get("reasoning_details").is_none());
}

#[tokio::test]
async fn gemini_thought_signatures_survive_two_turns() {
    let server = MockServer::start().await;
    let signed = json!({ "role": "assistant", "tool_calls": [{
        "id": "function-call-1",
        "type": "function",
        "function": { "name": "weather__get", "arguments": "{\"location\":\"Paris\"}" },
        "extra_content": { "google": { "thought_signature": "c2lnbmF0dXJl" } },
    }] });
    Mock::given(method("POST"))
        .respond_with(sse(&[&data(&signed), &finish("tool_calls"), "[DONE]"]))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            &chunk("15 degrees in Paris."),
            &finish("stop"),
            "[DONE]",
        ]))
        .mount(&server)
        .await;

    let target = target(&server, Preset::Gemini);
    let mut first = request("Weather in Paris?");
    first.tools = vec![weather()];
    let events = ok(stream_all(&target, first.clone()).await);
    let Some(LlmEvent::ReplayState(state)) = events
        .iter()
        .find(|event| matches!(event, LlmEvent::ReplayState(_)))
        .cloned()
    else {
        panic!("a replay state: {events:?}");
    };

    // Core replays the answer with the provider's call id and the stored state.
    let mut second = first;
    second.messages.extend([
        message(
            Role::Assistant,
            vec![call_part("function-call-1", json!({ "location": "Paris" }))],
            Some(state),
        ),
        message(
            Role::Tool,
            vec![result_part("function-call-1", "15 degrees")],
            None,
        ),
    ]);
    let events = ok(stream_all(&target, second).await);
    assert_eq!(events.last(), Some(&finished(FinishReason::Stop)));

    let sent: Value = requests(&server).await[1].body_json().expect("a JSON body");
    assert_eq!(
        sent["messages"][2]["tool_calls"][0],
        json!({
            "extra_content": { "google": { "thought_signature": "c2lnbmF0dXJl" } },
            "id": "function-call-1",
            "type": "function",
            "function": { "name": "weather__get", "arguments": "{\"location\":\"Paris\"}" },
        })
    );
}

#[tokio::test]
async fn sends_binary_parts_and_streams_a_spoken_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "modalities": ["text", "audio"],
            "audio": { "voice": "alloy", "format": "pcm16" },
        })))
        .respond_with(sse(&[
            &data(&json!({ "role": "assistant", "content": null, "audio": { "id": "audio_1", "transcript": "Hi" } })),
            &data(&json!({ "audio": { "data": "AAEC" } })),
            &data(&json!({ "audio": { "transcript": " there", "data": "AwQ=" } })),
            &finish("stop"),
            "[DONE]",
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut request = request("Say this back.");
    request.messages[0].parts.extend([
        LlmPart::Audio {
            format: "wav".to_owned(),
            data: Bytes::from_static(b"RIFF"),
        },
        LlmPart::Image {
            media_type: "image/jpeg".to_owned(),
            data: Bytes::from_static(b"\xff\xd8\xff"),
        },
        LlmPart::Pdf {
            name: "a.pdf".to_owned(),
            data: Bytes::from_static(b"%PDF"),
        },
    ]);
    request.audio_out = Some(AudioOut {
        voice: "alloy".to_owned(),
        format: "pcm16".to_owned(),
    });
    let events = ok(stream_all(&target(&server, Preset::OpenAi), request).await);
    assert_eq!(
        events,
        [
            LlmEvent::AudioDelta {
                data: Bytes::new(),
                transcript: Some("Hi".to_owned()),
            },
            LlmEvent::AudioDelta {
                data: Bytes::from_static(&[0, 1, 2]),
                transcript: None,
            },
            LlmEvent::AudioDelta {
                data: Bytes::from_static(&[3, 4]),
                transcript: Some(" there".to_owned()),
            },
            finished(FinishReason::Stop),
        ]
    );
    let sent: Value = requests(&server).await[0].body_json().expect("a JSON body");
    assert_eq!(
        sent["messages"][1]["content"],
        json!([
            { "type": "text", "text": "Say this back." },
            { "type": "input_audio", "input_audio": { "data": "UklGRg==", "format": "wav" } },
            { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,/9j/" } },
            { "type": "file", "file": { "filename": "a.pdf", "file_data": "data:application/pdf;base64,JVBERg==" } },
        ])
    );
}

#[tokio::test]
async fn maps_every_finish_reason_over_the_wire() {
    for (wire, reason) in [
        ("stop", FinishReason::Stop),
        ("length", FinishReason::Length),
        ("tool_calls", FinishReason::ToolCalls),
        ("function_call", FinishReason::ToolCalls),
        ("content_filter", FinishReason::Refusal),
        ("error", FinishReason::Error),
        ("whatever", FinishReason::Other),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(sse(&[&finish(wire), "[DONE]"]))
            .mount(&server)
            .await;
        let events = ok(stream_all(&target(&server, Preset::Custom), request("Hi")).await);
        assert_eq!(events, [finished(reason)], "{wire}");
    }
}

#[tokio::test]
async fn mid_stream_errors_end_the_answer_without_echoing_the_provider() {
    let bodies = [
        // A named `error` event.
        format!(
            "data: {}\n\nevent: error\ndata: {{\"message\":\"secret detail\"}}\n\n",
            chunk("Part")
        ),
        // vLLM's error object.
        format!(
            "data: {}\n\ndata: {{\"object\":\"error\",\"message\":\"secret detail\",\"code\":500}}\n\n",
            chunk("Part")
        ),
        // llama.cpp's error chunk.
        format!(
            "data: {}\n\ndata: {{\"error\":{{\"code\":500,\"message\":\"secret detail\",\"type\":\"server_error\"}}}}\n\n",
            chunk("Part")
        ),
    ];
    for body in bodies {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body.clone()),
            )
            .mount(&server)
            .await;
        let events = stream_all(&target(&server, Preset::Custom), request("Hi")).await;
        assert_eq!(events.len(), 2, "{body}");
        assert_eq!(events[0].as_ref().expect("text"), &text("Part"));
        let error = events[1].as_ref().expect_err("the error");
        assert_eq!(error.code(), "PROVIDER_ERROR");
        assert!(!error.to_string().contains("secret"));
    }
}

#[tokio::test]
async fn a_json_reply_to_a_streamed_request_fails_before_the_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "error": { "message": "secret detail", "code": 400 } })),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": "chatcmpl-1", "choices": [] })),
        )
        .mount(&server)
        .await;

    let target = target(&server, Preset::Custom);
    let error = gateway()
        .stream(&target, request("Hi"))
        .await
        .err()
        .expect("an error reply");
    assert_eq!(error.code(), "PROVIDER_ERROR");
    assert!(!error.to_string().contains("secret"));
    let error = gateway()
        .stream(&target, request("Hi"))
        .await
        .err()
        .expect("not a stream");
    assert_eq!(error.code(), "PROVIDER_BAD_RESPONSE");
}

#[tokio::test]
async fn more_http_statuses_map_to_readable_errors() {
    for (status, code, message) in [
        (
            403,
            "PROVIDER_AUTH",
            "Key rejected: open provider settings.",
        ),
        (400, "PROVIDER_HTTP", "The provider answered with HTTP 400."),
        (404, "PROVIDER_HTTP", "The provider answered with HTTP 404."),
        (503, "PROVIDER_HTTP", "The provider answered with HTTP 503."),
        (
            429,
            "PROVIDER_RATE_LIMITED",
            "The provider is rate limiting requests. Try again later.",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(
                json!({ "error": { "message": "secret detail", "type": "invalid_request_error" } }),
            ))
            .mount(&server)
            .await;
        let error = gateway()
            .stream(&target(&server, Preset::OpenRouter), request("Hi"))
            .await
            .err()
            .expect("an error status");
        assert_eq!(error.code(), code, "{status}");
        assert_eq!(error.to_string(), message, "{status}");
    }
}

#[tokio::test]
async fn lists_openrouter_models_with_their_reported_capabilities() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{
            "id": "vendor/vision-model",
            "name": "Vendor: Vision Model",
            "context_length": 200_000,
            "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] },
            "top_provider": { "context_length": 200_000, "max_completion_tokens": 64_000 },
            "supported_parameters": ["tools", "reasoning"],
        }] })))
        .mount(&server)
        .await;

    let models = gateway()
        .models(&provider(&server, Preset::OpenRouter, Some("sk-test")))
        .await
        .expect("listed");
    assert_eq!(
        models,
        [RemoteModel {
            remote_id: "vendor/vision-model".to_owned(),
            display_name: "Vendor: Vision Model".to_owned(),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            reported: ReportedCapabilities {
                vision: Some(true),
                tools: Some(true),
                audio_in: Some(false),
                audio_out: Some(false),
                reasoning: Some(true),
                ..ReportedCapabilities::default()
            },
        }]
    );
}

#[tokio::test]
async fn lists_ollama_models_with_their_details() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "object": "list", "data": [
            { "id": "gemma4:latest", "object": "model", "owned_by": "library" },
            { "id": "broken:latest", "object": "model", "owned_by": "library" },
        ] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .and(body_partial_json(json!({ "model": "gemma4:latest" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": ["completion", "vision", "tools", "thinking"],
            "model_info": { "gemma4.context_length": 131_072 },
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .and(body_partial_json(json!({ "model": "broken:latest" })))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let models = gateway()
        .models(&provider(&server, Preset::Ollama, None))
        .await
        .expect("listed");
    assert_eq!(models[0].remote_id, "gemma4:latest");
    assert_eq!(models[0].context_window, Some(131_072));
    assert_eq!(models[0].reported.vision, Some(true));
    assert_eq!(models[0].reported.tools, Some(true));
    assert_eq!(models[0].reported.reasoning, Some(true));
    assert_eq!(
        models[1],
        RemoteModel {
            remote_id: "broken:latest".to_owned(),
            display_name: "broken:latest".to_owned(),
            ..RemoteModel::default()
        },
        "a model whose details fail keeps the defaults"
    );
}

#[tokio::test]
async fn lists_gemini_models_without_their_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "object": "list", "data": [
            { "id": "models/gemini-4-hydrogen", "object": "model", "owned_by": "google" },
        ] })),
        )
        .mount(&server)
        .await;
    let models = gateway()
        .models(&provider(&server, Preset::Gemini, Some("key")))
        .await
        .expect("listed");
    assert_eq!(models[0].remote_id, "gemini-4-hydrogen");
    assert_eq!(
        models[0].context_window, None,
        "a base URL without /openai has no native list next to it"
    );
}

/// A `gemini` connection at `{server}/v1beta/openai`, Google's layout: the native API is
/// `{server}/v1beta`.
fn gemini_provider(server: &MockServer) -> ProviderTarget {
    ProviderTarget {
        base_url: BaseUrl::parse(
            &format!("{}/v1beta/openai", server.uri()),
            NetworkScope::Private,
        )
        .expect("a loopback URL"),
        ..provider(server, Preset::Gemini, Some("gemini-key"))
    }
}

async fn mount_gemini_list(server: &MockServer, ids: &[&str]) {
    let data: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "id": format!("models/{id}"), "object": "model", "owned_by": "google" }))
        .collect();
    Mock::given(method("GET"))
        .and(path("/v1beta/openai/models"))
        .and(header("authorization", "Bearer gemini-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "object": "list", "data": data })),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn gemini_limits_come_from_the_native_model_list() {
    let server = MockServer::start().await;
    mount_gemini_list(
        &server,
        &["gemini-4-hydrogen", "gemini-3-helium", "gemma-5"],
    )
    .await;
    // Two pages, as Google pages its list: the second page is asked for by its token.
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .and(query_param("pageSize", "1000"))
        .and(query_param("pageToken", "page-2"))
        .and(header("x-goog-api-key", "gemini-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
            {
                "name": "models/gemini-3-helium",
                "displayName": "Gemini 3 Helium",
                "inputTokenLimit": 1_048_576,
                "outputTokenLimit": 65_536,
                "thinking": false,
            },
            { "name": "models/embedding-9", "inputTokenLimit": 2_048, "outputTokenLimit": 1 },
        ] })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .and(query_param("pageSize", "1000"))
        .and(query_param_is_missing("pageToken"))
        .and(header("x-goog-api-key", "gemini-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{
                "name": "models/gemini-4-hydrogen",
                "version": "001",
                "displayName": "Gemini 4 Hydrogen",
                "description": "A model shaped after Google's documented Model resource.",
                "inputTokenLimit": 2_097_152,
                "outputTokenLimit": 131_072,
                "supportedGenerationMethods": ["generateContent", "countTokens"],
                "temperature": 1.0,
                "thinking": true,
            }],
            "nextPageToken": "page-2",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let models = gateway()
        .models(&gemini_provider(&server))
        .await
        .expect("listed");
    assert_eq!(
        models,
        [
            RemoteModel {
                remote_id: "gemini-4-hydrogen".to_owned(),
                display_name: "Gemini 4 Hydrogen".to_owned(),
                context_window: Some(2_097_152),
                max_output_tokens: Some(131_072),
                reported: ReportedCapabilities {
                    reasoning: Some(true),
                    ..ReportedCapabilities::default()
                },
            },
            RemoteModel {
                remote_id: "gemini-3-helium".to_owned(),
                display_name: "Gemini 3 Helium".to_owned(),
                context_window: Some(1_048_576),
                max_output_tokens: Some(65_536),
                reported: ReportedCapabilities {
                    reasoning: Some(false),
                    ..ReportedCapabilities::default()
                },
            },
            RemoteModel {
                remote_id: "gemma-5".to_owned(),
                display_name: "gemma-5".to_owned(),
                ..RemoteModel::default()
            },
        ],
        "models the native list does not name keep what the OpenAI-compatible list said"
    );
    for request in requests(&server).await {
        if request.url.path() == "/v1beta/models" {
            assert!(
                request.headers.get("authorization").is_none(),
                "the native list takes the key in x-goog-api-key only"
            );
        }
    }
}

#[tokio::test]
async fn a_failing_native_list_keeps_the_gemini_models_as_listed() {
    let server = MockServer::start().await;
    mount_gemini_list(&server, &["gemini-4-hydrogen"]).await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let models = gateway()
        .models(&gemini_provider(&server))
        .await
        .expect("the refresh still succeeds");
    assert_eq!(
        models,
        [RemoteModel {
            remote_id: "gemini-4-hydrogen".to_owned(),
            display_name: "gemini-4-hydrogen".to_owned(),
            ..RemoteModel::default()
        }]
    );

    let unreadable = MockServer::start().await;
    mount_gemini_list(&unreadable, &["gemini-4-hydrogen"]).await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .expect(1)
        .mount(&unreadable)
        .await;
    let models = gateway()
        .models(&gemini_provider(&unreadable))
        .await
        .expect("the refresh still succeeds");
    assert_eq!(models[0].context_window, None);
}

#[tokio::test]
async fn the_native_list_is_read_for_at_most_three_pages() {
    let server = MockServer::start().await;
    mount_gemini_list(&server, &["gemini-4-hydrogen"]).await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{ "name": "models/other", "inputTokenLimit": 1_000 }],
            "nextPageToken": "again",
        })))
        .expect(3)
        .mount(&server)
        .await;
    let models = gateway()
        .models(&gemini_provider(&server))
        .await
        .expect("listed");
    assert_eq!(models[0].context_window, None);
}

// The Gemini spike (P2.B.5): Google's OpenAI-compatible endpoint, with fixtures shaped after
// Google's documentation (thought signatures on function calls, only the first call of a
// parallel step signed, every step of the current turn replayed with its signature). See
// "Gemini Through the OpenAI-Compatible Endpoint" in docs/adr/005-copilot-architecture.md.

fn signed_call(index: u32, id: &str, location: &str, signature: Option<&str>) -> Value {
    let mut call = json!({
        "index": index,
        "id": id,
        "type": "function",
        "function": {
            "name": "weather__get",
            "arguments": json!({ "location": location }).to_string(),
        },
    });
    if let Some(signature) = signature {
        call["extra_content"] = json!({ "google": { "thought_signature": signature } });
    }
    call
}

fn gemini_request(text: &str) -> LlmRequest {
    let mut request = request(text);
    request.tools = vec![weather()];
    request.reasoning = Some(ReasoningLevel::High);
    request
}

#[tokio::test]
async fn gemini_parallel_calls_keep_the_signature_on_the_first_call_only() {
    let server = MockServer::start().await;
    let step = json!({ "role": "assistant", "tool_calls": [
        signed_call(0, "function-call-1", "Paris", Some("c2lnbmF0dXJlLUE=")),
        signed_call(1, "function-call-2", "Rome", None),
    ] });
    Mock::given(method("POST"))
        .respond_with(sse(&[&data(&step), &finish("tool_calls"), "[DONE]"]))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            &chunk("Paris 15, Rome 20."),
            &finish("stop"),
            "[DONE]",
        ]))
        .mount(&server)
        .await;

    let target = target(&server, Preset::Gemini);
    let first = gemini_request("Weather in Paris and Rome?");
    let events = ok(stream_all(&target, first.clone()).await);
    let started: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LlmEvent::ToolCallStarted {
                provider_call_id, ..
            } => provider_call_id.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(started, ["function-call-1", "function-call-2"]);
    let state = events
        .iter()
        .find_map(|event| match event {
            LlmEvent::ReplayState(state) => Some(state.clone()),
            _ => None,
        })
        .expect("a replay state");
    assert_eq!(
        state,
        json!({ "tool_call_extra": {
            "function-call-1": { "extra_content": { "google": { "thought_signature": "c2lnbmF0dXJlLUE=" } } },
        } })
    );

    // FC1 + signature, FC2 in the model turn; FR1, FR2 after it, never interleaved.
    let mut second = first;
    second.messages.extend([
        message(
            Role::Assistant,
            vec![
                call_part("function-call-1", json!({ "location": "Paris" })),
                call_part("function-call-2", json!({ "location": "Rome" })),
            ],
            Some(state),
        ),
        message(
            Role::Tool,
            vec![
                result_part("function-call-1", "15 degrees"),
                result_part("function-call-2", "20 degrees"),
            ],
            None,
        ),
    ]);
    let events = ok(stream_all(&target, second).await);
    assert_eq!(events.last(), Some(&finished(FinishReason::Stop)));

    let sent: Value = requests(&server).await[1].body_json().expect("a JSON body");
    assert_eq!(sent["reasoning_effort"], "high");
    let roles: Vec<&str> = sent["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|message| message["role"].as_str().expect("a role"))
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "tool"]);
    let calls = &sent["messages"][2]["tool_calls"];
    assert_eq!(
        calls[0]["extra_content"],
        json!({ "google": { "thought_signature": "c2lnbmF0dXJlLUE=" } })
    );
    assert!(calls[1].get("extra_content").is_none());
    assert_eq!(sent["messages"][3]["tool_call_id"], "function-call-1");
    assert_eq!(sent["messages"][4]["tool_call_id"], "function-call-2");
}

#[tokio::test]
async fn gemini_sequential_steps_replay_every_signature_of_the_turn() {
    let server = MockServer::start().await;
    for (id, location, signature) in [
        ("function-call-a", "Paris", "c2lnLUE="),
        ("function-call-b", "Rome", "c2lnLUI="),
    ] {
        let step = json!({ "role": "assistant", "tool_calls": [
            signed_call(0, id, location, Some(signature)),
        ] });
        Mock::given(method("POST"))
            .respond_with(sse(&[&data(&step), &finish("tool_calls"), "[DONE]"]))
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .respond_with(sse(&[&chunk("Both checked."), &finish("stop"), "[DONE]"]))
        .mount(&server)
        .await;

    let target = target(&server, Preset::Gemini);
    let mut turn = gemini_request("Check Paris, then Rome.");
    for (id, location) in [("function-call-a", "Paris"), ("function-call-b", "Rome")] {
        let events = ok(stream_all(&target, turn.clone()).await);
        let state = events.iter().find_map(|event| match event {
            LlmEvent::ReplayState(state) => Some(state.clone()),
            _ => None,
        });
        turn.messages.extend([
            message(
                Role::Assistant,
                vec![call_part(id, json!({ "location": location }))],
                state,
            ),
            message(Role::Tool, vec![result_part(id, "ok")], None),
        ]);
    }
    let events = ok(stream_all(&target, turn).await);
    assert_eq!(events.last(), Some(&finished(FinishReason::Stop)));

    let sent: Value = requests(&server).await[2].body_json().expect("a JSON body");
    let signatures: Vec<&Value> = sent["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|message| message.get("tool_calls"))
        .map(|calls| &calls[0]["extra_content"]["google"]["thought_signature"])
        .collect();
    assert_eq!(signatures, [&json!("c2lnLUE="), &json!("c2lnLUI=")]);
}

#[tokio::test]
async fn gemini_takes_images_and_wav_audio_as_documented() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            &chunk("A cat and a meow."),
            &finish("stop"),
            "[DONE]",
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut with_media = request("What is this?");
    with_media.messages[0].parts.extend([
        LlmPart::Image {
            media_type: "image/jpeg".to_owned(),
            data: Bytes::from_static(b"\xff\xd8\xff"),
        },
        LlmPart::Audio {
            format: "wav".to_owned(),
            data: Bytes::from_static(b"RIFF"),
        },
    ]);
    let events = ok(stream_all(&target(&server, Preset::Gemini), with_media).await);
    assert_eq!(events.last(), Some(&finished(FinishReason::Stop)));

    let sent: Value = requests(&server).await[0].body_json().expect("a JSON body");
    let content = &sent["messages"][1]["content"];
    assert_eq!(
        content[1],
        json!({ "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,/9j/" } })
    );
    assert_eq!(
        content[2],
        json!({ "type": "input_audio", "input_audio": { "data": "UklGRg==", "format": "wav" } })
    );
    assert_eq!(sent["messages"][0]["role"], "system");
    assert!(sent.get("stream_options").is_none());
}
