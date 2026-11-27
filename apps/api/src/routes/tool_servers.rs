//! `/api/tool-servers`: MCP tool servers, their tool modes, tests, reviews, and OAuth sign-in
//! (`ToolService`). Credentials go in and never come out: responses say whether they are stored
//! and, for OAuth, the sign-in's state; tokens never appear. Deleting a server removes it from
//! every project and chat (and their "Approve for this chat" rows). A test connects through the
//! MCP adapter (`copilot-mcp`), lists the tools, stores them with the negotiated revision, and
//! reports what changed; a failure is a `502` with the adapter's code. Signing in starts here
//! (`/oauth/start` answers the URL to open) and finishes at the public `/api/oauth/callback`
//! (`routes::oauth`).

use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{
    CreateToolServerRequest, Credentials, DetailsResponse, ErrorResponse, ListResponse,
    OAuthClientInput, OAuthStartResponse, OAuthStatus, ServerTool, ToolChanges, ToolServerResponse,
    ToolTestResponse, UpdateToolServerRequest,
};
use copilot_core::{
    ClientChange, ClientInput, CredentialChange, KeyState, NewToolServer, SignInState, ToolServer,
    ToolServerId, ToolServerUpdate,
};
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::path::Id;
use crate::wire::convert::{ToCore, ToDto};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, update, delete))
        .routes(routes!(test))
        .routes(routes!(review))
        .routes(routes!(sign_in))
        .routes(routes!(sign_out))
}

/// Whether a server's secrets are stored (and open), and its sign-in's state.
struct Secrets {
    credentials: KeyState,
    client_secret: KeyState,
    sign_in: Option<SignInState>,
}

/// A server as the API shows it.
fn response(server: &ToolServer, secrets: Secrets) -> ToolServerResponse {
    let cache = server.cache();
    ToolServerResponse {
        id: server.id().uuid(),
        name: server.name().to_owned(),
        slug: server.slug().as_str().to_owned(),
        url: server.url().as_str().to_owned(),
        approval: server.approval().to_dto(),
        tool_modes: server
            .tool_modes()
            .iter()
            .map(|(name, mode)| (name.clone(), mode.to_dto()))
            .collect(),
        enabled: server.enabled(),
        credentials: secrets.credentials.to_dto(),
        auth_mode: server.auth().to_dto(),
        oauth: secrets.sign_in.map(|state| OAuthStatus {
            state: state.to_dto(),
            client_id: server.client().map(|client| client.id.clone()),
            client_secret: secrets.client_secret.to_dto(),
            expires_at: server.sign_in().and_then(|sign_in| sign_in.expires_at),
        }),
        protocol_version: server.protocol_version().map(ToOwned::to_owned),
        tools: cache.map(|cache| {
            cache
                .tools
                .iter()
                .map(|tool| ServerTool {
                    name: tool.name.clone(),
                    title: tool.title.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                    read_only: tool.read_only,
                })
                .collect()
        }),
        tools_fetched_at: cache.map(|cache| cache.fetched_at),
        review_required: server.review_required(),
        created_at: server.created_at(),
        updated_at: server.updated_at(),
    }
}

/// A server with whether its secrets are stored, without opening them (lists and writes).
fn shown(state: &AppState, server: &ToolServer) -> ToolServerResponse {
    let secrets = Secrets {
        credentials: KeyState::stored(server.headers_sealed()),
        client_secret: KeyState::stored(
            server
                .client()
                .and_then(|client| client.secret_sealed.as_deref()),
        ),
        sign_in: state.tools.sign_in_state(server),
    };
    response(server, secrets)
}

fn client(input: OAuthClientInput) -> ClientInput {
    ClientInput {
        id: input.client_id,
        secret: input.client_secret,
    }
}

#[utoipa::path(
    get,
    path = "/tool-servers",
    operation_id = "tool_servers_list",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's tool servers, oldest first", body = ListResponse<ToolServerResponse>)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
) -> Result<axum::Json<ListResponse<ToolServerResponse>>, HttpError> {
    let servers = state.tools.list(&caller).await?;
    let items: Vec<ToolServerResponse> =
        servers.iter().map(|server| shown(&state, server)).collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
}

