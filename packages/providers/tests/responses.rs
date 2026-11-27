//! The Responses API adapter against a wiremock provider, through the hardened outbound clients
//! the composition root injects (`copilot_security::clients(Purpose::Providers)`). The fixtures
//! follow OpenAI's streaming event reference (`docs/research/providers.md`, OpenAI Responses):
//! every event type the adapter reads, a tool loop replayed statelessly, refusals, incomplete and
//! failed answers, and errors mid-stream.

use bytes::Bytes;
use copilot_core::{
    BaseUrl, CallId, Dialect, DomainResult, FinishReason, LlmEvent, LlmGateway, LlmMessage,
    LlmPart, LlmRequest, ModelTarget, NetworkScope, OpenAiApi, Preset, ProviderId, ProviderKind,
    ProviderTarget, ReasoningLevel, Refusal, Role, Secret, ToolAlias, ToolContent, ToolSpec, Usage,
};
use copilot_providers::{Gateway, UNSUPPORTED};
use copilot_security::{Purpose, clients};
use futures_util::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn gateway() -> Gateway {
    Gateway::new(clients(Purpose::Providers).expect("clients build"))
}

fn target(server: &MockServer, preset: Preset) -> ModelTarget {
    ModelTarget {
        provider: ProviderTarget {
            provider_id: ProviderId::generate(),
            kind: ProviderKind::OpenAi,
            preset,
            api: Some(OpenAiApi::Responses),
            base_url: BaseUrl::parse(&format!("{}/v1", server.uri()), NetworkScope::Private)
                .expect("a loopback URL"),
            api_key: Some(Secret::new("sk-test")),
            network: NetworkScope::Private,
        },
        remote_id: "gpt-test".to_owned(),
        dialect: Dialect::new(ProviderKind::OpenAi, preset),
    }
}

fn request(messages: Vec<LlmMessage>) -> LlmRequest {
    LlmRequest {
        model: "gpt-test".to_owned(),
        system: "Be brief.".to_owned(),
        messages,
        tools: Vec::new(),
        max_output_tokens: 256,
        reasoning: None,
        can_reason: false,
        audio_out: None,
        cache_breakpoints: false,
    }
}

fn user(text: &str) -> LlmMessage {
    LlmMessage {
        role: Role::User,
        parts: vec![LlmPart::Text(text.to_owned())],
        replay: None,
    }
}

/// An SSE body of named events, the name repeated in each data's `type`, as OpenAI sends them.
fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|event| {
            let name = event["type"].as_str().expect("a type");
            format!("event: {name}\ndata: {event}\n\n")
        })
        .collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn created() -> Value {
    json!({ "type": "response.created", "sequence_number": 0, "response": {
        "id": "resp_1", "object": "response", "status": "in_progress", "store": false, "output": [],
    } })
}

fn added(output: u64, item: Value) -> Value {
    json!({ "type": "response.output_item.added", "output_index": output, "item": item, "sequence_number": 1 })
}

fn done(output: u64, item: Value) -> Value {
    json!({ "type": "response.output_item.done", "output_index": output, "item": item, "sequence_number": 8 })
}

fn text_delta(output: u64, delta: &str) -> Value {
    json!({ "type": "response.output_text.delta", "item_id": "msg_1", "output_index": output, "content_index": 0, "delta": delta, "logprobs": [], "sequence_number": 3 })
}

fn message(text: &str) -> Value {
    json!({ "id": "msg_1", "type": "message", "role": "assistant", "status": "completed",
        "content": [{ "type": "output_text", "text": text, "annotations": [], "logprobs": [] }] })
}

fn reasoning() -> Value {
    json!({ "id": "rs_1", "type": "reasoning", "status": "completed",
        "summary": [{ "type": "summary_text", "text": "**Checking the docs**" }],
        "encrypted_content": "gAAAAB-opaque" })
}

fn function_call() -> Value {
    json!({ "id": "fc_1", "type": "function_call", "status": "completed", "call_id": "call_abc",
        "name": "docs__search", "arguments": "{\"q\":\"x\"}" })
}

fn completed(output: &[Value]) -> Value {
    json!({ "type": "response.completed", "sequence_number": 20, "response": {
        "id": "resp_1", "object": "response", "status": "completed", "error": null,
        "incomplete_details": null, "store": false, "output": output,
        "usage": {
            "input_tokens": 50, "input_tokens_details": { "cached_tokens": 0 },
            "output_tokens": 12, "output_tokens_details": { "reasoning_tokens": 4 },
            "total_tokens": 62,
        },
    } })
}

