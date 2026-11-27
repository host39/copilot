# Environment Variables

Every setting is an environment variable. The Rust binaries read the process environment only; nothing reads a file by itself.

| Where | How the variables get there |
| --- | --- |
| Development | `make setup` (through `scripts/env-local.sh`) copies `apps/api/.env.sample` to `apps/api/.env.local` and `apps/web/.env.example` to `apps/web/.env.local` when they are missing, and fills in a generated `SECRET_KEY` when the API's is blank. `scripts/with-env.sh <file> <command>` loads a file first (variables already set in the shell win), which is how `make dev`, `make migrate`, `make seed`, and `make test-db` use `apps/api/.env.local` |
| Compose | [`deploy/compose.yml`](../deploy/compose.yml) sets most of them and reads the rest from the shell or `deploy/.env` (see [self-hosting.md](./self-hosting.md)) |
| Image | The `Dockerfile` sets `HOST=0.0.0.0`, `PORT=3000`, `WEB_DIR=/app/web`, and glibc's `MALLOC_MMAP_THRESHOLD_=1048576` (buffers of 1 MiB and more go back to the system when freed) |

Values are trimmed, and a blank value counts as unset. Invalid or missing variables stop the process with a dotted key and every offending name at once, for example `api.config.env.invalid ENVIRONMENT, PORT, AUTH_SETUP_TOKEN`; the `DB_*` ones are reported as `database.env.invalid DB_HOST, DB_PORT`.

## Who Reads What

