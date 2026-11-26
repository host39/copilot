# Environment Variables

`make setup` copies the samples when the targets are missing:

| App | Sample | Target |
| --- | --- | --- |
| API | `apps/api/.env.sample` | `apps/api/.env.local` |
| Web | `apps/web/.env.example` | `apps/web/.env.local` |

Rust binaries read the process environment only. `scripts/with-env.sh <file> <command>` loads a file first (variables already set in the shell win), which is how `make dev`, `make migrate`, `make seed`, and `make test-db` use `apps/api/.env.local`.

Invalid or missing variables stop the process with a dotted key and every offending name, for example `api.config.env.invalid ENVIRONMENT, PORT`.

## API (`apps/api/.env.local`)

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `ENVIRONMENT` | Yes | none | `local`, `development`, `staging`, or `production` |
| `HOST` | No | `127.0.0.1` | Bind address (`0.0.0.0` in containers) |
| `PORT` | No | `3000` | HTTP port |
| `LOG_LEVEL` | No | `info` | `debug`, `info`, `warn`, or `error` |
| `AUTH_DISABLED` | No | `true` for `local`, else `false` | `true` accepts every request (`NoopTokenVerifier`); `false` uses the configured `TokenVerifier`. Refused in `production` |
| `CORS_ALLOWED_ORIGINS` | No | none | Comma-separated hostnames (`app.example.com`) or suffixes (`.example.com`) browsers may call from. Localhost is always allowed in `local` |
| `DB_HOST` | Yes | none | Postgres host |
| `DB_PORT` | Yes | none | Postgres port |
| `DB_NAME` | Yes | none | Postgres database name |
| `DB_USER` | Yes | none | Postgres username |
| `DB_PASSWORD` | Yes | none | Postgres password (inject from your secret store outside `local`) |
| `DB_SSL` | No | off for `local`, on elsewhere | `true` or `false`; overrides the default |
| `DB_SSL_ROOT_CERT` | No | bundled Mozilla roots | PEM file with the CA that signed the server certificate (for example a cloud provider bundle) |
| `DB_READONLY_USER` | No | none | Must be `readonly` when set; used by `make migrate` to create the role |
| `DB_READONLY_PASSWORD` | No | none | Password for the readonly role; `make migrate` skips the role when both readonly variables are unset |

With SSL on, connections use `verify-full`: the certificate must chain to a trusted root and match `DB_HOST`.

## Migrate and Seed Binaries

Both read the `DB_*` variables above, plus:

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `ENVIRONMENT` | Yes | none | `seed` refuses to run in `production` |
| `LOG_LEVEL` | No | `info` | Log level |
| `SEED_DIR` | No | `packages/database/seed` | Root folder that holds seed folders |
| `SEED_SKIP_IF_POPULATED` | No | none | `1` enables `SEED_SKIP_QUERY` |
| `SEED_SKIP_QUERY` | No | none | SQL returning one boolean; `true` skips seed |

## Authentication

| `AUTH_DISABLED` | Behavior |
| --- | --- |
| `true` (default for `local`) | `NoopTokenVerifier`: every route passes; the caller is `dev-user` |
| `false` | `RejectingTokenVerifier` placeholder: returns `401` until you wire a real provider |

Implement the `TokenVerifier` trait from `copilot-core` and pass it to `create_app` (`AppDependencies::token_verifier`) when enabling auth.

Public routes (no token required): `/`, `/health`, `/health/ready`, `/doc`, `/ui`.

## Web (`apps/web/.env.local`)

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `PUBLIC_API_URL` | No | `http://localhost:3000` | Base URL for API requests from the browser (inlined at build time) |