/// A whole streamed text answer.
fn hello() -> Vec<Value> {
    vec![
        created(),
        json!({ "type": "response.in_progress", "sequence_number": 1, "response": { "id": "resp_1", "status": "in_progress" } }),
        added(
            0,
            json!({ "id": "msg_1", "type": "message", "role": "assistant", "status": "in_progress", "content": [] }),
        ),
        json!({ "type": "response.content_part.added", "item_id": "msg_1", "output_index": 0, "content_index": 0, "part": { "type": "output_text", "text": "", "annotations": [] } }),
        text_delta(0, "Hello"),
        text_delta(0, ", world"),
        json!({ "type": "response.output_text.done", "item_id": "msg_1", "output_index": 0, "content_index": 0, "text": "Hello, world" }),
        json!({ "type": "response.content_part.done", "item_id": "msg_1", "output_index": 0, "content_index": 0, "part": { "type": "output_text", "text": "Hello, world" } }),
        done(0, message("Hello, world")),
        completed(&[message("Hello, world")]),
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

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.expect("recording")
}

fn body(request: &Request) -> Value {
    serde_json::from_slice(&request.body).expect("a JSON body")
}

#[tokio::test]
async fn streams_text_and_usage_from_a_stateless_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer sk-test"))
        .and(header("accept", "text/event-stream"))
        .respond_with(sse(&hello()))
        .expect(1)
        .mount(&server)
        .await;
    let events = ok(stream_all(&target(&server, Preset::OpenAi), request(vec![user("Hi")])).await);
    assert_eq!(
        events,
        [
            LlmEvent::TextDelta {
                index: 0,
                text: "Hello".to_owned()
            },
            LlmEvent::TextDelta {
                index: 0,
                text: ", world".to_owned()
            },
            LlmEvent::Usage(Usage {
                input_tokens: Some(50),
                output_tokens: Some(12),
                reasoning_tokens: Some(4),
                cache_read_tokens: Some(0),
                cache_write_tokens: None,
            }),
            LlmEvent::ReplayState(
                json!({ "api": "responses", "items": [message("Hello, world")] })
            ),
            LlmEvent::Finished {
                reason: FinishReason::Stop,
                refusal: None
            },
        ]
    );
    let sent = body(&requests(&server).await[0]);
    assert_eq!(
        sent,
        json!({
            "model": "gpt-test",
            "stream": true,
            "store": false,
            "instructions": "Be brief.",
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Hi" }] }],
            "max_output_tokens": 256,
        })
    );
}

#[tokio::test]
async fn a_custom_connection_on_responses_uses_its_own_base_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&hello()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let events = ok(stream_all(&target(&server, Preset::Custom), request(vec![user("Hi")])).await);
    assert!(matches!(
        events.last(),
        Some(LlmEvent::Finished {
            reason: FinishReason::Stop,
            ..
        })
    ));
}

fn search() -> ToolSpec {
    ToolSpec {
        alias: ToolAlias::restore("docs__search".to_owned()),
        description: "Search the docs.".to_owned(),
        input_schema: json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
    }
}

/// The first round: reasoning, then a function call.
fn calling() -> Vec<Value> {
    vec![
        created(),
        added(
            0,
            json!({ "id": "rs_1", "type": "reasoning", "summary": [] }),
        ),
        json!({ "type": "response.reasoning_summary_part.added", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "part": { "type": "summary_text", "text": "" } }),
        json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "delta": "**Checking the docs**" }),
        json!({ "type": "response.reasoning_summary_text.done", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "text": "**Checking the docs**" }),
        json!({ "type": "response.reasoning_summary_part.done", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "part": { "type": "summary_text", "text": "**Checking the docs**" } }),
        done(0, reasoning()),
        added(
            1,
            json!({ "id": "fc_1", "type": "function_call", "status": "in_progress", "call_id": "call_abc", "name": "docs__search", "arguments": "" }),
        ),
        json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "{\"q\":" }),
        json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "\"x\"}" }),
        json!({ "type": "response.function_call_arguments.done", "item_id": "fc_1", "output_index": 1, "arguments": "{\"q\":\"x\"}" }),
        done(1, function_call()),
        completed(&[reasoning(), function_call()]),
    ]
}

