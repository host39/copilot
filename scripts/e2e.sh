#!/usr/bin/env bash
# Browser test at phone size (apps/e2e, Playwright with the system's Google Chrome): the server
# serving the production web bundle, against the fake model server, on a throwaway database.
#
# It creates a database (copilot_e2e_<random>), migrates it, starts fake-llm and the server (with
# WEB_DIR pointing at the built bundle, as in the image), and prepares over HTTP what a person
# would have set up before picking up their phone: the administrator account, an
# OpenAI-compatible connection to the fake server with only fake-chat on, and the fake MCP server
# as a tool server that asks before each call. Then `pnpm --filter @copilot/e2e e2e` runs the
# tests in apps/e2e/tests against it, and everything stops and the database is dropped
# (E2E_KEEP=1 keeps the database).
#
# Settings: DB_HOST, DB_PORT, DB_USER, DB_PASSWORD (the shell, then apps/api/.env.local, then
# localhost:5432 and postgres/postgres); E2E_API_PORT (3095) and E2E_FAKE_PORT (4095);
# E2E_BIN_DIR, a folder with prebuilt `server`, `migrate`, and `fake-llm` (CI), otherwise this
# checkout's debug builds. The web bundle is built for the run into target/e2e/web the way the
# image builds it (no `.env` files, so no PUBLIC_API_URL: the app calls the server that served
# it). Needs curl, jq, psql, and Google Chrome (Playwright's `chrome` channel).
#
# Usage: bash scripts/e2e.sh [playwright arguments]   (or `make e2e`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENV_FILE="apps/api/.env.local"
if [[ -z "${E2E_ENV_LOADED:-}" && -f "$ENV_FILE" ]]; then
  export E2E_ENV_LOADED=1
  exec bash scripts/with-env.sh "$ENV_FILE" bash "$0" "$@"
fi

