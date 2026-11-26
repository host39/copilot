//! `/api/providers`: provider connections, their models, and refreshes (`ProviderService`,
//! `ModelService`).

use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{
    CreateProviderRequest, DetailsResponse, ErrorResponse, ListResponse, ModelResponse,
    ProviderDetailsResponse, ProviderResponse, ProviderUsage, RefreshResponse,
    UpdateProviderRequest,
};
use copilot_core::{
    KeyChange, KeyState, ModelQuery, NewProvider, Provider, ProviderId, ProviderUpdate,
};
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::convert::{ToCore, ToDto};
use crate::routes::models;
use crate::routes::path::Id;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, update, delete))
        .routes(routes!(refresh))
        .routes(routes!(models))
}

/// A connection as the API shows it. `key` comes from the caller: lists report `set` for any
/// stored key without opening it.
pub fn response(provider: &Provider, key: KeyState) -> ProviderResponse {
    ProviderResponse {
        id: provider.id().uuid(),
        kind: provider.kind().to_dto(),
        preset: provider.preset().to_dto(),
        name: provider.name().to_owned(),
        base_url: provider.base_url().as_str().to_owned(),
        enabled: provider.enabled(),
        key: key.to_dto(),
        created_at: provider.created_at(),
        updated_at: provider.updated_at(),
    }
}

/// `set` when a key is stored, without opening it.
fn stored_key(provider: &Provider) -> KeyState {
    if provider.has_key() {
        KeyState::Set
    } else {
        KeyState::Missing
    }
}

#[utoipa::path(
    get,
    path = "/providers",
    operation_id = "providers_list",
    tag = "providers",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's connections, oldest first", body = ListResponse<ProviderResponse>)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
) -> Result<axum::Json<ListResponse<ProviderResponse>>, HttpError> {
    let providers = state.providers.list(&caller).await?;
    let items: Vec<ProviderResponse> = providers
        .iter()
        .map(|provider| response(provider, stored_key(provider)))
        .collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
}

#[utoipa::path(
    post,
    path = "/providers",
    operation_id = "providers_create",
    tag = "providers",
    security(("bearerAuth" = [])),
    request_body = CreateProviderRequest,
    responses(
        (status = CREATED, description = "The new connection", body = DetailsResponse<ProviderResponse>),
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<CreateProviderRequest>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<ProviderResponse>>), HttpError> {
    let provider = state
        .providers
        .create(
            &caller,
            NewProvider {
                kind: body.kind.to_core(),
                preset: body.preset.to_core(),
                name: body.name,
                base_url: body.base_url,
                api_key: body.api_key,
            },
        )
        .await?;
    let data = response(&provider, stored_key(&provider));
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
}

#[utoipa::path(
    get,
    path = "/providers/{id}",
    operation_id = "providers_get",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = OK, description = "The connection with its usage counts", body = DetailsResponse<ProviderDetailsResponse>),
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ProviderDetailsResponse>>, HttpError> {
    let details = state.providers.get(&caller, ProviderId::new(id)).await?;
    let usage = details.usage;
    Ok(axum::Json(DetailsResponse {
        data: ProviderDetailsResponse {
            provider: response(&details.provider, details.key),
            usage: ProviderUsage {
                chats: usage.chats,
                projects: usage.projects,
                schedules: usage.schedules,
            },
        },
    }))
}

#[utoipa::path(
    patch,
    path = "/providers/{id}",
    operation_id = "providers_update",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    request_body = UpdateProviderRequest,
    responses(
        (status = OK, description = "The saved connection", body = DetailsResponse<ProviderResponse>),
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateProviderRequest>,
) -> Result<axum::Json<DetailsResponse<ProviderResponse>>, HttpError> {
    let changes = ProviderUpdate {
        name: body.name,
        base_url: body.base_url,
        // A blank key clears the stored one (`ProviderService` seals only non-blank keys).
        api_key: body.api_key.map(KeyChange::Set),
        enabled: body.enabled,
    };
    let provider = state
        .providers
        .update(&caller, ProviderId::new(id), changes)
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&provider, stored_key(&provider)),
    }))
}

