---
name: api-response-patterns
description: Canonical API success and error response envelopes. Use when designing or changing API routes, contract DTOs, Zod schemas, web clients, or Bruno asserts.
paths:
  - "packages/contracts/**"
  - "apps/api/**"
  - "apps/web/**"
---

# API Response Patterns

The contract crate is the source of truth: Rust DTOs in `packages/contracts/src/` are serialized by the API, documented in OpenAPI, exported to TypeScript, and mirrored by Zod schemas that must satisfy the generated types. Bruno asserts on envelope paths.

**Do not** add `errors: []` to success responses or use snake_case JSON field names on the wire.

## Principles

- Success envelopes are predictable: the domain payload is never at the JSON root (except health endpoints).
- Errors never appear on `2xx` responses. Services fail with `DomainError`; `HttpError` maps it to a status and the error body.
- When absence is valid, return `200` with `{ "data": null }`, not `404`. Use `404` only when the requested id does not exist.

## Success Envelopes

| Type | HTTP | Body | Rust | Zod |
| --- | --- | --- | --- | --- |
| List | 200 | `{ items: T[], total: number }` | `ListResponse<T>` | `listResponseSchema(item)` |
| Details | 200 | `{ data: T }` | `DetailsResponse<T>` | `detailsResponseSchema(data)` |
| Nullable details | 200 | `{ data: T \| null }` | `DetailsResponse<Option<T>>` | `detailsResponseSchema(data.nullable())` |
| Action | 200/201 | `{ data: ActionPayload }` | `DetailsResponse<T>` | `detailsResponseSchema(data)` |
| Delete | 204 | empty body | `StatusCode::NO_CONTENT` | none |
| Health | 200/503 | flat `{ status }` (and `database` for readiness) | `HealthResponse`, `ReadyResponse` | `healthResponseSchema`, `readyResponseSchema` |

Envelopes live in [`packages/contracts/src/envelopes.rs`](../../../packages/contracts/src/envelopes.rs) and [`packages/contracts/ts/index.ts`](../../../packages/contracts/ts/index.ts). Full JSON examples: [examples.md](examples.md).

## Error Envelope

Every failure uses `ErrorResponse` from [`packages/contracts/src/error.rs`](../../../packages/contracts/src/error.rs) (`errorResponseSchema` in TypeScript):

```typescript
{ error: string; code?: string; field?: string; details?: Record<string, JsonValue>; requestId?: string }
```

Status mapping lives in [`packages/middleware/src/error_handler.rs`](../../../packages/middleware/src/error_handler.rs):

| `DomainError` | Status | Body |
| --- | --- | --- |
| `Validation` | 400 | `error`, `code: VALIDATION_ERROR`, `field` |
| `Domain` | 400 | `error`, `code`, `details` |
| `Forbidden` | 403 | `error` |
| `EntityNotFound` | 404 | `error`, `code: ENTITY_NOT_FOUND`, `details.entityName`, `details.entityId` |
| `BusinessRule` | 422 | `error`, `code` (the rule code), `details` |
| `Internal` | 500 | `error: "An unexpected error occurred"`, `code: INTERNAL_ERROR` |

Transport rejections: unknown route `404 NOT_FOUND`, missing token `401 UNAUTHORIZED`, malformed JSON `400 VALIDATION_ERROR`, oversized body `413 PAYLOAD_TOO_LARGE`, wrong content type `415 UNSUPPORTED_MEDIA_TYPE`. Every error body carries `requestId` when the request id middleware ran.

## JSON Field Names

**camelCase** on the wire (`createdAt`, `requestId`): every DTO has `#[serde(rename_all = "camelCase")]`. Map entity fields to DTOs in the API, never in core.

## Paths

- Collections: `GET /api/v1/widgets` (list), `POST /api/v1/widgets` (create)
- Items: `GET`, `PATCH`, `DELETE /api/v1/widgets/{widgetId}`
- Path params use `{camelCaseId}` (axum 0.8 and OpenAPI share this syntax)
- The web app keeps its path constants in `apps/web/src/api/endpoints.ts`

## Layer Wiring

### API Route

```rust
Ok(Json(DetailsResponse { data: to_dto(widget) }))
Ok(Json(ListResponse { items: page.items.into_iter().map(to_dto).collect(), total: page.total }))
Ok(StatusCode::NO_CONTENT)
```

### Web Client

Parse the envelope with the contract schema; return `.data` for details and the full `{ items, total }` for lists.

### Bruno

- List: `res.body.items`, `res.body.total`
- Details: `res.body.data.*`
- Error: `res.body.error`, `res.body.code`, `res.status`

## Related

- Examples: [examples.md](examples.md)
- Bruno style: `/bruno-tests`
- New domains: `/add-domain`
