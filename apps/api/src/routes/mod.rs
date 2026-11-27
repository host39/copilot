//! Route modules. Each exposes `router()` with its OpenAPI-documented handlers; mount it here.
//!
//! [`router`] puts `/health` at the root and everything else under `/api`: lib-rustauth's routes
//! and Copilot's, which [`Identity::api`] puts behind the guard. A handler that a later step
//! writes (P3.2 the schedule's Run now) answers [`pending`] (`501`) until then. [`convert`] maps
//! core values to their wire forms, [`path::Id`] reads `{id}`, and [`path::MessageIds`] reads
//! `{id}` and `{messageId}`.

pub mod chats;
pub mod convert;
pub mod docs;
pub mod events;
pub mod files;
pub mod health;
pub mod models;
pub mod path;
pub mod projects;
pub mod providers;
pub mod schedules;
pub mod search;
pub mod settings;
pub mod tool_servers;
pub mod web;

use axum::http::StatusCode;
use copilot_contracts::ErrorResponse;
use copilot_middleware::HttpError;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::config::AppState;
use crate::helpers::auth::Identity;
use crate::routes::docs::ApiDoc;

/// Every documented route, collected into one router plus the OpenAPI document.
pub fn router(identity: &Identity) -> OpenApiRouter<AppState> {
    // utoipa fills `info.license` from the workspace `license` in Cargo.toml.
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(health::router())
        .nest("/api", identity.api(api()))
}

/// Copilot's own `/api` routes, one module per route family (see "HTTP API" in
/// `docs/design/copilot.md`). Every one needs a signed-in caller.
pub fn api() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(settings::router())
        .merge(providers::router())
        .merge(models::router())
        .merge(projects::router())
        .merge(files::router())
        .merge(chats::router())
        .merge(search::router())
        .merge(tool_servers::router())
        .merge(schedules::router())
        .merge(events::router())
}

