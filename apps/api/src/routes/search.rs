//! `/api/search`: chat titles and message text (`ChatService::search`), the caller's only.

use axum::extract::State;
use copilot_contracts::{ErrorResponse, ListResponse, SearchHitResponse, SearchQuery};
use copilot_core::SearchHit;
use copilot_core::shared::constants::DEFAULT_PAGE_SIZE;
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::routes::path::Params;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(search))
}

fn response(hit: &SearchHit) -> SearchHitResponse {
    SearchHitResponse {
        chat_id: hit.chat_id.uuid(),
        chat_title: hit.chat_title.clone(),
        message_id: hit.message_id.map(|id| id.uuid()),
        snippet: hit.snippet.clone(),
    }
}

#[utoipa::path(
    get,
    path = "/search",
    operation_id = "search_search",
    tag = "search",
    security(("bearerAuth" = [])),
    params(SearchQuery),
    responses(
        (status = OK, description = "Matching chat titles first, then messages, best first; every word matches as a prefix", body = ListResponse<SearchHitResponse>),
        (status = BAD_REQUEST, description = "The search text is missing, blank, or longer than 200 characters", body = ErrorResponse)
    )
)]
pub async fn search(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Params(query): Params<SearchQuery>,
) -> Result<axum::Json<ListResponse<SearchHitResponse>>, HttpError> {
    let hits = state
        .chats
        .search(&caller, &query.q, query.limit.unwrap_or(DEFAULT_PAGE_SIZE))
        .await?;
    let items: Vec<SearchHitResponse> = hits.iter().map(response).collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use copilot_database::DbPool;
    use serde_json::{Value, json};

    use crate::helpers::testing::{live_app, other_user, register, request, send};

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn search_finds_only_the_callers_chats(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let created = send(
            app.clone(),
            request(Method::POST, "/api/chats", Some(&alice), Some(json!({}))),
        )
        .await;
        let chat = created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let renamed = send(
            app.clone(),
            request(
                Method::PATCH,
                &format!("/api/chats/{chat}"),
                Some(&alice),
                Some(json!({ "title": "Rust ownership notes" })),
            ),
        )
        .await;
        assert_eq!(renamed.status, StatusCode::OK);

        let found = send(
            app.clone(),
            request(Method::GET, "/api/search?q=rust%20own", Some(&alice), None),
        )
        .await;
        assert_eq!(found.status, StatusCode::OK, "{}", found.body);
        assert_eq!(found.body["total"], 1);
        assert_eq!(
            found.body["items"][0],
            json!({
                "chatId": chat,
                "chatTitle": "Rust ownership notes",
                "messageId": null,
                "snippet": "Rust ownership notes",
            })
        );
        let theirs = send(
            app.clone(),
            request(Method::GET, "/api/search?q=rust", Some(&bob), None),
        )
        .await;
        assert_eq!(theirs.status, StatusCode::OK);
        assert_eq!(
            theirs.body["items"],
            json!([]),
            "never another user's chats"
        );

        let long = format!("/api/search?q={}", "a".repeat(201));
        for (uri, field) in [
            ("/api/search", Value::Null),
            ("/api/search?q=%20%20", json!("q")),
            (long.as_str(), json!("q")),
            ("/api/search?q=rust&limit=lots", Value::Null),
        ] {
            let response = send(app.clone(), request(Method::GET, uri, Some(&alice), None)).await;
            assert_eq!(response.status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(response.body["code"], "VALIDATION_ERROR", "{uri}");
            assert_eq!(response.body["field"], field, "{uri}");
        }
    }
}
