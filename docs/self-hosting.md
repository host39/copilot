# Self-Hosting

Copilot runs as one server process next to one Postgres database. The image holds everything: the `server` binary (the API, which also serves the web app), the `migrate` binary (lib-rustauth's `auth` migrations, then Copilot's, plus the database roles), and the web bundle at `/app/web`. File bytes live in Postgres too, so the database is the only state besides `SECRET_KEY`. Run exactly one server per database ([ADR-005](./adr/005-copilot-architecture.md)): runs, the scheduler, and stream limits live in that process.

What it needs: Docker with Compose v2, a few hundred MB of disk for the image and Postgres (plus the files people upload), and little memory (the server idles at about 20 MB; see [footprint.md](./footprint.md)). Each release publishes the image as `ghcr.io/host39/copilot:<version>` (amd64), so an install downloads about 20 MB and builds nothing. Building it from source instead compiles the Rust binaries, which takes a few minutes and a couple of GB of memory once.

## What Runs

[`deploy/compose.yml`](../deploy/compose.yml) starts three services:

| Service | Role |
| --- | --- |
| `postgres` | Postgres 18 with one named volume (`postgres`), reachable only on the compose network |
| `migrate` | The image with `command: migrate`: runs on every `up` and exits; connects as the database owner (`copilot`), applies lib-rustauth's `auth` migrations, then Copilot's, and creates or updates the server's role |
| `copilot` | The server, started only after `migrate` succeeded; connects as `copilot_server` and listens on `127.0.0.1:3000` |

The image is built from the [`Dockerfile`](../Dockerfile) at the repository root in three stages: the web bundle with Node and pnpm (plus `.gz` and `.br` copies the server sends to browsers that accept them), the Rust binaries in the `dist` profile (release with fat LTO, one codegen unit, and no symbols), and a distroless `cc-debian12` runtime (glibc, CA roots, and tzdata; no shell or package manager) that runs as its unprivileged `nonroot` user. Every base image is pinned by digest.

## Install