| Binary | Variables |
| --- | --- |
| `server` | [Server](#server), [Database](#database), and [Identity](#identity-lib-rustauth) |
| `migrate` | `ENVIRONMENT`, `LOG_LEVEL`, [Database](#database), and the [roles](#migrate-roles) |
| `seed` | `ENVIRONMENT`, `LOG_LEVEL`, [Database](#database), and the [seed settings](#seed) |

## Server

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `ENVIRONMENT` | Yes | none | `local`, `development`, `staging`, or `production`. `local` means readable logs, localhost allowed by CORS, and `DB_SSL` off by default; every other value means one JSON object per log line and `DB_SSL` on by default. `seed` refuses to run in `production` |
| `HOST` | No | `127.0.0.1` | Bind address (`0.0.0.0` in the image) |
| `PORT` | No | `3000` | HTTP port, 1 to 65535 |
| `LOG_LEVEL` | No | `info` | `debug`, `info`, `warn`, or `error`, for every crate except rmcp, whose own events never reach the log (they can carry tool server URLs and server text; `copilot-mcp` logs `mcp.*` events instead) |
| `CORS_ALLOWED_ORIGINS` | No | none | Comma-separated hostnames (`app.example.com`) or suffixes (`.example.com`) browsers may call from. Only needed when a web build on another origin calls this server (`PUBLIC_API_URL`); the image serves the web app from its own origin. Localhost is always allowed in `local` |
| `SECRET_KEY` | Yes | none | 32 random bytes in standard base64 (`openssl rand -base64 32`; surrounding whitespace ignored). Seals provider keys and tool server headers with AES-256-GCM. Keep it with your backups: a different key makes every stored secret unreadable, and the app then asks for each key again ("Key must be re-entered") |
| `ALLOW_PRIVATE_NETWORK` | No | `admins` | `none`, `admins`, or `all`: whose provider connections and tool servers may reach private and loopback addresses (RFC 1918, loopback, unique local `fc00::/7`, CGNAT `100.64.0.0/10`). `admins`: those an administrator owns. Plain `http` URLs are allowed only to such addresses. Link-local and cloud metadata addresses are always blocked. See [Private Network](./self-hosting.md#private-network-and-ollama) |
| `MAX_UPLOAD_MB` | No | `25` | Per-file upload limit, 1 to 1000 (file bytes live in one Postgres value, at most 1 GB) |
| `MAX_STORAGE_MB_PER_USER` | No | `1024` | Total file storage per user, 1 to 1048576 |
| `PUBLIC_URL` | No | none | The origin people reach the server at, for tool server sign-in with OAuth (the callback `<PUBLIC_URL>/api/oauth/callback` and the page it returns to, `<PUBLIC_URL>/oauth/done`): `https://host[:port]`, or `http://` with a loopback host (`localhost`, `127.0.0.1`, `[::1]`) for development, without a path, query, or credentials. Unset: sign-in is off (`SIGN_IN_UNAVAILABLE`), and the sign-in routes answer `404`. See [Tool Server Sign-In](./self-hosting.md#tool-server-sign-in-oauth) |
| `OAUTH_CLIENT_METADATA` | No | `true` | `true` or `false`: whether an `https` `PUBLIC_URL` serves `<PUBLIC_URL>/api/oauth/client` as Copilot's client id to authorization servers that accept Client ID Metadata Documents. `false` when they cannot reach this server (an install on a private network): Copilot then registers dynamically, or uses a client id entered for the server |
| `WEB_DIR` | No | none | Directory of the built web bundle (it must hold `index.html`). The server then serves it for every `GET` outside `/api` and `/health`, with `index.html` for client routes and the `.br` or `.gz` copy when one exists and the browser accepts it; the image sets `/app/web` |

## Database

Read by `server`, `migrate`, and `seed`. Each process opens at most 4 connections (`DB_POOL_MAX`, a constant) and waits up to 30 seconds for one.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `DB_HOST` | Yes | none | Postgres host |
| `DB_PORT` | Yes | none | Postgres port |
| `DB_NAME` | Yes | none | Postgres database name |
| `DB_USER` | Yes | none | Postgres role. The server can connect as `copilot_server`, the role `migrate` creates (see `DB_SERVER_PASSWORD`); `migrate` needs the database owner |
| `DB_PASSWORD` | Yes | none | That role's password (inject it from your secret store outside `local`) |
| `DB_SSL` | No | off for `local`, on elsewhere | `true` or `false`; overrides the default. With SSL on, connections use `verify-full`: the certificate must chain to a trusted root and match `DB_HOST` |
| `DB_SSL_ROOT_CERT` | No | bundled Mozilla roots | PEM file with the CA that signed the server certificate (for example a cloud provider bundle) |

### Migrate Roles

Read by `migrate` only, after it applies the migrations (lib-rustauth's `auth` migrations first, then Copilot's).

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `DB_SERVER_PASSWORD` | No | none | Creates or updates the role `copilot_server` with this password and grants it the rows of every table in `public` and `auth` (no DDL, no superuser), for the server to connect as (`DB_USER=copilot_server`). Unset: the role is left alone, and the server uses `DB_USER` as given |
| `DB_READONLY_USER` | No | none | Must be `readonly` when set: creates the role `readonly` for analytics, which may read only the registered columns (no messages, file contents or names, instructions, secrets, or anything in `auth`) |
| `DB_READONLY_PASSWORD` | No | none | Password for the readonly role. Both readonly variables unset: the role is left alone; one without the other is an error |

### Seed

Read by `seed` only (`make seed`, development data).

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `SEED_DIR` | No | `packages/database/seed` | Root folder that holds seed folders (`local` by default; pass another folder name as the first argument) |
| `SEED_SKIP_IF_POPULATED` | No | none | `1` enables `SEED_SKIP_QUERY` |
| `SEED_SKIP_QUERY` | No | none | SQL returning one boolean; `true` skips the seed |

## Identity (lib-rustauth)

lib-rustauth reads these itself (`AuthConfig::parse`); one startup error names every invalid one together with the server's. Numbers are whole numbers from 1.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `AUTH_REGISTRATION_OPEN` | No | `false` (`true` in `.env.sample`) | `true` or `false` (any letter case): allow sign-ups after the first account. Closed, an administrator adds people in Settings, Users |
| `AUTH_SETUP_TOKEN` | No | generated | The token the first registration needs (that account becomes the administrator): 16 or more characters without whitespace. Unset: the server generates one at each start and logs it (`rustauth.setup.token-issued`) while no account exists |
| `AUTH_SESSION_TTL_DAYS` | No | `30` | Sliding session lifetime: a session unused this long ends. 1 to 3650 |
| `AUTH_SESSION_MAX_DAYS` | No | `90` | Absolute session lifetime, 1 to 3650 and at least `AUTH_SESSION_TTL_DAYS` |
| `AUTH_TRUSTED_PROXIES` | No | none | Comma-separated IPs or CIDRs (`10.0.0.0/8, ::1`) whose `X-Forwarded-For` is trusted for the client address. List what connects to the server on behalf of clients (your reverse proxy, or Docker's port proxy, see [Trusted Proxies](./self-hosting.md#trusted-proxies)); otherwise every client shares one address in the per-IP limits below |
| `AUTH_LOCKOUT_THRESHOLD` | No | `5` | Wrong passwords in a row before the account locks |
| `AUTH_LOCKOUT_MINUTES` | No | `15` | First lockout length, up to 525600; each lockout in a row doubles it |
| `AUTH_LOCKOUT_MAX_MINUTES` | No | `1440` | Longest lockout, up to 525600 and at least `AUTH_LOCKOUT_MINUTES` |
| `AUTH_IP_FAILURE_LIMIT` | No | `30` | Failed sign-ins per client address per 15 minutes |
| `AUTH_RATE_LIMIT_PER_MINUTE` | No | `60` | Requests per client address per minute to the public auth routes |

## Authentication

Authentication comes from lib-rustauth and has no bypass setting. Its routes live under `/api/auth/*` and `/api/admin/users*`, and its guard answers `401` on every other `/api` route without a valid `Authorization: Bearer <token>`. Public: `GET /api/auth/setup`, `POST /api/auth/register`, `POST /api/auth/login`, `/api/doc`, `/api/ui`, `/health`, `/health/ready`, the tool server sign-in's `GET /api/oauth/callback` and `GET /api/oauth/client` (with `PUBLIC_URL`), and the web bundle outside `/api`.

The server starts only when lib-rustauth's `auth` migrations are applied (`migrate` runs them before Copilot's); otherwise it stops with `api.server.start.failed api.auth.start.failed: Auth schema is out of date: run migrations`. While no account exists, the first registration needs the setup token, and that account becomes the administrator.

`make test-bruno` registers a run-unique account in the database it tests (the first one becomes the administrator of an empty database), so register your own account first, or create a throwaway database and run `DB_NAME=<name> make migrate test-bruno`. The server it starts gets `AUTH_REGISTRATION_OPEN=true` and a setup token unless the shell sets them.

## Compose

[`deploy/compose.yml`](../deploy/compose.yml) passes the variables above to the containers and reads these itself:

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `SECRET_KEY`, `DB_PASSWORD`, `DB_SERVER_PASSWORD` | Yes | none | The three secrets: `DB_PASSWORD` is the database owner's (Postgres and `migrate`), `DB_SERVER_PASSWORD` the server role's |
| `COPILOT_IMAGE` | No | `copilot:local` | The image to run instead of building one from this checkout: a release's `ghcr.io/host39/copilot:<version>` |
| `COPILOT_SUBNET`, `COPILOT_GATEWAY` | No | `172.31.239.0/24`, `172.31.239.1` | The compose network; change both (and `AUTH_TRUSTED_PROXIES`) when the subnet overlaps one of the host's networks |
| `AUTH_TRUSTED_PROXIES` | No | `172.31.239.1` | The compose network's gateway, which is where Docker's port proxy connects from |
| `ALLOW_PRIVATE_NETWORK`, `AUTH_REGISTRATION_OPEN`, `AUTH_SETUP_TOKEN`, `LOG_LEVEL` | No | `admins`, `false`, generated, `info` | Passed through |
| `PUBLIC_URL`, `OAUTH_CLIENT_METADATA` | No | unset, `true` | Passed through (tool server sign-in) |

The compose file fixes `ENVIRONMENT=production`, `DB_SSL=false` (Postgres never leaves the compose network), and the database name and roles.

## Web (`apps/web/.env.local`)

The web client calls the server that served it (`/api` and `/health` on the same origin); the image and `WEB_DIR` serve both from one origin. In development the dev server (port 4000) proxies `/api` and `/health` to the API.

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `API_PROXY_TARGET` | No | `http://localhost:3000` | Where the dev server proxies `/api` and `/health` (for example a worktree's API on another port) |
| `PUBLIC_API_URL` | No | none (same origin) | Another server to call directly, inlined at build time: requests go to `<PUBLIC_API_URL>/api/...` and `<PUBLIC_API_URL>/health`, and that server must list this origin in `CORS_ALLOWED_ORIGINS` |

Native builds ignore both: they call the server URL stored by the Connect screen. The Tauri CLI sets two more for the dev server it starts (`make native-dev`, `make android-dev`); nobody sets them by hand. `TAURI_ENV_PLATFORM` makes the dev server keep port 4000 (the shell's `devUrl`) instead of moving to a free port, and `TAURI_DEV_HOST` (with `tauri android dev --host`) makes it listen on that address, with live reload connecting there, so a phone on the network can load it. Release signing for Android reads `ANDROID_KEYSTORE_PATH` and its passwords ([apps/native/README.md](../apps/native/README.md#release-signing)).

## Development Tools

| Variable | Used by | Description |
| --- | --- | --- |
| `DATABASE_URL` | `make test-db` | Set by `scripts/test-db.sh` to the `postgres` maintenance database on the `DB_*` server, where `#[sqlx::test]` creates one throwaway database per test |
| `BASE_URL` | `make test-bruno` | Test a server that is already running instead of starting this checkout's |
| `SMOKE_API_PORT`, `SMOKE_FAKE_PORT` | `make smoke` | Ports of the API and the fake model server the smoke test starts (`3091`, `4091`) |
| `SMOKE_KEEP` | `make smoke` | Any value keeps the throwaway database and prints its name |
| `SMOKE_BIN_DIR` | `make smoke` | A folder with prebuilt `server`, `migrate`, and `fake-llm` binaries (CI); otherwise the debug builds |
| `FAKE_LLM_HOST`, `FAKE_LLM_PORT`, `FAKE_LLM_KEY`, `FAKE_LLM_CHUNK_MS`, `FAKE_LLM_SLOW_MS`, `FAKE_LLM_MCP_TOKEN` | `make fake-llm` | The fake model server's address, key, pacing, and MCP token ([tools/fake-llm/README.md](../tools/fake-llm/README.md)) |
