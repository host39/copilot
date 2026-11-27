//! Contract conformance, Rust side: writes sample JSON for every contract type on the wire to
//! `packages/contracts/test/fixtures/`, where `test/fixtures.test.ts` parses each sample with its
//! Zod schema.
//!
//! - `types/<Type>.json`: an array of samples of one generated type. Enums list every value,
//!   tagged unions every variant, and structs a sample with every optional field set (plus a
//!   minimal one where fields are optional or nullable).
//! - `bodies/<name>.json`: samples of the success body that `ts/index.ts` names
//!   `<name>BodySchema` (the envelope around a response DTO).
//!
//! Every sample must read back in Rust to the value that wrote it. `cargo test` (and
//! `make contracts`) rewrites the files and removes stale ones; CI fails when the committed files
//! differ from what this test writes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use copilot_contracts::{
    Approval, ApprovalMode, ApprovedTool, AuthMode, BuiltinTool, CallDecision, Capabilities,
    Capability, CapabilityLock, CapabilityOverrides, Capacity, CapacityLevel, ChatListQuery,
    ChatResponse, ChatStreamEvent, ChatSummaryResponse, CreateChatRequest, CreateProjectRequest,
    CreateProviderRequest, CreateScheduleRequest, CreateToolServerRequest, CreatedBy, Credentials,
    Decision, DecisionScope, DecisionsRequest, DetailsResponse, ErrorResponse, FileResponse,
    FinishReason, HealthResponse, KeyState, ListResponse, MessagePartDto, MessageResponse,
    MessageStatus, ModelResponse, MoveFileRequest, OAuthClientInput, OAuthStartResponse,
    OAuthStatus, OnAsk, PageResponse, PartKind, Preset, ProjectDetailsResponse, ProjectListQuery,
    ProjectResponse, ProviderApi, ProviderDetailsResponse, ProviderKind, ProviderResponse,
    ProviderUsage, ReadyResponse, ReasoningLevel, RefreshResponse, Refusal, ResultContent, Role,
    RunStatus, ScheduleListQuery, SchedulePreviewRequest, SchedulePreviewResponse,
    ScheduleResponse, ScheduleRunResponse, SearchHitResponse, SearchQuery, SendMessageRequest,
    ServerTool, SettingsResponse, SignInState, StreamQuery, ToolChanges, ToolMode, ToolOverride,
    ToolServerResponse, ToolStatus, ToolTestResponse, UpdateChatRequest, UpdateFileRequest,
    UpdateModelRequest, UpdateProjectRequest, UpdateProviderRequest, UpdateScheduleRequest,
    UpdateSettingsRequest, UpdateToolServerRequest, Usage, UserEvent,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

/// Writes the fixture files and remembers them, so stale ones can be removed afterwards.
struct Fixtures {
    root: PathBuf,
    written: BTreeSet<PathBuf>,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("test/fixtures"),
            written: BTreeSet::new(),
        }
    }

    /// `types/<name>.json`: samples of the generated type `name`.
    fn types<T>(&mut self, name: &str, samples: &[T])
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        self.write("types", name, samples);
    }

    /// `bodies/<name>.json`: samples of the route body `<name>BodySchema`.
    fn bodies<T>(&mut self, name: &str, samples: &[T])
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        self.write("bodies", name, samples);
    }

    fn write<T>(&mut self, folder: &str, name: &str, samples: &[T])
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        assert!(!samples.is_empty(), "{folder}/{name} needs a sample");
        let values: Vec<Value> = samples
            .iter()
            .map(|sample| readback(name, sample))
            .collect();
        let text = serde_json::to_string_pretty(&values).expect("samples serialize") + "\n";
        let path = self.root.join(folder).join(format!("{name}.json"));
        assert!(
            self.written.insert(path.clone()),
            "{folder}/{name} written twice"
        );
        fs::create_dir_all(path.parent().expect("fixture folder")).expect("creates the folder");
        if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            fs::write(&path, text).expect("writes the fixture");
        }
    }

    /// Removes fixture files this run did not write (a type or body that no longer exists).
    fn prune(&self) {
        for folder in ["types", "bodies"] {
            let entries = fs::read_dir(self.root.join(folder)).expect("reads the folder");
            for entry in entries {
                let path = entry.expect("reads an entry").path();
                if !self.written.contains(&path) {
                    fs::remove_file(&path).expect("removes a stale fixture");
                }
            }
        }
    }
}

/// The sample as JSON, after checking that it reads back to the same value.
fn readback<T>(name: &str, sample: &T) -> Value
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let value = serde_json::to_value(sample).expect("sample serializes");
    let back: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("{name} does not read its own JSON: {error}"));
    assert_eq!(&back, sample, "{name} reads back to another value");
    value
}

/// A distinct, valid version 7 UUID per `n` (Zod's `z.uuid()` checks version and variant).
fn id(n: u16) -> Uuid {
    Uuid::from_u128(0x019a_bf6c_2f6e_7a51_8c3e_0000_0000_0000 | u128::from(n))
}

/// A timestamp with microseconds, `minute` minutes after noon UTC on 2026-11-26.
fn at(minute: u32) -> DateTime<Utc> {
    format!("2026-11-26T12:{minute:02}:00.123456Z")
        .parse()
        .expect("valid timestamp")
}

#[test]
fn export_fixtures() {
    let mut fixtures = Fixtures::new();
    shared(&mut fixtures);
    providers(&mut fixtures);
    models(&mut fixtures);
    files(&mut fixtures);
    projects(&mut fixtures);
    tool_servers(&mut fixtures);
    schedules(&mut fixtures);
    messages(&mut fixtures);
    chats(&mut fixtures);
    events(&mut fixtures);
    fixtures.prune();
}

