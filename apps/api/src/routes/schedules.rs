//! `/api/schedules`: prompts that run at set times (`ScheduleService`). A schedule's runs are
//! `GET /api/chats?scheduleId=`. Roadmap step P2.A wrote the settings, the list, and the preview;
//! P3.2 wrote Run now, which starts a run exactly like the scheduler's (`helpers::scheduler`).

use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{
    CreateScheduleRequest, DetailsResponse, ErrorResponse, ListResponse, ScheduleListQuery,
    SchedulePreviewRequest, SchedulePreviewResponse, ScheduleResponse, ScheduleRunResponse,
    UpdateScheduleRequest,
};
use copilot_core::{
    DomainError, ModelId, NewSchedule, ProjectId, Schedule, ScheduleId, ScheduleQuery,
    ScheduleUpdate,
};
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::helpers::scheduler;
use crate::routes::path::{Id, Params};
use crate::wire::convert::{ToCore, ToDto};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, update, delete))
        .routes(routes!(run))
        .routes(routes!(preview))
}

/// A schedule as the API shows it, with its rule in plain words.
pub fn response(schedule: &Schedule) -> ScheduleResponse {
    ScheduleResponse {
        id: schedule.id().uuid(),
        project_id: schedule.project_id().map(ProjectId::uuid),
        model_id: schedule.model_id().map(ModelId::uuid),
        source_chat_id: schedule.source_chat_id().map(|id| id.uuid()),
        name: schedule.name().to_owned(),
        cron: schedule.rule().expression().to_owned(),
        timezone: schedule.rule().timezone().to_owned(),
        description: schedule.rule().describe(),
        prompt: schedule.prompt().to_owned(),
        instructions: schedule.instructions().to_owned(),
        reasoning: schedule.reasoning().map(ToDto::to_dto),
        on_ask: schedule.on_ask().to_dto(),
        created_by: schedule.created_by().to_dto(),
        enabled: schedule.enabled(),
        next_run_at: schedule.next_run_at(),
        last_run_at: schedule.last_run_at(),
        last_status: schedule.last_status().map(ToDto::to_dto),
        last_error: schedule.last_error().map(ToOwned::to_owned),
        consecutive_failures: schedule.consecutive_failures(),
        disabled_reason: schedule.disabled_reason().map(ToOwned::to_owned),
        created_at: schedule.created_at(),
        updated_at: schedule.updated_at(),
    }
}

#[utoipa::path(
    get,
    path = "/schedules",
    operation_id = "schedules_list",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(ScheduleListQuery),
    responses(
        (status = OK, description = "The caller's schedules, oldest first", body = ListResponse<ScheduleResponse>),
        (status = BAD_REQUEST, description = "A query parameter is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such project", body = ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Params(query): Params<ScheduleListQuery>,
) -> Result<axum::Json<ListResponse<ScheduleResponse>>, HttpError> {
    let query = ScheduleQuery {
        project: query.project_id.map(ProjectId::new),
        ..ScheduleQuery::default()
    };
    let schedules = state.schedules.list(&caller, query).await?;
    let items: Vec<ScheduleResponse> = schedules.iter().map(response).collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
}

#[utoipa::path(
    post,
    path = "/schedules",
    operation_id = "schedules_create",
    tag = "schedules",
    security(("bearerAuth" = [])),
    request_body = CreateScheduleRequest,
    responses(
        (status = CREATED, description = "The new schedule, turned on", body = DetailsResponse<ScheduleResponse>),
        (status = BAD_REQUEST, description = "A field is not valid (for example the rule, the time zone, a gap under 15 minutes, or no usable model)", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such project or model", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The caller has 20 schedules turned on (`TOO_MANY_SCHEDULES`)", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<CreateScheduleRequest>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<ScheduleResponse>>), HttpError> {
    let input = NewSchedule {
        name: body.name,
        cron: body.cron,
        timezone: body.timezone,
        prompt: body.prompt,
        project_id: body.project_id.map(ProjectId::new),
        model_id: body.model_id.map(ModelId::new),
        instructions: body.instructions.unwrap_or_default(),
        reasoning: body.reasoning.map(ToCore::to_core),
        on_ask: body.on_ask.unwrap_or_default().to_core(),
    };
    let schedule = state.schedules.create(&caller, input).await?;
    let data = response(&schedule);
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
}

#[utoipa::path(
    get,
    path = "/schedules/{id}",
    operation_id = "schedules_get",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    responses(
        (status = OK, description = "The schedule", body = DetailsResponse<ScheduleResponse>),
        (status = NOT_FOUND, description = "The caller has no such schedule", body = ErrorResponse)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ScheduleResponse>>, HttpError> {
    let schedule = state.schedules.get(&caller, ScheduleId::new(id)).await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&schedule),
    }))
}