#[tokio::test]
async fn a_tool_loop_replays_the_encrypted_reasoning_statelessly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(json!({ "input": [{ "role": "user" }] })))
        .respond_with(sse(&calling()))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&hello()))
        .mount(&server)
        .await;
    let target = target(&server, Preset::OpenAi);

    let mut first = request(vec![user("Find x")]);
    first.tools = vec![search()];
    first.can_reason = true;
    first.reasoning = Some(ReasoningLevel::Medium);
    let events = ok(stream_all(&target, first.clone()).await);
    assert_eq!(
        events[..5],
        [
            LlmEvent::ReasoningDelta {
                index: 0,
                text: "**Checking the docs**".to_owned()
            },
            LlmEvent::ToolCallStarted {
                index: 1,
                provider_call_id: Some("call_abc".to_owned()),
                alias: "docs__search".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 1,
                json: "{\"q\":".to_owned()
            },
            LlmEvent::ToolCallArgs {
                index: 1,
                json: "\"x\"}".to_owned()
            },
            LlmEvent::ToolCallEnded { index: 1 },
        ]
    );
    let LlmEvent::ReplayState(state) = &events[6] else {
        panic!("replay state before the finish: {:?}", events[6]);
    };
    assert_eq!(
        state,
        &json!({ "api": "responses", "items": [reasoning(), function_call()] })
    );
    assert_eq!(
        events[7],
        LlmEvent::Finished {
            reason: FinishReason::ToolCalls,
            refusal: None
        }
    );

    // The next request carries the answer's items and the result, as core lowers them.
    let mut second = first;
    second.messages.push(LlmMessage {
        role: Role::Assistant,
        parts: vec![LlmPart::ToolCall {
            call_id: CallId::generate().uuid(),
            provider_call_id: "call_abc".to_owned(),
            alias: "docs__search".to_owned(),
            arguments: json!({ "q": "x" }),
        }],
        replay: Some(state.clone()),
    });
    second.messages.push(LlmMessage {
        role: Role::Tool,
        parts: vec![LlmPart::ToolResult {
            call_id: CallId::generate().uuid(),
            provider_call_id: "call_abc".to_owned(),
            alias: "docs__search".to_owned(),
            is_error: false,
            content: vec![ToolContent::Text("x is 42".to_owned())],
        }],
        replay: None,
    });
    let events = ok(stream_all(&target, second).await);
    assert!(matches!(
        events.last(),
        Some(LlmEvent::Finished {
            reason: FinishReason::Stop,
            ..
        })
    ));

    let sent = requests(&server).await;
    assert_eq!(sent.len(), 2);
    let first = body(&sent[0]);
    assert_eq!(
        first["reasoning"],
        json!({ "effort": "medium", "summary": "auto" })
    );
    assert_eq!(first["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(first["tools"][0]["name"], "docs__search");
    assert_eq!(first["tools"][0]["strict"], false);
    assert_eq!(first["tool_choice"], "auto");
    let second = body(&sent[1]);
    assert_eq!(
        second["input"],
        json!([
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Find x" }] },
            reasoning(),
            function_call(),
            { "type": "function_call_output", "call_id": "call_abc", "output": "x is 42" },
        ])
    );
}

#[tokio::test]
async fn refusals_and_incomplete_answers_finish_by_their_reason() {
    let server = MockServer::start().await;
    let refused = vec![
        created(),
        added(
            0,
            json!({ "id": "msg_1", "type": "message", "role": "assistant", "content": [] }),
        ),
        json!({ "type": "response.content_part.added", "item_id": "msg_1", "output_index": 0, "content_index": 0, "part": { "type": "refusal", "refusal": "" } }),
        json!({ "type": "response.refusal.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "I can't help with that." }),
        json!({ "type": "response.refusal.done", "item_id": "msg_1", "output_index": 0, "content_index": 0, "refusal": "I can't help with that." }),
        completed(&[]),
    ];
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "refuse" }] }] }),
    ))
    .respond_with(sse(&refused))
    .mount(&server)
    .await;
    let cut = vec![
        created(),
        text_delta(0, "Partial"),
        json!({ "type": "response.incomplete", "response": {
            "id": "resp_1", "status": "incomplete", "incomplete_details": { "reason": "max_output_tokens" },
            "output": [], "usage": { "input_tokens": 5, "output_tokens": 256 },
        } }),
    ];
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "long" }] }] }),
    ))
    .respond_with(sse(&cut))
    .mount(&server)
    .await;
    let target = target(&server, Preset::OpenAi);

    let events = ok(stream_all(&target, request(vec![user("refuse")])).await);
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
    let events = ok(stream_all(&target, request(vec![user("long")])).await);
    assert_eq!(
        events.last(),
        Some(&LlmEvent::Finished {
            reason: FinishReason::Length,
            refusal: None
        })
    );
    assert!(events.iter().any(|event| matches!(
        event,
        LlmEvent::Usage(Usage {
            output_tokens: Some(256),
            ..
        })
    )));
}

