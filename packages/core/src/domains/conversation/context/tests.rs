use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;
use crate::domains::chat::{
    Approval, ContextHash, MessageParts, MessageStatus, ToolCallPart, ToolResultPart, ToolStatus,
};
use crate::domains::conversation::lowering::UNFINISHED;
use crate::domains::file::{FileOwner, MediaType, StoredFileParts};
use crate::domains::provider::{CapabilityOverrides, ModelParts, OpenAiApi, ProviderKind};
use crate::domains::schedule::CronRule;
use crate::domains::tool::{EffectiveTool, ToolAlias, ToolMode};
use crate::ports::llm_gateway::{LlmMessage, LlmPart, LlmRequest};
use crate::ports::tool_host::ToolContent;
use crate::shared::base_url::BaseUrl;
use crate::shared::ids::{CallId, ModelId, ProviderId, ToolServerId, UserId};
use crate::shared::net::NetworkScope;

const MIB: u64 = 1_024 * 1_024;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap()
}

/// Owned inputs of one context build.
struct Setup {
    settings: UserSettings,
    chat: Chat,
    project: Option<Project>,
    schedule: Option<Schedule>,
    model: Model,
    provider: Provider,
    capabilities: Capabilities,
    mode: RunMode,
    project_files: Vec<StoredFile>,
    tools: EffectiveTools,
    files: HashMap<FileId, StoredFile>,
    history: Vec<Message>,
}

impl Setup {
    /// An OpenAI model with tool use only and no context window.
    fn new() -> Self {
        let user = UserId::new(Uuid::now_v7());
        let base_url = BaseUrl::parse("https://api.openai.com/v1", NetworkScope::Public).unwrap();
        let provider = Provider::create(
            user,
            ProviderKind::OpenAi,
            Preset::OpenAi,
            "OpenAI",
            base_url,
            now(),
        )
        .unwrap();
        let capabilities = Capabilities {
            tools: true,
            ..Capabilities::default()
        };
        let model = Model::restore(ModelParts {
            id: ModelId::generate(),
            user_id: user,
            provider_id: provider.id(),
            remote_id: "gpt-test".to_owned(),
            display_name: "GPT".to_owned(),
            enabled: true,
            available: true,
            context_window: None,
            max_output_tokens: None,
            capabilities: capabilities.clone(),
            overrides: CapabilityOverrides::default(),
            created_at: now(),
            updated_at: now(),
        });
        Self {
            settings: UserSettings::empty(user, now()),
            chat: Chat::create(user, now()).with_model(Some(model.id())),
            project: None,
            schedule: None,
            model,
            provider,
            capabilities,
            mode: RunMode::Interactive,
            project_files: Vec::new(),
            tools: EffectiveTools::default(),
            files: HashMap::new(),
            history: Vec::new(),
        }
    }

    fn user_id(&self) -> UserId {
        self.chat.user_id()
    }

