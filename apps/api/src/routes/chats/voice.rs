//! Voice in real turns, end to end (`make test-db`), with wiremock as the provider: audio clips
//! (WAV and MP3 chat files) go to `audio_in` models as `input_audio` parts, also through the
//! `gemini` preset's OpenAI-compatible endpoint, and as a notice to models without it; spoken
//! replies stream `audio.delta` events, keep the transcript as the message text, and save the
//! audio as a WAV chat file with an `audio` part, while later turns send the transcript only;
//! audio in a chat is at most five minutes long; and Claude models never get audio.

use axum::Router;
use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use copilot_database::DbPool;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::helpers::testing::{self, multipart, register, request, send};
use crate::routes::chats::flows::{call, content, messages, setup, text, turn};
use crate::routes::chats::tools::{answer, completions, script, stream};

/// A WAV file of mono PCM16 `pcm` at `rate`, written here apart from the server's writer.
fn wav(pcm: &[u8], rate: u32) -> Vec<u8> {
    let length = u32::try_from(pcm.len()).expect("a short clip");
    let mut file = Vec::with_capacity(44 + pcm.len());
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + length).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    file.extend_from_slice(&16_u32.to_le_bytes());
    file.extend_from_slice(&1_u16.to_le_bytes());
    file.extend_from_slice(&1_u16.to_le_bytes());
    file.extend_from_slice(&rate.to_le_bytes());
    file.extend_from_slice(&(rate * 2).to_le_bytes());
    file.extend_from_slice(&2_u16.to_le_bytes());
    file.extend_from_slice(&16_u16.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&length.to_le_bytes());
    file.extend_from_slice(pcm);
    file
}

/// One second of a 16 kHz push-to-talk clip.
fn clip() -> Vec<u8> {
    let pcm: Vec<u8> = (0..32_000_u32).map(|byte| (byte % 251) as u8).collect();
    wav(&pcm, 16_000)
}

/// Two seconds of a constant-bitrate MP3 (MPEG-1 Layer III, 128 kbps): one frame header, then
/// silence.
fn mp3() -> Vec<u8> {
    let mut file = vec![0xff, 0xfb, 0x90, 0x00];
    file.resize(32_000, 0);
    file
}