/// The answer of a route whose handler a later roadmap step writes.
pub fn pending() -> HttpError {
    HttpError::Status {
        status: StatusCode::NOT_IMPLEMENTED,
        body: Box::new(ErrorResponse::new(
            "This feature is not available yet.",
            "NOT_IMPLEMENTED",
        )),
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::header::WWW_AUTHENTICATE;
    use axum::http::{HeaderMap, Method, StatusCode};
    use copilot_middleware::REQUEST_ID_HEADER;
    use serde_json::Value;

    use crate::helpers::testing::{TestResponse, app, get, open, register, request, send};

    const ID: &str = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192";

    /// Every Copilot operation in the OpenAPI document, with ids filled in.
    async fn operations(app: &Router) -> Vec<(Method, String)> {
        let doc = get(app.clone(), "/api/doc").await.body;
        let mut operations = Vec::new();
        for (path, item) in doc["paths"].as_object().expect("paths") {
            let rustauth = path.starts_with("/api/auth/") || path.starts_with("/api/admin/");
            if !path.starts_with("/api/") || rustauth {
                continue;
            }
            let uri = path.replace("{id}", ID).replace("{messageId}", ID);
            for method in item.as_object().expect("path item").keys() {
                let method = method.to_uppercase().parse().expect("an HTTP method");
                operations.push((method, uri.clone()));
            }
        }
        assert!(operations.len() > 50, "the document lists the routes");
        operations
    }

    #[tokio::test]
    async fn the_guard_answers_every_copilot_route_without_a_token() {
        let app = app().await;
        for (method, uri) in operations(&app).await {
            let response = send(app.clone(), request(method.clone(), &uri, None, None)).await;
            assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{method} {uri}");
            assert_eq!(response.body["code"], "UNAUTHORIZED", "{method} {uri}");
            assert_eq!(response.headers[WWW_AUTHENTICATE], "Bearer");
        }
    }

    #[tokio::test]
    async fn the_guard_refuses_an_unknown_token() {
        let response = send(
            app().await,
            request(Method::GET, "/api/settings", Some("not-a-session"), None),
        )
        .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }

    /// The operations the walking skeleton (P1.9) and roadmap steps P2.A and P3.1 wrote; every
    /// other one answers `501` until its step.
    const WRITTEN: &[(&str, &str)] = &[
        ("GET", "/api/settings"),
        ("PATCH", "/api/settings"),
        ("GET", "/api/providers"),
        ("POST", "/api/providers"),
        ("GET", "/api/providers/{id}"),
        ("PATCH", "/api/providers/{id}"),
        ("DELETE", "/api/providers/{id}"),
        ("POST", "/api/providers/{id}/refresh"),
        ("GET", "/api/providers/{id}/models"),
        ("GET", "/api/models"),
        ("PATCH", "/api/models/{id}"),
        ("GET", "/api/projects"),
        ("POST", "/api/projects"),
        ("GET", "/api/projects/{id}"),
        ("PATCH", "/api/projects/{id}"),
        ("DELETE", "/api/projects/{id}"),
        ("POST", "/api/projects/{id}/files"),
        ("GET", "/api/projects/{id}/files"),
        ("POST", "/api/chats/{id}/files"),
        ("PATCH", "/api/files/{id}"),
        ("DELETE", "/api/files/{id}"),
        ("POST", "/api/files/{id}/move"),
        ("GET", "/api/files/{id}/content"),
        ("GET", "/api/chats"),
        ("POST", "/api/chats"),
        ("GET", "/api/chats/{id}"),
        ("PATCH", "/api/chats/{id}"),
        ("DELETE", "/api/chats/{id}"),
        ("POST", "/api/chats/{id}/read"),
        ("GET", "/api/chats/{id}/messages"),
        ("POST", "/api/chats/{id}/messages"),
        ("POST", "/api/chats/{id}/regenerate"),
        ("POST", "/api/chats/{id}/retry"),
        ("POST", "/api/chats/{id}/messages/{messageId}/decisions"),
        ("GET", "/api/chats/{id}/stream"),
        ("POST", "/api/chats/{id}/stop"),
        ("POST", "/api/chats/{id}/save-as-project"),
        ("POST", "/api/chats/{id}/save-instructions"),
        ("GET", "/api/search"),
        ("GET", "/api/tool-servers"),
        ("POST", "/api/tool-servers"),
        ("GET", "/api/tool-servers/{id}"),
        ("PATCH", "/api/tool-servers/{id}"),
        ("DELETE", "/api/tool-servers/{id}"),
        ("POST", "/api/tool-servers/{id}/test"),
        ("POST", "/api/tool-servers/{id}/review"),
        ("GET", "/api/schedules"),
        ("POST", "/api/schedules"),
        ("GET", "/api/schedules/{id}"),
        ("PATCH", "/api/schedules/{id}"),
        ("DELETE", "/api/schedules/{id}"),
        ("POST", "/api/schedules/preview"),
        ("GET", "/api/events"),
    ];

    fn written(method: &Method, uri: &str) -> bool {
        let path = uri.replacen(ID, "{id}", 1).replacen(ID, "{messageId}", 1);
        WRITTEN
            .iter()
            .any(|(written, template)| method.as_str() == *written && path == *template)
    }

    #[tokio::test]
    async fn signed_in_callers_reach_the_handlers() {
        let app = app().await;
        let token = register(&app, "alice").await;
        let mut reached = 0;
        for (method, uri) in operations(&app).await {
            // An event stream that opens is written; it is not read to its end.
            let response = match open(
                app.clone(),
                request(method.clone(), &uri, Some(&token), None),
            )
            .await
            {
                Ok(_) => Box::new(TestResponse {
                    status: StatusCode::OK,
                    headers: HeaderMap::new(),
                    body: Value::Null,
                }),
                Err(response) => response,
            };
            assert_ne!(response.status, StatusCode::UNAUTHORIZED, "{method} {uri}");
            if written(&method, &uri) {
                reached += 1;
                assert_ne!(
                    response.status,
                    StatusCode::NOT_IMPLEMENTED,
                    "{method} {uri}"
                );
            } else {
                assert_eq!(
                    response.status,
                    StatusCode::NOT_IMPLEMENTED,
                    "{method} {uri}"
                );
                assert_eq!(response.body["code"], "NOT_IMPLEMENTED");
            }
        }
        assert_eq!(
            reached,
            WRITTEN.len(),
            "every written operation is documented"
        );
    }

    #[tokio::test]
    async fn malformed_ids_are_validation_errors() {
        let app = app().await;
        let token = register(&app, "alice").await;
        let response = send(
            app,
            request(Method::GET, "/api/chats/not-a-uuid", Some(&token), None),
        )
        .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(response.body["code"], "VALIDATION_ERROR");
        assert_eq!(response.body["field"], "id");
    }

    #[tokio::test]
    async fn rustauth_routes_are_mounted_under_api() {
        let app = app().await;
        let setup = get(app.clone(), "/api/auth/setup").await;
        assert_eq!(setup.status, StatusCode::OK, "public");
        assert_eq!(setup.body["data"]["needsSetup"], true);

        let token = register(&app, "alice").await;
        let me = send(
            app.clone(),
            request(Method::GET, "/api/auth/me", Some(&token), None),
        )
        .await;
        assert_eq!(me.status, StatusCode::OK);
        assert_eq!(me.body["data"]["username"], "alice");
        assert_eq!(
            me.body["data"]["role"], "administrator",
            "the first account"
        );

        let anonymous = get(app, "/api/admin/users").await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rustauth_errors_echo_the_generated_request_id() {
        let response = get(app().await, "/api/settings").await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        let header = response.headers[REQUEST_ID_HEADER].to_str().expect("ascii");
        assert_eq!(response.body["requestId"], Value::from(header));
    }

    #[tokio::test]
    async fn unknown_api_paths_are_not_found() {
        let response = get(app().await, "/api/does-not-exist").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert_eq!(response.body["code"], "NOT_FOUND");
    }
}