    fn inputs(&self) -> ContextInputs<'_> {
        ContextInputs {
            settings: &self.settings,
            chat: &self.chat,
            project: self.project.as_ref(),
            schedule: self.schedule.as_ref(),
            model: &self.model,
            provider: &self.provider,
            capabilities: &self.capabilities,
            mode: self.mode,
            project_files: &self.project_files,
            tools: &self.tools,
            files: &self.files,
            history: &self.history,
        }
    }

    fn limits(&mut self, window: Option<u32>, output: Option<u32>) {
        self.model = Model::restore(ModelParts {
            context_window: window,
            max_output_tokens: output,
            ..parts(&self.model)
        });
    }

    fn user(&mut self, text: &str) -> Message {
        self.user_parts(vec![Part::Text {
            text: text.to_owned(),
        }])
    }

    fn user_parts(&mut self, parts: Vec<Part>) -> Message {
        let message = Message::user(self.user_id(), self.chat.id(), parts, now());
        self.history.push(message.clone());
        message
    }

    fn answer(&mut self, parts: Vec<Part>, status: MessageStatus) -> Message {
        let base = Message::assistant(
            self.user_id(),
            self.chat.id(),
            self.model.id(),
            self.provider.id(),
            now(),
        );
        let message = Message::restore(MessageParts {
            status,
            parts,
            ..base.to_parts()
        });
        self.history.push(message.clone());
        message
    }

    fn results(&mut self, results: Vec<ToolResultPart>) -> Message {
        let message = Message::tool(self.user_id(), self.chat.id(), results, now());
        self.history.push(message.clone());
        message
    }

    /// A file known to the context (in `files`), owned by the chat.
    fn file(&mut self, name: &str, media: MediaType, size_bytes: u64) -> StoredFile {
        let file = StoredFile::restore(StoredFileParts {
            id: FileId::generate(),
            user_id: self.user_id(),
            owner: FileOwner::Chat(self.chat.id()),
            name: name.to_owned(),
            media_type: media.as_str().to_owned(),
            size_bytes,
            sha256: [7; 32],
            token_estimate: crate::domains::file::estimate(media, size_bytes, Some(1), None),
            page_count: (media == MediaType::Pdf).then_some(1),
            included: true,
            created_at: now(),
        });
        self.files.insert(file.id(), file.clone());
        file
    }

    /// An included project file of a new project of the chat.
    fn project_file(&mut self, name: &str, media: MediaType, size_bytes: u64) -> StoredFile {
        if self.project.is_none() {
            let project = Project::create(self.user_id(), "Work", now()).unwrap();
            self.chat = self.chat.clone().with_project(Some(project.id()));
            self.project = Some(project);
        }
        let file = self.file(name, media, size_bytes);
        self.project_files.push(file.clone());
        file
    }

    fn server_tool(&mut self, name: &str) {
        self.tools.tools.push(EffectiveTool {
            alias: ToolAlias::restore(format!("gh__{name}")),
            server_id: ToolServerId::generate(),
            server_name: "GitHub".to_owned(),
            name: name.to_owned(),
            title: None,
            description: format!("The {name} tool."),
            input_schema: json!({"type": "object"}),
            mode: ToolMode::Ask,
            review_required: false,
        });
    }

    fn build(&self) -> TurnContext {
        build(&self.inputs()).unwrap()
    }

    /// The request of `context`, with every file's bytes reading `bytes of <name>`.
    fn request_with(&self, context: &TurnContext) -> LlmRequest {
        let draft = request(&self.inputs(), context).unwrap();
        let contents = draft
            .files()
            .into_iter()
            .map(|id| {
                let name = self.files[&id].name().to_owned();
                (id, Bytes::from(format!("bytes of {name}")))
            })
            .collect();
        draft.resolve(&contents)
    }

    fn request(&self) -> LlmRequest {
        self.request_with(&self.build())
    }
}

fn parts(model: &Model) -> ModelParts {
    ModelParts {
        id: model.id(),
        user_id: model.user_id(),
        provider_id: model.provider_id(),
        remote_id: model.remote_id().to_owned(),
        display_name: model.display_name().to_owned(),
        enabled: model.enabled(),
        available: model.available(),
        context_window: model.context_window(),
        max_output_tokens: model.max_output_tokens(),
        capabilities: model.capabilities().clone(),
        overrides: model.overrides().clone(),
        created_at: model.created_at(),
        updated_at: model.updated_at(),
    }
}

fn call(alias: &str, name: &str, raw: &str) -> ToolCallPart {
    ToolCallPart {
        id: CallId::generate(),
        provider_call_id: Some("toolu_1".to_owned()),
        server_id: None,
        tool_name: name.to_owned(),
        alias: ToolAlias::restore(alias.to_owned()),
        arguments_raw: raw.to_owned(),
        arguments: serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_object),
        complete: true,
        approval: Approval::Auto,
        deny_reason: None,
    }
}

fn ok(call: &ToolCallPart, text: &str) -> ToolResultPart {
    ToolResultPart {
        call_id: call.id,
        status: ToolStatus::Ok,
        is_error: false,
        content: vec![ResultContent::Text(text.to_owned())],
        truncated: false,
    }
}

fn text(text: &str) -> LlmPart {
    LlmPart::Text(text.to_owned())
}

fn roles(request: &LlmRequest) -> Vec<Role> {
    request
        .messages
        .iter()
        .map(|message| message.role)
        .collect()
}