fn shared(fixtures: &mut Fixtures) {
    fixtures.types(
        "ReasoningLevel",
        &[
            ReasoningLevel::Off,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
        ],
    );
    fixtures.types("BuiltinTool", &[BuiltinTool::Schedules]);
    fixtures.types(
        "KeyState",
        &[KeyState::Missing, KeyState::Set, KeyState::Unreadable],
    );
    fixtures.types(
        "HealthResponse",
        &[HealthResponse {
            status: "healthy".to_owned(),
        }],
    );
    fixtures.types(
        "ReadyResponse",
        &[ReadyResponse {
            status: "ready".to_owned(),
            database: "connected".to_owned(),
        }],
    );
    fixtures.types(
        "ErrorResponse",
        &[
            ErrorResponse {
                error: "Name is required.".to_owned(),
                code: Some("VALIDATION_ERROR".to_owned()),
                field: Some("name".to_owned()),
                details: Some(BTreeMap::from([("retryAfterSecs".to_owned(), json!(30))])),
                request_id: Some("c0ffee".to_owned()),
            },
            ErrorResponse::new("Chat not found.", "NOT_FOUND"),
        ],
    );

    let settings = SettingsResponse {
        instructions: "Answer in English.".to_owned(),
    };
    fixtures.types("SettingsResponse", std::slice::from_ref(&settings));
    fixtures.types(
        "UpdateSettingsRequest",
        &[
            UpdateSettingsRequest {
                instructions: Some("Be brief.".to_owned()),
            },
            UpdateSettingsRequest::default(),
        ],
    );
    fixtures.bodies("settings", &[DetailsResponse { data: settings }]);
}

fn provider() -> ProviderResponse {
    ProviderResponse {
        id: id(1),
        kind: ProviderKind::OpenAi,
        preset: Preset::OpenRouter,
        api: Some(ProviderApi::ChatCompletions),
        name: "OpenRouter".to_owned(),
        base_url: "https://openrouter.ai/api/v1".to_owned(),
        enabled: true,
        key: KeyState::Set,
        created_at: at(0),
        updated_at: at(1),
    }
}

fn providers(fixtures: &mut Fixtures) {
    fixtures.types(
        "ProviderKind",
        &[ProviderKind::OpenAi, ProviderKind::Anthropic],
    );
    fixtures.types(
        "ProviderApi",
        &[ProviderApi::ChatCompletions, ProviderApi::Responses],
    );
    fixtures.types(
        "Preset",
        &[
            Preset::OpenAi,
            Preset::Anthropic,
            Preset::Gemini,
            Preset::OpenRouter,
            Preset::Ollama,
            Preset::Custom,
        ],
    );
    let usage = ProviderUsage {
        chats: 3,
        projects: 1,
        schedules: 2,
    };
    let details = ProviderDetailsResponse {
        provider: provider(),
        usage,
    };
    let refresh = RefreshResponse {
        added: 2,
        updated: 1,
        unavailable: 0,
        enabled: 2,
    };
    let claude = ProviderResponse {
        kind: ProviderKind::Anthropic,
        preset: Preset::Anthropic,
        api: None,
        name: "Anthropic".to_owned(),
        base_url: "https://api.anthropic.com".to_owned(),
        ..provider()
    };
    fixtures.types("ProviderResponse", &[provider(), claude]);
    fixtures.types("ProviderUsage", &[usage]);
    fixtures.types("ProviderDetailsResponse", std::slice::from_ref(&details));
    fixtures.types("RefreshResponse", &[refresh]);
    fixtures.types(
        "CreateProviderRequest",
        &[
            CreateProviderRequest {
                kind: ProviderKind::OpenAi,
                preset: Preset::Custom,
                api: Some(ProviderApi::Responses),
                name: "Local".to_owned(),
                base_url: Some("http://192.168.1.20:8000/v1".to_owned()),
                api_key: Some("sk-sample".to_owned()),
            },
            CreateProviderRequest {
                kind: ProviderKind::Anthropic,
                preset: Preset::Anthropic,
                api: None,
                name: "Anthropic".to_owned(),
                base_url: None,
                api_key: None,
            },
        ],
    );
    fixtures.types(
        "UpdateProviderRequest",
        &[
            UpdateProviderRequest {
                name: Some("Renamed".to_owned()),
                api: Some(ProviderApi::ChatCompletions),
                base_url: Some("https://api.example.com/v1".to_owned()),
                api_key: Some("sk-new".to_owned()),
                enabled: Some(false),
            },
            UpdateProviderRequest::default(),
        ],
    );
    fixtures.bodies(
        "providerList",
        &[
            ListResponse {
                items: vec![provider()],
                total: 1,
            },
            ListResponse {
                items: Vec::new(),
                total: 0,
            },
        ],
    );
    fixtures.bodies("provider", &[DetailsResponse { data: provider() }]);
    fixtures.bodies("providerDetails", &[DetailsResponse { data: details }]);
    fixtures.bodies("refresh", &[DetailsResponse { data: refresh }]);
}

fn capabilities(reasoning: bool) -> Capabilities {
    Capabilities {
        vision: true,
        pdf: false,
        tools: true,
        audio_in: false,
        audio_out: false,
        reasoning,
        reasoning_levels: if reasoning {
            vec![
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
            ]
        } else {
            Vec::new()
        },
        default_reasoning: reasoning.then_some(ReasoningLevel::Medium),
    }
}

