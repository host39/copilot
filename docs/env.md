# Environment Variables

`make setup` (through `scripts/env-local.sh`) copies the samples when the targets are missing and fills in a generated `SECRET_KEY` when the API's is blank:

| App | Sample | Target |
| --- | --- | --- |
| API | `apps/api/.env.sample` | `apps/api/.env.local` |
| Web | `apps/web/.env.example` | `apps/web/.env.local` |

Rust binaries read the process environment only. `scripts/with-env.sh <file> <command>` loads a file first (variables already set in the shell win), which is how `make dev`, `make migrate`, `make seed`, and `make test-db` use `apps/api/.env.local`. `make test-db` also sets `DATABASE_URL` to the `postgres` maintenance database on the `DB_*` server, where `#[sqlx::test]` creates one throwaway database per test.

Invalid or missing variables stop the process with a dotted key and every offending name, for example `api.config.env.invalid ENVIRONMENT, PORT`.

## API (`apps/api/.env.local`)

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `ENVIRONMENT` | Yes | none | `local`, `development`, `staging`, or `production` |
| `HOST` | No | `127.0.0.1` | Bind address (`0.0.0.0` in containers) |
| `PORT` | No | `3000` | HTTP port |
| `LOG_LEVEL` | No | `info` | `debug`, `info`, `warn`, or `error` |
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
| `DB_SERVER_PASSWORD` | No | none | Read by `migrate` only: creates or updates the role `copilot_server` with this password and grants it the rows of every table in `public` and `auth` (no DDL, no superuser), for the server to connect as (`DB_USER=copilot_server`). Unset: the role is left alone, and the server uses `DB_USER` as given |
| `SECRET_KEY` | Yes | none | 32 random bytes in standard base64 (`openssl rand -base64 32`; surrounding whitespace ignored). Seals provider keys and tool server headers; changing it makes every stored secret unreadable ("Key must be re-entered") |
| `ALLOW_PRIVATE_NETWORK` | No | `admins` | `none`, `admins`, or `all`: whose connections may reach private and loopback addresses (`admins`: connections owned by an administrator) |
| `MAX_UPLOAD_MB` | No | `25` | Per-file upload limit, 1 to 1000 (file bytes live in one Postgres value, at most 1 GB) |
| `MAX_STORAGE_MB_PER_USER` | No | `1024` | Total file storage per user, 1 to 1048576 |
| `WEB_DIR` | No | none | Directory of the built web bundle (it must hold `index.html`). The server then serves it for every `GET` outside `/api` and `/health`, with `index.html` for client routes; the image sets `/app/web` |

With SSL on, connections use `verify-full`: the certificate must chain to a trusted root and match `DB_HOST`.

### Identity (lib-rustauth)

lib-rustauth reads these itself (`AuthConfig::parse`); blank values count as unset, and one startup error names every invalid one with the API's.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `AUTH_REGISTRATION_OPEN` | No | `false` (`true` in `.env.sample`) | `true` or `false`: allow sign-ups after the first (administrator) account |
| `AUTH_SETUP_TOKEN` | No | random, logged once at startup (`rustauth.setup.token-issued`) | Token the first registration needs, 16 or more characters without whitespace |
| `AUTH_SESSION_TTL_DAYS` | No | `30` | Sliding session lifetime, 1 to 3650 |
| `AUTH_SESSION_MAX_DAYS` | No | `90` | Absolute session lifetime, at least the TTL |
| `AUTH_TRUSTED_PROXIES` | No | none | Comma-separated IPs or CIDRs whose `X-Forwarded-For` is trusted; list your reverse proxy, or every client shares its address in the per-IP limits |
| `AUTH_LOCKOUT_THRESHOLD` | No | `5` | Wrong passwords in a row before the account locks |
| `AUTH_LOCKOUT_MINUTES` | No | `15` | First lockout length (doubles each time) |
| `AUTH_LOCKOUT_MAX_MINUTES` | No | `1440` | Longest lockout, at least the first |
| `AUTH_IP_FAILURE_LIMIT` | No | `30` | Failed logins per client IP per 15 minutes |
| `AUTH_RATE_LIMIT_PER_MINUTE` | No | `60` | Requests per client IP per minute to the public auth routes |

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

Authentication comes from lib-rustauth and has no bypass setting. Its routes live under `/api/auth/*` and `/api/admin/users*`, and its guard answers `401` on every other `/api` route without a valid `Authorization: Bearer <token>`. Public: `GET /api/auth/setup`, `POST /api/auth/register`, `POST /api/auth/login`, `/api/doc`, `/api/ui`, and `/health`, `/health/ready`, and the web bundle outside `/api`.

The API starts only when lib-rustauth's `auth` migrations are applied (`make migrate` runs them before Copilot's); otherwise it stops with `api.server.start.failed api.auth.start.failed: Auth schema is out of date: run migrations`. While no account exists, the first registration needs the setup token, and that account becomes the administrator.

`make test-bruno` registers a run-unique account in the database it tests (the first one becomes the administrator of an empty database), so register your own account first, or create a throwaway database and run `DB_NAME=<name> make migrate test-bruno`. The server it starts gets `AUTH_REGISTRATION_OPEN=true` and a setup token unless the shell sets them.

## Web (`apps/web/.env.local`)

The web client calls the server that served it (`/api` and `/health` on the same origin); the image and `WEB_DIR` serve both from one origin. In development the dev server (port 4000) proxies `/api` and `/health` to the API.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `API_PROXY_TARGET` | No | `http://localhost:3000` | Where the dev server proxies `/api` and `/health` (for example a worktree's API on another port) |
| `PUBLIC_API_URL` | No | none (same origin) | Another server to call directly, inlined at build time: requests go to `<PUBLIC_API_URL>/api/...` and `<PUBLIC_API_URL>/health`, and that server must list this origin in `CORS_ALLOWED_ORIGINS` |

Native builds ignore both: they call the server URL stored by the Connect screen. The Tauri CLI sets two more for the dev server it starts (`make native-dev`, `make android-dev`); nobody sets them by hand. `TAURI_ENV_PLATFORM` makes the dev server keep port 4000 (the shell's `devUrl`) instead of moving to a free port, and `TAURI_DEV_HOST` (with `tauri android dev --host`) makes it listen on that address, with live reload connecting there, so a phone on the network can load it.