#[test]
fn the_system_text_joins_instructions_and_notes() {
    let mut setup = Setup::new();
    setup.settings = setup.settings.clone().with_instructions("Personal.", now());
    setup.project = Some(
        Project::create(setup.user_id(), "Work", now())
            .unwrap()
            .with_instructions("Project."),
    );
    setup.chat = setup.chat.clone().with_instructions("Chat.");
    setup.user("Hi");
    assert_eq!(
        setup.build().system,
        format!("Personal.\n\nProject.\n\nChat.\n\n{DATA_NOTE}")
    );

    setup.chat = setup
        .chat
        .clone()
        .with_builtin_tools(Some(vec![BuiltinTool::Schedules]));
    let context = setup.build();
    assert_eq!(
        context.system,
        format!("Personal.\n\nProject.\n\nChat.\n\n{SCHEDULE_NOTE}\n\n{DATA_NOTE}")
    );

    let rule = CronRule::parse("0 9 * * *", "UTC", now()).unwrap();
    let schedule = Schedule::create(setup.user_id(), "Digest", rule, "News?", now())
        .unwrap()
        .with_instructions("Scheduled.");
    setup.schedule = Some(schedule);
    setup.project = None;
    setup.mode = RunMode::Unattended {
        on_ask: crate::domains::schedule::OnAsk::Deny,
    };
    assert_eq!(
        setup.build().system,
        format!("Personal.\n\nScheduled.\n\n{DATA_NOTE}"),
        "a standalone schedule's instructions replace the chat's, and unattended runs get no \
         schedule tools"
    );
}

#[test]
fn tools_are_sorted_by_alias_and_need_tool_use() {
    let mut setup = Setup::new();
    setup.server_tool("search");
    setup.chat = setup
        .chat
        .clone()
        .with_builtin_tools(Some(vec![BuiltinTool::Schedules]));
    setup.user("Hi");
    let context = setup.build();
    let aliases: Vec<&str> = context
        .tools
        .iter()
        .map(|tool| tool.alias.as_str())
        .collect();
    assert_eq!(
        aliases,
        [
            "cancel_schedule",
            "gh__search",
            "list_schedules",
            "schedule_prompt"
        ]
    );
    assert!(context.tools[0].server_id.is_none());
    assert!(context.tools[1].server_id.is_some());
    let request = setup.request_with(&context);
    let sent: Vec<&str> = request
        .tools
        .iter()
        .map(|tool| tool.alias.as_str())
        .collect();
    assert_eq!(sent, aliases);

    setup.capabilities.tools = false;
    assert!(setup.build().tools.is_empty());
    assert!(
        setup.request_with(&context).tools.is_empty(),
        "a model that lost tool use gets none, even from an older snapshot"
    );
}

#[test]
fn parts_the_model_cannot_read_become_notices() {
    let mut setup = Setup::new();
    let notes = setup.project_file("notes.md", MediaType::Markdown, 40);
    let image = setup.file("a.png", MediaType::Png, 100);
    let pdf = setup.file("b.pdf", MediaType::Pdf, 100);
    let audio = setup.file("c.wav", MediaType::Wav, 100);
    let plain = setup.file("d.txt", MediaType::Text, 100);
    let attachment = |file: &StoredFile| Part::File {
        file_id: file.id(),
        name: file.name().to_owned(),
        media_type: file.media_type().to_owned(),
    };
    let gone = Part::File {
        file_id: FileId::generate(),
        name: "gone.txt".to_owned(),
        media_type: "text/plain".to_owned(),
    };
    setup.user_parts(vec![
        Part::Text {
            text: "Look".to_owned(),
        },
        attachment(&image),
        attachment(&pdf),
        Part::Audio {
            file_id: audio.id(),
            format: "wav".to_owned(),
            transcript: None,
            duration_ms: None,
        },
        attachment(&plain),
        gone,
        attachment(&notes),
    ]);
    let request = setup.request();
    assert_eq!(roles(&request), [Role::User]);
    assert_eq!(
        request.messages[0].parts,
        [
            text("<file name=\"notes.md\">\nbytes of notes.md\n</file>"),
            text("Look"),
            text("[image \"a.png\" omitted: this model cannot read images]"),
            text("[PDF \"b.pdf\" omitted: this model cannot read PDFs]"),
            text("[audio \"c.wav\" omitted: this model cannot read audio]"),
            text("<file name=\"d.txt\">\nbytes of d.txt\n</file>"),
            text("[file \"gone.txt\" was deleted]"),
            text("[file \"notes.md\" is in the project files]"),
        ]
    );

    setup.capabilities = Capabilities {
        vision: true,
        pdf: true,
        audio_in: true,
        ..setup.capabilities.clone()
    };
    let request = setup.request();
    assert_eq!(
        request.messages[0].parts[2..5],
        [
            LlmPart::Image {
                media_type: "image/png".to_owned(),
                data: Bytes::from_static(b"bytes of a.png"),
            },
            LlmPart::Pdf {
                name: "b.pdf".to_owned(),
                data: Bytes::from_static(b"bytes of b.pdf"),
            },
            LlmPart::Audio {
                format: "wav".to_owned(),
                data: Bytes::from_static(b"bytes of c.wav"),
            },
        ]
    );
}