/// Uploads `bytes` as `name` to the chat; the file's id.
async fn attach(app: &Router, token: &str, chat: &str, name: &str, bytes: &[u8]) -> String {
    let uploaded = send(
        app.clone(),
        multipart(
            &format!("/api/chats/{chat}/files"),
            token,
            &[("file", name, bytes)],
        ),
    )
    .await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.body);
    uploaded.body["data"]["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

/// Sets the model's capability overrides.
async fn overrides(app: &Router, token: &str, model: &str, flags: Value) -> Value {
    call(
        app,
        token,
        Method::PATCH,
        &format!("/api/models/{model}"),
        Some(json!({ "capabilityOverrides": flags })),
    )
    .await["data"]
        .clone()
}

/// Sends `text` with `files` and reads the turn to its end; the events.
async fn ask(app: &Router, token: &str, chat: &str, text: &str, files: &[&str]) -> Vec<Value> {
    let body = json!({ "text": text, "fileIds": files });
    let all = turn(app, token, chat, "messages", Some(body))
        .await
        .rest()
        .await;
    assert_eq!(all.last().map(|event| &event["type"]), Some(&json!("done")));
    all
}

/// The content parts of the last user message of a completion request.
fn user_parts(request: &Value) -> Vec<Value> {
    let messages = request["messages"].as_array().expect("messages");
    let user = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .expect("a user message");
    user["content"].as_array().cloned().unwrap_or_default()
}

/// The status, headers, and raw bytes of a response.
async fn bytes(app: &Router, uri: &str, token: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.expect("body");
    (status, headers, body.to_bytes().to_vec())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn audio_clips_reach_audio_in_models_as_input_audio(pool: DbPool) {
    let provider = testing::provider().await;
    let (app, token, model) = setup(pool, &format!("{}/v1", provider.uri())).await;
    let app = app.router;
    let model_view = overrides(&app, &token, &model, json!({ "audioIn": true })).await;
    assert_eq!(model_view["capabilities"]["audioIn"], true);
    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model })),
    )
    .await["data"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let wav = clip();
    let mp3 = mp3();
    let spoken = attach(&app, &token, &chat, "clip.wav", &wav).await;
    let note = attach(&app, &token, &chat, "note.mp3", &mp3).await;

    script(&provider, answer("I heard you.")).await;
    let events = ask(&app, &token, &chat, "What did I say?", &[&spoken, &note]).await;
    assert_eq!(text(&events), "I heard you.");
    let sent = completions(&provider).await;
    assert_eq!(
        user_parts(&sent[0]),
        [
            json!({ "type": "text", "text": "What did I say?" }),
            json!({ "type": "input_audio", "input_audio": {
                "data": STANDARD.encode(&wav), "format": "wav",
            } }),
            json!({ "type": "input_audio", "input_audio": {
                "data": STANDARD.encode(&mp3), "format": "mp3",
            } }),
        ]
    );

    let history = messages(&app, &token, &chat).await;
    assert_eq!(
        history[0]["parts"],
        json!([
            { "type": "text", "text": "What did I say?" },
            { "type": "audio", "fileId": spoken, "format": "wav", "transcript": null, "durationMs": null },
            { "type": "audio", "fileId": note, "format": "mp3", "transcript": null, "durationMs": null },
        ])
    );

    // A model without audio input reads a notice instead.
    overrides(&app, &token, &model, json!({ "audioIn": false })).await;
    script(&provider, answer("Fine.")).await;
    ask(&app, &token, &chat, "And now?", &[]).await;
    let sent = completions(&provider).await;
    let first = sent[1]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["role"] == "user")
        .expect("the first question")
        .clone();
    assert_eq!(
        first["content"],
        json!([
            { "type": "text", "text": "What did I say?" },
            { "type": "text", "text": "[audio \"clip.wav\" omitted: this model cannot read audio]" },
            { "type": "text", "text": "[audio \"note.mp3\" omitted: this model cannot read audio]" },
        ])
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn gemini_takes_audio_clips_through_its_openai_compatible_endpoint(pool: DbPool) {
    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/openai/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "id": "models/gemini-4-hydrogen", "object": "model", "owned_by": "google" }],
        })))
        .mount(&provider)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1beta/openai/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(answer("You said hello.")),
        )
        .mount(&provider)
        .await;
    let app = testing::live(pool, testing::config()).await.router;
    let token = register(&app, "admin").await;
    let created = call(
        &app,
        &token,
        Method::POST,
        "/api/providers",
        Some(json!({
            "kind": "openai",
            "preset": "gemini",
            "name": "Gemini",
            "baseUrl": format!("{}/v1beta/openai", provider.uri()),
            "apiKey": "gemini-key",
        })),
    )
    .await;
    let connection = created["data"]["id"].as_str().expect("an id");
    call(
        &app,
        &token,
        Method::POST,
        &format!("/api/providers/{connection}/refresh"),
        None,
    )
    .await;
    let models = call(
        &app,
        &token,
        Method::GET,
        &format!("/api/providers/{connection}/models"),
        None,
    )
    .await;
    assert_eq!(models["items"][0]["remoteId"], "gemini-4-hydrogen");
    let model = models["items"][0]["id"].as_str().expect("an id").to_owned();
    let view = overrides(&app, &token, &model, json!({ "audioIn": true })).await;
    assert_eq!(view["locks"], json!([]), "Gemini audio is not locked");
    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model })),
    )
    .await["data"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let wav = clip();
    let spoken = attach(&app, &token, &chat, "clip.wav", &wav).await;
    let events = ask(&app, &token, &chat, "Transcribe this.", &[&spoken]).await;
    assert_eq!(text(&events), "You said hello.");

    let received = provider.received_requests().await.unwrap_or_default();
    let completion = received
        .iter()
        .find(|request| request.url.path() == "/v1beta/openai/chat/completions")
        .expect("a completion");
    let body: Value = serde_json::from_slice(&completion.body).expect("JSON");
    assert_eq!(body["model"], "gemini-4-hydrogen");
    assert_eq!(
        user_parts(&body)[1],
        json!({ "type": "input_audio", "input_audio": {
            "data": STANDARD.encode(&wav), "format": "wav",
        } })
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn spoken_replies_stream_audio_and_keep_a_wav_file(pool: DbPool) {
    let provider = testing::provider().await;
    let (app, token, model) = setup(pool, &format!("{}/v1", provider.uri())).await;
    let app = app.router;
    overrides(&app, &token, &model, json!({ "audioOut": true })).await;
    let created = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model, "voiceReplies": true })),
    )
    .await;
    assert_eq!(created["data"]["voiceReplies"], true);
    let chat = created["data"]["id"].as_str().expect("an id").to_owned();

    // Half a second of 24 kHz PCM16, in two pieces, with the transcript beside it.
    let first: Vec<u8> = (0..12_000_u32).map(|byte| (byte % 199) as u8).collect();
    let second = vec![9_u8; 12_000];
    script(
        &provider,
        stream(&[
            (
                json!({ "role": "assistant", "content": null, "audio": {
                    "id": "audio_1", "data": STANDARD.encode(&first), "transcript": "Hello ",
                } }),
                Value::Null,
            ),
            (
                json!({ "audio": { "data": STANDARD.encode(&second), "transcript": "there." } }),
                Value::Null,
            ),
            (
                json!({ "audio": { "expires_at": 1_800_000_000 } }),
                Value::Null,
            ),
            (json!({}), json!("stop")),
        ]),
    )
    .await;
    let events = ask(&app, &token, &chat, "Say hello", &[]).await;

    let sent = completions(&provider).await;
    assert_eq!(sent[0]["modalities"], json!(["text", "audio"]));
    assert_eq!(
        sent[0]["audio"],
        json!({ "voice": "alloy", "format": "pcm16" })
    );

    let audio: Vec<&Value> = events
        .iter()
        .filter(|event| event["type"] == "audio.delta")
        .collect();
    assert_eq!(audio.len(), 2);
    assert_eq!(audio[0]["data"], STANDARD.encode(&first));
    assert_eq!(audio[1]["data"], STANDARD.encode(&second));
    assert!(audio.iter().all(|event| event["format"] == "pcm16"));
    assert_eq!(
        text(&events),
        "Hello there.",
        "the transcript streams as text"
    );
    let parts: Vec<(&Value, &Value)> = events
        .iter()
        .filter(|event| event["type"] == "part.started")
        .map(|event| (&event["kind"], &event["index"]))
        .collect();
    assert_eq!(
        parts,
        [(&json!("text"), &json!(0)), (&json!("audio"), &json!(1))]
    );

    let history = messages(&app, &token, &chat).await;
    let reply = &history[1];
    assert_eq!(reply["status"], "complete");
    assert_eq!(content(reply), "Hello there.");
    let part = &reply["parts"][1];
    assert_eq!(part["type"], "audio");
    assert_eq!(part["format"], "wav");
    assert_eq!(part["transcript"], Value::Null);
    assert_eq!(part["durationMs"], 500);
    let file = part["fileId"].as_str().expect("a file");
    let (status, headers, saved) = bytes(&app, &format!("/api/files/{file}/content"), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[CONTENT_TYPE], "audio/wav");
    assert_eq!(saved, wav(&[first, second].concat(), 24_000));

    // Speak replies off: no audio is asked for, and the reply goes back as its transcript.
    let updated = call(
        &app,
        &token,
        Method::PATCH,
        &format!("/api/chats/{chat}"),
        Some(json!({ "voiceReplies": false })),
    )
    .await;
    assert_eq!(updated["data"]["voiceReplies"], false);
    script(&provider, answer("Hi again.")).await;
    ask(&app, &token, &chat, "Once more", &[]).await;
    let sent = completions(&provider).await;
    assert!(sent[1].get("modalities").is_none());
    assert!(sent[1].get("audio").is_none());
    let reply = sent[1]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("the spoken reply");
    assert_eq!(
        reply,
        &json!({ "role": "assistant", "content": "Hello there." }),
        "the transcript only, never the provider's audio id"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn audio_in_a_chat_is_at_most_five_minutes(pool: DbPool) {
    let provider = testing::provider().await;
    let (app, token, model) = setup(pool, &format!("{}/v1", provider.uri())).await;
    let app = app.router;
    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model })),
    )
    .await["data"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    // 1 kHz keeps five minutes small: 2,000 bytes per second.
    let longest = wav(&vec![0; 300 * 2_000], 1_000);
    attach(&app, &token, &chat, "longest.wav", &longest).await;
    let too_long = wav(&vec![0; 301 * 2_000], 1_000);
    let refused = send(
        app.clone(),
        multipart(
            &format!("/api/chats/{chat}/files"),
            &token,
            &[("file", "long.wav", &too_long)],
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused.body["error"],
        "Audio in a chat can be at most 5 minutes long."
    );
    assert_eq!(refused.body["field"], "file");
}

/// An Anthropic connection on `provider` with one Claude model; its id.
async fn claude(app: &Router, token: &str, provider: &MockServer) -> String {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "type": "model",
                "id": "claude-example-1",
                "display_name": "Claude Example",
                "created_at": "2026-01-01T00:00:00Z",
                "max_input_tokens": 200_000,
                "max_tokens": 8_192,
                "capabilities": {
                    "image_input": { "supported": true },
                    "pdf_input": { "supported": true },
                },
            }],
            "has_more": false,
            "first_id": "claude-example-1",
            "last_id": "claude-example-1",
        })))
        .mount(provider)
        .await;
    let created = call(
        app,
        token,
        Method::POST,
        "/api/providers",
        Some(json!({
            "kind": "anthropic",
            "preset": "anthropic",
            "name": "Anthropic",
            "baseUrl": provider.uri(),
            "apiKey": "sk-ant-test",
        })),
    )
    .await;
    let connection = created["data"]["id"].as_str().expect("an id");
    call(
        app,
        token,
        Method::POST,
        &format!("/api/providers/{connection}/refresh"),
        None,
    )
    .await;
    let models = call(
        app,
        token,
        Method::GET,
        &format!("/api/providers/{connection}/models"),
        None,
    )
    .await;
    models["items"][0]["id"].as_str().expect("an id").to_owned()
}

