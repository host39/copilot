# Container Deployment

One image holds everything: the `server` binary (the API, which also serves the web app), the `migrate` binary (lib-rustauth's `auth` migrations, then Copilot's, plus the optional readonly grants), and the web bundle at `/app/web`. The [`Dockerfile`](../Dockerfile) at the repository root builds it in three stages: the web bundle with Node and pnpm, the Rust release binaries, and a `debian:bookworm-slim` runtime that runs as an unprivileged user. Copilot runs exactly one server process per install ([ADR-005](./adr/005-copilot-architecture.md)).

## Compose

[`deploy/compose.yml`](../deploy/compose.yml) runs three services:

| Service | Role |
| --- | --- |
| `postgres` | Postgres 18 with one named volume, reachable only on the compose network |
| `migrate` | The image with `command: migrate`: runs once on every `up` and exits; applies lib-rustauth's `auth` migrations, then Copilot's |
| `copilot` | The server, started only after `migrate` succeeded; listens on `127.0.0.1:3000` |

The operator supplies two secrets and keeps them:

```bash
cat >deploy/.env <<EOF
SECRET_KEY=$(openssl rand -base64 32)
DB_PASSWORD=$(openssl rand -hex 24)
EOF
docker compose -f deploy/compose.yml up -d
```

Install and upgrade are the same command; migrations never run inside the server ([ADR-002](./adr/002-database-migrations-and-seed.md)). The file sets `ENVIRONMENT=production` and `DB_SSL=false` (Postgres never leaves the compose network) and leaves `DB_READONLY_*` unset. A new `SECRET_KEY` makes every stored provider key and tool server credential unreadable ("Key must be re-entered").

Optional settings, read from the shell or `deploy/.env`: `AUTH_SETUP_TOKEN`, `AUTH_REGISTRATION_OPEN` (default `false`), `AUTH_TRUSTED_PROXIES`, `ALLOW_PRIVATE_NETWORK` (default `admins`), `LOG_LEVEL`, and `COPILOT_IMAGE` (a prebuilt image instead of building `copilot:local`). [env.md](./env.md) describes every variable.

## First Start

The first registration creates the administrator and needs the setup token: set `AUTH_SETUP_TOKEN` (16 or more characters), or read the one the server logs once while no account exists:

```bash
docker compose -f deploy/compose.yml logs copilot | grep rustauth.setup.token-issued
```

## Reverse Proxy

For any access beyond the machine itself, terminate TLS at a reverse proxy in front of `127.0.0.1:3000` and list the proxy's address in `AUTH_TRUSTED_PROXIES`, or every client shares the proxy's address in the per-IP sign-in limits. Streaming responses send `Cache-Control: no-cache` and `X-Accel-Buffering: no`; with nginx, also turn off buffering for the API:

```nginx
location / {
    proxy_pass http://127.0.0.1:3000;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_buffering off;
}
```

## Routes and Probes

| Path | Purpose |
| --- | --- |
| `GET /health` | Liveness |
| `GET /health/ready` | Readiness (`503` while Postgres is unreachable) |
| `/api/*` | The API: lib-rustauth's `/api/auth/*` and `/api/admin/users*`, Copilot's routes, `/api/doc` (OpenAPI) and `/api/ui` (Swagger UI) |
| Any other `GET` | The web app (`WEB_DIR`), with `index.html` for client routes and the web Content-Security-Policy |

The server stops accepting connections on `SIGTERM`, ends open streams, finishes in-flight requests, then closes the pool.

## Backups

File bytes live in Postgres, so one `pg_dump` of the `copilot` database is the whole backup (with `SECRET_KEY`, without which the stored keys cannot be read):

```bash
docker compose -f deploy/compose.yml exec postgres pg_dump -U copilot copilot >copilot.sql
```

## Without Compose

```bash
docker build -t copilot .
docker run --rm --env-file copilot.env copilot migrate
docker run -d --env-file copilot.env -p 127.0.0.1:3000:3000 copilot
```

The image sets `HOST=0.0.0.0`, `PORT=3000`, and `WEB_DIR=/app/web`. CI builds the image and starts the compose stack whenever the image's inputs change (`.github/workflows/image.yml`).