fn model() -> ModelResponse {
    ModelResponse {
        id: id(2),
        provider_id: id(1),
        provider_name: "OpenRouter".to_owned(),
        kind: ProviderKind::OpenAi,
        preset: Preset::OpenRouter,
        remote_id: "example/model-large".to_owned(),
        display_name: "Model Large".to_owned(),
        enabled: true,
        available: true,
        context_window: Some(200_000),
        max_output_tokens: Some(32_768),
        capabilities: capabilities(true),
        detected: capabilities(false),
        overrides: CapabilityOverrides {
            reasoning: Some(true),
            ..CapabilityOverrides::default()
        },
        locks: vec![CapabilityLock {
            capability: Capability::AudioOut,
            reason: "This provider cannot stream audio.".to_owned(),
        }],
        reasoning_off_with_tools: false,
        created_at: at(2),
        updated_at: at(3),
    }
}

fn models(fixtures: &mut Fixtures) {
    fixtures.types(
        "Capability",
        &[
            Capability::Vision,
            Capability::Pdf,
            Capability::Tools,
            Capability::AudioIn,
            Capability::AudioOut,
            Capability::Reasoning,
        ],
    );
    fixtures.types("Capabilities", &[capabilities(true), capabilities(false)]);
    let overrides = CapabilityOverrides {
        vision: Some(true),
        pdf: Some(false),
        tools: Some(true),
        audio_in: Some(false),
        audio_out: Some(false),
        reasoning: Some(true),
    };
    fixtures.types(
        "CapabilityOverrides",
        &[overrides, CapabilityOverrides::default()],
    );
    fixtures.types(
        "CapabilityLock",
        &[CapabilityLock {
            capability: Capability::Pdf,
            reason: "This provider cannot read PDFs.".to_owned(),
        }],
    );
    let bare = ModelResponse {
        context_window: None,
        max_output_tokens: None,
        locks: Vec::new(),
        reasoning_off_with_tools: true,
        ..model()
    };
    fixtures.types("ModelResponse", &[model(), bare]);
    fixtures.types(
        "UpdateModelRequest",
        &[
            UpdateModelRequest {
                enabled: Some(true),
                display_name: Some("Large".to_owned()),
                capability_overrides: Some(overrides),
                context_window: Some(Some(128_000)),
                max_output_tokens: Some(Some(8_192)),
            },
            UpdateModelRequest {
                context_window: Some(None),
                max_output_tokens: Some(None),
                ..UpdateModelRequest::default()
            },
            UpdateModelRequest::default(),
        ],
    );
    fixtures.bodies(
        "modelList",
        &[ListResponse {
            items: vec![model()],
            total: 1,
        }],
    );
    fixtures.bodies("model", &[DetailsResponse { data: model() }]);
}

fn file() -> FileResponse {
    FileResponse {
        id: id(3),
        project_id: Some(id(4)),
        chat_id: None,
        name: "report.pdf".to_owned(),
        media_type: "application/pdf".to_owned(),
        size_bytes: 524_288,
        token_estimate: 12_000,
        page_count: Some(12),
        included: true,
        created_at: at(4),
    }
}

fn files(fixtures: &mut Fixtures) {
    let attachment = FileResponse {
        project_id: None,
        chat_id: Some(id(5)),
        name: "photo.png".to_owned(),
        media_type: "image/png".to_owned(),
        page_count: None,
        included: false,
        ..file()
    };
    fixtures.types("FileResponse", &[file(), attachment]);
    fixtures.types(
        "UpdateFileRequest",
        &[
            UpdateFileRequest {
                name: Some("summary.pdf".to_owned()),
                included: Some(false),
            },
            UpdateFileRequest::default(),
        ],
    );
    fixtures.types("MoveFileRequest", &[MoveFileRequest { project_id: id(4) }]);
    fixtures.types(
        "CapacityLevel",
        &[
            CapacityLevel::Unknown,
            CapacityLevel::Fits,
            CapacityLevel::Warn,
            CapacityLevel::Over,
        ],
    );
    fixtures.types(
        "Capacity",
        &[
            Capacity {
                used: 12_000,
                window: Some(200_000),
                level: CapacityLevel::Fits,
            },
            Capacity {
                used: 0,
                window: None,
                level: CapacityLevel::Unknown,
            },
        ],
    );
    fixtures.bodies(
        "fileList",
        &[ListResponse {
            items: vec![file()],
            total: 1,
        }],
    );
    fixtures.bodies("file", &[DetailsResponse { data: file() }]);
}

fn project() -> ProjectResponse {
    ProjectResponse {
        id: id(4),
        name: "Research".to_owned(),
        icon: "🔬".to_owned(),
        instructions: "Cite sources.".to_owned(),
        default_model_id: Some(id(2)),
        reasoning: Some(ReasoningLevel::High),
        builtin_tools: vec![BuiltinTool::Schedules],
        tool_server_ids: vec![id(6)],
        pinned: true,
        created_at: at(5),
        updated_at: at(6),
    }
}