#[utoipa::path(
    patch,
    path = "/schedules/{id}",
    operation_id = "schedules_update",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    request_body = UpdateScheduleRequest,
    responses(
        (status = OK, description = "The saved schedule", body = DetailsResponse<ScheduleResponse>),
        (status = BAD_REQUEST, description = "A field is not valid, or turning it on needs a usable model", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such schedule or model", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "Turning it on would pass 20 schedules (`TOO_MANY_SCHEDULES`), or a run is starting right now (`SCHEDULE_BUSY`)", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateScheduleRequest>,
) -> Result<axum::Json<DetailsResponse<ScheduleResponse>>, HttpError> {
    let changes = ScheduleUpdate {
        name: body.name,
        cron: body.cron,
        timezone: body.timezone,
        prompt: body.prompt,
        instructions: body.instructions,
        model_id: body.model_id.map(ModelId::new),
        reasoning: body.reasoning.map(|level| level.map(ToCore::to_core)),
        on_ask: body.on_ask.map(ToCore::to_core),
        enabled: body.enabled,
    };
    let schedule = state
        .schedules
        .update(&caller, ScheduleId::new(id), changes)
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&schedule),
    }))
}

#[utoipa::path(
    delete,
    path = "/schedules/{id}",
    operation_id = "schedules_delete",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted; the chats of its runs stay"),
        (status = NOT_FOUND, description = "The caller has no such schedule", body = ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    state.schedules.delete(&caller, ScheduleId::new(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/schedules/{id}/run",
    operation_id = "schedules_run",
    tag = "schedules",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The schedule's id")),
    responses(
        (status = OK, description = "The run started in a new chat, unattended like a scheduled fire, and the next run stays; attach with `GET /chats/{id}/stream`", body = DetailsResponse<ScheduleRunResponse>),
        (status = BAD_REQUEST, description = "The run could not start (for example, the project's files no longer fit the model); recorded as a failed run", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such schedule", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The schedule's model was removed or turned off, so the schedule paused (`MODEL_UNAVAILABLE`), two scheduled runs or one of the caller's are running (`SCHEDULER_BUSY`), or the server is shutting down (`SERVER_STOPPING`)", body = ErrorResponse)
    )
)]
pub async fn run(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ScheduleRunResponse>>, HttpError> {
    let firing = state
        .schedules
        .run_now(&caller, ScheduleId::new(id))
        .await?;
    let reservation = state.runs.reserve_now(caller.user_id, firing.chat_id)?;
    // On a task of its own: a client that disconnects never leaves a written chat without its
    // run (dropping a `JoinHandle` does not cancel its task).
    let task = tokio::spawn({
        let state = state.clone();
        async move { scheduler::fire(&state, reservation, firing).await }
    });
    let chat = task
        .await
        .map_err(|error| DomainError::internal("api.schedules.run.panicked", error))??;
    Ok(axum::Json(DetailsResponse {
        data: ScheduleRunResponse {
            chat_id: chat.uuid(),
        },
    }))
}

#[utoipa::path(
    post,
    path = "/schedules/preview",
    operation_id = "schedules_preview",
    tag = "schedules",
    security(("bearerAuth" = [])),
    request_body = SchedulePreviewRequest,
    responses(
        (status = OK, description = "The rule in words and its next 3 runs", body = DetailsResponse<SchedulePreviewResponse>),
        (status = BAD_REQUEST, description = "The rule or the time zone is not valid, or two fires are less than 15 minutes apart", body = ErrorResponse)
    )
)]
pub async fn preview(
    State(state): State<AppState>,
    Caller(_caller): Caller,
    JsonBody(body): JsonBody<SchedulePreviewRequest>,
) -> Result<axum::Json<DetailsResponse<SchedulePreviewResponse>>, HttpError> {
    let preview = state.schedules.preview(&body.cron, &body.timezone)?;
    Ok(axum::Json(DetailsResponse {
        data: SchedulePreviewResponse {
            description: preview.description,
            timezone: preview.timezone,
            next_runs: preview.next_runs,
        },
    }))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{Method, StatusCode};
    use copilot_core::shared::constants::MAX_SCHEDULES_PER_USER;
    use copilot_database::DbPool;
    use serde_json::{Value, json};

    use crate::helpers::testing::{
        TestResponse, live_app, model, other_user, provider, register, request, send,
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
    async fn schedules_are_created_listed_changed_and_deleted(pool: DbPool) {
        let server = provider().await;
        let app = live_app(pool).await;
        let token = register(&app, "admin").await;
        let model = model(&app, &token, &server).await;
        let project = call(
            &app,
            &token,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "News" })),
        )
        .await;
        let project = id(&project);

        let created = call(
            &app,
            &token,
            Method::POST,
            "/api/schedules",
            Some(json!({
                "name": " Morning news ",
                "cron": "0 7 * * 1-5",
                "timezone": "Europe/Berlin",
                "prompt": "Summarize the news.",
                "modelId": model,
                "instructions": "Be brief.",
                "reasoning": "low",
                "onAsk": "pause",
            })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let data = &created.body["data"];
        assert_eq!(data["name"], "Morning news");
        assert_eq!(data["cron"], "0 7 * * 1-5");
        assert_eq!(data["timezone"], "Europe/Berlin");
        assert!(
            data["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
        assert_eq!(data["instructions"], "Be brief.");
        assert_eq!(data["reasoning"], "low");
        assert_eq!(data["onAsk"], "pause");
        assert_eq!(data["createdBy"], "user");
        assert_eq!(data["enabled"], true);
        assert!(data["nextRunAt"].is_string());
        assert_eq!(data["lastRunAt"], Value::Null);
        assert_eq!(data["lastStatus"], Value::Null);
        assert_eq!(data["consecutiveFailures"], 0);
        assert_eq!(data["projectId"], Value::Null);
        let standalone = id(&created);
        let in_project = call(
            &app,
            &token,
            Method::POST,
            "/api/schedules",
            Some(json!({
                "name": "Digest",
                "cron": "30 18 * * 5",
                "timezone": "UTC",
                "prompt": "Weekly digest.",
                "projectId": project,
                "modelId": model,
                "instructions": "Ignored in a project.",
            })),
        )
        .await;
        assert_eq!(
            in_project.status,
            StatusCode::CREATED,
            "{}",
            in_project.body
        );
        assert_eq!(in_project.body["data"]["projectId"], project.as_str());
        assert_eq!(in_project.body["data"]["instructions"], "");
        assert_eq!(in_project.body["data"]["onAsk"], "deny", "the default");
        let in_project = id(&in_project);

        let all = call(&app, &token, Method::GET, "/api/schedules", None).await;
        assert_eq!(all.body["total"], 2);
        assert_eq!(
            all.body["items"][0]["id"],
            standalone.as_str(),
            "oldest first"
        );
        let filtered = call(
            &app,
            &token,
            Method::GET,
            &format!("/api/schedules?projectId={project}"),
            None,
        )
        .await;
        assert_eq!(filtered.body["total"], 1);
        assert_eq!(filtered.body["items"][0]["id"], in_project.as_str());
        let runs = call(
            &app,
            &token,
            Method::GET,
            &format!("/api/chats?scheduleId={standalone}"),
            None,
        )
        .await;
        assert_eq!(runs.status, StatusCode::OK);
        assert_eq!(runs.body["items"], json!([]), "no runs yet");

        let uri = format!("/api/schedules/{standalone}");
        let before = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(before.body["data"]["name"], "Morning news");
        let changed = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "cron": "0 12 * * *", "timezone": "UTC", "reasoning": null })),
        )
        .await;
        assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        let data = &changed.body["data"];
        assert_eq!(data["cron"], "0 12 * * *");
        assert_eq!(data["timezone"], "UTC");
        assert_eq!(data["reasoning"], Value::Null);
        assert!(
            data["nextRunAt"]
                .as_str()
                .is_some_and(|next| next.contains("T12:00:00")),
            "the next run moved: {data}"
        );
        assert_ne!(data["description"], before.body["data"]["description"]);
        let off = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "enabled": false })),
        )
        .await;
        assert_eq!(off.body["data"]["enabled"], false);
        assert_eq!(off.body["data"]["nextRunAt"], Value::Null);
        assert_eq!(off.body["data"]["disabledReason"], Value::Null);
        let on = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "enabled": true, "prompt": "Lunch ideas." })),
        )
        .await;
        assert_eq!(on.body["data"]["enabled"], true);
        assert!(on.body["data"]["nextRunAt"].is_string());
        assert_eq!(on.body["data"]["prompt"], "Lunch ideas.");

        for (body, field) in [
            (json!({ "cron": "*/5 * * * *" }), "cron"),
            (json!({ "cron": "0 9 * *" }), "cron"),
            (json!({ "timezone": "Mars/Base" }), "timezone"),
            (json!({ "name": " " }), "name"),
        ] {
            let refused = call(&app, &token, Method::PATCH, &uri, Some(body.clone())).await;
            assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(refused.body["field"], field, "{body}: {}", refused.body);
        }
        let without_model = call(
            &app,
            &token,
            Method::POST,
            "/api/schedules",
            Some(json!({
                "name": "No model",
                "cron": "0 9 * * *",
                "timezone": "UTC",
                "prompt": "Hello.",
            })),
        )
        .await;
        assert_eq!(without_model.status, StatusCode::BAD_REQUEST);
        assert_eq!(without_model.body["field"], "modelId");

        let deleted = call(&app, &token, Method::DELETE, &uri, None).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        for method in [Method::GET, Method::DELETE] {
            let gone = call(&app, &token, method.clone(), &uri, None).await;
            assert_eq!(gone.status, StatusCode::NOT_FOUND, "{method}");
        }
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn a_user_has_at_most_twenty_schedules_turned_on(pool: DbPool) {
        let server = provider().await;
        let app = live_app(pool).await;
        let token = register(&app, "admin").await;
        let model = model(&app, &token, &server).await;
        let body = json!({
            "name": "Hourly",
            "cron": "0 * * * *",
            "timezone": "UTC",
            "prompt": "Check.",
            "modelId": model,
        });
        let mut first = String::new();
        for index in 0..MAX_SCHEDULES_PER_USER {
            let created = call(
                &app,
                &token,
                Method::POST,
                "/api/schedules",
                Some(body.clone()),
            )
            .await;
            assert_eq!(created.status, StatusCode::CREATED, "{index}");
            if index == 0 {
                first = id(&created);
            }
        }
        let refused = call(
            &app,
            &token,
            Method::POST,
            "/api/schedules",
            Some(body.clone()),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused.body["code"], "TOO_MANY_SCHEDULES");

        let uri = format!("/api/schedules/{first}");
        let off = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "enabled": false })),
        )
        .await;
        assert_eq!(off.status, StatusCode::OK);
        let created = call(&app, &token, Method::POST, "/api/schedules", Some(body)).await;
        assert_eq!(created.status, StatusCode::CREATED, "a slot is free again");
        let on = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "enabled": true })),
        )
        .await;
        assert_eq!(on.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(on.body["code"], "TOO_MANY_SCHEDULES");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn another_user_gets_404_for_schedules_and_for_ids_in_bodies(pool: DbPool) {
        let server = provider().await;
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let model = model(&app, &alice, &server).await;
        let project = call(
            &app,
            &alice,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "Private" })),
        )
        .await;
        let project = id(&project);
        let created = call(
            &app,
            &alice,
            Method::POST,
            "/api/schedules",
            Some(json!({
                "name": "Secret",
                "cron": "0 9 * * *",
                "timezone": "UTC",
                "prompt": "Secret plans.",
                "projectId": project,
                "modelId": model,
            })),
        )
        .await;
        let schedule = id(&created);
        let uri = format!("/api/schedules/{schedule}");

        for (method, body) in [
            (Method::GET, None),
            (Method::PATCH, Some(json!({ "name": "Taken" }))),
            (Method::PATCH, Some(json!({ "enabled": false }))),
            (Method::DELETE, None),
        ] {
            let response = call(&app, &bob, method.clone(), &uri, body).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{method}");
            assert_eq!(response.body["code"], "ENTITY_NOT_FOUND");
        }
        let list = call(&app, &bob, Method::GET, "/api/schedules", None).await;
        assert_eq!(list.body["total"], 0);
        for uri in [
            format!("/api/schedules?projectId={project}"),
            format!("/api/chats?scheduleId={schedule}"),
        ] {
            let response = call(&app, &bob, Method::GET, &uri, None).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{uri}");
        }
        for body in [
            json!({ "projectId": project, "modelId": model }),
            json!({ "modelId": model }),
        ] {
            let mut body = body;
            body["name"] = json!("Mine");
            body["cron"] = json!("0 9 * * *");
            body["timezone"] = json!("UTC");
            body["prompt"] = json!("Hello.");
            let response = call(
                &app,
                &bob,
                Method::POST,
                "/api/schedules",
                Some(body.clone()),
            )
            .await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{body}");
        }

        let mine = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(mine.body["data"]["name"], "Secret", "unchanged");
        assert_eq!(mine.body["data"]["enabled"], true);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn preview_describes_a_rule_and_its_next_three_runs(pool: DbPool) {
        let app = live_app(pool).await;
        let token = register(&app, "admin").await;
        let preview = call(
            &app,
            &token,
            Method::POST,
            "/api/schedules/preview",
            Some(json!({ "cron": "0 9 * * 1-5", "timezone": "America/New_York" })),
        )
        .await;
        assert_eq!(preview.status, StatusCode::OK, "{}", preview.body);
        let data = &preview.body["data"];
        assert_eq!(data["timezone"], "America/New_York");
        assert!(
            data["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
        let runs: Vec<&str> = data["nextRuns"]
            .as_array()
            .expect("runs")
            .iter()
            .map(|run| run.as_str().expect("a time"))
            .collect();
        assert_eq!(runs.len(), 3);
        assert!(runs.windows(2).all(|pair| pair[0] < pair[1]), "{runs:?}");
        assert!(
            runs.iter()
                .all(|run| run.contains("T13:00:00") || run.contains("T14:00:00")),
            "09:00 in New York is 13:00 or 14:00 UTC: {runs:?}"
        );

        for (body, field) in [
            (
                json!({ "cron": "* * * * *", "timezone": "UTC" }),
                Some("cron"),
            ),
            (
                json!({ "cron": "0 9 * * *", "timezone": "Nowhere" }),
                Some("timezone"),
            ),
            (json!({ "cron": "0 9 * * *" }), None),
        ] {
            let refused = call(
                &app,
                &token,
                Method::POST,
                "/api/schedules/preview",
                Some(body.clone()),
            )
            .await;
            assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
            if let Some(field) = field {
                assert_eq!(refused.body["field"], field, "{body}: {}", refused.body);
            }
        }
    }
}
