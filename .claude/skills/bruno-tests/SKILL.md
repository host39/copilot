---
name: bruno-tests
description: Write Bruno API tests with assert-first checks. Use when adding or editing .bru files or end-to-end API tests under apps/api/tests/bruno.
paths:
  - "apps/api/tests/bruno/**"
---

# Bruno Tests

Collection: `apps/api/tests/bruno`. The `development` environment points `{{baseUrl}}` at the local API on port **3000** (`apps/api/.env.local`).

## Prefer `assert` Over `tests`

Use an `assert` block for status, codes, scalars, `isDefined`, `isArray`, `gte` / `eq`, headers, and nested fields.

Use `tests { }` only when JavaScript is required (`find`, `filter`, `every`, computed values). Keep an `assert` for `res.status` even when a `tests` block is required.

```bru
assert {
  res.status: eq 200
  res.body.status: eq healthy
}
```

## Response Envelope Paths

Follow `/api-response-patterns`:

- List: `res.body.items`, `res.body.total`
- Details: `res.body.data.*` (camelCase fields)
- Error: `res.body.error`, `res.body.code`, `res.body.requestId`, `res.status`

## Request Shape

Match the existing numbered folders (`00 Health`, `01 Errors`, ...):

- Folder: `NN Title` with a `folder.bru` that sets `seq`
- File name and `meta.name`: `NN-MM Short Title`
- `docs` block with `**Endpoint:**` and `**This test verifies:**`
- `{{baseUrl}}` from the environment; never hard-code the host
- Send `X-Request-Id` when asserting on `requestId`

## Reserved `res` Variable

Bruno injects `res` as the main response in `assert`, `script:post-response`, and `tests`. **Never declare `const res`** in scripts that call `bru.sendRequest`; use `listRes`, `createRes`, and so on.

## Shared Scripts

Put helpers under `apps/api/tests/bruno/lib/` and require them from the collection root (`require("./lib/<name>/index.cjs")`). Do not use `../lib/...`; the Bruno CLI blocks paths that escape the collection.

## Running

```bash
make test-bruno
```

`scripts/test-bruno.sh` reuses an API already listening on `{{baseUrl}}`, or builds and starts one with `apps/api/.env.local` and stops it afterwards. Readiness checks need Postgres: if the run cannot reach the database, tell the user to run `make setup` (or `pnpm db:start` once set up) instead of treating it as a test failure.

## Related

- Envelope shapes: `/api-response-patterns`
- API routes: `/add-domain`
