# Container Deployment

[`apps/api/Dockerfile`](../apps/api/Dockerfile) builds a small Debian image with two binaries: `server` (the API) and `migrate` (migrations + readonly grants). The web app is a static bundle served separately.

## Build

From the repository root:

```bash
docker build -f apps/api/Dockerfile -t copilot-api .
```

## Run

Apply migrations first, as a one-off task, then start the API:

```bash
docker run --rm --env-file api.env copilot-api migrate
```

```bash
docker run -d --env-file api.env -p 3000:3000 copilot-api
```

The image sets `HOST=0.0.0.0` and `PORT=3000`. Provide the remaining variables from [env.md](./env.md); in production set `ENVIRONMENT=production`, keep `AUTH_DISABLED=false`, and inject `DB_PASSWORD` from your secret store.

Health checks:

| Probe | Path |
| --- | --- |
| Liveness | `GET /health` |
| Readiness | `GET /health/ready` (`503` while Postgres is unreachable) |

The server stops accepting connections on `SIGTERM`, finishes in-flight requests, then closes the pool.

## Web App

```bash
pnpm install --frozen-lockfile
PUBLIC_API_URL=https://api.example.com pnpm --filter @copilot/web build
```

Serve `apps/web/dist` from any static host or CDN with a fallback to `index.html` for client-side routes, and list the web hostname in the API's `CORS_ALLOWED_ORIGINS`.