#[tokio::test]
async fn mid_stream_failures_end_the_answer_without_echoing_the_provider() {
    let server = MockServer::start().await;
    let failed = vec![
        created(),
        text_delta(0, "Hel"),
        json!({ "type": "response.failed", "response": {
            "id": "resp_1", "status": "failed", "output": [],
            "error": { "code": "server_error", "message": "secret internals" },
        } }),
    ];
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "failed" }] }] }),
    ))
    .respond_with(sse(&failed))
    .mount(&server)
    .await;
    let error_event = vec![
        created(),
        text_delta(0, "Hel"),
        json!({ "type": "error", "code": "rate_limit_exceeded", "message": "secret", "param": null, "sequence_number": 3 }),
        text_delta(0, "never read"),
    ];
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "error" }] }] }),
    ))
    .respond_with(sse(&error_event))
    .mount(&server)
    .await;
    let cut_off = vec![created(), text_delta(0, "Hel")];
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "cut" }] }] }),
    ))
    .respond_with(sse(&cut_off))
    .mount(&server)
    .await;
    let target = target(&server, Preset::OpenAi);

    let events = stream_all(&target, request(vec![user("failed")])).await;
    assert_eq!(events.len(), 2, "text, then the failure: {events:?}");
    assert!(events[0].is_ok());
    let error = events[1].as_ref().expect_err("a failure");
    assert_eq!(error.code(), "PROVIDER_ERROR");
    assert_eq!(
        error.to_string(),
        "The provider ended the answer with an error."
    );

    let events = stream_all(&target, request(vec![user("error")])).await;
    assert_eq!(events.len(), 2, "nothing after the error is read");
    let error = events[1].as_ref().expect_err("a failure");
    assert_eq!(error.code(), "PROVIDER_RATE_LIMITED");
    assert!(!error.to_string().contains("secret"));

    let events = ok(stream_all(&target, request(vec![user("cut")])).await);
    assert_eq!(
        events,
        [LlmEvent::TextDelta {
            index: 0,
            text: "Hel".to_owned()
        }],
        "a body that ends without a terminal event reports no finish"
    );
}

#[tokio::test]
async fn http_errors_and_json_replies_fail_before_the_stream() {
    let server = MockServer::start().await;
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "key" }] }] }),
    ))
    .respond_with(
        ResponseTemplate::new(401).set_body_json(json!({ "error": { "message": "bad key" } })),
    )
    .mount(&server)
    .await;
    Mock::given(body_partial_json(
        json!({ "input": [{ "content": [{ "text": "json" }] }] }),
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(
        json!({ "error": { "message": "Unsupported parameter: store", "code": "unsupported" } }),
    ))
    .mount(&server)
    .await;
    let target = target(&server, Preset::Custom);
    let gateway = gateway();
    let error = gateway
        .stream(&target, request(vec![user("key")]))
        .await
        .err()
        .expect("a failure");
    assert_eq!(error.code(), "PROVIDER_AUTH");
    let error = gateway
        .stream(&target, request(vec![user("json")]))
        .await
        .err()
        .expect("a failure");
    assert_eq!(error.code(), "PROVIDER_ERROR");
    assert!(!error.to_string().contains("store"));
}

#[tokio::test]
async fn sends_images_and_pdfs_and_never_sends_audio() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse(&hello()))
        .mount(&server)
        .await;
    let target = target(&server, Preset::OpenAi);
    let files = LlmMessage {
        role: Role::User,
        parts: vec![
            LlmPart::Text("What is in these?".to_owned()),
            LlmPart::Image {
                media_type: "image/png".to_owned(),
                data: Bytes::from_static(b"\x89PNG"),
            },
            LlmPart::Pdf {
                name: "report.pdf".to_owned(),
                data: Bytes::from_static(b"%PDF"),
            },
        ],
        replay: None,
    };
    ok(stream_all(&target, request(vec![files])).await);
    let sent = body(&requests(&server).await[0]);
    assert_eq!(
        sent["input"][0]["content"],
        json!([
            { "type": "input_text", "text": "What is in these?" },
            { "type": "input_image", "image_url": "data:image/png;base64,iVBORw==" },
            { "type": "input_file", "filename": "report.pdf", "file_data": "data:application/pdf;base64,JVBERg==" },
        ])
    );

    let audio = LlmMessage {
        role: Role::User,
        parts: vec![LlmPart::Audio {
            format: "wav".to_owned(),
            data: Bytes::from_static(b"RIFF"),
        }],
        replay: None,
    };
    let error = gateway()
        .stream(&target, request(vec![audio]))
        .await
        .err()
        .expect("refused");
    assert_eq!(error.code(), UNSUPPORTED);
    assert_eq!(requests(&server).await.len(), 1, "nothing more was sent");
}

#[tokio::test]
async fn models_are_listed_from_models_on_the_responses_api() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "id": "gpt-6.1-sol", "object": "model", "created": 1, "owned_by": "openai" }],
        })))
        .expect(1)
        .mount(&server)
        .await;
    let listed = gateway()
        .models(&target(&server, Preset::OpenAi).provider)
        .await
        .expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].remote_id, "gpt-6.1-sol");
}