fn projects(fixtures: &mut Fixtures) {
    let bare = ProjectResponse {
        icon: String::new(),
        instructions: String::new(),
        default_model_id: None,
        reasoning: None,
        builtin_tools: Vec::new(),
        tool_server_ids: Vec::new(),
        pinned: false,
        ..project()
    };
    let details = ProjectDetailsResponse {
        project: project(),
        capacity: Capacity {
            used: 180_000,
            window: Some(200_000),
            level: CapacityLevel::Warn,
        },
    };
    fixtures.types("ProjectResponse", &[project(), bare.clone()]);
    fixtures.types("ProjectDetailsResponse", std::slice::from_ref(&details));
    fixtures.types(
        "ProjectListQuery",
        &[
            ProjectListQuery {
                pinned: Some(true),
                limit: Some(5),
            },
            ProjectListQuery::default(),
        ],
    );
    fixtures.types(
        "CreateProjectRequest",
        &[
            CreateProjectRequest {
                name: "Research".to_owned(),
                icon: Some("🔬".to_owned()),
                instructions: Some("Cite sources.".to_owned()),
                default_model_id: Some(id(2)),
                reasoning: Some(ReasoningLevel::Low),
                builtin_tools: Some(vec![BuiltinTool::Schedules]),
                tool_server_ids: Some(vec![id(6)]),
                pinned: Some(true),
            },
            CreateProjectRequest {
                name: "Notes".to_owned(),
                ..CreateProjectRequest::default()
            },
        ],
    );
    fixtures.types(
        "UpdateProjectRequest",
        &[
            UpdateProjectRequest {
                name: Some("Renamed".to_owned()),
                icon: Some("📚".to_owned()),
                instructions: Some(String::new()),
                default_model_id: Some(Some(id(2))),
                reasoning: Some(Some(ReasoningLevel::Off)),
                builtin_tools: Some(Vec::new()),
                tool_server_ids: Some(vec![id(6)]),
                pinned: Some(false),
            },
            UpdateProjectRequest {
                default_model_id: Some(None),
                reasoning: Some(None),
                ..UpdateProjectRequest::default()
            },
            UpdateProjectRequest::default(),
        ],
    );
    fixtures.bodies(
        "projectList",
        &[ListResponse {
            items: vec![project(), bare],
            total: 2,
        }],
    );
    fixtures.bodies("project", &[DetailsResponse { data: project() }]);
    fixtures.bodies("projectDetails", &[DetailsResponse { data: details }]);
}

fn tool_server() -> ToolServerResponse {
    ToolServerResponse {
        id: id(6),
        name: "Weather".to_owned(),
        slug: "weather".to_owned(),
        url: "https://mcp.example.com/weather".to_owned(),
        approval: ApprovalMode::Ask,
        tool_modes: BTreeMap::from([
            ("forecast".to_owned(), ToolMode::Auto),
            ("set_alert".to_owned(), ToolMode::Ask),
        ]),
        enabled: true,
        credentials: KeyState::Set,
        auth_mode: AuthMode::Headers,
        oauth: None,
        protocol_version: Some("2025-11-25".to_owned()),
        tools: Some(vec![ServerTool {
            name: "forecast".to_owned(),
            title: Some("Forecast".to_owned()),
            description: "The forecast for a city.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"]
            }),
            read_only: true,
        }]),
        tools_fetched_at: Some(at(7)),
        review_required: false,
        created_at: at(7),
        updated_at: at(8),
    }
}

fn tool_servers(fixtures: &mut Fixtures) {
    fixtures.types("ApprovalMode", &[ApprovalMode::Ask, ApprovalMode::Auto]);
    fixtures.types("ToolMode", &[ToolMode::Off, ToolMode::Ask, ToolMode::Auto]);
    fixtures.types(
        "ServerTool",
        &[
            ServerTool {
                name: "set_alert".to_owned(),
                title: Some("Set an alert".to_owned()),
                description: "Alerts when rain is expected.".to_owned(),
                input_schema: json!({ "type": "object", "properties": {} }),
                read_only: false,
            },
            ServerTool {
                name: "ping".to_owned(),
                title: None,
                description: String::new(),
                input_schema: json!({ "type": "object" }),
                read_only: true,
            },
        ],
    );
    let untested = ToolServerResponse {
        id: id(7),
        tool_modes: BTreeMap::new(),
        credentials: KeyState::Missing,
        auth_mode: AuthMode::Oauth,
        oauth: Some(OAuthStatus {
            state: SignInState::NeedsSignIn,
            client_id: None,
            client_secret: KeyState::Missing,
            expires_at: None,
        }),
        protocol_version: None,
        tools: None,
        tools_fetched_at: None,
        review_required: true,
        ..tool_server()
    };
    let signed_in = OAuthStatus {
        state: SignInState::SignedIn,
        client_id: Some("copilot-client".to_owned()),
        client_secret: KeyState::Set,
        expires_at: Some(at(9)),
    };
    let oauth = ToolServerResponse {
        id: id(9),
        credentials: KeyState::Missing,
        auth_mode: AuthMode::Oauth,
        oauth: Some(signed_in.clone()),
        ..tool_server()
    };
    fixtures.types(
        "AuthMode",
        &[AuthMode::None, AuthMode::Headers, AuthMode::Oauth],
    );
    fixtures.types(
        "SignInState",
        &[
            SignInState::SignedIn,
            SignInState::Expired,
            SignInState::NeedsSignIn,
        ],
    );
    fixtures.types(
        "OAuthStatus",
        &[
            signed_in,
            OAuthStatus {
                state: SignInState::Expired,
                client_id: None,
                client_secret: KeyState::Unreadable,
                expires_at: Some(at(8)),
            },
        ],
    );
    let client = OAuthClientInput {
        client_id: "copilot-client".to_owned(),
        client_secret: Some("secret-sample".to_owned()),
    };
    fixtures.types(
        "OAuthClientInput",
        &[
            client.clone(),
            OAuthClientInput {
                client_id: "public-client".to_owned(),
                client_secret: None,
            },
        ],
    );
    let start = OAuthStartResponse {
        authorization_url: "https://auth.example.com/authorize?response_type=code&client_id=copilot-client&code_challenge_method=S256".to_owned(),
        expires_at: at(9),
    };
    fixtures.types("OAuthStartResponse", std::slice::from_ref(&start));
    fixtures.bodies("oauthStart", &[DetailsResponse { data: start }]);
    let test = ToolTestResponse {
        server: tool_server(),
        changes: ToolChanges {
            added: vec!["forecast".to_owned()],
            removed: vec!["radar".to_owned()],
            changed: vec!["set_alert".to_owned()],
        },
    };
    fixtures.types(
        "ToolServerResponse",
        &[tool_server(), untested.clone(), oauth.clone()],
    );
    fixtures.types(
        "ToolChanges",
        &[test.changes.clone(), ToolChanges::default()],
    );
    fixtures.types("ToolTestResponse", std::slice::from_ref(&test));
    let credentials = Credentials {
        bearer: Some("token-sample".to_owned()),
        headers: BTreeMap::from([("X-Api-Key".to_owned(), "key-sample".to_owned())]),
    };
    fixtures.types(
        "Credentials",
        &[credentials.clone(), Credentials::default()],
    );
    fixtures.types(
        "CreateToolServerRequest",
        &[
            CreateToolServerRequest {
                name: "Weather".to_owned(),
                slug: Some("weather".to_owned()),
                url: "https://mcp.example.com/weather".to_owned(),
                bearer: Some("token-sample".to_owned()),
                headers: BTreeMap::from([("X-Team".to_owned(), "blue".to_owned())]),
                approval: Some(ApprovalMode::Auto),
                auth_mode: Some(AuthMode::Headers),
                oauth_client: None,
            },
            CreateToolServerRequest {
                name: "Files".to_owned(),
                url: "https://mcp.example.com/files".to_owned(),
                ..CreateToolServerRequest::default()
            },
            CreateToolServerRequest {
                name: "Docs".to_owned(),
                url: "https://mcp.example.com/docs".to_owned(),
                auth_mode: Some(AuthMode::Oauth),
                oauth_client: Some(client.clone()),
                ..CreateToolServerRequest::default()
            },
        ],
    );
    fixtures.types(
        "UpdateToolServerRequest",
        &[
            UpdateToolServerRequest {
                name: Some("Weather".to_owned()),
                url: Some("https://mcp.example.com/v2".to_owned()),
                credentials: Some(Some(credentials)),
                approval: Some(ApprovalMode::Ask),
                tool_modes: Some(BTreeMap::from([("forecast".to_owned(), ToolMode::Off)])),
                enabled: Some(false),
                auth_mode: Some(AuthMode::Headers),
                oauth_client: None,
            },
            UpdateToolServerRequest {
                credentials: Some(None),
                ..UpdateToolServerRequest::default()
            },
            UpdateToolServerRequest {
                auth_mode: Some(AuthMode::Oauth),
                oauth_client: Some(Some(client)),
                ..UpdateToolServerRequest::default()
            },
            UpdateToolServerRequest {
                auth_mode: Some(AuthMode::None),
                oauth_client: Some(None),
                ..UpdateToolServerRequest::default()
            },
            UpdateToolServerRequest::default(),
        ],
    );
    fixtures.bodies(
        "toolServerList",
        &[ListResponse {
            items: vec![tool_server(), untested, oauth],
            total: 3,
        }],
    );
    fixtures.bodies(
        "toolServer",
        &[DetailsResponse {
            data: tool_server(),
        }],
    );
    fixtures.bodies("toolTest", &[DetailsResponse { data: test }]);
}

