# Container Deployment

One image holds everything: the `server` binary (the API, which also serves the web app), the `migrate` binary (lib-rustauth's `auth` migrations, then Copilot's, plus the optional readonly grants), and the web bundle at `/app/web`. The [`Dockerfile`](../Dockerfile) at the repository root builds it in three stages: the web bundle with Node and pnpm, the Rust release binaries, and a `debian:bookworm-slim` runtime that runs as an unprivileged user. Copilot runs exactly one server process per install ([ADR-005](./adr/005-copilot-architecture.md)).

## Compose

[`deploy/compose.yml`](../deploy/compose.yml) runs three services:

| Service | Role |
| --- | --- |
| `postgres` | Postgres 18 with one named volume, reachable only on the compose network |
| `migrate` | The image with `command: migrate`: runs once on every `up` and exits; connects as the database owner (`copilot`), applies lib-rustauth's `auth` migrations, then Copilot's, and grants the server's role |
| `copilot` | The server, started only after `migrate` succeeded; connects as `copilot_server` and listens on `127.0.0.1:3000` |

The operator supplies three secrets and keeps them:

```bash
cat >deploy/.env <<EOF
SECRET_KEY=$(openssl rand -base64 32)
DB_PASSWORD=$(openssl rand -hex 24)
DB_SERVER_PASSWORD=$(openssl rand -hex 24)
EOF
docker compose -f deploy/compose.yml up -d
```

Install and upgrade are the same command; migrations never run inside the server ([ADR-002](./adr/002-database-migrations-and-seed.md)). `DB_PASSWORD` belongs to the database owner, which only `migrate` uses. After the migrations, `migrate` creates or updates the role `copilot_server` with `DB_SERVER_PASSWORD`: it may read and write the rows of every table in `public` and `auth`, read but not change the migration histories, and nothing else (no superuser, no `CREATE`, no roles, no `COPY ... TO PROGRAM`). The server connects as that role, so it never holds the owner's password. An install from before this role existed gets it on the next `up` once `DB_SERVER_PASSWORD` is set. The file sets `ENVIRONMENT=production` and `DB_SSL=false` (Postgres never leaves the compose network) and leaves `DB_READONLY_*` unset. A new `SECRET_KEY` makes every stored provider key and tool server credential unreadable ("Key must be re-entered").

Optional settings, read from the shell or `deploy/.env`: `AUTH_SETUP_TOKEN`, `AUTH_REGISTRATION_OPEN` (default `false`), `AUTH_TRUSTED_PROXIES` (default the compose network's gateway, see below), `COPILOT_SUBNET` and `COPILOT_GATEWAY` (default `172.31.239.0/24` and `172.31.239.1`), `ALLOW_PRIVATE_NETWORK` (default `admins`), `LOG_LEVEL`, and `COPILOT_IMAGE` (a prebuilt image instead of building `copilot:local`). [env.md](./env.md) describes every variable.

## First Start

The first registration creates the administrator and needs the setup token: set `AUTH_SETUP_TOKEN` (16 or more characters), or read the one the server logs once while no account exists:

```bash
docker compose -f deploy/compose.yml logs copilot | grep rustauth.setup.token-issued
```

## Reverse Proxy

For any access beyond the machine itself, terminate TLS at a reverse proxy in front of `127.0.0.1:3000` that appends the client's address to `X-Forwarded-For`. The server never sees the proxy's own address: Docker's port proxy connects to it from the compose network's gateway, so that gateway is what `AUTH_TRUSTED_PROXIES` must list, and the compose file pins the network (`COPILOT_SUBNET`, `COPILOT_GATEWAY`) and trusts its gateway by default. Trusting it is safe because the port is published on the host's loopback only, so only processes on this host connect through it. If the subnet overlaps one of the host's networks, choose another and set all three variables to match. With a proxy that runs as a container on the compose network instead, list that container's address. Without a trusted address, every client shares one bucket in the per-IP sign-in limits, so one client's failed sign-ins lock everyone out for 15 minutes. CI checks that the limits key on the forwarded address. Streaming responses send `Cache-Control: no-cache` and `X-Accel-Buffering: no`; with nginx, also turn off buffering for the API:

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
