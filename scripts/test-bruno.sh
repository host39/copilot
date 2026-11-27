#!/usr/bin/env bash
# Runs the Bruno collection against the API.
#
# Without BASE_URL, it builds and starts the API from this checkout with `apps/api/.env.local`
# (base URL http://localhost:$PORT, so a worktree with its own PORT tests its own server), runs
# the collection, and stops the API afterwards. If something already answers on that port it
# refuses to run, because that server may be another worktree's or an older build.
#
# With BASE_URL set explicitly, it tests the server already running there and starts nothing.
#
# The "02 Auth" folder registers a run-unique account and later folders use its session token.
# On a database without accounts that first registration needs the setup token, and the account
# becomes the administrator; afterwards it needs open registration. So the server this script
# starts gets AUTH_REGISTRATION_OPEN=true and AUTH_SETUP_TOKEN (the env file's, or a generated
# one), unless the shell sets them, PUBLIC_URL set to its own base URL (the sign-in callback
# tests), and stdio tool servers on (MCP_STDIO_ENABLED, MCP_STDIO_DIR in the run folder; the
# stdio tests, which also need the run's account to be the administrator). Against BASE_URL, pass
# the server's AUTH_SETUP_TOKEN in the shell or the env file when its database has no account
# yet.
#
# Usage: bash scripts/test-bruno.sh                                  (or `make test-bruno`)
#        BASE_URL=http://localhost:3001 bash scripts/test-bruno.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENV_FILE="apps/api/.env.local"
RUN_DIR="apps/api/tests/bruno/.run"
mkdir -p "$RUN_DIR"

up() {
  curl -sf "$1/health" >/dev/null
}

api_pid=""
cleanup() {
  if [[ -n "$api_pid" ]]; then
    kill "$api_pid" 2>/dev/null || true
    wait "$api_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ -n "${BASE_URL:-}" ]]; then
  if ! up "$BASE_URL"; then
    echo "scripts.test-bruno.not-reachable $BASE_URL (start the API there, or unset BASE_URL to start one)" >&2
    exit 1
  fi
else
  # The port the API will listen on: PORT from the shell or the env file, else the API default.
  port="$(bash scripts/with-env.sh "$ENV_FILE" bash -c 'printf "%s" "${PORT:-3000}"')"
  BASE_URL="http://localhost:$port"

  if up "$BASE_URL"; then
    echo "scripts.test-bruno.server-running $BASE_URL (stop it, or set BASE_URL=$BASE_URL to test it)" >&2
    exit 1
  fi

  export AUTH_REGISTRATION_OPEN="${AUTH_REGISTRATION_OPEN:-true}"
  if [[ -z "${AUTH_SETUP_TOKEN:-}" ]]; then
    AUTH_SETUP_TOKEN="$(bash scripts/with-env.sh "$ENV_FILE" bash -c 'printf "%s" "${AUTH_SETUP_TOKEN:-}"')"
  fi
  if [[ -z "$AUTH_SETUP_TOKEN" ]]; then
    AUTH_SETUP_TOKEN="bruno-$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  fi
  export AUTH_SETUP_TOKEN
  # Tool server sign-in comes back to the server it started on (the callback tests).
  export PUBLIC_URL="$BASE_URL"
  # stdio tool servers on, with their directories in this run's folder (the stdio tests).
  export MCP_STDIO_ENABLED=true
  export MCP_STDIO_DIR="$ROOT/$RUN_DIR/mcp-stdio"

  cargo build -q -p copilot-api --bin server
  bash scripts/with-env.sh "$ENV_FILE" "${CARGO_TARGET_DIR:-target}/debug/server" >"$RUN_DIR/api.log" 2>&1 &
  api_pid=$!

  for _ in $(seq 1 50); do
    up "$BASE_URL" && break
    if ! kill -0 "$api_pid" 2>/dev/null; then
      break
    fi
    sleep 0.2
  done

  if ! up "$BASE_URL"; then
    echo "scripts.test-bruno.api-not-ready $BASE_URL" >&2
    tail -n 20 "$RUN_DIR/api.log" >&2 || true
    exit 1
  fi
fi

setup_token="${AUTH_SETUP_TOKEN:-}"
if [[ -z "$setup_token" && -f "$ENV_FILE" ]]; then
  setup_token="$(bash scripts/with-env.sh "$ENV_FILE" bash -c 'printf "%s" "${AUTH_SETUP_TOKEN:-}"')"
fi

cd apps/api/tests/bruno
"$ROOT/node_modules/.bin/bru" run --env development \
  --env-var "baseUrl=$BASE_URL" \
  --env-var "setupToken=$setup_token"
