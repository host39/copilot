#!/usr/bin/env bash
# Runs the API with `apps/api/.env.local`, restarting on changes when watchexec is installed
# (`cargo install --locked watchexec-cli`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

run=(cargo run -p copilot-api --bin server)

if command -v watchexec >/dev/null 2>&1; then
  exec bash scripts/with-env.sh apps/api/.env.local \
    watchexec --restart --exts rs,toml,sql --watch apps/api --watch packages -- "${run[@]}"
fi

echo "scripts.dev-api.no-watch install watchexec-cli for restarts on change" >&2
exec bash scripts/with-env.sh apps/api/.env.local "${run[@]}"
