//! The web bundle (`WEB_DIR`): any `GET` or `HEAD` outside `/api` that no route matched gets the
//! file at that path, or `index.html`, so React Router paths survive a reload. Responses carry
//! the web Content-Security-Policy ("Rendering Untrusted Output" in `docs/design/copilot.md`).
//! A file's Brotli or gzip copy (`.br`, `.gz`, written by `scripts/compress-web.mjs` in the image
//! build) is sent as it is to a client that accepts it, so nothing is compressed at run time.
//! Files under `/static/` have content-hashed names and may be cached for a year; `index.html`
//! is revalidated on every load.

use std::path::Path;

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use copilot_core::DomainError;
use copilot_middleware::{HttpError, not_found};
use tower_http::services::{ServeDir, ServeFile};

/// The policy for every response of the bundle.
pub const CSP: &str = "default-src 'self'; script-src 'self'; img-src 'self' blob: data:; \
media-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; \
frame-ancestors 'none'";

/// The files of the bundle in `dir`, with `index.html` for paths that name no file.
pub fn files(dir: &Path) -> ServeDir<ServeFile> {
    let index = ServeFile::new(dir.join("index.html"))
        .precompressed_br()
        .precompressed_gzip();
    ServeDir::new(dir)
        .precompressed_br()
        .precompressed_gzip()
        .fallback(index)
}

/// `Cache-Control` of the content-hashed files under `/static/`.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The fallback for paths no route matched: the bundle for `GET` and `HEAD` outside `/api`, the
/// JSON `404` otherwise.
pub async fn serve(mut files: ServeDir<ServeFile>, request: Request) -> Response {
    let path = request.uri().path();
    let api = path == "/api" || path.starts_with("/api/");
    let hashed = path.starts_with("/static/");
    if api || !matches!(*request.method(), Method::GET | Method::HEAD) {
        return not_found().await;
    }
    match files.try_call(request).await {
        Ok(response) => {
            let mut response = response.map(Body::new);
            let found = response.status().is_success();
            let headers = response.headers_mut();
            let html = headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/html"));
            headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
            headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
            if html {
                // A new deploy changes the asset names `index.html` points at.
                headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            } else if hashed && found {
                headers.insert(CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
            }
            response
        }
        Err(error) => {
            HttpError::from(DomainError::internal("api.web.read.failed", error)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::helpers::testing::{app, app_with, config, get, request};

    const INDEX: &str = "<!doctype html><title>Copilot</title>";

    /// A bundle with `index.html` and one asset, removed on drop.
    struct Bundle(PathBuf);

    impl Bundle {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("copilot-{name}-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("static/js")).expect("bundle dir");
            std::fs::write(dir.join("index.html"), INDEX).expect("index");
            std::fs::write(dir.join("static/js/app.js"), "console.log(1)").expect("asset");
            Self(dir)
        }

        async fn app(&self) -> Router {
            let mut config = config();
            config.web_dir = Some(self.0.clone());
            let db = copilot_database::create_pool(&config.db);
            app_with(config, db).await
        }
    }

    impl Drop for Bundle {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn text(
        app: Router,
        method: Method,
        uri: &str,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = app.oneshot(request).await.expect("infallible");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    #[tokio::test]
    async fn client_routes_get_index_html_with_the_policy() {
        let bundle = Bundle::new("spa-routes");
        let app = bundle.app().await;
        for uri in [
            "/",
            "/chats/0196f6a4-7c1e-7d2a-9b3c-4d5e6f708192",
            "/settings/providers",
        ] {
            let (status, headers, body) = text(app.clone(), Method::GET, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(body, INDEX, "{uri}");
            assert_eq!(headers[CONTENT_SECURITY_POLICY], CSP);
            assert_eq!(headers[X_CONTENT_TYPE_OPTIONS], "nosniff");
            assert_eq!(headers[CACHE_CONTROL], "no-cache");
        }
        let (status, _, body) = text(app, Method::HEAD, "/projects").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn assets_are_served_as_files() {
        let bundle = Bundle::new("spa-assets");
        let (status, headers, body) =
            text(bundle.app().await, Method::GET, "/static/js/app.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "console.log(1)");
        assert!(
            headers[CONTENT_TYPE]
                .to_str()
                .expect("ascii")
                .contains("javascript")
        );
        assert_eq!(headers[CONTENT_SECURITY_POLICY], CSP);
        assert_eq!(
            headers[CACHE_CONTROL], IMMUTABLE,
            "hashed assets are cached for a year"
        );
        let (status, headers, _) =
            text(bundle.app().await, Method::GET, "/static/js/gone.js").await;
        assert_eq!(status, StatusCode::OK, "the index for a name that is gone");
        assert_eq!(headers[CACHE_CONTROL], "no-cache");
    }

    #[tokio::test]
    async fn precompressed_copies_go_to_clients_that_accept_them() {
        let bundle = Bundle::new("spa-precompressed");
        std::fs::write(bundle.0.join("static/js/app.js.gz"), b"gzip bytes").expect("gzip");
        std::fs::write(bundle.0.join("static/js/app.js.br"), b"brotli bytes").expect("brotli");
        let app = bundle.app().await;
        let fetch = |encoding: Option<&'static str>| {
            let app = app.clone();
            async move {
                let mut request = Request::builder().uri("/static/js/app.js");
                if let Some(encoding) = encoding {
                    request = request.header("accept-encoding", encoding);
                }
                let response = app
                    .oneshot(request.body(Body::empty()).expect("request"))
                    .await
                    .expect("infallible");
                let headers = response.headers().clone();
                let bytes = response
                    .into_body()
                    .collect()
                    .await
                    .expect("body")
                    .to_bytes();
                (headers, bytes)
            }
        };
        let (headers, body) = fetch(Some("gzip, br")).await;
        assert_eq!(headers["content-encoding"], "br");
        assert_eq!(body.as_ref(), b"brotli bytes");
        assert!(
            headers[CONTENT_TYPE]
                .to_str()
                .expect("ascii")
                .contains("javascript"),
            "the type of the original file"
        );
        let (headers, body) = fetch(Some("gzip")).await;
        assert_eq!(headers["content-encoding"], "gzip");
        assert_eq!(body.as_ref(), b"gzip bytes");
        let (headers, body) = fetch(None).await;
        assert!(headers.get("content-encoding").is_none());
        assert_eq!(body.as_ref(), b"console.log(1)");
    }

    #[tokio::test]
    async fn api_paths_and_other_methods_stay_json_errors() {
        let bundle = Bundle::new("spa-api");
        let app = bundle.app().await;
        for uri in ["/api", "/api/unknown"] {
            let response = get(app.clone(), uri).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(response.body["code"], "NOT_FOUND");
        }
        let response =
            crate::helpers::testing::send(app.clone(), request(Method::POST, "/chats", None, None))
                .await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert_eq!(response.body["code"], "NOT_FOUND");
        let health = get(app, "/health").await;
        assert_eq!(health.body["status"], "healthy");
    }

    #[tokio::test]
    async fn without_a_bundle_the_root_is_not_found() {
        let response = get(app().await, "/").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert_eq!(response.body["code"], "NOT_FOUND");
    }
}
