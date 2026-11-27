//! `/api/tool-servers`: MCP tool servers, their tool modes, tests, and reviews (`ToolService`).
//! Credentials go in and never come out: responses say whether they are stored. Deleting a
//! server removes it from every project and chat (and their "Approve for this chat" rows). A test
//! connects through the MCP adapter (`copilot-mcp`), lists the tools, stores them with the
//! negotiated revision, and reports what changed; a failure is a `502` with the adapter's code.

use axum::extract::State;
use axum::http::StatusCode;
use copilot_contracts::{
    CreateToolServerRequest, Credentials, DetailsResponse, ErrorResponse, ListResponse, ServerTool,
    ToolChanges, ToolServerResponse, ToolTestResponse, UpdateToolServerRequest,
};
use copilot_core::{
    CredentialChange, KeyState, NewToolServer, ToolServer, ToolServerId, ToolServerUpdate,
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
}

/// A server as the API shows it; `credentials` says whether they are stored and open.
pub fn response(server: &ToolServer, credentials: KeyState) -> ToolServerResponse {
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
        credentials: credentials.to_dto(),
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

/// Whether credentials are stored, without opening them (lists and writes).
fn stored(server: &ToolServer) -> KeyState {
    KeyState::stored(server.headers_sealed())
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
    let items: Vec<ToolServerResponse> = servers
        .iter()
        .map(|server| response(server, stored(server)))
        .collect();
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
        auth: None,
        client: None,
        approval: body.approval.unwrap_or_default().to_core(),
    };
    let server = state.tools.create(&caller, input).await?;
    let data = response(&server, stored(&server));
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
    Ok(axum::Json(DetailsResponse {
        data: response(&details.server, details.credentials),
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
        auth: None,
        client: None,
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
        data: response(&server, stored(&server)),
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
            server: response(&tested.server, stored(&tested.server)),
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
        data: response(&server, stored(&server)),
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
            "The tool server requires sign-in. Add a bearer token or headers in its settings."
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
}