#[utoipa::path(
    post,
    path = "/tool-servers",
    operation_id = "tool_servers_create",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    request_body = CreateToolServerRequest,
    responses(
        (status = CREATED, description = "The new tool server, without tools until its first test", body = DetailsResponse<ToolServerResponse>),
        (status = BAD_REQUEST, description = "A field is not valid (for example the URL, the slug, or a header)", body = ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<CreateToolServerRequest>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<ToolServerResponse>>), HttpError> {
    let input = NewToolServer {
        name: body.name,
        slug: body.slug,
        url: body.url,
        bearer: body.bearer,
        headers: body.headers,
        auth: body.auth_mode.map(ToCore::to_core),
        client: body.oauth_client.map(client),
        approval: body.approval.unwrap_or_default().to_core(),
    };
    let server = state.tools.create(&caller, input).await?;
    let data = shown(&state, &server);
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
}

#[utoipa::path(
    get,
    path = "/tool-servers/{id}",
    operation_id = "tool_servers_get",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "The tool server with its tool modes; `credentials` is `unreadable` after a `SECRET_KEY` change", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ToolServerResponse>>, HttpError> {
    let details = state.tools.get(&caller, ToolServerId::new(id)).await?;
    let secrets = Secrets {
        credentials: details.credentials,
        client_secret: details.client_secret,
        sign_in: details.sign_in,
    };
    Ok(axum::Json(DetailsResponse {
        data: response(&details.server, secrets),
    }))
}

#[utoipa::path(
    patch,
    path = "/tool-servers/{id}",
    operation_id = "tool_servers_update",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    request_body = UpdateToolServerRequest,
    responses(
        (status = OK, description = "The saved tool server", body = DetailsResponse<ToolServerResponse>),
        (status = BAD_REQUEST, description = "A field is not valid, or a tool mode names a tool the server does not list", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateToolServerRequest>,
) -> Result<axum::Json<DetailsResponse<ToolServerResponse>>, HttpError> {
    let changes = ToolServerUpdate {
        name: body.name,
        url: body.url,
        credentials: body.credentials.map(|credentials| match credentials {
            Some(Credentials { bearer, headers }) => CredentialChange::Set { bearer, headers },
            None => CredentialChange::Clear,
        }),
        auth: body.auth_mode.map(ToCore::to_core),
        client: body.oauth_client.map(|input| match input {
            Some(input) => ClientChange::Set(client(input)),
            None => ClientChange::Clear,
        }),
        approval: body.approval.map(ToCore::to_core),
        tool_modes: body.tool_modes.map(|modes| {
            modes
                .into_iter()
                .map(|(name, mode)| (name, mode.to_core()))
                .collect()
        }),
        enabled: body.enabled,
    };
    let server = state
        .tools
        .update(&caller, ToolServerId::new(id), changes)
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: shown(&state, &server),
    }))
}

