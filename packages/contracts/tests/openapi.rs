//! Every contract builds its OpenAPI schema, and the schemas say what the wire carries.

use copilot_contracts::{
    ApprovalMode, BuiltinTool, Capabilities, Capability, CapabilityLock, CapabilityOverrides,
    Capacity, CapacityLevel, CreateProjectRequest, CreateProviderRequest, CreateScheduleRequest,
    CreateToolServerRequest, CreatedBy, Credentials, FileResponse, KeyState, ModelResponse,
    MoveFileRequest, OnAsk, Preset, ProjectDetailsResponse, ProjectListQuery, ProjectResponse,
    ProviderDetailsResponse, ProviderKind, ProviderResponse, ProviderUsage, ReasoningLevel,
    RefreshResponse, RunStatus, ScheduleListQuery, SchedulePreviewRequest, SchedulePreviewResponse,
    ScheduleResponse, ScheduleRunResponse, ServerTool, SettingsResponse, ToolChanges, ToolMode,
    ToolServerResponse, ToolTestResponse, UpdateFileRequest, UpdateModelRequest,
    UpdateProjectRequest, UpdateProviderRequest, UpdateScheduleRequest, UpdateSettingsRequest,
    UpdateToolServerRequest, UploadFileForm,
};
use serde_json::Value;
use utoipa::{IntoParams, OpenApi};

#[derive(OpenApi)]
#[openapi(components(schemas(
    ApprovalMode,
    BuiltinTool,
    Capabilities,
    Capability,
    CapabilityLock,
    CapabilityOverrides,
    Capacity,
    CapacityLevel,
    CreateProjectRequest,
    CreateProviderRequest,
    CreateScheduleRequest,
    CreateToolServerRequest,
    CreatedBy,
    Credentials,
    FileResponse,
    KeyState,
    ModelResponse,
    MoveFileRequest,
    OnAsk,
    Preset,
    ProjectDetailsResponse,
    ProjectResponse,
    ProviderDetailsResponse,
    ProviderKind,
    ProviderResponse,
    ProviderUsage,
    ReasoningLevel,
    RefreshResponse,
    RunStatus,
    SchedulePreviewRequest,
    SchedulePreviewResponse,
    ScheduleResponse,
    ScheduleRunResponse,
    ServerTool,
    SettingsResponse,
    ToolChanges,
    ToolMode,
    ToolServerResponse,
    ToolTestResponse,
    UpdateFileRequest,
    UpdateModelRequest,
    UpdateProjectRequest,
    UpdateProviderRequest,
    UpdateScheduleRequest,
    UpdateSettingsRequest,
    UpdateToolServerRequest,
    UploadFileForm,
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
    if let Some(parts) = schema["allOf"].as_array() {
        names.extend(parts.iter().flat_map(properties));
    }
    names
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
}
