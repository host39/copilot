# OpenAPI Documentation

The API uses [`utoipa`](https://docs.rs/utoipa) and [`utoipa-axum`](https://docs.rs/utoipa-axum) to generate an OpenAPI 3.1 document from route annotations and contract types.

## Viewing the Spec

With the API running locally:

- **OpenAPI JSON**: <http://localhost:3000/api/doc>
- **Swagger UI**: <http://localhost:3000/api/ui> (assets load from the jsDelivr CDN)

Both endpoints are public (no token required). Swagger UI shares the web app's origin, where the browser keeps the session token, so `/api/ui` pins its two CDN files with Subresource Integrity (`integrity="sha384-..."`) and sends a Content-Security-Policy (`UI_CSP` in `apps/api/src/routes/docs.rs`) that runs only those files and its one inline script, by hash. To move to another `swagger-ui-dist` version, change the version in both URLs and in `UI_CSP`, and take the new hashes from the files in the npm package (`openssl dgst -sha384 -binary swagger-ui-bundle.js | openssl base64 -A`); a test checks that the policy names every file and the inline script's hash. The document holds Copilot's routes and lib-rustauth's (`/api/auth/*`, `/api/admin/users*`, with schemas named `rustauth.*` and tags `rustauth.auth` and `rustauth.admin`). Every Copilot route lists the `bearerAuth` security scheme and has an operation id `<module>_<handler>` (`chats_send`); a test checks that ids are unique.

## Adding a Documented Route

1. Define request and response DTOs in `packages/contracts` with `ToSchema` (see [ADR-004](./adr/004-contracts-and-type-sharing.md)).
2. Annotate the handler and register it with `routes!`:

   ```rust
   use axum::Json;
   use axum::extract::{Path, State};
   use copilot_contracts::{DetailsResponse, WidgetDto};
   use copilot_core::{Widget, WidgetId};
   use copilot_middleware::HttpError;
   use utoipa_axum::router::OpenApiRouter;
   use utoipa_axum::routes;
   use uuid::Uuid;

   use crate::config::AppState;
   use crate::helpers::auth::Caller;

   pub fn router() -> OpenApiRouter<AppState> {
       OpenApiRouter::new().routes(routes!(get))
   }

   #[utoipa::path(
       get,
       path = "/widgets/{id}",
       operation_id = "widgets_get",
       tag = "widgets",
       security(("bearerAuth" = [])),
       params(("id" = Uuid, Path, description = "The widget's id")),
       responses(
           (status = OK, body = DetailsResponse<WidgetDto>),
           (status = NOT_FOUND, body = copilot_contracts::ErrorResponse)
       )
   )]
   pub async fn get(
       Caller(caller): Caller,
       State(state): State<AppState>,
       Path(id): Path<Uuid>,
   ) -> Result<Json<DetailsResponse<WidgetDto>>, HttpError> {
       let widget = state.widgets.get(&caller, WidgetId::new(id)).await?;
       Ok(Json(DetailsResponse { data: response(&widget) }))
   }

   /// A widget as the API shows it. Core entities and contract DTOs are both foreign to this
   /// crate, so `From` cannot join them: map in the route module (or with the local `ToDto`
   /// traits of `apps/api/src/wire/convert.rs` for shared enums and values).
   fn response(widget: &Widget) -> WidgetDto {
       WidgetDto {
           id: widget.id().to_string(),
           name: widget.name().to_owned(),
       }
   }
   ```

3. Merge the router into `api()` in `apps/api/src/routes/mod.rs` (`.merge(widgets::router())`), which mounts it under `/api` behind lib-rustauth's guard, and add a tag to `ApiDoc` in `apps/api/src/routes/docs.rs`. Paths in the annotation are relative to `/api`.
4. Restart the API and refresh `/api/ui` to see the new operation.

Routes registered with `routes!` share one handler function per path and method, and `split_for_parts` turns the collected router into the axum `Router` plus the OpenAPI document.

## Health Routes (Reference)

See [`apps/api/src/routes/health.rs`](../apps/api/src/routes/health.rs) for the current example using `HealthResponse` and `ReadyResponse` from `copilot-contracts`.