fn schedule() -> ScheduleResponse {
    ScheduleResponse {
        id: id(8),
        project_id: Some(id(4)),
        model_id: Some(id(2)),
        source_chat_id: Some(id(5)),
        name: "Morning news".to_owned(),
        cron: "0 7 * * 1-5".to_owned(),
        timezone: "Europe/Berlin".to_owned(),
        description: "At 07:00, Monday through Friday".to_owned(),
        prompt: "Summarize the news.".to_owned(),
        instructions: "Keep it short.".to_owned(),
        reasoning: Some(ReasoningLevel::Low),
        on_ask: OnAsk::Pause,
        created_by: CreatedBy::Assistant,
        enabled: true,
        next_run_at: Some(at(30)),
        last_run_at: Some(at(9)),
        last_status: Some(RunStatus::Error),
        last_error: Some("The provider is overloaded.".to_owned()),
        consecutive_failures: 1,
        disabled_reason: None,
        created_at: at(9),
        updated_at: at(10),
    }
}

fn schedules(fixtures: &mut Fixtures) {
    fixtures.types("OnAsk", &[OnAsk::Deny, OnAsk::Pause]);
    fixtures.types("CreatedBy", &[CreatedBy::User, CreatedBy::Assistant]);
    fixtures.types(
        "RunStatus",
        &[RunStatus::Ok, RunStatus::Error, RunStatus::AwaitingApproval],
    );
    let disabled = ScheduleResponse {
        project_id: None,
        model_id: None,
        source_chat_id: None,
        instructions: String::new(),
        reasoning: None,
        on_ask: OnAsk::Deny,
        created_by: CreatedBy::User,
        enabled: false,
        next_run_at: None,
        last_run_at: None,
        last_status: None,
        last_error: None,
        consecutive_failures: 0,
        disabled_reason: Some("The model was removed.".to_owned()),
        ..schedule()
    };
    let preview = SchedulePreviewResponse {
        description: "At 07:00, Monday through Friday".to_owned(),
        timezone: "Europe/Berlin".to_owned(),
        next_runs: vec![at(30), at(31), at(32)],
    };
    let run = ScheduleRunResponse { chat_id: id(9) };
    fixtures.types("ScheduleResponse", &[schedule(), disabled.clone()]);
    fixtures.types("SchedulePreviewResponse", std::slice::from_ref(&preview));
    fixtures.types("ScheduleRunResponse", &[run]);
    fixtures.types(
        "ScheduleListQuery",
        &[
            ScheduleListQuery {
                project_id: Some(id(4)),
            },
            ScheduleListQuery::default(),
        ],
    );
    fixtures.types(
        "SchedulePreviewRequest",
        &[SchedulePreviewRequest {
            cron: "0 7 * * 1-5".to_owned(),
            timezone: "Europe/Berlin".to_owned(),
        }],
    );
    fixtures.types(
        "CreateScheduleRequest",
        &[
            CreateScheduleRequest {
                name: "Morning news".to_owned(),
                cron: "0 7 * * 1-5".to_owned(),
                timezone: "Europe/Berlin".to_owned(),
                prompt: "Summarize the news.".to_owned(),
                project_id: Some(id(4)),
                model_id: Some(id(2)),
                instructions: Some("Keep it short.".to_owned()),
                reasoning: Some(ReasoningLevel::Medium),
                on_ask: Some(OnAsk::Pause),
            },
            CreateScheduleRequest {
                name: "Weekly review".to_owned(),
                cron: "0 9 * * 1".to_owned(),
                timezone: "UTC".to_owned(),
                prompt: "Review the week.".to_owned(),
                ..CreateScheduleRequest::default()
            },
        ],
    );
    fixtures.types(
        "UpdateScheduleRequest",
        &[
            UpdateScheduleRequest {
                name: Some("Evening news".to_owned()),
                cron: Some("0 19 * * *".to_owned()),
                timezone: Some("America/New_York".to_owned()),
                prompt: Some("Summarize the day.".to_owned()),
                instructions: Some(String::new()),
                model_id: Some(id(2)),
                reasoning: Some(Some(ReasoningLevel::High)),
                on_ask: Some(OnAsk::Deny),
                enabled: Some(true),
            },
            UpdateScheduleRequest {
                reasoning: Some(None),
                ..UpdateScheduleRequest::default()
            },
            UpdateScheduleRequest::default(),
        ],
    );
    fixtures.bodies(
        "scheduleList",
        &[ListResponse {
            items: vec![schedule(), disabled],
            total: 2,
        }],
    );
    fixtures.bodies("schedule", &[DetailsResponse { data: schedule() }]);
    fixtures.bodies("schedulePreview", &[DetailsResponse { data: preview }]);
    fixtures.bodies("scheduleRun", &[DetailsResponse { data: run }]);
}

