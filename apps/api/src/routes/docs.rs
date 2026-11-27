//! OpenAPI document (`/api/doc`) and Swagger UI (`/api/ui`), both public. See `docs/openapi.md`.

use std::sync::Arc;

use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS};
use axum::response::{Html, IntoResponse, Response};
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
        description = "The API of Copilot, a self-hosted chat app: providers, chats, projects, files, tools, and schedules."
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

/// Swagger UI loads its assets from a CDN, pinned by Subresource Integrity (the hashes of
/// `swagger-ui-dist` 5.17.14 as published to npm), so a changed file never runs. Only the spec is
/// served by the API. The page shares the web app's origin, and so its stored session token:
/// [`UI_CSP`] lets it run only those files and its one inline script.
const SWAGGER_UI: &str = r##"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Copilot API</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui.css" integrity="sha384-wxLW6kwyHktdDGr6Pv1zgm/VGJh99lfUbzSn6HNHBENZlCN7W602k9VkGdxuFvPn" crossorigin="anonymous" />
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui-bundle.js" integrity="sha384-wmyclcVGX/WhUkdkATwhaK1X1JtiNrr2EoYJ+diV3vj4v6OC5yCeSu+yW13SYJep" crossorigin="anonymous"></script>
    <script>window.ui = SwaggerUIBundle({ url: "/api/doc", dom_id: "#swagger-ui", validatorUrl: null });</script>
  </body>
</html>
"##;

/// The Content-Security-Policy of `/api/ui`: the two pinned CDN files, the inline script by its
/// SHA-256, requests to this origin only, and no framing. The page turns off Swagger's online
/// validator, so the spec's address never leaves the browser.
pub const UI_CSP: &str = "default-src 'none'; \
script-src https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui-bundle.js \
'sha256-2xuGPRyoCqUCTS8R6qrhRqSIi/e7WDzG1xXLsZ2Mtgs='; \
style-src https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.17.14/swagger-ui.css 'unsafe-inline'; \
img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; \
form-action 'none'; frame-ancestors 'none'";

/// `/api/ui`: the page with its policy, `nosniff`, and no referrer.
async fn ui() -> Response {
    (
        [
            (CONTENT_SECURITY_POLICY, UI_CSP),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (REFERRER_POLICY, "no-referrer"),
        ],
        Html(SWAGGER_UI),
    )
        .into_response()
}

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
        .route("/api/ui", get(ui))
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use http_body_util::BodyExt;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    use super::*;
    use crate::helpers::testing::{app, get, request};

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
            doc["info"]["version"],
            env!("CARGO_PKG_VERSION"),
            "the release version from the workspace"
        );
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
            // The tool server sign-in's callback and client document are public on purpose.
            let public = path.starts_with("/api/oauth/");
            if public {
                for operation in item.as_object().expect("path item").values() {
                    assert!(operation.get("security").is_none(), "{path} is public");
                }
            }
            if !path.starts_with("/api/") || rustauth || public {
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
        assert_eq!(checked, 56, "every Copilot route of the spec");
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
    async fn swagger_ui_runs_only_its_pinned_files_and_inline_script() {
        let response = app()
            .await
            .oneshot(request(Method::GET, "/api/ui", None, None))
            .await
            .expect("a response");
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers().clone();
        assert_eq!(headers[CONTENT_SECURITY_POLICY], UI_CSP);
        assert_eq!(headers[X_CONTENT_TYPE_OPTIONS], "nosniff");
        let page = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .expect("a body")
                .to_bytes()
                .to_vec(),
        )
        .expect("UTF-8");

        // Every external file is pinned, and the policy allows exactly those files.
        let sources: Vec<&str> = page
            .split(['"', ' '])
            .filter(|part| part.starts_with("https://"))
            .collect();
        assert_eq!(sources.len(), 2);
        for source in sources {
            assert!(UI_CSP.contains(source), "{source} is in the policy");
        }
        assert_eq!(page.matches("integrity=\"sha384-").count(), 2);
        assert_eq!(page.matches("crossorigin=\"anonymous\"").count(), 2);

        // The one inline script is allowed by its hash, and nothing else is.
        let inline: Vec<&str> = page
            .split("<script>")
            .skip(1)
            .map(|rest| rest.split("</script>").next().expect("a closed script"))
            .collect();
        assert_eq!(inline.len(), 1);
        let hash = STANDARD.encode(Sha256::digest(inline[0].as_bytes()));
        assert!(UI_CSP.contains(&format!("'sha256-{hash}'")));
        for unsafe_source in ["'unsafe-eval'", "script-src 'self'", "*"] {
            assert!(!UI_CSP.contains(unsafe_source), "{unsafe_source}");
        }
        assert!(UI_CSP.contains("connect-src 'self'"));
        assert!(UI_CSP.contains("frame-ancestors 'none'"));
    }

    #[tokio::test]
    async fn serves_swagger_ui_under_api() {
        let app = app().await;
        assert_eq!(get(app.clone(), "/api/ui").await.status, StatusCode::OK);
        assert_eq!(get(app.clone(), "/doc").await.status, StatusCode::NOT_FOUND);
        assert_eq!(get(app, "/ui").await.status, StatusCode::NOT_FOUND);
    }
}