#[test]
fn answers_keep_text_and_transcripts_but_not_reasoning() {
    let mut setup = Setup::new();
    setup.user("Hi");
    setup.answer(
        vec![
            Part::Reasoning {
                text: "Thinking".to_owned(),
            },
            Part::Audio {
                file_id: FileId::generate(),
                format: "wav".to_owned(),
                transcript: Some("Spoken".to_owned()),
                duration_ms: None,
            },
            Part::Text {
                text: "Written".to_owned(),
            },
        ],
        MessageStatus::Complete,
    );
    setup.answer(
        vec![Part::Text {
            text: "Refused".to_owned(),
        }],
        MessageStatus::Refused,
    );
    setup.user("Again");
    let request = setup.request();
    assert_eq!(roles(&request), [Role::User, Role::Assistant, Role::User]);
    assert_eq!(request.messages[1].parts, [text("Spoken"), text("Written")]);
}

#[test]
fn calls_become_text_without_tools_or_when_their_alias_is_gone() {
    let mut setup = Setup::new();
    setup.server_tool("search");
    setup.user("Find it");
    let known = call("gh__search", "search", r#"{"q":"x"}"#);
    let gone = call("old__find", "find", r#"{"q":1}"#);
    setup.answer(
        vec![Part::ToolCall(known.clone()), Part::ToolCall(gone.clone())],
        MessageStatus::Complete,
    );
    setup.results(vec![ok(&known, "found"), ok(&gone, "old result")]);
    setup.user("Thanks");
    let request = setup.request();
    assert_eq!(
        roles(&request),
        [Role::User, Role::Assistant, Role::Tool, Role::User]
    );
    let id = format!("call_{}", known.id.uuid().simple());
    assert_eq!(
        request.messages[1].parts,
        [
            LlmPart::ToolCall {
                call_id: known.id.uuid(),
                provider_call_id: id.clone(),
                alias: "gh__search".to_owned(),
                arguments: json!({"q": "x"}),
            },
            text(r#"[called find with {"q":1}]"#),
        ]
    );
    assert_eq!(
        request.messages[2].parts,
        [
            LlmPart::ToolResult {
                call_id: known.id.uuid(),
                provider_call_id: id,
                alias: "gh__search".to_owned(),
                is_error: false,
                content: vec![ToolContent::Text("found".to_owned())],
            },
            text("[result: old result]"),
        ]
    );

    setup.capabilities.tools = false;
    let request = setup.request();
    assert_eq!(
        roles(&request),
        [Role::User, Role::Assistant, Role::User, Role::User]
    );
    assert_eq!(
        request.messages[1].parts,
        [
            text(r#"[called search with {"q":"x"}]"#),
            text(r#"[called find with {"q":1}]"#),
        ]
    );
    assert_eq!(
        request.messages[2].parts,
        [text("[result: found]"), text("[result: old result]")]
    );
}

#[test]
fn unfinished_answers_keep_finished_calls_and_get_synthesized_results() {
    let mut setup = Setup::new();
    setup.server_tool("search");
    setup.user("Find it");
    let finished = call("gh__search", "search", "{}");
    let cut = ToolCallPart {
        complete: false,
        arguments_raw: "{\"q\":".to_owned(),
        arguments: None,
        ..call("gh__search", "search", "{}")
    };
    setup.answer(
        vec![
            Part::Text {
                text: "Partial".to_owned(),
            },
            Part::ToolCall(finished.clone()),
            Part::ToolCall(cut),
        ],
        MessageStatus::Error,
    );
    setup.user("Try again");
    let request = setup.request();
    assert_eq!(
        roles(&request),
        [Role::User, Role::Assistant, Role::Tool, Role::User]
    );
    assert_eq!(
        request.messages[1].parts.len(),
        2,
        "the cut call is dropped"
    );
    assert_eq!(
        request.messages[2].parts,
        [LlmPart::ToolResult {
            call_id: finished.id.uuid(),
            provider_call_id: format!("call_{}", finished.id.uuid().simple()),
            alias: "gh__search".to_owned(),
            is_error: true,
            content: vec![ToolContent::Text(UNFINISHED.to_owned())],
        }]
    );
}

#[test]
fn replays_only_with_the_same_provider_and_context_hash() {
    let mut setup = Setup::new();
    setup.user("Hi");
    let state = json!({"reasoning": "signed"});
    let earlier = setup.answer(
        vec![Part::Text {
            text: "Hello".to_owned(),
        }],
        MessageStatus::Complete,
    );
    setup.user("More");
    let context = setup.build();
    let with = |setup: &mut Setup, hash: ContextHash, provider: Option<ProviderId>| {
        setup.history[1] = Message::restore(MessageParts {
            context_hash: Some(hash),
            provider_state: Some(state.clone()),
            provider_id: provider,
            ..earlier.to_parts()
        });
        setup.request_with(&context).messages[1].replay.clone()
    };
    let provider = Some(setup.provider.id());
    assert_eq!(
        with(&mut setup, context.hash, provider),
        Some(state.clone())
    );
    assert_eq!(
        with(&mut setup, ContextHash::new([5; 32]), provider),
        None,
        "another context"
    );
    assert_eq!(
        with(&mut setup, context.hash, Some(ProviderId::generate())),
        None,
        "another connection"
    );
    assert_eq!(
        context,
        setup.build(),
        "the hash ignores stored answers' hashes"
    );
}

#[test]
fn the_current_turn_replays_with_the_providers_call_ids() {
    let mut setup = Setup::new();
    setup.server_tool("search");
    setup.user("Find it");
    let found = call("gh__search", "search", "{}");
    let state = json!([{"type": "tool_use", "id": "toolu_1"}]);
    let answer = setup.answer(vec![Part::ToolCall(found.clone())], MessageStatus::Complete);
    setup.history[1] = Message::restore(MessageParts {
        context_hash: Some(ContextHash::new([5; 32])),
        provider_state: Some(state.clone()),
        ..answer.to_parts()
    });
    setup.results(vec![ok(&found, "found")]);
    let request = setup.request();
    assert_eq!(request.messages[1].replay, Some(state));
    let ids: Vec<&str> = request.messages[1..]
        .iter()
        .flat_map(|message: &LlmMessage| &message.parts)
        .filter_map(|part| match part {
            LlmPart::ToolCall {
                provider_call_id, ..
            }
            | LlmPart::ToolResult {
                provider_call_id, ..
            } => Some(provider_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, ["toolu_1", "toolu_1"]);
}

#[test]
fn the_window_leaves_out_the_oldest_whole_turns() {
    let mut setup = Setup::new();
    setup.limits(Some(1_000), Some(200));
    setup.project_file("notes.md", MediaType::Markdown, 40);
    let long = "x".repeat(1_200);
    let mut starts = Vec::new();
    for _ in 0..3 {
        starts.push(setup.user(&long).id());
        setup.answer(
            vec![Part::Text {
                text: "y".repeat(400),
            }],
            MessageStatus::Complete,
        );
    }
    setup.user(&"z".repeat(400));
    let context = setup.build();
    assert_eq!(context.window_start_id, Some(starts[2]));
    let request = setup.request_with(&context);
    assert_eq!(
        roles(&request),
        [Role::User, Role::Assistant, Role::User],
        "only the newest earlier turn fits next to the current one"
    );
    assert_eq!(
        request.messages[0].parts[0],
        text("<file name=\"notes.md\">\nbytes of notes.md\n</file>"),
        "the preamble opens the first user message in the window"
    );
    assert_eq!(request.messages[0].parts[1], text(&long));

    setup.limits(None, None);
    let everything = setup.build();
    assert_eq!(everything.window_start_id, None);
    assert_ne!(everything.hash, context.hash, "the window start is hashed");
}

#[test]
fn a_turn_that_cannot_fit_names_the_largest_project_files() {
    let mut setup = Setup::new();
    setup.limits(Some(1_000), Some(200));
    setup.project_file("big.txt", MediaType::Text, 20_000);
    setup.project_file("small.txt", MediaType::Text, 40);
    setup.user("Hi");
    let error = build(&setup.inputs()).unwrap_err();
    assert!(matches!(error, DomainError::Validation { .. }));
    assert_eq!(
        error.to_string(),
        "This message and the project files do not fit in the model's context window. Turn off \
         some project files, for example \"big.txt\", \"small.txt\"."
    );

    let mut alone = Setup::new();
    alone.limits(Some(100), Some(50));
    alone.user(&"x".repeat(1_000));
    assert_eq!(
        build(&alone.inputs()).unwrap_err().to_string(),
        "This message does not fit in the model's context window."
    );
}

#[test]
fn oversized_requests_leave_out_older_binaries_audio_first() {
    let mut setup = Setup::new();
    setup.capabilities.vision = true;
    setup.capabilities.audio_in = true;
    let old_image = setup.file("old.png", MediaType::Png, 9 * MIB);
    let old_audio = setup.file("old.wav", MediaType::Wav, 9 * MIB);
    let new_image = setup.file("new.png", MediaType::Png, 9 * MIB);
    let attach = |file: &StoredFile| Part::File {
        file_id: file.id(),
        name: file.name().to_owned(),
        media_type: file.media_type().to_owned(),
    };
    setup.user_parts(vec![attach(&old_image), attach(&old_audio)]);
    setup.answer(
        vec![Part::Text {
            text: "Seen".to_owned(),
        }],
        MessageStatus::Complete,
    );
    setup.user_parts(vec![attach(&new_image)]);
    let context = setup.build();
    let request = setup.request_with(&context);
    assert!(matches!(
        request.messages[0].parts[0],
        LlmPart::Image { .. }
    ));
    assert_eq!(
        request.messages[0].parts[1],
        text("[audio \"old.wav\" omitted: the request is too large]")
    );
    assert!(matches!(
        request.messages[2].parts[0],
        LlmPart::Image { .. }
    ));

    let mut huge = Setup::new();
    huge.capabilities.vision = true;
    let image = huge.file("huge.png", MediaType::Png, 30 * MIB);
    huge.user_parts(vec![attach(&image)]);
    assert_eq!(
        build(&huge.inputs()).unwrap_err().to_string(),
        "This message is too large to send: a request can carry at most 32 MB and 100 PDF pages."
    );
}

#[test]
fn reasoning_is_fixed_per_turn_and_off_with_tools_where_needed() {
    let mut setup = Setup::new();
    setup.user("Hi");
    setup.chat = setup
        .chat
        .clone()
        .with_reasoning(Some(ReasoningLevel::High));
    assert_eq!(setup.build().reasoning, None, "no reasoning capability");

    setup.capabilities.reasoning = true;
    assert_eq!(setup.build().reasoning, Some(ReasoningLevel::High));
    setup.model = Model::restore(ModelParts {
        remote_id: "gpt-5.4-mini".to_owned(),
        ..parts(&setup.model)
    });
    assert_eq!(setup.build().reasoning, Some(ReasoningLevel::High));
    setup.server_tool("search");
    assert_eq!(
        setup.build().reasoning,
        Some(ReasoningLevel::High),
        "the Responses API reasons with tools"
    );
    setup.provider = setup
        .provider
        .clone()
        .with_api(OpenAiApi::ChatCompletions)
        .unwrap();
    let context = setup.build();
    assert_eq!(context.reasoning, Some(ReasoningLevel::Off));
    assert_eq!(
        setup.request_with(&context).reasoning,
        Some(ReasoningLevel::Off)
    );
}

#[test]
fn levels_the_model_does_not_list_become_the_model_default() {
    let mut setup = Setup::new();
    setup.user("Hi");
    setup.capabilities.reasoning = true;
    setup.capabilities.reasoning_levels = vec![
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
    ];
    let project = Project::create(setup.user_id(), "Work", now())
        .unwrap()
        .with_reasoning(Some(ReasoningLevel::Off));
    setup.chat = setup.chat.clone().with_project(Some(project.id()));
    setup.project = Some(project);
    let context = setup.build();
    assert_eq!(context.reasoning, None, "the project's off is not listed");
    assert_eq!(setup.request_with(&context).reasoning, None);

    setup.chat = setup
        .chat
        .clone()
        .with_reasoning(Some(ReasoningLevel::Medium));
    assert_eq!(setup.build().reasoning, Some(ReasoningLevel::Medium));

    let listed = setup.build();
    setup.capabilities.reasoning_levels = vec![ReasoningLevel::Low];
    assert_eq!(
        setup.request_with(&listed).reasoning,
        None,
        "a snapshot level the current model does not list"
    );
}

#[test]
fn request_settings_follow_the_model_and_chat() {
    let mut setup = Setup::new();
    setup.user("Hi");
    let request = setup.request();
    assert_eq!(request.model, "gpt-test");
    assert_eq!(request.max_output_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
    assert!(request.audio_out.is_none());
    assert!(!request.cache_breakpoints);
    assert!(!request.can_reason);

    setup.limits(Some(16_000), None);
    assert_eq!(output_limit(&setup.model, &setup.capabilities), 4_000);
    setup.limits(Some(16_000), Some(1_000));
    assert_eq!(output_limit(&setup.model, &setup.capabilities), 1_000);

    setup.capabilities.audio_out = true;
    setup.chat = setup.chat.clone().with_voice_replies(true);
    let audio = setup.request().audio_out.unwrap();
    assert_eq!(audio.format, "pcm16");
}

#[test]
fn a_reasoning_model_gets_a_larger_default_output_and_says_it_reasons() {
    let mut setup = Setup::new();
    setup.user("Hi");
    setup.capabilities.reasoning = true;
    let request = setup.request();
    assert!(request.can_reason);
    assert_eq!(
        request.reasoning, None,
        "no level chosen: the model default"
    );
    assert_eq!(request.max_output_tokens, DEFAULT_REASONING_OUTPUT_TOKENS);

    setup.limits(Some(64_000), None);
    assert_eq!(
        output_limit(&setup.model, &setup.capabilities),
        16_000,
        "a quarter of a smaller window"
    );
    setup.limits(Some(1_000_000), None);
    assert_eq!(
        output_limit(&setup.model, &setup.capabilities),
        DEFAULT_REASONING_OUTPUT_TOKENS
    );
    setup.limits(None, Some(2_000));
    assert_eq!(
        output_limit(&setup.model, &setup.capabilities),
        2_000,
        "the model's own limit wins"
    );
}

#[test]
fn references_name_every_file_once() {
    let mut setup = Setup::new();
    let file = FileId::generate();
    let image = FileId::generate();
    setup.user_parts(vec![Part::File {
        file_id: file,
        name: "a.txt".to_owned(),
        media_type: "text/plain".to_owned(),
    }]);
    let found = call("gh__search", "search", "{}");
    setup.answer(vec![Part::ToolCall(found.clone())], MessageStatus::Complete);
    setup.results(vec![ToolResultPart {
        content: vec![ResultContent::Image { file_id: image }],
        ..ok(&found, "")
    }]);
    setup.user_parts(vec![Part::File {
        file_id: file,
        name: "a.txt".to_owned(),
        media_type: "text/plain".to_owned(),
    }]);
    assert_eq!(references(&setup.history), [file, image]);
}

#[test]
fn a_request_never_ends_with_an_answer() {
    let mut setup = Setup::new();
    setup.user("Hi");
    setup.answer(
        vec![Part::Text {
            text: "Hello".to_owned(),
        }],
        MessageStatus::Complete,
    );
    let request = setup.request();
    assert_eq!(roles(&request), [Role::User]);
}