fn usage() -> Usage {
    Usage {
        input_tokens: Some(1_200),
        output_tokens: Some(300),
        reasoning_tokens: Some(120),
        cache_read_tokens: Some(1_000),
        cache_write_tokens: Some(0),
    }
}

fn parts() -> Vec<MessagePartDto> {
    vec![
        MessagePartDto::Text {
            text: "Here is the forecast.".to_owned(),
        },
        MessagePartDto::File {
            file_id: id(3),
            name: "report.pdf".to_owned(),
            media_type: "application/pdf".to_owned(),
        },
        MessagePartDto::Audio {
            file_id: id(10),
            format: "wav".to_owned(),
            transcript: Some("What is the weather?".to_owned()),
            duration_ms: Some(2_500),
        },
        MessagePartDto::Audio {
            file_id: id(10),
            format: "wav".to_owned(),
            transcript: None,
            duration_ms: None,
        },
        MessagePartDto::Reasoning {
            text: "The user wants a forecast.".to_owned(),
        },
        MessagePartDto::ToolCall {
            call_id: id(11),
            server_id: Some(id(6)),
            tool_name: "forecast".to_owned(),
            arguments: json!({ "city": "Berlin" }),
            complete: true,
            approval: Approval::Denied,
            deny_reason: Some("Not now.".to_owned()),
        },
        MessagePartDto::ToolCall {
            call_id: id(12),
            server_id: None,
            tool_name: "schedule_prompt".to_owned(),
            arguments: json!({}),
            complete: false,
            approval: Approval::Pending,
            deny_reason: None,
        },
        MessagePartDto::ToolResult {
            call_id: id(11),
            status: ToolStatus::Ok,
            is_error: false,
            content: vec![
                ResultContent::Text {
                    text: "Sunny, 21 °C.".to_owned(),
                },
                ResultContent::Image { file_id: id(13) },
            ],
            truncated: true,
        },
    ]
}

fn message() -> MessageResponse {
    MessageResponse {
        id: id(14),
        chat_id: id(5),
        role: Role::Assistant,
        status: MessageStatus::Complete,
        finish_reason: Some(FinishReason::Stop),
        model_id: Some(id(2)),
        parts: parts(),
        usage: Some(usage()),
        error: None,
        created_at: at(11),
        updated_at: at(12),
    }
}

fn messages(fixtures: &mut Fixtures) {
    fixtures.types("Role", &[Role::User, Role::Assistant, Role::Tool]);
    fixtures.types(
        "MessageStatus",
        &[
            MessageStatus::Streaming,
            MessageStatus::Complete,
            MessageStatus::AwaitingApproval,
            MessageStatus::Cancelled,
            MessageStatus::Error,
            MessageStatus::Refused,
        ],
    );
    fixtures.types(
        "FinishReason",
        &[
            FinishReason::Stop,
            FinishReason::ToolCalls,
            FinishReason::Length,
            FinishReason::Refusal,
            FinishReason::Error,
            FinishReason::Other,
            FinishReason::ToolLimit,
        ],
    );
    fixtures.types(
        "ToolStatus",
        &[
            ToolStatus::Ok,
            ToolStatus::Error,
            ToolStatus::Denied,
            ToolStatus::Cancelled,
            ToolStatus::Timeout,
        ],
    );
    fixtures.types(
        "Approval",
        &[
            Approval::Auto,
            Approval::Pending,
            Approval::Approved,
            Approval::Denied,
        ],
    );
    fixtures.types("Usage", &[usage(), Usage::default()]);
    fixtures.types(
        "ResultContent",
        &[
            ResultContent::Text {
                text: "Done.".to_owned(),
            },
            ResultContent::Image { file_id: id(13) },
        ],
    );
    fixtures.types("MessagePartDto", &parts());
    let user = MessageResponse {
        id: id(15),
        role: Role::User,
        finish_reason: None,
        model_id: None,
        parts: vec![MessagePartDto::Text {
            text: "What is the weather?".to_owned(),
        }],
        usage: None,
        ..message()
    };
    let failed = MessageResponse {
        id: id(16),
        status: MessageStatus::Error,
        finish_reason: Some(FinishReason::Error),
        parts: Vec::new(),
        usage: None,
        error: Some("The provider rejected the API key.".to_owned()),
        ..message()
    };
    fixtures.types("MessageResponse", &[message(), user.clone(), failed]);
    fixtures.bodies(
        "messageList",
        &[ListResponse {
            items: vec![user, message()],
            total: 2,
        }],
    );
}