Pick a release from the [releases page](https://github.com/host39/copilot/releases) (the commands use `v0.1.0`) and check out its tag, which holds the matching `deploy/compose.yml`:

```bash
git clone --branch v0.1.0 https://github.com/host39/copilot.git
cd copilot

umask 077
cat >deploy/.env <<EOF
COPILOT_IMAGE=ghcr.io/host39/copilot:0.1.0
SECRET_KEY=$(openssl rand -base64 32)
DB_PASSWORD=$(openssl rand -hex 24)
DB_SERVER_PASSWORD=$(openssl rand -hex 24)
EOF

docker compose -f deploy/compose.yml pull
docker compose -f deploy/compose.yml up -d --no-build
docker compose -f deploy/compose.yml logs copilot | grep rustauth.setup.token-issued
```

`--no-build` makes a mistyped `COPILOT_IMAGE` fail instead of quietly building that name from the checkout. To build from source instead (any commit, or a machine the image does not support), leave `COPILOT_IMAGE` out of `deploy/.env` and run `docker compose -f deploy/compose.yml up -d --build`; the image is then `copilot:local`.

Open <http://127.0.0.1:3000> on the same machine: the setup screen creates the administrator account with the setup token from the log. Then the first-run setup asks for a provider ([Adding Providers](../README.md#adding-providers)).

For access from other machines, put a [reverse proxy with TLS](#reverse-proxy-with-tls) in front; the server publishes its port on the host's loopback only. Browsers allow the microphone only on `https` origins (and `localhost`), so voice input needs the proxy too.

## Configuration

Compose reads `deploy/.env` (and the shell, which wins) for the variables it passes on. The ones you may want to change:

| Variable | Default | When to change it |
| --- | --- | --- |
| `AUTH_SETUP_TOKEN` | generated at each start while no account exists | Set your own (16 or more characters) instead of reading the log |
| `AUTH_REGISTRATION_OPEN` | `false` | `true` lets anyone who reaches the server sign up. Closed, the administrator adds people in Settings, Users |
| `ALLOW_PRIVATE_NETWORK` | `admins` | See [Private Network and Ollama](#private-network-and-ollama) |
| `AUTH_TRUSTED_PROXIES` | the compose gateway | See [Trusted Proxies](#trusted-proxies) |
| `COPILOT_SUBNET`, `COPILOT_GATEWAY` | `172.31.239.0/24`, `172.31.239.1` | When the subnet overlaps one of the host's networks |
| `LOG_LEVEL` | `info` | `debug` while chasing a problem |
| `PUBLIC_URL` | unset (sign-in off) | The address people open Copilot at (`https://copilot.example.com`), for [tool server sign-in](#tool-server-sign-in-oauth) |
| `OAUTH_CLIENT_METADATA` | `true` | `false` when authorization servers cannot reach your server (see [Tool Server Sign-In](#tool-server-sign-in-oauth)) |
| `COPILOT_IMAGE` | `copilot:local` (built from the checkout) | The published image of a release, `ghcr.io/host39/copilot:<version>` |

The compose file fixes `ENVIRONMENT=production` (JSON logs) and `DB_SSL=false` (Postgres never leaves the compose network). Upload limits (`MAX_UPLOAD_MB`, `MAX_STORAGE_MB_PER_USER`) and every other variable are in [env.md](./env.md); to set one the compose file does not pass, add it to the `copilot` service's `environment` in a `deploy/compose.override.yml` and start with `-f deploy/compose.yml -f deploy/compose.override.yml`.

## Secrets

| Secret | Used by | If it is lost or changed |
| --- | --- | --- |
| `SECRET_KEY` | The server: seals provider API keys and tool server headers (AES-256-GCM) | Every stored key becomes unreadable; the app shows "Key must be re-entered" on each provider and tool server, and nothing else is lost. There is no key rotation: keep this value with your backups |
| `DB_PASSWORD` | Postgres and `migrate`: the database owner, which owns the schema | Postgres sets it only when it creates the volume. To change it later, run `ALTER ROLE copilot PASSWORD '...'` in `psql` first, then update `deploy/.env` |
| `DB_SERVER_PASSWORD` | The server's role `copilot_server`: reads and writes rows only (no DDL, no superuser, no roles) | `migrate` sets the role's password on every `up`, so changing it in `deploy/.env` and running `up -d` rotates it |
| `AUTH_SETUP_TOKEN` | The first registration only | Irrelevant once an account exists |

Keep `deploy/.env` readable by its owner only (`umask 077` above), out of version control (`.gitignore` covers `.env`), and in your password manager or secret store. The server never holds the owner's password, and it never logs keys, tokens, or passwords (the one exception is a generated setup token, logged while no account exists so you can create the first one).

## First Start

The first registration creates the administrator and needs the setup token. Either set `AUTH_SETUP_TOKEN`, or read the one the server logs while no account exists (a new one at each start, so use the latest):

```bash
docker compose -f deploy/compose.yml logs copilot | grep rustauth.setup.token-issued
```

The administrator manages people in Settings, Users (add, disable, reset a password, unlock, change the role) and is the only one whose provider connections may reach private addresses by default. Every person has their own provider connections, projects, chats, files, tool servers, and schedules; nobody sees another person's data.

## Reverse Proxy with TLS

Terminate TLS at a reverse proxy in front of `127.0.0.1:3000` that adds the client's address to `X-Forwarded-For`. The web app, the API, and the streams share that one origin, so nothing else needs configuring (no CORS). Answers stream as server-sent events, which must reach the browser unbuffered; uploads need a body limit a little above `MAX_UPLOAD_MB` (25 by default).

### Caddy

[Caddy](https://caddyserver.com/) gets and renews the certificate itself. With Caddy on the same host and DNS for `copilot.example.com` pointing at it, the whole `Caddyfile` is:

```caddyfile
copilot.example.com {
    request_body {
        max_size 26MB
    }
    reverse_proxy 127.0.0.1:3000 {
        flush_interval -1
    }
}
```

Caddy sets `X-Forwarded-For` to the client's address and already flushes `text/event-stream` responses as they arrive; `flush_interval -1` makes that explicit for every response. It streams request bodies to the server unbuffered: Copilot refuses an upload that falls behind its pace (30 seconds of grace, then 16 KiB per second) and drops a connection whose headers take longer than 30 seconds, so a slow client cannot hold an upload slot for long.

### nginx

```nginx
server {
    listen 443 ssl;
    http2 on;
    server_name copilot.example.com;
    ssl_certificate     /etc/letsencrypt/live/copilot.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/copilot.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_read_timeout 1h;
        client_max_body_size 26m;
        proxy_request_buffering on;
        client_header_timeout 30s;
        client_body_timeout 30s;
    }
}
```

Copilot sends `X-Accel-Buffering: no` and `Cache-Control: no-cache` on its streams, and a keep-alive comment every 15 seconds; `proxy_buffering off` covers everything else. nginx refuses bodies over 1 MB by default, hence `client_max_body_size`. Keep request buffering on, so a slow client uploads to nginx and holds no Copilot upload slot meanwhile.

## Trusted Proxies

lib-rustauth limits sign-in attempts per client address (`AUTH_IP_FAILURE_LIMIT`, `AUTH_RATE_LIMIT_PER_MINUTE`). It reads the address from `X-Forwarded-For` only when the connection comes from an address in `AUTH_TRUSTED_PROXIES`; otherwise it uses the connection's own address. Without the right entry, every client shares the proxy's address, so one person's failed sign-ins can lock everyone out for 15 minutes.

- **Proxy on the host (the default).** The server never sees the proxy's address: Docker's port proxy connects to the container from the compose network's gateway, so the gateway is what `AUTH_TRUSTED_PROXIES` lists. The compose file pins the network (`COPILOT_SUBNET`, `COPILOT_GATEWAY`) and trusts its gateway by default. This is safe because the port is published on the host's loopback only, so only processes on this host connect through it. If you change the subnet, set all three variables to match.
- **Proxy in a container on the compose network.** Give it a fixed address (`ipv4_address` in an override file) and list that address instead.
- **Publishing the port on another interface.** Do not, unless you list exactly the proxies that connect through it: a trusted address lets whoever connects from it choose the client address.

CI checks that the limits key on the forwarded address through the compose stack (`.github/workflows/image.yml`).

## Private Network and Ollama

Provider connections and tool servers make outbound requests from the server, so Copilot decides who may point them at private addresses (RFC 1918 ranges, loopback, unique local IPv6, CGNAT) with `ALLOW_PRIVATE_NETWORK`:

| Value | Who may connect to private addresses |
| --- | --- |
| `none` | Nobody: only public `https` endpoints |
| `admins` (default) | Connections and tool servers an administrator owns |
| `all` | Everyone. Only when you trust every account: any of them can then make the server send requests into your network |

Plain `http` is allowed only to private addresses (a name that resolves to a public address fails instead of sending a key in the clear). Link-local and cloud metadata addresses (`169.254.0.0/16`, `fe80::/10`, `100.100.100.200`, and the like) are always blocked, redirects are never followed, and failed requests never echo the response body or the resolved address. Checks run on the resolved address of every connection, so DNS rebinding does not get around them.

An Ollama connection (the `ollama` preset) needs a URL the server container can reach, which is not `localhost` (that is the container itself):

- **Ollama in the same compose project.** Add it with an override file, then use the base URL `http://ollama:11434/v1`:

  ```yaml
  # deploy/compose.override.yml
  services:
    ollama:
      image: ollama/ollama
      volumes:
        - ollama:/root/.ollama
      restart: unless-stopped
  volumes:
    ollama:
  ```

  ```bash
  docker compose -f deploy/compose.yml -f deploy/compose.override.yml up -d
  docker compose -f deploy/compose.yml -f deploy/compose.override.yml exec ollama ollama pull llama3.2
  ```

- **Ollama on another machine of your network.** Use its address, for example `http://192.168.1.20:11434/v1`, and make Ollama listen there (`OLLAMA_HOST=0.0.0.0`) behind your firewall.
- **Ollama on the Docker host.** The host is the compose gateway as seen from the server: `http://172.31.239.1:11434/v1`. Ollama listens on the host's loopback by default, so set `OLLAMA_HOST` to an address the containers can reach and keep it closed to other machines with your firewall.

With the default `admins`, the administrator adds the Ollama connection; other people add their own only with `ALLOW_PRIVATE_NETWORK=all`. Ollama needs no API key. The same rules apply to tool servers on your network (for example an MCP server at `http://mcp.lan:8000/mcp`).

## Local (stdio) MCP Servers

Copilot talks to MCP servers over Streamable HTTP only: it never starts programs itself. Many MCP servers (most of those started with `npx` or `uvx`) speak stdio instead. Run such a server in a container of its own behind a bridge that turns stdio into Streamable HTTP, and register the bridge's URL as a tool server. The server then runs as the bridge's child process in that container, away from Copilot's process, its environment (`SECRET_KEY`, the database password), and its files.

This example runs the reference memory server behind [supergateway](https://github.com/supercorp-ai/supergateway), whose `streamableHttp` output serves the MCP endpoint at `/mcp`:

```yaml
# deploy/compose.override.yml
services:
  mcp-memory:
    # Pin a tag or digest you have tested; `:uvx` adds uv for Python servers (`uvx mcp-server-time`).
    image: supercorp/supergateway:latest
    command:
      - --stdio
      - npx -y @modelcontextprotocol/server-memory
      - --outputTransport
      - streamableHttp
      - --stateful
      - --port
      - "8000"
    environment:
      # Secrets the stdio server needs go here, never into Copilot's environment.
      MEMORY_FILE_PATH: /data/memory.json
    volumes:
      - mcp-memory:/data
    restart: unless-stopped
volumes:
  mcp-memory:
```

```bash
docker compose -f deploy/compose.yml -f deploy/compose.override.yml up -d
```

Then, as the administrator, add a tool server in Settings, Tool servers with the URL `http://mcp-memory:8000/mcp` and no token, and press Test: the server's tools appear with their approval modes like any other tool server's.

- **Reach.** Publish no port for the bridge: the compose network is enough, and the bridge has no authentication of its own. Its address on that network is private, so [`ALLOW_PRIVATE_NETWORK`](#private-network-and-ollama) decides who may register it (the administrator by default).
- **Sessions.** Copilot opens one MCP session per turn and closes it when the turn ends, and `--stateful` keeps the bridge's state per session. Keep anything a server must remember between turns in a volume, as the memory server's file above.
- **Start-up.** `npx -y` and `uvx` download the server when the bridge starts, so the container needs internet access then. For an offline or pinned setup, build a small image that installs the server and runs the bridge.
- **Other bridges.** Any stdio to Streamable HTTP bridge works the same way, for example [mcp-proxy](https://github.com/sparfenyuk/mcp-proxy) (`--host 0.0.0.0 --port 8000 --allowed-host mcp-memory:8000`, followed by the server's command; its log lists the Streamable HTTP endpoint to register). The flags above follow each project's README; check them against the version you pin.

## Tool Server Sign-In (OAuth)

Some remote MCP servers take no static token: they ask you to sign in at their sign-in page (OAuth). Copilot handles that sign-in on the server, so it needs to know the address you reach it at:

1. Set `PUBLIC_URL` in `deploy/.env` to the address of your reverse proxy, without a path: `PUBLIC_URL=https://copilot.example.com`. It must be `https`, or plain `http` on `localhost` for development; anything else stops the server at start (`api.config.env.invalid PUBLIC_URL`). Restart with `docker compose -f deploy/compose.yml up -d`.
2. In Settings, Tool servers, add the server with **Sign in** as its authentication, open it, and press **Sign in**. The sign-in page opens (in the system browser from the Linux and Android apps); after you allow access there, it comes back to `<PUBLIC_URL>/api/oauth/callback`, which stores the tokens sealed under `SECRET_KEY` and shows a "Signed in" page. The app updates by itself.

What travels where:

- The browser goes to the authorization server and back to `PUBLIC_URL`, so `PUBLIC_URL` must be reachable from the device you sign in on. The callback needs no session and works once, for 10 minutes.
- Copilot itself fetches the MCP server's metadata, registers itself, and exchanges and refreshes tokens, through the same outbound rules as the MCP server's URL ([Private Network](#private-network-and-ollama)): a sign-in server on a private address needs an account that may reach one.
- **Client ids.** Copilot uses, in this order: a client id you entered for the server (for sign-in servers that want you to register an app with them first, with its secret when they gave you one); its own client document at `<PUBLIC_URL>/api/oauth/client`, when the sign-in server accepts such documents and `PUBLIC_URL` is `https`; dynamic registration, when the sign-in server offers it. A sign-in server that fetches the client document must reach `PUBLIC_URL`: for an install only your own devices can reach (a home network, a VPN), set `OAUTH_CLIENT_METADATA=false` so Copilot registers instead, or enter a client id.
- Tokens are refreshed before they expire and when the MCP server refuses one. When the sign-in server refuses the refresh, the server shows **Needs sign-in**; sign in again there.
- The proxy needs nothing new: `/api/oauth/callback` and `/oauth/done` are ordinary routes on the same origin.

## Upgrades

Install and upgrade are the same `up`: `migrate` runs first and the server starts only after it succeeded. Migrations go forward only, each in its own transaction, lib-rustauth's (schema `auth`, history `auth._sqlx_migrations`) before Copilot's (history `public._sqlx_migrations`). The order:

First read the "Upgrade Notes" in [CHANGELOG.md](../CHANGELOG.md) (also on each release's page) for every release you skip. Then, with `v0.2.0` as the new version:

```bash
cd copilot
# 1. Back up (see below) and keep the dump until the new version has run for a while.
docker compose -f deploy/compose.yml exec -T postgres pg_dump -U copilot -Fc copilot >copilot-$(date +%F).dump
# 2. Get the new version while the old one keeps running.
git fetch --tags && git checkout v0.2.0
sed -i 's|^COPILOT_IMAGE=.*|COPILOT_IMAGE=ghcr.io/host39/copilot:0.2.0|' deploy/.env
docker compose -f deploy/compose.yml pull
# 3. Stop the old server, so it never runs against the new schema.
docker compose -f deploy/compose.yml stop copilot
# 4. Migrate, then start the new server.
docker compose -f deploy/compose.yml up -d --no-build
docker compose -f deploy/compose.yml logs migrate copilot
```

Built from source, step 2 is `git pull` (or the checkout) followed by `docker compose -f deploy/compose.yml build`, and step 4 is `up -d`.

Stopping lets answers being written finish or save what they have (the compose file gives the server 20 seconds). When a migration fails, `up` reports the error and the server stays stopped: that migration rolls back, the ones before it stay applied, and `docker compose logs migrate` says which one failed (`database.migrate.auth.failed` or `database.migrate.copilot.failed`). The new server also refuses to start while lib-rustauth's migrations are missing (`Auth schema is out of date: run migrations`). `migrate` never reverts anything; to go back to an older version, restore the dump taken before the upgrade and start the older image (or build the older tag). Running `up` again with nothing new to apply is safe: CI starts the stack twice and checks that the second `migrate` changes nothing. CI also upgrades a database written by the previous release (its own smoke test, on its own binaries) with the new `migrate` and checks that the new server signs its administrator in, reads every chat, still opens the stored keys, and answers a new turn (`scripts/check-upgrade.sh`).

## Backups

One `pg_dump` of the `copilot` database is the whole backup, because file bytes live in Postgres. Keep `SECRET_KEY` with it, or the stored provider keys and tool server headers cannot be read after a restore. A dump is consistent without stopping the server.

[`deploy/backup.sh`](../deploy/backup.sh) writes one dump (`copilot-<UTC time>.dump`, readable by its owner only) into `backups/` next to `deploy/`, or into the directory you pass, checks it with `pg_restore --list` before keeping it, and deletes dumps older than `BACKUP_KEEP_DAYS` (14) days, never the newest one:

```bash
deploy/backup.sh                      # or: deploy/backup.sh /srv/backups/copilot
```

The same by hand: `docker compose -f deploy/compose.yml exec -T postgres pg_dump -U copilot -Fc copilot >copilot-$(date +%F).dump`. Run the script every night and copy the directory off the machine (rsync, restic, or your backup tool), for example from root's crontab:

```cron
17 3 * * * cd /srv/copilot && deploy/backup.sh >>backups/backup.log 2>&1
```

or with a systemd timer:

```ini
# /etc/systemd/system/copilot-backup.service
[Unit]
Description=Copilot database backup

[Service]
Type=oneshot
WorkingDirectory=/srv/copilot
ExecStart=/srv/copilot/deploy/backup.sh

# /etc/systemd/system/copilot-backup.timer
[Unit]
Description=Nightly Copilot database backup

[Timer]
OnCalendar=*-*-* 03:17
RandomizedDelaySec=15m
Persistent=true

[Install]
WantedBy=timers.target
```

Then `systemctl daemon-reload && systemctl enable --now copilot-backup.timer`; `journalctl -u copilot-backup` shows each run.

To restore, into this install or a fresh one with the same `deploy/.env`:

```bash
docker compose -f deploy/compose.yml stop copilot
docker compose -f deploy/compose.yml up -d --wait postgres
docker compose -f deploy/compose.yml exec -T postgres \
  pg_restore -U copilot -d copilot --clean --if-exists --no-owner --no-privileges <backups/copilot-20261127T031700Z.dump
docker compose -f deploy/compose.yml up -d
```

The dump's grants are skipped (`--no-privileges`) because `up` runs `migrate`, which creates the server's role if it is missing and grants it the restored tables again. CI runs this exact sequence on every image build: a backup with `deploy/backup.sh`, `down -v` to lose everything, the restore onto the empty database, and a sign-in that finds the data again. Try a restore of your own once before you need it.

## Logs

The server writes one JSON object per line to standard output, which Docker collects:

```bash
docker compose -f deploy/compose.yml logs -f copilot
docker compose -f deploy/compose.yml logs copilot | grep '"level":"ERROR"'
```

Each line's `message` is a stable dotted key (`api.server.listening`, `middleware.http.client-error`, `provider.http.failed`, `api.scheduler.fired`) with the details in fields, so you can filter on the key. Every request gets an id that the response returns in `X-Request-Id` and the log carries as `request_id`; a person reporting an error can quote it. Requests are logged with method, path, status, and duration (query parameter names only, never their values). Logs never contain message text, file contents, API keys, passwords, or session tokens, and rmcp's own events are silenced because they can carry tool server URLs and server text (Copilot logs `mcp.*` keys instead).

`LOG_LEVEL` (`debug`, `info`, `warn`, `error`) sets how much is written. Docker's default `json-file` driver keeps logs forever; cap them with `logging: { driver: local }` on the services in an override file, or set `log-opts` with `max-size` and `max-file` in `/etc/docker/daemon.json`.

## Routes and Probes

| Path | Purpose |
| --- | --- |
| `GET /health` | Liveness |
| `GET /health/ready` | Readiness (`503` while Postgres is unreachable) |
| `/api/*` | The API: lib-rustauth's `/api/auth/*` and `/api/admin/users*`, Copilot's routes, `/api/doc` (OpenAPI) and `/api/ui` (Swagger UI); public besides the sign-in routes: `/api/oauth/callback` and `/api/oauth/client` (with `PUBLIC_URL`) |
| Any other `GET` | The web app (`WEB_DIR`), with `index.html` for client routes, the web Content-Security-Policy, and a year-long cache on the hashed files under `/static/` |

On `SIGTERM` the server stops accepting connections, gives answers being written 5 seconds to finish and up to 10 more to save what they have, ends open streams, finishes in-flight requests, then closes the pool. A run cut off by a crash is marked interrupted at the next start, so Retry can answer it.

## Without Compose

```bash
docker pull ghcr.io/host39/copilot:0.1.0
docker run --rm --env-file copilot.env ghcr.io/host39/copilot:0.1.0 migrate
docker run -d --env-file copilot.env -p 127.0.0.1:3000:3000 ghcr.io/host39/copilot:0.1.0
```

`copilot.env` holds `ENVIRONMENT`, the `DB_*` variables, `SECRET_KEY`, and whatever else you set ([env.md](./env.md)); run `migrate` as the database owner with `DB_SERVER_PASSWORD`, and the server as `copilot_server`. The image sets `HOST=0.0.0.0`, `PORT=3000`, and `WEB_DIR=/app/web`. With no shell in the image, inspect a container with `docker logs` rather than `exec`. To build the image yourself, run `docker build -t copilot .` at the repository root. CI builds it and starts the compose stack whenever the image's inputs change, and on a version tag pushes the image it tested to `ghcr.io/host39/copilot` (`.github/workflows/image.yml`).
