//! OpenAPI document (`/api/doc`) and Swagger UI (`/api/ui`), both public. See `docs/openapi.md`.

use std::sync::Arc;

use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use copilot_contracts::ErrorResponse;
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDocument;

use crate::config::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Copilot API",
        version = "0.0.0",
        description = "HTTP API built from the Copilot starter."
    ),
    tags(
        (name = "health", description = "Liveness and readiness checks"),
        (name = "settings", description = "The caller's personal instructions"),
        (name = "providers", description = "Provider connections and their models"),
        (name = "models", description = "Models for the picker"),
        (name = "projects", description = "Projects: instructions, files, default model, tools"),
        (name = "files", description = "Project files and chat attachments"),
        (name = "chats", description = "Chats, messages, turns, and approvals"),
        (name = "search", description = "Search over chat titles and messages"),
        (name = "tool-servers", description = "MCP tool servers"),
        (name = "schedules", description = "Prompts that run at set times"),
        (name = "events", description = "The caller's live updates")
    ),
    components(schemas(ErrorResponse))
)]
pub struct ApiDoc;

/// Swagger UI loads its assets from a CDN; only the spec is served by the API.
const SWAGGER_UI: &str = r##"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Copilot API</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui.css" />
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui-bundle.js" crossorigin></script>
    <script>
      window.ui = SwaggerUIBundle({ url: "/api/doc", dom_id: "#swagger-ui" });
    </script>
  </body>
</html>
"##;

/// `/api/doc` and `/api/ui`, outside the guard.
pub fn router(openapi: OpenApiDocument) -> Router<AppState> {
    let openapi = Arc::new(openapi);
    Router::new()
        .route(
            "/api/doc",
            get(move || {
                let openapi = Arc::clone(&openapi);
                async move { Json(openapi) }
            }),
        )
        .route("/api/ui", get(|| async { Html(SWAGGER_UI) }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::Value;

    use crate::helpers::testing::{app, get};

    async fn document() -> Value {
        let response = get(app().await, "/api/doc").await;
        assert_eq!(response.status, StatusCode::OK);
        response.body
    }

    #[tokio::test]
    async fn serves_the_openapi_document() {
        let doc = document().await;
        assert_eq!(doc["info"]["title"], "Copilot API");
        assert_eq!(
            doc["info"]["license"]["name"],
            "PolyForm-Noncommercial-1.0.0"
        );
        assert!(doc["paths"]["/health/ready"].is_object());
        assert!(doc["paths"]["/"].is_null(), "the web bundle owns the root");
        assert!(doc["components"]["schemas"]["ErrorResponse"].is_object());
    }

    #[tokio::test]
    async fn documents_rustauth_and_copilot_routes_under_api() {
        let doc = document().await;
        for path in [
            "/api/auth/setup",
            "/api/auth/register",
            "/api/auth/login",
            "/api/admin/users",
            "/api/settings",
            "/api/providers/{id}/refresh",
            "/api/models",
            "/api/projects/{id}/files",
            "/api/files/{id}/content",
            "/api/chats/{id}/messages/{messageId}/decisions",
            "/api/search",
            "/api/tool-servers/{id}/test",
            "/api/schedules/preview",
            "/api/events",
        ] {
            assert!(doc["paths"][path].is_object(), "{path} is documented");
        }
        assert!(doc["components"]["securitySchemes"]["bearerAuth"].is_object());
        assert!(
            doc["components"]["schemas"]["rustauth.SignInResponse"].is_object(),
            "rustauth's schemas join the document"
        );
    }

    #[tokio::test]
    async fn every_copilot_route_requires_the_bearer_token() {
        let doc = document().await;
        let paths = doc["paths"].as_object().expect("paths");
        let mut checked = 0;
        for (path, item) in paths {
            let rustauth = path.starts_with("/api/auth/") || path.starts_with("/api/admin/");
            if !path.starts_with("/api/") || rustauth {
                continue;
            }
            for (method, operation) in item.as_object().expect("path item") {
                assert_eq!(
                    operation["security"][0]["bearerAuth"],
                    Value::Array(Vec::new()),
                    "{method} {path} lists bearerAuth"
                );
                checked += 1;
            }
        }
        // The routes of the spec's HTTP API table; adding one is an interface change.
        assert_eq!(checked, 54, "every Copilot route of the spec");
    }

    #[tokio::test]
    async fn operation_ids_are_unique() {
        let doc = document().await;
        let mut ids: Vec<&str> = doc["paths"]
            .as_object()
            .expect("paths")
            .values()
            .flat_map(|item| item.as_object().expect("path item").values())
            .map(|operation| operation["operationId"].as_str().expect("an operation id"))
            .collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate operation ids");
    }

    #[tokio::test]
    async fn serves_swagger_ui_under_api() {
        let app = app().await;
        assert_eq!(get(app.clone(), "/api/ui").await.status, StatusCode::OK);
        assert_eq!(get(app.clone(), "/doc").await.status, StatusCode::NOT_FOUND);
        assert_eq!(get(app, "/ui").await.status, StatusCode::NOT_FOUND);
    }
}