fn chat() -> ChatResponse {
    ChatResponse {
        id: id(5),
        project_id: Some(id(4)),
        model_id: Some(id(2)),
        schedule_id: Some(id(8)),
        title: "Weather in Berlin".to_owned(),
        instructions: "Use Celsius.".to_owned(),
        reasoning: Some(ReasoningLevel::Medium),
        builtin_tools: Some(vec![BuiltinTool::Schedules]),
        tool_overrides: vec![ToolOverride {
            server_id: id(6),
            enabled: false,
        }],
        approved_tools: vec![ApprovedTool {
            server_id: id(6),
            name: "forecast".to_owned(),
        }],
        pinned: true,
        voice_replies: true,
        last_read_at: Some(at(13)),
        created_at: at(12),
        updated_at: at(13),
    }
}

fn summary() -> ChatSummaryResponse {
    ChatSummaryResponse {
        id: id(5),
        project_id: Some(id(4)),
        model_id: Some(id(2)),
        schedule_id: Some(id(8)),
        title: "Weather in Berlin".to_owned(),
        pinned: true,
        unread: true,
        awaiting_approval: false,
        created_at: at(12),
        updated_at: at(13),
    }
}

fn chats(fixtures: &mut Fixtures) {
    let bare = ChatResponse {
        project_id: None,
        model_id: None,
        schedule_id: None,
        title: String::new(),
        instructions: String::new(),
        reasoning: None,
        builtin_tools: None,
        tool_overrides: Vec::new(),
        approved_tools: Vec::new(),
        pinned: false,
        voice_replies: false,
        last_read_at: None,
        ..chat()
    };
    let standalone = ChatSummaryResponse {
        id: id(17),
        project_id: None,
        model_id: None,
        schedule_id: None,
        pinned: false,
        unread: false,
        awaiting_approval: true,
        ..summary()
    };
    let hit = SearchHitResponse {
        chat_id: id(5),
        chat_title: "Weather in Berlin".to_owned(),
        message_id: Some(id(14)),
        snippet: "Sunny, 21 °C.".to_owned(),
    };
    let title_hit = SearchHitResponse {
        message_id: None,
        snippet: "Weather in Berlin".to_owned(),
        ..hit.clone()
    };
    fixtures.types(
        "ToolOverride",
        &[ToolOverride {
            server_id: id(6),
            enabled: true,
        }],
    );
    fixtures.types(
        "ApprovedTool",
        &[ApprovedTool {
            server_id: id(6),
            name: "forecast".to_owned(),
        }],
    );
    fixtures.types("ChatResponse", &[chat(), bare]);
    fixtures.types("ChatSummaryResponse", &[summary(), standalone.clone()]);
    fixtures.types("SearchHitResponse", &[hit.clone(), title_hit.clone()]);
    fixtures.types(
        "ChatListQuery",
        &[
            ChatListQuery {
                project_id: Some(id(4)),
                schedule_id: Some(id(8)),
                pinned: Some(false),
                scheduled: Some(true),
                cursor: Some("MjAyNi0xMS0yNlQxMjowMDowMC4xMjM0NTZaXzAxOWFiZjZj".to_owned()),
                limit: Some(50),
            },
            ChatListQuery::default(),
        ],
    );
    fixtures.types(
        "StreamQuery",
        &[StreamQuery { after: Some(42) }, StreamQuery::default()],
    );
    fixtures.types(
        "SearchQuery",
        &[
            SearchQuery {
                q: "weather berl".to_owned(),
                limit: Some(20),
            },
            SearchQuery {
                q: "forecast".to_owned(),
                limit: None,
            },
        ],
    );
    requests(fixtures);
    fixtures.bodies(
        "chatPage",
        &[
            PageResponse {
                items: vec![summary(), standalone],
                next: Some("MjAyNi0xMS0yNlQxMjowMDowMC4xMjM0NTZaXzAxOWFiZjZj".to_owned()),
            },
            PageResponse {
                items: Vec::new(),
                next: None,
            },
        ],
    );
    fixtures.bodies("chat", &[DetailsResponse { data: chat() }]);
    fixtures.bodies(
        "searchList",
        &[ListResponse {
            items: vec![title_hit, hit],
            total: 2,
        }],
    );
}

