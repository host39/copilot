//! `/api/projects`: projects (`ProjectService`). Their files are in `routes::files`. Deleting a
//! project stops the live runs of its chats first, so no run writes into a chat that the delete
//! removes.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{
    CreateProjectRequest, DetailsResponse, ErrorResponse, ListResponse, ProjectDetailsResponse,
    ProjectListQuery, ProjectResponse, UpdateProjectRequest,
};
use copilot_core::shared::constants::SHUTDOWN_DRAIN_SECS;
use copilot_core::{
    DomainError, ModelId, NewProject, Project, ProjectId, ProjectQuery, ProjectUpdate, ToolServerId,
};
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::path::{Id, Params};
use crate::wire::convert::{ToCore, ToDto};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, update, delete))
}

/// A project as the API shows it.
pub fn response(project: &Project) -> ProjectResponse {
    ProjectResponse {
        id: project.id().uuid(),
        name: project.name().to_owned(),
        icon: project.icon().to_owned(),
        instructions: project.instructions().to_owned(),
        default_model_id: project.default_model_id().map(ModelId::uuid),
        reasoning: project.reasoning().map(ToDto::to_dto),
        builtin_tools: project
            .builtin_tools()
            .iter()
            .map(|tool| tool.to_dto())
            .collect(),
        tool_server_ids: project
            .tool_server_ids()
            .iter()
            .map(|id| id.uuid())
            .collect(),
        pinned: project.pinned(),
        created_at: project.created_at(),
        updated_at: project.updated_at(),
    }
}

fn servers(ids: Vec<Uuid>) -> Vec<ToolServerId> {
    ids.into_iter().map(ToolServerId::new).collect()
}

#[utoipa::path(
    get,
    path = "/projects",
    operation_id = "projects_list",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(ProjectListQuery),
    responses(
        (status = OK, description = "The caller's projects, most recently updated first", body = ListResponse<ProjectResponse>),
        (status = BAD_REQUEST, description = "A query parameter is not valid", body = ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Params(query): Params<ProjectListQuery>,
) -> Result<axum::Json<ListResponse<ProjectResponse>>, HttpError> {
    let projects = state
        .projects
        .list(
            &caller,
            ProjectQuery {
                pinned: query.pinned,
                limit: query.limit,
            },
        )
        .await?;
    let items: Vec<ProjectResponse> = projects.iter().map(response).collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
}

#[utoipa::path(
    post,
    path = "/projects",
    operation_id = "projects_create",
    tag = "projects",
    security(("bearerAuth" = [])),
    request_body = CreateProjectRequest,
    responses(
        (status = CREATED, description = "The new project", body = DetailsResponse<ProjectResponse>),
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such model or tool server", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<CreateProjectRequest>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<ProjectResponse>>), HttpError> {
    let input = NewProject {
        name: body.name,
        icon: body.icon.unwrap_or_default(),
        instructions: body.instructions.unwrap_or_default(),
        default_model_id: body.default_model_id.map(ModelId::new),
        reasoning: body.reasoning.map(ToCore::to_core),
        builtin_tools: body
            .builtin_tools
            .unwrap_or_default()
            .into_iter()
            .map(ToCore::to_core)
            .collect(),
        tool_server_ids: servers(body.tool_server_ids.unwrap_or_default()),
        pinned: body.pinned.unwrap_or(false),
    };
    let project = state.projects.create(&caller, input).await?;
    let data = response(&project);
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
}

#[utoipa::path(
    get,
    path = "/projects/{id}",
    operation_id = "projects_get",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    responses(
        (status = OK, description = "The project with its capacity", body = DetailsResponse<ProjectDetailsResponse>),
        (status = NOT_FOUND, description = "The caller has no such project", body = ErrorResponse)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ProjectDetailsResponse>>, HttpError> {
    let id = ProjectId::new(id);
    let project = state.projects.get(&caller, id).await?;
    let capacity = state.files.capacity(&caller, id).await?;
    Ok(axum::Json(DetailsResponse {
        data: ProjectDetailsResponse {
            project: response(&project),
            capacity: capacity.to_dto(),
        },
    }))
}

#[utoipa::path(
    patch,
    path = "/projects/{id}",
    operation_id = "projects_update",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    request_body = UpdateProjectRequest,
    responses(
        (status = OK, description = "The saved project", body = DetailsResponse<ProjectResponse>),
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such project, model, or tool server", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateProjectRequest>,
) -> Result<axum::Json<DetailsResponse<ProjectResponse>>, HttpError> {
    let changes = ProjectUpdate {
        name: body.name,
        icon: body.icon,
        instructions: body.instructions,
        default_model_id: body.default_model_id.map(|id| id.map(ModelId::new)),
        reasoning: body.reasoning.map(|level| level.map(ToCore::to_core)),
        builtin_tools: body
            .builtin_tools
            .map(|tools| tools.into_iter().map(ToCore::to_core).collect()),
        tool_server_ids: body.tool_server_ids.map(servers),
        pinned: body.pinned,
    };
    let project = state
        .projects
        .update(&caller, ProjectId::new(id), changes)
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&project),
    }))
}

