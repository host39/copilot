//! The OpenAI-compatible adapter against a wiremock provider, through the hardened outbound
//! clients the composition root injects (`copilot_security::clients(Purpose::Providers)`). The
//! mock listens on loopback over plain `http`, which only a private-scope connection may reach.

use copilot_core::{
    BaseUrl, Dialect, DomainResult, FinishReason, LlmEvent, LlmGateway, LlmMessage, LlmPart,
    LlmRequest, ModelTarget, NetworkScope, Preset, ProviderId, ProviderKind, ProviderTarget,
    Refusal, Role, Secret, Usage,
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
async fn requests_the_skeleton_cannot_send_never_reach_the_provider() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&["[DONE]"]))
        .expect(0)
        .mount(&server)
        .await;

    let mut with_image = request("Look");
    with_image.messages[0].parts.push(LlmPart::Image {
        media_type: "image/png".to_owned(),
        data: bytes::Bytes::from_static(b"\x89PNG"),
    });
    let error = gateway()
        .stream(&target(&server, Preset::OpenAi), with_image)
        .await
        .err()
        .expect("unsupported");
    assert_eq!(error.code(), UNSUPPORTED);

    let mut anthropic = target(&server, Preset::Anthropic);
    anthropic.provider.kind = ProviderKind::Anthropic;
    anthropic.dialect = Dialect::Anthropic;
    let error = gateway()
        .stream(&anthropic, request("Hi"))
        .await
        .err()
        .expect("unsupported");
    assert_eq!(error.code(), UNSUPPORTED);
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