/// The request bodies of the chat routes.
fn requests(fixtures: &mut Fixtures) {
    fixtures.types(
        "CreateChatRequest",
        &[
            CreateChatRequest {
                project_id: Some(id(4)),
                model_id: Some(id(2)),
                instructions: Some("Use Celsius.".to_owned()),
                reasoning: Some(ReasoningLevel::Off),
                builtin_tools: Some(Vec::new()),
                tool_overrides: Some(vec![ToolOverride {
                    server_id: id(6),
                    enabled: false,
                }]),
                voice_replies: Some(true),
            },
            CreateChatRequest::default(),
        ],
    );
    fixtures.types(
        "UpdateChatRequest",
        &[
            UpdateChatRequest {
                title: Some("Renamed".to_owned()),
                project_id: Some(Some(id(4))),
                model_id: Some(Some(id(2))),
                instructions: Some("Use Fahrenheit.".to_owned()),
                reasoning: Some(Some(ReasoningLevel::High)),
                builtin_tools: Some(Some(vec![BuiltinTool::Schedules])),
                tool_overrides: Some(Vec::new()),
                pinned: Some(true),
                voice_replies: Some(false),
            },
            UpdateChatRequest {
                project_id: Some(None),
                model_id: Some(None),
                reasoning: Some(None),
                builtin_tools: Some(None),
                ..UpdateChatRequest::default()
            },
            UpdateChatRequest::default(),
        ],
    );
    fixtures.types(
        "SendMessageRequest",
        &[
            SendMessageRequest {
                text: "And tomorrow?".to_owned(),
                file_ids: vec![id(3)],
                edit_from: Some(id(15)),
            },
            SendMessageRequest {
                text: "Hello!".to_owned(),
                ..SendMessageRequest::default()
            },
        ],
    );
    fixtures.types("Decision", &[Decision::Approve, Decision::Deny]);
    fixtures.types("DecisionScope", &[DecisionScope::Once, DecisionScope::Chat]);
    let decisions = vec![
        CallDecision {
            call_id: id(11),
            decision: Decision::Deny,
            scope: Some(DecisionScope::Once),
            reason: Some("Not now.".to_owned()),
        },
        CallDecision {
            call_id: id(12),
            decision: Decision::Approve,
            scope: None,
            reason: None,
        },
    ];
    fixtures.types("CallDecision", &decisions);
    fixtures.types("DecisionsRequest", &[DecisionsRequest { decisions }]);
}

fn events(fixtures: &mut Fixtures) {
    fixtures.types(
        "PartKind",
        &[
            PartKind::Text,
            PartKind::Reasoning,
            PartKind::ToolCall,
            PartKind::Audio,
        ],
    );
    let refusal = Refusal {
        category: Some("cyber".to_owned()),
        explanation: "The model declined to answer.".to_owned(),
    };
    fixtures.types(
        "Refusal",
        &[
            refusal.clone(),
            Refusal {
                category: None,
                explanation: "The model declined to answer.".to_owned(),
            },
        ],
    );
    fixtures.types("ChatStreamEvent", &stream(refusal));
    fixtures.types(
        "UserEvent",
        &[
            UserEvent::ChatUpdated { chat_id: id(5) },
            UserEvent::RunStarted { chat_id: id(5) },
            UserEvent::RunFinished {
                chat_id: id(5),
                status: MessageStatus::AwaitingApproval,
            },
            UserEvent::ApprovalPending {
                chat_id: id(5),
                message_id: id(14),
            },
            UserEvent::ScheduleUpdated { schedule_id: id(8) },
        ],
    );
}

/// A run with every event: a tool round that pauses, then a refusal, an error, and the end.
fn stream(refusal: Refusal) -> Vec<ChatStreamEvent> {
    let message_id = id(14);
    vec![
        ChatStreamEvent::MessageStarted {
            seq: 1,
            message_id,
            role: Role::Assistant,
            window_start_id: Some(id(15)),
            unavailable: vec!["Docs".to_owned()],
        },
        ChatStreamEvent::PartStarted {
            seq: 2,
            message_id,
            index: 0,
            kind: PartKind::Reasoning,
        },
        ChatStreamEvent::ReasoningDelta {
            seq: 3,
            message_id,
            index: 0,
            text: "The user wants a forecast.".to_owned(),
        },
        ChatStreamEvent::TextDelta {
            seq: 4,
            message_id,
            index: 1,
            text: "Checking.".to_owned(),
        },
        ChatStreamEvent::AudioDelta {
            seq: 5,
            message_id,
            data: "AAAA".to_owned(),
            format: "pcm16".to_owned(),
        },
        ChatStreamEvent::ToolRequested {
            seq: 6,
            message_id,
            call_id: id(11),
            server_id: Some(id(6)),
            tool_name: "forecast".to_owned(),
            title: "Forecast".to_owned(),
            arguments: json!({ "city": "Berlin", "days": 2 }),
            needs_approval: false,
        },
        ChatStreamEvent::ToolFinished {
            seq: 7,
            message_id,
            call_id: id(11),
            status: ToolStatus::Timeout,
            output: vec![ResultContent::Text {
                text: "The tool did not answer in time.".to_owned(),
            }],
            truncated: false,
        },
        ChatStreamEvent::ToolRequested {
            seq: 8,
            message_id,
            call_id: id(12),
            server_id: None,
            tool_name: "schedule_prompt".to_owned(),
            title: "Schedule a prompt".to_owned(),
            arguments: json!({ "name": "News" }),
            needs_approval: true,
        },
        ChatStreamEvent::MessageFinished {
            seq: 9,
            message_id,
            status: MessageStatus::AwaitingApproval,
            finish_reason: Some(FinishReason::ToolCalls),
            refusal: None,
            usage: Some(usage()),
        },
        ChatStreamEvent::MessageFinished {
            seq: 10,
            message_id,
            status: MessageStatus::Refused,
            finish_reason: Some(FinishReason::Refusal),
            refusal: Some(refusal),
            usage: None,
        },
        ChatStreamEvent::MessageFinished {
            seq: 11,
            message_id,
            status: MessageStatus::Cancelled,
            finish_reason: None,
            refusal: None,
            usage: Some(Usage::default()),
        },
        ChatStreamEvent::Error {
            seq: 12,
            message: "The provider rejected the API key.".to_owned(),
            code: "PROVIDER_AUTH".to_owned(),
        },
        ChatStreamEvent::Done { seq: 13 },
    ]
}