TARGET="${CARGO_TARGET_DIR:-target}"
[[ "$TARGET" == /* ]] || TARGET="$ROOT/$TARGET"
RUN_DIR="$TARGET/e2e"
mkdir -p "$RUN_DIR"

export DB_HOST="${DB_HOST:-localhost}"
export DB_PORT="${DB_PORT:-5432}"
export DB_USER="${DB_USER:-postgres}"
export DB_PASSWORD="${DB_PASSWORD:-postgres}"
API_PORT="${E2E_API_PORT:-3095}"
FAKE_PORT="${E2E_FAKE_PORT:-4095}"
API="http://127.0.0.1:$API_PORT"
FAKE="http://127.0.0.1:$FAKE_PORT"
WEB="$RUN_DIR/web"
FAKE_KEY="e2e-key-$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')"
SETUP_TOKEN="e2e-setup-$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"
PASSWORD="e2e-password-$(head -c 6 /dev/urandom | od -An -tx1 | tr -d ' \n')"
DB="copilot_e2e_$(head -c 6 /dev/urandom | od -An -tx1 | tr -d ' \n')"

fail() {
  echo "scripts.e2e.failed $*" >&2
  exit 1
}

pass() {
  echo "scripts.e2e.passed $*"
}

for tool in curl jq psql; do
  command -v "$tool" >/dev/null || fail "missing $tool"
done
for url in "$API" "$FAKE"; do
  if curl -s -o /dev/null "$url"; then
    fail "something already answers on $url (stop it, or set E2E_API_PORT / E2E_FAKE_PORT)"
  fi
done

sql() {
  PGPASSWORD="$DB_PASSWORD" psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d postgres \
    -v ON_ERROR_STOP=1 -qAtc "$1"
}

fake_pid=""
api_pid=""
created=""
cleanup() {
  local code=$?
  for pid in "$api_pid" "$fake_pid"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ $code -ne 0 ]]; then
    for log in api fake-llm; do
      if [[ -s "$RUN_DIR/$log.log" ]]; then
        echo "--- last lines of $RUN_DIR/$log.log" >&2
        tail -n 25 "$RUN_DIR/$log.log" >&2
      fi
    done
  fi
  if [[ -n "$created" ]]; then
    if [[ -n "${E2E_KEEP:-}" ]]; then
      echo "scripts.e2e.kept $DB"
    else
      sql "DROP DATABASE IF EXISTS $DB WITH (FORCE)" || echo "scripts.e2e.drop-failed $DB" >&2
    fi
  fi
  exit "$code"
}
trap cleanup EXIT

if [[ -n "${E2E_BIN_DIR:-}" ]]; then
  BIN="$(cd "$E2E_BIN_DIR" && pwd)"
else
  # One build per package, as scripts/smoke.sh builds them.
  cargo build -q -p copilot-api --bin server
  cargo build -q -p copilot-database --bin migrate
  cargo build -q -p fake-llm
  BIN="$TARGET/debug"
fi
for binary in server migrate fake-llm; do
  [[ -x "$BIN/$binary" ]] || fail "no executable $BIN/$binary"
done
rm -rf "$WEB"
env -u PUBLIC_API_URL pnpm --filter @copilot/web exec rsbuild build --no-env --dist-path "$WEB" \
  >"$RUN_DIR/web.log" 2>&1 || fail "the web build (see $RUN_DIR/web.log)"
[[ -f "$WEB/index.html" ]] || fail "the web build wrote no index.html to $WEB"

sql "CREATE DATABASE $DB" || fail "could not create the database $DB on $DB_HOST:$DB_PORT"
created=yes
DB_NAME="$DB" ENVIRONMENT=local "$BIN/migrate" >"$RUN_DIR/migrate.log" 2>&1 ||
  fail "migrate (see $RUN_DIR/migrate.log)"

FAKE_LLM_PORT="$FAKE_PORT" FAKE_LLM_KEY="$FAKE_KEY" FAKE_LLM_CHUNK_MS=5 \
  "$BIN/fake-llm" >"$RUN_DIR/fake-llm.log" 2>&1 &
fake_pid=$!
env -u CORS_ALLOWED_ORIGINS \
  ENVIRONMENT=local HOST=127.0.0.1 PORT="$API_PORT" LOG_LEVEL=info WEB_DIR="$WEB" \
  DB_HOST="$DB_HOST" DB_PORT="$DB_PORT" DB_USER="$DB_USER" DB_PASSWORD="$DB_PASSWORD" DB_NAME="$DB" \
  SECRET_KEY="$(head -c 32 /dev/urandom | base64 | tr -d '\n')" \
  ALLOW_PRIVATE_NETWORK=admins AUTH_REGISTRATION_OPEN=false AUTH_SETUP_TOKEN="$SETUP_TOKEN" \
  "$BIN/server" >"$RUN_DIR/api.log" 2>&1 &
api_pid=$!

wait_for() {
  local url="$1" pid="$2"
  for _ in $(seq 1 100); do
    curl -sf "$url" >/dev/null && return 0
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 0.2
  done
  return 1
}
wait_for "$FAKE/health" "$fake_pid" || fail "fake-llm did not start on $FAKE"
wait_for "$API/health/ready" "$api_pid" || fail "the server did not get ready on $API"

# --- What the person set up before (over HTTP) --------------------------------------------------

token=""
body=""
# request <method> <path> [json]: the body of a successful answer in $body.
request() {
  local args=(-sS -o "$RUN_DIR/body" -w '%{http_code}' -X "$1" "$API/api$2") status
  if [[ -n "$token" ]]; then args+=(-H "authorization: Bearer $token"); fi
  if [[ $# -ge 3 ]]; then args+=(-H 'content-type: application/json' --data "$3"); fi
  status="$(curl "${args[@]}")"
  body="$(cat "$RUN_DIR/body")"
  [[ "$status" == 2* ]] || fail "$1 $2: HTTP $status: $body"
}

request POST /auth/register \
  "{\"username\":\"phone\",\"password\":\"$PASSWORD\",\"displayName\":\"Phone\",\"setupToken\":\"$SETUP_TOKEN\"}"
token="$(jq -er .data.token <<<"$body")"
request POST /providers \
  "{\"kind\":\"openai\",\"preset\":\"openai\",\"api\":\"chat_completions\",\"name\":\"Fake OpenAI\",\"baseUrl\":\"$FAKE/v1\",\"apiKey\":\"$FAKE_KEY\"}"
provider="$(jq -er .data.id <<<"$body")"
request POST "/providers/$provider/refresh"
request GET "/providers/$provider/models"
# Only fake-chat stays on, so the new chat screen picks it.
for model in $(jq -r '.items[] | select(.remoteId != "fake-chat") | .id' <<<"$body"); do
  request PATCH "/models/$model" '{"enabled":false}'
done
request POST /tool-servers "{\"name\":\"Fake tools\",\"url\":\"$FAKE/mcp\",\"approval\":\"ask\"}"
server="$(jq -er .data.id <<<"$body")"
request POST "/tool-servers/$server/test"
pass "server on $API with the administrator, a connection, and a tool server"

# --- The browser -------------------------------------------------------------------------------

E2E_BASE_URL="$API" E2E_USERNAME=phone E2E_PASSWORD="$PASSWORD" E2E_OUTPUT="$RUN_DIR/results" \
  pnpm --filter @copilot/e2e e2e "$@"
pass "the phone-size browser tests"