/// A Messages API stream of one text block.
fn claude_answer(text: &str) -> String {
    let events = [
        json!({ "type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-example-1",
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 1 },
        } }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": text } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn", "stop_sequence": null }, "usage": { "output_tokens": 4 } }),
        json!({ "type": "message_stop" }),
    ];
    events
        .iter()
        .map(|event| {
            let name = event["type"].as_str().expect("a type");
            format!("event: {name}\ndata: {event}\n\n")
        })
        .collect()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run `make test-db`"]
async fn claude_models_never_get_audio(pool: DbPool) {
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(claude_answer("I cannot listen.")),
        )
        .mount(&provider)
        .await;
    let app = testing::live(pool, testing::config()).await.router;
    let token = register(&app, "admin").await;
    let model = claude(&app, &token, &provider).await;

    let view = call(&app, &token, Method::GET, "/api/models", None).await["items"][0].clone();
    assert_eq!(view["capabilities"]["audioIn"], false);
    assert_eq!(view["capabilities"]["audioOut"], false);
    let locked: Vec<&Value> = view["locks"]
        .as_array()
        .expect("locks")
        .iter()
        .map(|lock| &lock["capability"])
        .collect();
    assert_eq!(locked, [&json!("audioIn"), &json!("audioOut")]);
    for flag in ["audioIn", "audioOut"] {
        let refused = send(
            app.clone(),
            request(
                Method::PATCH,
                &format!("/api/models/{model}"),
                Some(&token),
                Some(json!({ "capabilityOverrides": { flag: true } })),
            ),
        )
        .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{flag}");
        assert_eq!(refused.body["field"], "capabilityOverrides");
    }

    let chat = call(
        &app,
        &token,
        Method::POST,
        "/api/chats",
        Some(json!({ "modelId": model, "voiceReplies": true })),
    )
    .await["data"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let spoken = attach(&app, &token, &chat, "clip.wav", &clip()).await;
    let events = ask(&app, &token, &chat, "Can you hear me?", &[&spoken]).await;
    assert_eq!(text(&events), "I cannot listen.");
    assert!(
        events.iter().all(|event| event["type"] != "audio.delta"),
        "no spoken reply"
    );

    let received = provider.received_requests().await.unwrap_or_default();
    let sent = received
        .iter()
        .find(|request| request.url.path() == "/v1/messages")
        .expect("a request");
    let body: Value = serde_json::from_slice(&sent.body).expect("JSON");
    assert!(body.get("modalities").is_none());
    assert!(body.get("audio").is_none());
    let blocks = body["messages"][0]["content"].as_array().expect("blocks");
    let texts: Vec<&str> = blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "Can you hear me?",
            "[audio \"clip.wav\" omitted: this model cannot read audio]"
        ]
    );
    let history = messages(&app, &token, &chat).await;
    assert_eq!(
        history[1]["parts"],
        json!([{ "type": "text", "text": "I cannot listen." }])
    );
}