#[utoipa::path(
    delete,
    path = "/providers/{id}",
    operation_id = "providers_delete",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its models; schedules that ran one are turned off"),
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    state.providers.delete(&caller, ProviderId::new(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/providers/{id}/refresh",
    operation_id = "providers_refresh",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = OK, description = "What the refresh changed", body = DetailsResponse<RefreshResponse>),
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse),
        (status = BAD_GATEWAY, description = "The provider could not list its models", body = ErrorResponse)
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<RefreshResponse>>, HttpError> {
    let report = state
        .providers
        .refresh(&caller, ProviderId::new(id))
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: RefreshResponse {
            added: report.added,
            updated: report.updated,
            unavailable: report.unavailable,
            enabled: report.enabled,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/providers/{id}/models",
    operation_id = "providers_models",
    tag = "providers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The connection's id")),
    responses(
        (status = OK, description = "Every known model of the connection", body = ListResponse<ModelResponse>),
        (status = NOT_FOUND, description = "The caller has no such connection", body = ErrorResponse)
    )
)]
pub async fn models(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<ListResponse<ModelResponse>>, HttpError> {
    let views = state
        .models
        .list(&caller, ModelQuery::provider(ProviderId::new(id)))
        .await?;
    Ok(axum::Json(models::list_response(&views)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{Method, StatusCode};
    use copilot_database::DbPool;
    use serde_json::{Value, json};

    use crate::helpers::testing::{
        TestResponse, live_app, other_user, provider, register, request, send,
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

    /// A connection of `token`'s user with a key; its id.
    async fn connection(app: &Router, token: &str) -> String {
        let created = call(
            app,
            token,
            Method::POST,
            "/api/providers",
            Some(json!({
                "kind": "openai",
                "preset": "custom",
                "name": "Gateway",
                "baseUrl": "https://api.example.com/v1",
                "apiKey": "sk-test",
            })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned()
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn a_connection_is_read_changed_and_deleted_by_its_owner(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let id = connection(&app, &alice).await;
        let uri = format!("/api/providers/{id}");

        let details = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(details.status, StatusCode::OK);
        assert_eq!(details.body["data"]["id"], id.as_str());
        assert_eq!(details.body["data"]["key"], "set", "the key opens");
        assert_eq!(
            details.body["data"]["usage"],
            json!({ "chats": 0, "projects": 0, "schedules": 0 })
        );

        let moved = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({ "baseUrl": "https://api.example.com/v2", "name": "Renamed" })),
        )
        .await;
        assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
        assert_eq!(moved.body["data"]["name"], "Renamed");
        assert_eq!(moved.body["data"]["baseUrl"], "https://api.example.com/v2");
        assert_eq!(moved.body["data"]["key"], "set", "the same origin keeps it");

        let elsewhere = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({ "baseUrl": "https://other.example.com/v1" })),
        )
        .await;
        assert_eq!(elsewhere.status, StatusCode::OK);
        assert_eq!(
            elsewhere.body["data"]["key"], "missing",
            "a new origin clears the key"
        );

        let keyed = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({ "apiKey": "sk-new", "enabled": false })),
        )
        .await;
        assert_eq!(keyed.body["data"]["key"], "set");
        assert_eq!(keyed.body["data"]["enabled"], false);
        let cleared = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({ "apiKey": "" })),
        )
        .await;
        assert_eq!(
            cleared.body["data"]["key"], "missing",
            "a blank key clears it"
        );

        let invalid = call(
            &app,
            &alice,
            Method::PATCH,
            &uri,
            Some(json!({ "baseUrl": "ftp://example.com" })),
        )
        .await;
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid.body["field"], "baseUrl");

        let deleted = call(&app, &alice, Method::DELETE, &uri, None).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        let gone = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND);
        let again = call(&app, &alice, Method::DELETE, &uri, None).await;
        assert_eq!(again.status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn another_user_gets_404_for_every_connection_route(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let id = connection(&app, &alice).await;
        let uri = format!("/api/providers/{id}");

        for (method, path, body) in [
            (Method::GET, uri.clone(), None),
            (Method::PATCH, uri.clone(), Some(json!({ "name": "Mine" }))),
            (Method::DELETE, uri.clone(), None),
            (Method::POST, format!("{uri}/refresh"), None),
            (Method::GET, format!("{uri}/models"), None),
        ] {
            let response = call(&app, &bob, method.clone(), &path, body).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(response.body["code"], "ENTITY_NOT_FOUND", "{method} {path}");
        }
        let list = call(&app, &bob, Method::GET, "/api/providers", None).await;
        assert_eq!(list.body["total"], 0);

        let mine = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(mine.status, StatusCode::OK);
        assert_eq!(mine.body["data"]["name"], "Gateway", "unchanged");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn deleting_a_connection_counts_then_releases_what_used_its_models(pool: DbPool) {
        let server = provider().await;
        let app = live_app(pool.clone()).await;
        let alice = register(&app, "alice").await;
        let created = call(
            &app,
            &alice,
            Method::POST,
            "/api/providers",
            Some(json!({
                "kind": "openai",
                "preset": "custom",
                "name": "Mock",
                "baseUrl": format!("{}/v1", server.uri()),
            })),
        )
        .await;
        let id = created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let uri = format!("/api/providers/{id}");
        let refreshed = call(&app, &alice, Method::POST, &format!("{uri}/refresh"), None).await;
        assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.body);
        let models = call(&app, &alice, Method::GET, &format!("{uri}/models"), None).await;
        let model = models.body["items"][0]["id"]
            .as_str()
            .expect("a model")
            .to_owned();

        let chat = call(
            &app,
            &alice,
            Method::POST,
            "/api/chats",
            Some(json!({ "modelId": model })),
        )
        .await;
        assert_eq!(chat.status, StatusCode::CREATED, "{}", chat.body);
        let chat = chat.body["data"]["id"].as_str().expect("a chat").to_owned();
        let schedule = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO schedules (created_at, updated_at, next_run_at, id, user_id, model_id, \
             enabled, name, cron, timezone, prompt, created_by) \
             SELECT now(), now(), now(), $1, user_id, id, true, 'Daily', '0 9 * * *', 'UTC', \
             'Summarize.', 'user' FROM models WHERE id = $2",
        )
        .bind(schedule)
        .bind(uuid::Uuid::parse_str(&model).expect("a uuid"))
        .execute(&pool)
        .await
        .expect("schedule inserted");

        let bob = other_user(&app, &alice, "bob").await;
        let foreign = call(
            &app,
            &bob,
            Method::PATCH,
            &format!("/api/models/{model}"),
            Some(json!({ "enabled": false })),
        )
        .await;
        assert_eq!(
            foreign.status,
            StatusCode::NOT_FOUND,
            "another user's model"
        );
        let picker = call(&app, &bob, Method::GET, "/api/models", None).await;
        assert_eq!(picker.body["total"], 0);

        let details = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(
            details.body["data"]["usage"],
            json!({ "chats": 1, "projects": 0, "schedules": 1 })
        );
        assert_eq!(details.body["data"]["key"], "missing");

        let deleted = call(&app, &alice, Method::DELETE, &uri, None).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        let (enabled, reason): (bool, Option<String>) =
            sqlx::query_as("SELECT enabled, disabled_reason FROM schedules WHERE id = $1")
                .bind(schedule)
                .fetch_one(&pool)
                .await
                .expect("the schedule stays");
        assert!(!enabled, "a schedule that loses its model is turned off");
        assert_eq!(reason.as_deref(), Some("Model removed"));
        let kept = call(
            &app,
            &alice,
            Method::GET,
            &format!("/api/chats/{chat}"),
            None,
        )
        .await;
        assert_eq!(kept.status, StatusCode::OK, "the chat stays");
        assert_eq!(kept.body["data"]["modelId"], Value::Null);
    }
}