#[utoipa::path(
    delete,
    path = "/tool-servers/{id}",
    operation_id = "tool_servers_delete",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted; projects and chats lose it"),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    state.tools.delete(&caller, ToolServerId::new(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/tool-servers/{id}/test",
    operation_id = "tool_servers_test",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "Connected, listed the tools, and refreshed the cache", body = DetailsResponse<ToolTestResponse>),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The stored credentials no longer open (`SECRET_UNREADABLE`), or the URL changed during the test (`TOOL_SERVER_CHANGED`)", body = ErrorResponse),
        (status = BAD_GATEWAY, description = "The server could not be reached or listed", body = ErrorResponse)
    )
)]
pub async fn test(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ToolTestResponse>>, HttpError> {
    let tested = state.tools.test(&caller, ToolServerId::new(id)).await?;
    Ok(axum::Json(DetailsResponse {
        data: ToolTestResponse {
            server: shown(&state, &tested.server),
            changes: ToolChanges {
                added: tested.changes.added,
                removed: tested.changes.removed,
                changed: tested.changes.changed,
            },
        },
    }))
}

#[utoipa::path(
    post,
    path = "/tool-servers/{id}/review",
    operation_id = "tool_servers_review",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "The changed tool list is accepted: tools follow their modes again", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse)
    )
)]
pub async fn review(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ToolServerResponse>>, HttpError> {
    let server = state.tools.review(&caller, ToolServerId::new(id)).await?;
    Ok(axum::Json(DetailsResponse {
        data: shown(&state, &server),
    }))
}

#[utoipa::path(
    post,
    path = "/tool-servers/{id}/oauth/start",
    operation_id = "tool_servers_sign_in",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "The authorization URL to open in the browser; the sign-in comes back to `/api/oauth/callback` within 10 minutes", body = DetailsResponse<OAuthStartResponse>),
        (status = BAD_REQUEST, description = "The server's authentication is not OAuth sign-in", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "Sign-in is off on this server (`SIGN_IN_UNAVAILABLE`, no `PUBLIC_URL`), or the authorization server offers no client id (`CLIENT_ID_REQUIRED`)", body = ErrorResponse),
        (status = BAD_GATEWAY, description = "Discovery or registration failed (`OAUTH_DISCOVERY`, `OAUTH_REGISTRATION`, `OAUTH_FAILED`, `OAUTH_BLOCKED`)", body = ErrorResponse)
    )
)]
pub async fn sign_in(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<OAuthStartResponse>>, HttpError> {
    let started = state.tools.sign_in(&caller, ToolServerId::new(id)).await?;
    Ok(axum::Json(DetailsResponse {
        data: OAuthStartResponse {
            authorization_url: started.url.to_string(),
            expires_at: started.expires_at,
        },
    }))
}

#[utoipa::path(
    delete,
    path = "/tool-servers/{id}/oauth",
    operation_id = "tool_servers_sign_out",
    tag = "tool-servers",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The tool server's id")),
    responses(
        (status = OK, description = "The server without its sign-in (the authorization server keeps its grant until it expires)", body = DetailsResponse<ToolServerResponse>),
        (status = NOT_FOUND, description = "The caller has no such tool server", body = ErrorResponse)
    )
)]
pub async fn sign_out(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<DetailsResponse<ToolServerResponse>>, HttpError> {
    let server = state.tools.sign_out(&caller, ToolServerId::new(id)).await?;
    Ok(axum::Json(DetailsResponse {
        data: shown(&state, &server),
    }))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{Method, StatusCode};
    use copilot_core::PrivateNetwork;
    use copilot_database::DbPool;
    use serde_json::{Value, json};

    use crate::helpers::mcp_server::McpServer;
    use crate::helpers::testing::{
        TestResponse, config, live_app, live_app_with, other_user, register, request, send,
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

    /// Stores a listing of the tools `search` and `delete` for the server, as a test would.
    async fn listed(db: &DbPool, id: &str) {
        let tools = json!({
            "v": 1,
            "ttl_ms": null,
            "tools": [
                {
                    "name": "search",
                    "title": "Search",
                    "description": "Search the docs.",
                    "input_schema": { "type": "object" },
                    "read_only": true,
                },
                {
                    "name": "delete",
                    "title": null,
                    "description": "Delete a page.",
                    "input_schema": { "type": "object" },
                    "read_only": false,
                },
            ],
        });
        sqlx::query(
            "UPDATE tool_servers SET tools_cached = $2, tools_fetched_at = now(), \
             protocol_version = '2025-11-25' WHERE id = $1::uuid",
        )
        .bind(id)
        .bind(tools)
        .execute(db)
        .await
        .expect("listing stored");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn tool_servers_are_created_changed_reviewed_and_deleted(pool: DbPool) {
        let app = live_app(pool.clone()).await;
        let token = register(&app, "admin").await;

        let created = call(
            &app,
            &token,
            Method::POST,
            "/api/tool-servers",
            Some(json!({
                "name": " Docs Search ",
                "url": "https://mcp.example.com/mcp",
                "bearer": "secret-token",
                "headers": { "X-Team": "secret-team" },
            })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let text = created.body.to_string();
        assert!(
            !text.contains("secret"),
            "credentials never come out: {text}"
        );
        let data = &created.body["data"];
        assert_eq!(data["name"], "Docs Search");
        assert_eq!(data["slug"], "docssearch");
        assert_eq!(data["url"], "https://mcp.example.com/mcp");
        assert_eq!(data["approval"], "ask", "new servers ask");
        assert_eq!(data["toolModes"], json!({}));
        assert_eq!(data["enabled"], true);
        assert_eq!(data["credentials"], "set");
        assert_eq!(data["tools"], Value::Null, "no tools before a test");
        assert_eq!(data["toolsFetchedAt"], Value::Null);
        assert_eq!(data["reviewRequired"], false);
        let id = data["id"].as_str().expect("an id").to_owned();
        let uri = format!("/api/tool-servers/{id}");

        for (body, field) in [
            (
                json!({ "name": "Docs", "slug": "docssearch", "url": "https://a.example.com" }),
                Some("slug"),
            ),
            (
                json!({ "name": "Docs", "slug": "Not A Slug", "url": "https://a.example.com" }),
                Some("slug"),
            ),
            (
                json!({ "name": "Docs", "url": "ftp://a.example.com" }),
                Some("url"),
            ),
            (
                json!({
                    "name": "Docs",
                    "url": "https://a.example.com",
                    "bearer": "token",
                    "headers": { "Authorization": "Bearer other" },
                }),
                None,
            ),
        ] {
            let refused = call(&app, &token, Method::POST, "/api/tool-servers", Some(body)).await;
            assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
            if let Some(field) = field {
                assert_eq!(refused.body["field"], field, "{}", refused.body);
            }
        }
        let second = call(
            &app,
            &token,
            Method::POST,
            "/api/tool-servers",
            Some(json!({ "name": "Docs Search", "url": "https://b.example.com", "approval": "auto" })),
        )
        .await;
        assert_eq!(second.body["data"]["slug"], "docssearch2", "a free slug");
        assert_eq!(second.body["data"]["approval"], "auto");
        assert_eq!(second.body["data"]["credentials"], "missing");

        let list = call(&app, &token, Method::GET, "/api/tool-servers", None).await;
        assert_eq!(list.body["total"], 2);
        assert_eq!(list.body["items"][0]["id"], id.as_str(), "oldest first");
        let details = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(details.body["data"]["credentials"], "set");

        let unlisted = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "toolModes": { "search": "auto" } })),
        )
        .await;
        assert_eq!(unlisted.status, StatusCode::BAD_REQUEST);
        assert_eq!(unlisted.body["field"], "toolModes");

        listed(&pool, &id).await;
        let changed = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({
                "name": "Docs",
                "approval": "auto",
                "toolModes": { "search": "auto", "delete": "off" },
                "enabled": false,
            })),
        )
        .await;
        assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        let data = &changed.body["data"];
        assert_eq!(data["name"], "Docs");
        assert_eq!(data["approval"], "auto");
        assert_eq!(
            data["toolModes"],
            json!({ "delete": "off", "search": "auto" })
        );
        assert_eq!(data["enabled"], false);
        assert_eq!(data["credentials"], "set", "kept");
        assert_eq!(data["protocolVersion"], "2025-11-25");
        assert_eq!(data["tools"][0]["name"], "search");
        assert_eq!(data["tools"][0]["readOnly"], true);
        assert_eq!(data["tools"][1]["title"], Value::Null);
        assert!(data["toolsFetchedAt"].is_string());

        let moved = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "url": "https://other.invalid/mcp" })),
        )
        .await;
        let data = &moved.body["data"];
        assert_eq!(data["credentials"], "missing", "a new origin drops them");
        assert_eq!(data["reviewRequired"], true, "every tool asks again");
        assert_eq!(data["protocolVersion"], Value::Null);
        let reviewed = call(&app, &token, Method::POST, &format!("{uri}/review"), None).await;
        assert_eq!(reviewed.status, StatusCode::OK);
        assert_eq!(reviewed.body["data"]["reviewRequired"], false);

        let keyed = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "credentials": { "bearer": "new-secret" } })),
        )
        .await;
        assert_eq!(keyed.body["data"]["credentials"], "set");
        let details = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(details.body["data"]["credentials"], "set", "they open");
        let cleared = call(
            &app,
            &token,
            Method::PATCH,
            &uri,
            Some(json!({ "credentials": null })),
        )
        .await;
        assert_eq!(cleared.body["data"]["credentials"], "missing");

        let tested = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(tested.status, StatusCode::BAD_GATEWAY, "{}", tested.body);
        assert_eq!(
            tested.body["code"], "TOOL_SERVER_UNREACHABLE",
            "no such host"
        );

        let deleted = call(&app, &token, Method::DELETE, &uri, None).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        for method in [Method::GET, Method::DELETE] {
            let gone = call(&app, &token, method.clone(), &uri, None).await;
            assert_eq!(gone.status, StatusCode::NOT_FOUND, "{method}");
        }
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn another_user_gets_404_for_tool_servers(pool: DbPool) {
        let app = live_app(pool.clone()).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let created = call(
            &app,
            &alice,
            Method::POST,
            "/api/tool-servers",
            Some(json!({ "name": "Docs", "url": "https://mcp.example.com", "bearer": "token" })),
        )
        .await;
        let id = created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let uri = format!("/api/tool-servers/{id}");
        listed(&pool, &id).await;

        for (method, path, body) in [
            (Method::GET, uri.clone(), None),
            (Method::PATCH, uri.clone(), Some(json!({ "name": "Taken" }))),
            (
                Method::PATCH,
                uri.clone(),
                Some(json!({ "toolModes": { "delete": "auto" } })),
            ),
            (Method::DELETE, uri.clone(), None),
            (Method::POST, format!("{uri}/test"), None),
            (Method::POST, format!("{uri}/review"), None),
        ] {
            let response = call(&app, &bob, method.clone(), &path, body).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(response.body["code"], "ENTITY_NOT_FOUND");
        }
        let list = call(&app, &bob, Method::GET, "/api/tool-servers", None).await;
        assert_eq!(list.body["total"], 0);
        let slug = call(
            &app,
            &bob,
            Method::POST,
            "/api/tool-servers",
            Some(json!({ "name": "Docs", "slug": "docs", "url": "https://mcp.example.com" })),
        )
        .await;
        assert_eq!(slug.status, StatusCode::CREATED, "slugs are per user");

        let mine = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(mine.body["data"]["name"], "Docs", "unchanged");
        assert_eq!(mine.body["data"]["toolModes"], json!({}));
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn deleting_a_server_takes_it_out_of_projects_and_chats(pool: DbPool) {
        let app = live_app(pool).await;
        let token = register(&app, "admin").await;
        let created = call(
            &app,
            &token,
            Method::POST,
            "/api/tool-servers",
            Some(json!({ "name": "Docs", "url": "https://mcp.example.com" })),
        )
        .await;
        let id = created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let project = call(
            &app,
            &token,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": "Research", "toolServerIds": [id] })),
        )
        .await;
        let project = project.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let chat = call(
            &app,
            &token,
            Method::POST,
            "/api/chats",
            Some(json!({ "toolOverrides": [{ "serverId": id, "enabled": true }] })),
        )
        .await;
        let chat = chat.body["data"]["id"].as_str().expect("an id").to_owned();

        let deleted = call(
            &app,
            &token,
            Method::DELETE,
            &format!("/api/tool-servers/{id}"),
            None,
        )
        .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        let project = call(
            &app,
            &token,
            Method::GET,
            &format!("/api/projects/{project}"),
            None,
        )
        .await;
        assert_eq!(project.body["data"]["toolServerIds"], json!([]));
        let chat = call(
            &app,
            &token,
            Method::GET,
            &format!("/api/chats/{chat}"),
            None,
        )
        .await;
        assert_eq!(chat.body["data"]["toolOverrides"], json!([]));
    }

    fn tool(name: &str, description: &str) -> (String, String, Value) {
        (
            name.to_owned(),
            description.to_owned(),
            json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
        )
    }

    fn names(server: &Value) -> Vec<&str> {
        server["tools"]
            .as_array()
            .expect("listed tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("a name"))
            .collect()
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn testing_a_server_lists_its_tools_and_asks_again_after_a_change(pool: DbPool) {
        let app = live_app(pool).await;
        // The first account is the administrator, so it may reach the loopback server.
        let token = register(&app, "admin").await;
        for (legacy, revision) in [(true, "2025-11-25"), (false, "2026-07-28")] {
            let mcp = McpServer::start(
                legacy,
                &[
                    tool("search", "Search the docs."),
                    tool("delete", "Delete a page."),
                ],
                Some("tok-1"),
            )
            .await;
            let created = call(
                &app,
                &token,
                Method::POST,
                "/api/tool-servers",
                Some(json!({ "name": "Docs", "url": mcp.url, "bearer": "tok-1" })),
            )
            .await;
            assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
            let uri = format!(
                "/api/tool-servers/{}",
                created.body["data"]["id"].as_str().expect("an id")
            );

            let tested = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
            assert_eq!(tested.status, StatusCode::OK, "{}", tested.body);
            let server = &tested.body["data"]["server"];
            assert_eq!(names(server), ["search", "delete"]);
            assert_eq!(server["protocolVersion"], revision);
            assert_eq!(server["tools"][0]["description"], "Search the docs.");
            assert!(server["toolsFetchedAt"].is_string());
            assert_eq!(server["reviewRequired"], false, "a first list is no change");
            assert_eq!(
                tested.body["data"]["changes"],
                json!({ "added": [], "removed": [], "changed": [] })
            );
            let moded = call(
                &app,
                &token,
                Method::PATCH,
                &uri,
                Some(json!({ "toolModes": { "search": "auto" } })),
            )
            .await;
            assert_eq!(moded.status, StatusCode::OK, "listed tools take modes");

            mcp.set_tools(&[
                tool("search", "Search the docs and the wiki."),
                tool("create", "Create a page."),
            ]);
            let changed = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
            assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
            assert_eq!(
                changed.body["data"]["changes"],
                json!({ "added": ["create"], "removed": ["delete"], "changed": ["search"] })
            );
            let server = &changed.body["data"]["server"];
            assert_eq!(names(server), ["search", "create"]);
            assert_eq!(server["reviewRequired"], true, "every tool asks again");
            let details = call(&app, &token, Method::GET, &uri, None).await;
            assert_eq!(names(&details.body["data"]), ["search", "create"], "stored");
            assert_eq!(details.body["data"]["reviewRequired"], true);

            let reviewed = call(&app, &token, Method::POST, &format!("{uri}/review"), None).await;
            assert_eq!(reviewed.body["data"]["reviewRequired"], false);
            let again = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
            assert_eq!(
                again.body["data"]["changes"],
                json!({ "added": [], "removed": [], "changed": [] })
            );
            assert_eq!(again.body["data"]["server"]["reviewRequired"], false);
        }
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn testing_reports_missing_credentials_and_refused_addresses(pool: DbPool) {
        let app = live_app(pool.clone()).await;
        let token = register(&app, "admin").await;
        let mcp = McpServer::start(false, &[tool("search", "Search.")], Some("tok-1")).await;
        let created = call(
            &app,
            &token,
            Method::POST,
            "/api/tool-servers",
            Some(json!({ "name": "Docs", "url": mcp.url })),
        )
        .await;
        let uri = format!(
            "/api/tool-servers/{}",
            created.body["data"]["id"].as_str().expect("an id")
        );
        let tested = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(tested.status, StatusCode::BAD_GATEWAY, "{}", tested.body);
        assert_eq!(tested.body["code"], "TOOL_SERVER_AUTH");
        assert_eq!(
            tested.body["error"],
            "The tool server requires sign-in. Add a bearer token or headers, or sign in with OAuth, in its settings."
        );
        let details = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(details.body["data"]["tools"], Value::Null, "nothing stored");

        // The same account on a server where nobody may reach private addresses: the stored
        // loopback URL is refused before any request leaves.
        let mut closed = config();
        closed.private_network = PrivateNetwork::None;
        let closed = live_app_with(pool, closed).await;
        let before = mcp.requests();
        let refused = call(&closed, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(refused.status, StatusCode::BAD_GATEWAY, "{}", refused.body);
        assert_eq!(refused.body["code"], "TOOL_SERVER_BLOCKED");
        assert_eq!(mcp.requests(), before, "nothing reached the server");
    }

    /// A scripted authorization server for the MCP server at `mcp`: its challenge points at
    /// protected resource metadata on `auth`, whose metadata, registration, and token endpoints
    /// answer like a real one. The first code exchange answers `access-1` and `refresh-1`; a
    /// refresh with `refresh-1` answers `access-2` once; any other refresh is refused.
    async fn authorization_server(mcp: &McpServer) -> wiremock::MockServer {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let auth = MockServer::start().await;
        let uri = auth.uri();
        mcp.challenge(&format!(
            r#"Bearer resource_metadata="{uri}/prm", scope="docs:read""#
        ));
        Mock::given(method("GET"))
            .and(path("/prm"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": mcp.url,
                "authorization_servers": [uri],
            })))
            .mount(&auth)
            .await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": uri,
                "authorization_endpoint": format!("{uri}/authorize"),
                "token_endpoint": format!("{uri}/token"),
                "registration_endpoint": format!("{uri}/register"),
                "code_challenge_methods_supported": ["S256"],
                "authorization_response_iss_parameter_supported": true,
            })))
            .mount(&auth)
            .await;
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "client_id": "dyn-client" })),
            )
            .mount(&auth)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code_verifier="))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-1", "token_type": "Bearer", "expires_in": 3600,
                "refresh_token": "refresh-1",
            })))
            .mount(&auth)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=refresh-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-2", "token_type": "Bearer", "expires_in": 3600,
            })))
            .up_to_n_times(1)
            .mount(&auth)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant", "error_description": "revoked by the admin",
            })))
            .mount(&auth)
            .await;
        auth
    }

    /// The query of an authorization URL.
    fn query(url: &str) -> std::collections::BTreeMap<String, String> {
        url::Url::parse(url)
            .expect("an authorization URL")
            .query_pairs()
            .into_owned()
            .collect()
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn signs_in_with_oauth_refreshes_on_401_and_signs_out(pool: DbPool) {
        let mut config = config();
        config.public_url = Some(url::Url::parse("http://localhost:4000").unwrap());
        let app = live_app_with(pool.clone(), config).await;
        // The first account is the administrator, so it may reach the loopback servers.
        let token = register(&app, "admin").await;
        let mcp = McpServer::start(
            false,
            &[tool("search", "Search the docs.")],
            Some("access-1"),
        )
        .await;
        let auth = authorization_server(&mcp).await;

        let created = call(
            &app,
            &token,
            Method::POST,
            "/api/tool-servers",
            Some(json!({ "name": "Docs", "url": mcp.url, "authMode": "oauth" })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        assert_eq!(created.body["data"]["authMode"], "oauth");
        assert_eq!(created.body["data"]["oauth"]["state"], "needs_sign_in");
        let id = created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let uri = format!("/api/tool-servers/{id}");
        let untested = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(
            untested.body["code"], "SIGN_IN_REQUIRED",
            "{}",
            untested.body
        );

        let started = call(
            &app,
            &token,
            Method::POST,
            &format!("{uri}/oauth/start"),
            None,
        )
        .await;
        assert_eq!(started.status, StatusCode::OK, "{}", started.body);
        let url = started.body["data"]["authorizationUrl"]
            .as_str()
            .expect("a URL");
        assert!(
            url.starts_with(&format!("{}/authorize?", auth.uri())),
            "{url}"
        );
        let params = query(url);
        assert_eq!(params["client_id"], "dyn-client");
        assert_eq!(
            params["redirect_uri"],
            "http://localhost:4000/api/oauth/callback"
        );
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["resource"], mcp.url);
        assert_eq!(params["scope"], "docs:read");
        let state = params["state"].clone();

        // The authorization server sends the browser back; no session comes with it.
        let issuer = auth.uri();
        let back = |state: &str| {
            let query: String = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("code", "code-1")
                .append_pair("state", state)
                .append_pair("iss", &issuer)
                .finish();
            request(
                Method::GET,
                &format!("/api/oauth/callback?{query}"),
                None,
                None,
            )
        };
        let tampered = send(app.clone(), back(&format!("{state}x"))).await;
        assert_eq!(tampered.status, StatusCode::SEE_OTHER);
        assert_eq!(
            tampered.headers["location"],
            "http://localhost:4000/oauth/done?result=expired"
        );
        let finished = send(app.clone(), back(&state)).await;
        assert_eq!(finished.status, StatusCode::SEE_OTHER);
        assert_eq!(
            finished.headers["location"],
            format!("http://localhost:4000/oauth/done?result=signed-in&server={id}").as_str()
        );
        let replayed = send(app.clone(), back(&state)).await;
        assert_eq!(
            replayed.headers["location"], "http://localhost:4000/oauth/done?result=expired",
            "a state works once"
        );

        let details = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(details.body["data"]["oauth"]["state"], "signed_in");
        assert!(details.body["data"]["oauth"]["expiresAt"].is_string());
        let listed = call(&app, &token, Method::GET, "/api/tool-servers", None).await;
        for body in [&details.body, &listed.body, &created.body] {
            let text = body.to_string();
            for secret in ["access-1", "refresh-1", "code-1", "dyn-client"] {
                assert!(!text.contains(secret), "{secret} in {text}");
            }
        }

        let tested = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(tested.status, StatusCode::OK, "{}", tested.body);
        assert_eq!(names(&tested.body["data"]["server"]), ["search"]);

        // The server stops taking the token: one refresh, and the test goes through.
        mcp.require("access-2");
        let refreshed = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.body);
        assert!(mcp.authorization().contains(&"Bearer access-2".to_owned()));

        // Again: the refresh is refused, so the server needs a new sign-in.
        mcp.require("access-3");
        let rejected = call(&app, &token, Method::POST, &format!("{uri}/test"), None).await;
        assert_eq!(
            rejected.body["code"], "SIGN_IN_REQUIRED",
            "{}",
            rejected.body
        );
        assert!(!rejected.body.to_string().contains("revoked by the admin"));
        let details = call(&app, &token, Method::GET, &uri, None).await;
        assert_eq!(details.body["data"]["oauth"]["state"], "needs_sign_in");

        let out = call(&app, &token, Method::DELETE, &format!("{uri}/oauth"), None).await;
        assert_eq!(out.status, StatusCode::OK);
        assert_eq!(out.body["data"]["oauth"]["state"], "needs_sign_in");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn sign_ins_and_callbacks_stay_with_their_user(pool: DbPool) {
        let mut config = config();
        config.public_url = Some(url::Url::parse("http://localhost:4000").unwrap());
        // Every account may reach the loopback servers, so the second user gets that far too.
        config.private_network = PrivateNetwork::All;
        let app = live_app_with(pool.clone(), config).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let mcp = McpServer::start(false, &[tool("search", "Search.")], Some("access-1")).await;
        let auth = authorization_server(&mcp).await;
        let body = json!({ "name": "Docs", "url": mcp.url, "authMode": "oauth" });
        let hers = call(
            &app,
            &alice,
            Method::POST,
            "/api/tool-servers",
            Some(body.clone()),
        )
        .await;
        let his = call(&app, &bob, Method::POST, "/api/tool-servers", Some(body)).await;
        let hers = format!(
            "/api/tool-servers/{}",
            hers.body["data"]["id"].as_str().unwrap()
        );
        let his_id = his.body["data"]["id"].as_str().unwrap().to_owned();
        let his = format!("/api/tool-servers/{his_id}");

        for (method, path) in [
            (Method::POST, format!("{hers}/oauth/start")),
            (Method::DELETE, format!("{hers}/oauth")),
        ] {
            let other = call(&app, &bob, method.clone(), &path, None).await;
            assert_eq!(other.status, StatusCode::NOT_FOUND, "{method} {path}");
        }

        // Bob's sign-in, completed by whoever opens the link, signs in Bob's server only.
        let started = call(
            &app,
            &bob,
            Method::POST,
            &format!("{his}/oauth/start"),
            None,
        )
        .await;
        let state =
            query(started.body["data"]["authorizationUrl"].as_str().unwrap())["state"].clone();
        let issuer = auth.uri();
        let finished = send(
            app.clone(),
            request(
                Method::GET,
                &format!(
                    "/api/oauth/callback?code=code-1&state={state}&iss={}",
                    url::form_urlencoded::byte_serialize(issuer.as_bytes()).collect::<String>()
                ),
                Some(&alice),
                None,
            ),
        )
        .await;
        assert_eq!(
            finished.headers["location"],
            format!("http://localhost:4000/oauth/done?result=signed-in&server={his_id}").as_str()
        );
        let his_details = call(&app, &bob, Method::GET, &his, None).await;
        assert_eq!(his_details.body["data"]["oauth"]["state"], "signed_in");
        let her_details = call(&app, &alice, Method::GET, &hers, None).await;
        assert_eq!(her_details.body["data"]["oauth"]["state"], "needs_sign_in");

        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tool_servers WHERE oauth_tokens_sealed IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("counted");
        assert_eq!(rows, 1, "one sealed sign-in, Bob's");
        let sealed: Vec<u8> =
            sqlx::query_scalar("SELECT oauth_tokens_sealed FROM tool_servers WHERE id = $1::uuid")
                .bind(&his_id)
                .fetch_one(&pool)
                .await
                .expect("sealed");
        assert!(
            !String::from_utf8_lossy(&sealed).contains("access-1"),
            "tokens are stored sealed"
        );
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn sign_in_is_off_without_a_public_url(pool: DbPool) {
        let app = live_app(pool).await;
        let token = register(&app, "admin").await;
        let created = call(
            &app,
            &token,
            Method::POST,
            "/api/tool-servers",
            Some(json!({
                "name": "Docs",
                "url": "https://mcp.example.com/mcp",
                "authMode": "oauth",
                "oauthClient": { "clientId": "my-client", "clientSecret": "my-secret" },
            })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        assert_eq!(created.body["data"]["oauth"]["clientId"], "my-client");
        assert_eq!(created.body["data"]["oauth"]["clientSecret"], "set");
        assert!(!created.body.to_string().contains("my-secret"));
        let uri = format!(
            "/api/tool-servers/{}/oauth/start",
            created.body["data"]["id"].as_str().unwrap()
        );
        let started = call(&app, &token, Method::POST, &uri, None).await;
        assert_eq!(started.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(started.body["code"], "SIGN_IN_UNAVAILABLE");
    }
}
