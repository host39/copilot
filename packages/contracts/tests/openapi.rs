//! Every contract builds its OpenAPI schema, and the schemas say what the wire carries.

use copilot_contracts::{
    Approval, ApprovalMode, ApprovedTool, BuiltinTool, CallDecision, Capabilities, Capability,
    CapabilityLock, CapabilityOverrides, Capacity, CapacityLevel, ChatListQuery, ChatResponse,
    ChatStreamEvent, ChatSummaryResponse, CreateChatRequest, CreateProjectRequest,
    CreateProviderRequest, CreateScheduleRequest, CreateToolServerRequest, CreatedBy, Credentials,
    Decision, DecisionScope, DecisionsRequest, FileResponse, FinishReason, KeyState,
    MessagePartDto, MessageResponse, MessageStatus, ModelResponse, MoveFileRequest, OnAsk,
    PartKind, Preset, ProjectDetailsResponse, ProjectListQuery, ProjectResponse,
    ProviderDetailsResponse, ProviderKind, ProviderResponse, ProviderUsage, ReasoningLevel,
    RefreshResponse, Refusal, ResultContent, Role, RunStatus, ScheduleListQuery,
    SchedulePreviewRequest, SchedulePreviewResponse, ScheduleResponse, ScheduleRunResponse,
    SearchHitResponse, SearchQuery, SendMessageRequest, ServerTool, SettingsResponse, ToolChanges,
    ToolMode, ToolOverride, ToolServerResponse, ToolStatus, ToolTestResponse, UpdateChatRequest,
    UpdateFileRequest, UpdateModelRequest, UpdateProjectRequest, UpdateProviderRequest,
    UpdateScheduleRequest, UpdateSettingsRequest, UpdateToolServerRequest, UploadFileForm, Usage,
    UserEvent,
};
use serde_json::Value;
use utoipa::{IntoParams, OpenApi};

#[derive(OpenApi)]
#[openapi(components(schemas(
    Approval,
    ApprovalMode,
    ApprovedTool,
    BuiltinTool,
    CallDecision,
    Capabilities,
    Capability,
    CapabilityLock,
    CapabilityOverrides,
    Capacity,
    CapacityLevel,
    ChatResponse,
    ChatStreamEvent,
    ChatSummaryResponse,
    CreateChatRequest,
    CreateProjectRequest,
    CreateProviderRequest,
    CreateScheduleRequest,
    CreateToolServerRequest,
    CreatedBy,
    Credentials,
    Decision,
    DecisionScope,
    DecisionsRequest,
    FileResponse,
    FinishReason,
    KeyState,
    MessagePartDto,
    MessageResponse,
    MessageStatus,
    ModelResponse,
    MoveFileRequest,
    OnAsk,
    PartKind,
    Preset,
    ProjectDetailsResponse,
    ProjectResponse,
    ProviderDetailsResponse,
    ProviderKind,
    ProviderResponse,
    ProviderUsage,
    ReasoningLevel,
    Refusal,
    RefreshResponse,
    ResultContent,
    Role,
    RunStatus,
    SchedulePreviewRequest,
    SchedulePreviewResponse,
    ScheduleResponse,
    ScheduleRunResponse,
    SearchHitResponse,
    SendMessageRequest,
    ServerTool,
    SettingsResponse,
    ToolChanges,
    ToolMode,
    ToolOverride,
    ToolServerResponse,
    ToolStatus,
    ToolTestResponse,
    UpdateChatRequest,
    UpdateFileRequest,
    UpdateModelRequest,
    UpdateProjectRequest,
    UpdateProviderRequest,
    UpdateScheduleRequest,
    UpdateSettingsRequest,
    UpdateToolServerRequest,
    UploadFileForm,
    Usage,
    UserEvent,
)))]
struct Doc;

fn schemas() -> Value {
    let doc = serde_json::to_value(Doc::openapi()).expect("serializes");
    doc["components"]["schemas"].clone()
}

/// Every property name a schema declares, also through `allOf` (flattened fields).
fn properties(schema: &Value) -> Vec<String> {
    let mut names: Vec<String> = schema["properties"]
        .as_object()
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default();
    for key in ["allOf", "oneOf"] {
        if let Some(parts) = schema[key].as_array() {
            names.extend(parts.iter().flat_map(properties));
        }
    }
    names
}

/// The `type` values of a tagged union's variants.
fn tags(schema: &Value) -> Vec<String> {
    schema["oneOf"]
        .as_array()
        .expect("a oneOf")
        .iter()
        .map(|variant| {
            let tag = &variant["properties"]["type"];
            tag["enum"][0]
                .as_str()
                .or_else(|| tag["const"].as_str())
                .unwrap_or_else(|| panic!("no tag in {variant}"))
                .to_owned()
        })
        .collect()
}

#[test]
fn fields_are_camel_case() {
    let schemas = schemas();
    for (name, schema) in schemas.as_object().expect("schemas") {
        for property in properties(schema) {
            assert!(
                !property.contains('_'),
                "{name}.{property} is not camelCase"
            );
        }
    }
}

#[test]
fn details_flatten_their_base() {
    let schemas = schemas();
    let provider = properties(&schemas["ProviderDetailsResponse"]);
    assert!(provider.contains(&"usage".to_owned()), "{provider:?}");
    let project = properties(&schemas["ProjectDetailsResponse"]);
    assert!(project.contains(&"capacity".to_owned()), "{project:?}");
}

#[test]
fn uploads_are_binary() {
    let schemas = schemas();
    let file = &schemas["UploadFileForm"]["properties"]["file"];
    assert_eq!(file["format"], "binary", "{file}");
}

#[test]
fn queries_name_camel_case_parameters() {
    let names = |parameters: Vec<utoipa::openapi::path::Parameter>| -> Vec<String> {
        parameters
            .into_iter()
            .map(|parameter| parameter.name)
            .collect()
    };
    assert_eq!(
        names(ProjectListQuery::into_params(|| None)),
        ["pinned", "limit"]
    );
    assert_eq!(
        names(ScheduleListQuery::into_params(|| None)),
        ["projectId"]
    );
    assert_eq!(
        names(ChatListQuery::into_params(|| None)),
        [
            "projectId",
            "scheduleId",
            "pinned",
            "scheduled",
            "cursor",
            "limit"
        ]
    );
    assert_eq!(names(SearchQuery::into_params(|| None)), ["q", "limit"]);
}

#[test]
fn unions_are_tagged_by_type() {
    let schemas = schemas();
    assert_eq!(
        tags(&schemas["ChatStreamEvent"]),
        [
            "message.started",
            "part.started",
            "text.delta",
            "reasoning.delta",
            "audio.delta",
            "tool.requested",
            "tool.finished",
            "message.finished",
            "error",
            "done"
        ]
    );
    assert_eq!(
        tags(&schemas["UserEvent"]),
        [
            "chat.updated",
            "run.started",
            "run.finished",
            "approval.pending",
            "schedule.updated"
        ]
    );
    assert_eq!(
        tags(&schemas["MessagePartDto"]),
        [
            "text",
            "file",
            "audio",
            "reasoning",
            "toolCall",
            "toolResult"
        ]
    );
    for variant in schemas["ChatStreamEvent"]["oneOf"]
        .as_array()
        .expect("variants")
    {
        let required = variant["required"].as_array().expect("required fields");
        assert!(required.contains(&Value::from("seq")), "{variant}");
    }
}
