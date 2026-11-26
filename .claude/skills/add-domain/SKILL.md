---
name: add-domain
description: Add a new domain end-to-end in this workspace (entity, repository port, service, migration, sqlx repository, contract, API route, web client). Use when adding features, CRUD, or new business domains.
argument-hint: "<DomainName>"
---

# Add Domain

End-to-end workflow for a new business domain: `$ARGUMENTS` (for example `Widget`; ask if empty). Follow the clean/DDD layering in [docs/adr/001-domain-driven-design.md](../../../docs/adr/001-domain-driven-design.md).

## Task Progress

```text
- [ ] 1. Entity in packages/core
- [ ] 2. Repository trait (port) in packages/core
- [ ] 3. Service in packages/core
- [ ] 4. Domain mod.rs barrel + re-export from core lib.rs
- [ ] 5. Migration (scaffold + SQL), apply with make migrate
- [ ] 6. sqlx repository in packages/database
- [ ] 7. Contract DTOs in packages/contracts (Rust + Zod), make contracts
- [ ] 8. API route with OpenAPI, wired in the composition root
- [ ] 9. Web API client + page
- [ ] 10. Colocated tests, Bruno folder; make check
```

## Steps

### 1. Entity

`packages/core/src/domains/<name>/entities.rs` (split into more files only when it grows)

A struct with private fields, a validating constructor (`Widget::new(input) -> DomainResult<Widget>`), a `restore` constructor for rows read back from storage, getters, and `with_*` methods that return an updated copy. No `axum`, `sqlx`, or `serde` derives tied to the wire format.

### 2. Repository Trait (Port)

`packages/core/src/domains/<name>/ports.rs`

```rust
#[async_trait]
pub trait WidgetRepository: Send + Sync {
    async fn get(&self, id: Uuid) -> DomainResult<Option<Widget>>;
    async fn list(&self, page: PaginationOptions) -> DomainResult<PaginatedResult<Widget>>;
    async fn save(&self, widget: &Widget) -> DomainResult<()>;
    async fn delete(&self, id: Uuid) -> DomainResult<()>;
}
```

Method names follow the `naming` rule: the trait already says `Widget`, so `get`, `list`, `save`, `delete`, not `get_widget`. Add `uuid` / `chrono` to core's `Cargo.toml` as workspace dependencies when the entity needs them.

### 3. Service

`packages/core/src/domains/<name>/service.rs`

`pub struct WidgetService { widgets: Arc<dyn WidgetRepository> }` with use-case methods returning `DomainResult<T>`. Fail with `DomainError` variants and **human-readable** messages (`DomainError::not_found("Widget", id.to_string())`, `DomainError::validation("Name is required.").with_field("name")`). Never return HTTP status codes.

### 4. Domain Barrel

- `packages/core/src/domains/<name>/mod.rs` declares the private modules `entities`, `ports`, `service` and re-exports their public types.
- Add `pub mod <name>;` to `packages/core/src/domains/mod.rs` and re-export from `packages/core/src/lib.rs`.

### 5. Migration

Run `make migrations name=create_<name>s`, then write the SQL in the new `.up.sql` / `.down.sql` files. Follow the `postgres-column-alignment` rule and keep `.down.sql` an exact revert of `.up.sql`. Register the table in `packages/database/src/operations/describe/mod.rs` (`SCHEMA_TABLES`) so readonly grants include it.

Apply it with `make migrate` (Postgres must be running). Do not edit a migration that a shared or deployed database has already applied; add a new one instead.

### 6. Repository Implementation

`packages/database/src/repositories/<name>s/sqlx_<name>_repository.rs`

`pub struct SqlxWidgetRepository { pool: DbPool }` implementing the core trait with runtime-checked queries (`sqlx::query_as::<_, WidgetRow>(...)`), so builds never need a live database. Map rows to entities with `Widget::restore`, and driver errors with `DomainError::internal("database.widgets.<operation>.failed", error)`. Re-export from `packages/database/src/repositories/mod.rs`. Use `tokio::try_join!` for independent reads (see `parallel-db-reads`).

### 7. Contract

`packages/contracts/src/<name>.rs`, re-exported from `packages/contracts/src/lib.rs`:

```rust
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct WidgetDto { pub id: String, pub name: String, pub created_at: String }
```

