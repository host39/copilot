//! `/api/capabilities`: what this Copilot server lets the caller do, so clients show only the
//! forms that work. `stdioServers` is true when `MCP_STDIO_ENABLED` is on and the caller is an
//! administrator (`ToolService::stdio_available`).

use axum::extract::State;
use copilot_contracts::{CapabilitiesResponse, DetailsResponse};
use copilot_middleware::HttpError;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get))
}

#[utoipa::path(
    get,
    path = "/capabilities",
    operation_id = "capabilities_get",
    tag = "settings",
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "What the caller may do on this server", body = DetailsResponse<CapabilitiesResponse>)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Caller(caller): Caller,
) -> Result<axum::Json<DetailsResponse<CapabilitiesResponse>>, HttpError> {
    Ok(axum::Json(DetailsResponse {
        data: CapabilitiesResponse {
            stdio_servers: state.tools.stdio_available(&caller),
        },
    }))
}
