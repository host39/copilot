# Changelog

All notable changes to Copilot are recorded here. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and releases follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with tags `vMAJOR.MINOR.PATCH`. One version covers the server image, the Linux and Android apps, and the web app they bundle: the root `Cargo.toml` (`workspace.package.version`) and `apps/native/package.json` hold it, and a release tag must name it.

Every release lists what an operator must do under "Upgrade Notes": schema changes, new or changed environment variables, and whether the release may be skipped. The steps of an upgrade are in [docs/self-hosting.md](./docs/self-hosting.md#upgrades). `make test-upgrade` and CI upgrade a database written by the previous release tag with the current code and use it ([scripts/check-upgrade.sh](./scripts/check-upgrade.sh)).

## [Unreleased]

The first release, planned as `v0.1.0`. It has no predecessor, so there is nothing to upgrade from.

### Added

- **Server:** one Rust binary (axum) that serves the API under `/api` and the web app, and a `migrate` binary that applies lib-rustauth's `auth` migrations, then Copilot's, and grants the server's role (`copilot_server`) the rows of every table. Postgres 18 holds everything, file bytes included.
- **Providers:** OpenAI-compatible connections (presets `openai`, `gemini`, `openrouter`, `ollama`, `custom`) and Anthropic connections (`anthropic`, `custom`), with streamed answers, reasoning, tool calls, images, PDFs, and audio where the model takes them. API keys are sealed with AES-256-GCM under `SECRET_KEY`. Model lists with capabilities from the provider's metadata or the preset's defaults; a `gemini` connection's models get their context window, output limit, and thinking flag from Google's native model list.
- **Chats:** streaming that survives a closed tab, reattaching from any device, stop, regenerate, retry, edit and resend, Continue, search, unread and approval dots.
- **Projects:** instructions, files, a default model and reasoning level, tools, and schedules shared by every chat in the project; Save as project.
- **MCP tools:** remote servers over Streamable HTTP (both protocol eras) with a bearer token or headers, Off, Ask, or Auto per tool, approval cards, and a review after a server changes its tools.
- **Schedules:** cron rules in the owner's time zone, fired by the server's clock; the model may propose a schedule, which the owner approves.
- **Voice:** push to talk (16 kHz WAV clips up to 5 minutes) for models that take audio, and spoken replies saved as WAV files for models that return audio. None for Claude models.
- **Accounts:** lib-rustauth `v0.1.0` (pinned by commit): the first account is the administrator, registration is closed by default, sessions, lockout, and per-address limits.
- **Apps:** the installable web app, a Linux app (`.deb` and AppImage, glibc 2.39 or newer), and an Android app (arm64 APK), attached to each GitHub Release with `SHA256SUMS`.
- **Deployment:** `deploy/compose.yml` (Postgres, `migrate`, the server on `127.0.0.1:3000`), the image `ghcr.io/host39/copilot:<version>` published on each release tag, `deploy/backup.sh` for nightly dumps with retention (with a crontab line and a systemd timer), and the self-hosting guide (reverse proxy, private network policy, Ollama, a stdio MCP server behind a bridge, upgrades, backups).

### Upgrade Notes

- **From nothing:** install with [docs/self-hosting.md](./docs/self-hosting.md#install), with `COPILOT_IMAGE=ghcr.io/host39/copilot:0.1.0`. Keep `deploy/.env`: a different `SECRET_KEY` makes every stored provider key and tool server header unreadable.
- **Schema:** the migrations in `packages/database/migrations/` become fixed with this release. Later releases change the schema only with new migrations, which `migrate` applies on the next `up`.
- **Development databases** created before the first release (from this branch) may have applied migrations that were edited before the release; recreate them with `make clean` and `make setup` if `migrate` reports a changed checksum.