Requests (`CreateWidgetRequest`) and responses get their own types; wrap responses in `ListResponse<T>` / `DetailsResponse<T>` (see `/api-response-patterns`). Then:

- `make contracts` regenerates `packages/contracts/ts/generated/`.
- Add Zod schemas to `packages/contracts/ts/index.ts` with `satisfies z.ZodType<WidgetDto>`, and re-export the generated types.

Core must **not** depend on contracts; map between entity and DTO in the API.

### 8. API Route

`apps/api/src/routes/<name>s.rs` (or a folder with `mod.rs` and `serializer.rs` once it grows)

- Annotate each handler with `#[utoipa::path(...)]` using contract types for `body`, register with `OpenApiRouter::new().routes(routes!(list, create))`, and merge it in `apps/api/src/routes/mod.rs`.
- Read bodies with `copilot_middleware::JsonBody<T>` (not `axum::Json`) so malformed JSON gets the shared `400` body.
- Return `Result<Json<DetailsResponse<WidgetDto>>, HttpError>`; `?` converts `DomainError`. Deletes return `StatusCode::NO_CONTENT`.
- Authenticated callers: authentication comes from lib-rustauth. From roadmap step P1.7 its guard wraps every `/api` route, and handlers take the caller as the `Caller(Actor)` extractor from `apps/api` and pass the `Actor` to core services (see "Identity (lib-rustauth)" in `docs/design/copilot.md`). Core never sees rustauth types.
- Wire the service in the composition root: add a field to `AppState` (`apps/api/src/config/state.rs`) built in `create_app` / `ApiContext::create` from `SqlxWidgetRepository::new(db.clone())`. Handlers never construct repositories.

### 9. Web

- Add the path to `apps/web/src/api/endpoints.ts`.
- `apps/web/src/api/routes/<name>s.ts`: `fetchWidgets()` etc. via `apiFetch` + the contract schema.
- `apps/web/src/routes/<name>s/page.tsx` (default export) with `<PageTitle page="Widgets" />`; add a path constant in `apps/web/src/routes/path.tsx` and an entry in `apps/web/src/routes/index.tsx`.
- Status, empty, and error states: reuse `ErrorView` / `CenteredPage`; `/undraw-illustrations` for new art.

The web app only talks to the API over HTTP and only imports `@copilot/contracts`.

### 10. Tests and Verification

- Core: `#[cfg(test)] mod tests` at the bottom of each entity and service file, using the fakes in `copilot_core::testing` (feature `testing`): add the repository trait's implementation to `MemoryStore` in `packages/core/src/testing/<name>.rs`.
- API: colocated tests with `crate::helpers::testing::{app_with, get, send}` and `tower::ServiceExt::oneshot`.
- Database: repository tests use `#[sqlx::test(migrations = false)]` plus `` #[ignore = "requires Postgres: run `make test-db`"] ``, take a `DbPool`, and start with `copilot_database::operations::migrate::run(&pool).await`, which applies the same migrations as a real install (lib-rustauth's `auth` schema first, from P1.4 on, because Copilot tables reference `auth.users`). `make test-db` points `DATABASE_URL` at the `postgres` maintenance database, so each test gets its own throwaway database and never touches `copilot`. API tests that need a database build the app with `crate::helpers::testing::app_on(pool)`.
- Bruno: a new numbered folder under `apps/api/tests/bruno/` (`/bruno-tests`).
- Run `make check` yourself; run `make test-db` (needs Postgres, not an applied migration) and `make test-bruno` (needs the migration applied with `make migrate`). Say so if you could not run them.

## Reference: Health Flow (No Domain Layer)

| Layer | File |
| --- | --- |
| Contract | `packages/contracts/src/health.rs`, `packages/contracts/ts/index.ts` (`healthResponseSchema`) |
| API route | `apps/api/src/routes/health.rs` (OpenAPI, colocated tests) |
| Web client | `apps/web/src/api/routes/health.ts` (`apiFetch` + schema parse) |
| Bruno | `apps/api/tests/bruno/00 Health/` |

## Related

- Layer boundaries: `/architecture`, [boundaries.config.mjs](../../../boundaries.config.mjs)
- Response envelopes: `/api-response-patterns`
- Error messages and log keys: `/dotted-error-messages`
- Rules: `import-paths`, `rust-imports`, `naming`, `postgres-column-alignment`, `document-titles`
