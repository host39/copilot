//! OpenAPI document (`/doc`) and Swagger UI (`/ui`). See `docs/openapi.md`.

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
    tags((name = "health", description = "Liveness and readiness checks")),
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
      window.ui = SwaggerUIBundle({ url: "/doc", dom_id: "#swagger-ui" });
    </script>
  </body>
</html>
"##;

pub fn router(openapi: OpenApiDocument) -> Router<AppState> {
    let openapi = Arc::new(openapi);
    Router::new()
        .route(
            "/doc",
            get(move || {
                let openapi = Arc::clone(&openapi);
                async move { Json(openapi) }
            }),
        )
        .route("/ui", get(|| async { Html(SWAGGER_UI) }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::helpers::testing::{app, get};

    #[tokio::test]
    async fn serves_the_openapi_document() {
        let response = get(app(), "/doc").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body["info"]["title"], "Copilot API");
        assert_eq!(
            response.body["info"]["license"]["name"],
            "PolyForm-Noncommercial-1.0.0"
        );
        assert!(response.body["paths"]["/health/ready"].is_object());
        assert!(response.body["components"]["schemas"]["ErrorResponse"].is_object());
    }

    #[tokio::test]
    async fn serves_swagger_ui() {
        assert_eq!(get(app(), "/ui").await.status, StatusCode::OK);
    }
}
