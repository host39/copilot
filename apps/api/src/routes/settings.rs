//! `/api/settings`: the caller's personal instructions (`SettingsService`). A user without a row
//! reads empty instructions; the first `PATCH` writes the row.

use axum::extract::State;
use copilot_contracts::{DetailsResponse, ErrorResponse, SettingsResponse, UpdateSettingsRequest};
use copilot_core::UserSettings;
use copilot_middleware::{HttpError, JsonBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get, update))
}

fn response(settings: &UserSettings) -> DetailsResponse<SettingsResponse> {
    DetailsResponse {
        data: SettingsResponse {
            instructions: settings.instructions().to_owned(),
        },
    }
}

#[utoipa::path(
    get,
    path = "/settings",
    operation_id = "settings_get",
    tag = "settings",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The caller's settings", body = DetailsResponse<SettingsResponse>)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
) -> Result<axum::Json<DetailsResponse<SettingsResponse>>, HttpError> {
    let settings = state.settings.get(&caller).await?;
    Ok(axum::Json(response(&settings)))
}

#[utoipa::path(
    patch,
    path = "/settings",
    operation_id = "settings_update",
    tag = "settings",
    security(("bearerAuth" = [])),
    request_body = UpdateSettingsRequest,
    responses(
        (status = OK, description = "The saved settings", body = DetailsResponse<SettingsResponse>),
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    JsonBody(body): JsonBody<UpdateSettingsRequest>,
) -> Result<axum::Json<DetailsResponse<SettingsResponse>>, HttpError> {
    let settings = match body.instructions {
        Some(instructions) => state.settings.update(&caller, &instructions).await?,
        None => state.settings.get(&caller).await?,
    };
    Ok(axum::Json(response(&settings)))
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use copilot_database::DbPool;
    use serde_json::json;

    use crate::helpers::testing::{live_app, other_user, register, request, send};

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn each_user_reads_and_saves_their_own_instructions(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;

        let empty = send(
            app.clone(),
            request(Method::GET, "/api/settings", Some(&alice), None),
        )
        .await;
        assert_eq!(empty.status, StatusCode::OK);
        assert_eq!(empty.body["data"], json!({ "instructions": "" }));

        let saved = send(
            app.clone(),
            request(
                Method::PATCH,
                "/api/settings",
                Some(&alice),
                Some(json!({ "instructions": "  Answer in French.  " })),
            ),
        )
        .await;
        assert_eq!(saved.status, StatusCode::OK);
        assert_eq!(saved.body["data"]["instructions"], "Answer in French.");

        let unchanged = send(
            app.clone(),
            request(
                Method::PATCH,
                "/api/settings",
                Some(&alice),
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(
            unchanged.status,
            StatusCode::OK,
            "an empty patch keeps them"
        );
        assert_eq!(unchanged.body["data"]["instructions"], "Answer in French.");

        let read = send(
            app.clone(),
            request(Method::GET, "/api/settings", Some(&alice), None),
        )
        .await;
        assert_eq!(read.body["data"]["instructions"], "Answer in French.");

        let other = send(
            app.clone(),
            request(Method::GET, "/api/settings", Some(&bob), None),
        )
        .await;
        assert_eq!(other.status, StatusCode::OK);
        assert_eq!(
            other.body["data"]["instructions"], "",
            "another user never reads them"
        );
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn a_body_that_is_not_settings_is_a_validation_error(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let response = send(
            app,
            request(
                Method::PATCH,
                "/api/settings",
                Some(&alice),
                Some(json!({ "instructions": 42 })),
            ),
        )
        .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(response.body["code"], "VALIDATION_ERROR");
    }
}