#[utoipa::path(
    delete,
    path = "/projects/{id}",
    operation_id = "projects_delete",
    tag = "projects",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its chats, files, and schedules"),
        (status = NOT_FOUND, description = "The caller has no such project", body = ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    let id = ProjectId::new(id);
    state.projects.get(&caller, id).await?;
    // At most `MAX_ACTIVE_RUNS_PER_USER` live chats to look at.
    for chat in state.runs.chats(caller.user_id) {
        let in_project = match state.chats.get(&caller, chat).await {
            Ok(found) => found.project_id() == Some(id),
            // Deleted meanwhile: its run is ending on its own.
            Err(DomainError::EntityNotFound { .. }) => false,
            Err(error) => return Err(error.into()),
        };
        if in_project
            && !state
                .runs
                .stop(chat, Duration::from_secs(SHUTDOWN_DRAIN_SECS))
                .await
        {
            tracing::warn!(chat_id = %chat, project_id = %id, "api.projects.delete.run-live");
        }
    }
    state.projects.delete(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use axum::Router;
    use axum::http::{Method, StatusCode};
    use copilot_database::DbPool;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    use crate::helpers::testing::{
        TestResponse, live_app, model, other_user, provider, read, register, request, send,
        tool_server,
    };

    async fn call(
        app: &Router,
        token: &str,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> TestResponse {
        send(app.clone(), request(method, uri, Some(token), body)).await
    }

    fn id(response: &TestResponse) -> String {
        response.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned()
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn projects_are_created_listed_changed_and_deleted(pool: DbPool) {
        let server = provider().await;
        let app = live_app(pool.clone()).await;
        let alice = register(&app, "alice").await;
        let default_model = model(&app, &alice, &server).await;
        let docs = tool_server(&pool, "alice", "docs").await;

        let created = call(
            &app,
            &alice,
            Method::POST,
            "/api/projects",
            Some(json!({
                "name": "  Research  ",
                "icon": "🔬",
                "instructions": "Cite sources.",
                "defaultModelId": default_model,
                "reasoning": "high",
                "builtinTools": ["schedules"],
                "toolServerIds": [docs],
                "pinned": true,
            })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let data = &created.body["data"];
        assert_eq!(data["name"], "Research");
        assert_eq!(data["icon"], "🔬");
        assert_eq!(data["defaultModelId"], default_model.as_str());
        assert_eq!(data["reasoning"], "high");
        assert_eq!(data["builtinTools"], json!(["schedules"]));
        assert_eq!(data["toolServerIds"], json!([docs]));
        assert_eq!(data["pinned"], true);
        let research = id(&created);
        let plain = call(
            &app,
            &alice,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "Plain" })),
        )
        .await;
        assert_eq!(plain.body["data"]["icon"], "");
        assert_eq!(plain.body["data"]["reasoning"], Value::Null);
        assert_eq!(plain.body["data"]["toolServerIds"], json!([]));
        let plain = id(&plain);

        let all = call(&app, &alice, Method::GET, "/api/projects", None).await;
        assert_eq!(all.body["total"], 2);
        assert_eq!(all.body["items"][0]["id"], plain.as_str(), "newest first");
        let pinned = call(
            &app,
            &alice,
            Method::GET,
            "/api/projects?pinned=true&limit=5",
            None,
        )
        .await;
        assert_eq!(pinned.body["total"], 1);
        assert_eq!(pinned.body["items"][0]["id"], research.as_str());
        let invalid = call(&app, &alice, Method::GET, "/api/projects?limit=lots", None).await;
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid.body["code"], "VALIDATION_ERROR");

        let uri = format!("/api/projects/{research}");
        let details = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(details.status, StatusCode::OK);
        assert_eq!(details.body["data"]["name"], "Research");
        assert_eq!(
            details.body["data"]["capacity"],
            json!({ "used": 0, "window": null, "level": "unknown" }),
            "the mock reports no context window"
        );

        let changed = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({
                "defaultModelId": null,
                "reasoning": null,
                "toolServerIds": [],
                "pinned": false,
            })),
        )
        .await;
        assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        let data = &changed.body["data"];
        assert_eq!(data["defaultModelId"], Value::Null);
        assert_eq!(data["reasoning"], Value::Null);
        assert_eq!(data["toolServerIds"], json!([]));
        assert_eq!(data["pinned"], false);
        assert_eq!(data["name"], "Research", "fields left out stay");
        assert_eq!(data["instructions"], "Cite sources.");

        let blank = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({ "name": " " })),
        )
        .await;
        assert_eq!(blank.status, StatusCode::BAD_REQUEST);

        let deleted = call(&app, &alice, Method::DELETE, &uri, None).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        let gone = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn another_user_gets_404_for_projects_and_for_ids_in_bodies(pool: DbPool) {
        let server = provider().await;
        let app = live_app(pool.clone()).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let alice_model = model(&app, &alice, &server).await;
        let alice_server = tool_server(&pool, "alice", "docs").await;
        let project = call(
            &app,
            &alice,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "Private", "instructions": "Secret plans." })),
        )
        .await;
        let project = id(&project);
        let uri = format!("/api/projects/{project}");

        for (method, body) in [
            (Method::GET, None),
            (Method::PATCH, Some(json!({ "name": "Taken" }))),
            (Method::DELETE, None),
        ] {
            let response = call(&app, &bob, method.clone(), &uri, body).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{method}");
            assert_eq!(response.body["code"], "ENTITY_NOT_FOUND");
        }
        let list = call(&app, &bob, Method::GET, "/api/projects", None).await;
        assert_eq!(list.body["total"], 0);

        for body in [
            json!({ "name": "Mine", "defaultModelId": alice_model }),
            json!({ "name": "Mine", "toolServerIds": [alice_server] }),
        ] {
            let response = call(&app, &bob, Method::POST, "/api/projects", Some(body)).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{}", response.body);
        }
        let own = call(
            &app,
            &bob,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "Mine" })),
        )
        .await;
        let own = format!("/api/projects/{}", id(&own));
        for body in [
            json!({ "defaultModelId": alice_model }),
            json!({ "toolServerIds": [alice_server] }),
        ] {
            let response = call(&app, &bob, Method::PATCH, &own, Some(body)).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{}", response.body);
        }
        let chat = call(
            &app,
            &bob,
            Method::POST,
            "/api/chats",
            Some(json!({ "projectId": project })),
        )
        .await;
        assert_eq!(chat.status, StatusCode::NOT_FOUND, "a chat in A's project");

        let mine = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(mine.body["data"]["name"], "Private", "unchanged");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn deleting_a_project_stops_the_live_runs_of_its_chats_first(pool: DbPool) {
        let server = provider().await;
        // A provider that takes long to answer keeps the run live.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
            .mount(&server)
            .await;
        let app = live_app(pool.clone()).await;
        let alice = register(&app, "alice").await;
        let default_model = model(&app, &alice, &server).await;
        let project = call(
            &app,
            &alice,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "Busy", "defaultModelId": default_model })),
        )
        .await;
        let project = id(&project);
        let chat = call(
            &app,
            &alice,
            Method::POST,
            "/api/chats",
            Some(json!({ "projectId": project, "modelId": default_model })),
        )
        .await;
        let chat = id(&chat);

        let turn = tokio::spawn(read(
            app.clone(),
            request(
                Method::POST,
                &format!("/api/chats/{chat}/messages"),
                Some(&alice),
                Some(json!({ "text": "Hello" })),
            ),
        ));
        // Wait until the run is waiting for the provider.
        let started = Instant::now();
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            if requests
                .iter()
                .any(|request| request.url.path() == "/v1/chat/completions")
            {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the run starts"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let started = Instant::now();
        let deleted = call(
            &app,
            &alice,
            Method::DELETE,
            &format!("/api/projects/{project}"),
            None,
        )
        .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        let (status, _, body) = tokio::time::timeout(Duration::from_secs(10), turn)
            .await
            .expect("the run ended with the delete, long before the provider answered")
            .expect("the turn task");
        assert_eq!(status, StatusCode::OK);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(body.contains("\"cancelled\""), "{body}");
        assert!(body.contains("\"type\":\"done\""), "{body}");
        let gone = call(
            &app,
            &alice,
            Method::GET,
            &format!("/api/chats/{chat}"),
            None,
        )
        .await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND, "the chat went with it");
    }
}
