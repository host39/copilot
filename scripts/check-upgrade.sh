#!/usr/bin/env bash
# The upgrade check (`make test-upgrade`, and a step of CI's Rust job right after `make test-db`
# and the smoke test): a database written by the previous release, migrated by this commit's
# `migrate`, keeps working with this commit's server.
#
# 1. The previous release is the newest `v*` tag reachable from HEAD, leaving out tags on HEAD
#    itself (the release being made is not its own predecessor). Its `server`, `migrate`, and
#    `fake-llm` are built in a temporary git worktree (target directory `<target>/upgrade`), and
#    its own scripts/smoke.sh runs on them with SMOKE_KEEP=1: a fresh database migrated by that
#    release and filled through its API (the administrator, its provider connections with
#    sealed keys, chats with answers and an error, its tool servers, an approved tool call). The
#    database is kept.
# 2. This commit's `migrate` runs on that database: the upgrade. Every migration of this commit
#    must then be applied.
# 3. This commit's fake-llm and server start on it with the same fake key and SECRET_KEY, and the
#    checks run over HTTP: the administrator signs in with the old password; the chats, their
#    messages, the connections (keys still readable), and the tool servers are there; and a new
#    turn in an old chat streams an answer through the old connection.
#
# Before the first release tag there is nothing to upgrade from: step 1 then uses this commit's
# code, so the check proves the harness and that `migrate` runs twice cleanly, and passes with a
# notice. The database is dropped at the end (UPGRADE_KEEP=1 keeps it).
#
# Settings: DB_HOST, DB_PORT, DB_USER, DB_PASSWORD as for scripts/smoke.sh (the shell, then
# apps/api/.env.local, then localhost:5432 and postgres/postgres); UPGRADE_BIN_DIR, a folder with
# this commit's prebuilt `server`, `migrate`, and `fake-llm` (CI's release build), otherwise
# debug builds; UPGRADE_API_PORT (3094), and the ports of the seeding smoke run
# (UPGRADE_SEED_API_PORT 3093, UPGRADE_FAKE_PORT 4093; the fake server keeps that port in step 3,
# because the stored connections name it). Needs the full git history with tags (in CI,
# actions/checkout with `fetch-depth: 0`), curl, jq, and psql.
#
# Usage: bash scripts/check-upgrade.sh   (or `make test-upgrade`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENV_FILE="apps/api/.env.local"
if [[ -z "${UPGRADE_ENV_LOADED:-}" && -f "$ENV_FILE" ]]; then
  export UPGRADE_ENV_LOADED=1
  exec bash scripts/with-env.sh "$ENV_FILE" bash "$0" "$@"
fi

export DB_HOST="${DB_HOST:-localhost}"
export DB_PORT="${DB_PORT:-5432}"
export DB_USER="${DB_USER:-postgres}"
export DB_PASSWORD="${DB_PASSWORD:-postgres}"
API_PORT="${UPGRADE_API_PORT:-3094}"
SEED_API_PORT="${UPGRADE_SEED_API_PORT:-3093}"
FAKE_PORT="${UPGRADE_FAKE_PORT:-4093}"
API="http://127.0.0.1:$API_PORT"
FAKE="http://127.0.0.1:$FAKE_PORT"
FAKE_KEY="upgrade-key-$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')"
SECRET="$(head -c 32 /dev/urandom | base64 | tr -d '\n')"
TARGET="${CARGO_TARGET_DIR:-target}"
[[ "$TARGET" == /* ]] || TARGET="$ROOT/$TARGET"
RUN_DIR="$TARGET/upgrade-check"
mkdir -p "$RUN_DIR"

fail() {
  echo "scripts.upgrade.failed $*" >&2
  exit 1
}

pass() {
  echo "scripts.upgrade.passed $*"
}

notice() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
    echo "::notice title=Upgrade check::$*"
  else
    echo "scripts.upgrade.notice $*"
  fi
}

for tool in curl jq psql git; do
  command -v "$tool" >/dev/null || fail "missing $tool"
done
[[ "$(git rev-parse --is-shallow-repository)" == false ]] ||
  fail "needs the full history and tags (actions/checkout with fetch-depth: 0)"

sql() {
  PGPASSWORD="$DB_PASSWORD" psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "${2:-postgres}" \
    -v ON_ERROR_STOP=1 -qAtc "$1"
}

scratch="$(mktemp -d)"
previous_dir="$scratch/previous"
fake_pid=""
api_pid=""
db=""
cleanup() {
  local code=$?
  for pid in "$api_pid" "$fake_pid"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ $code -ne 0 ]]; then
    for log in seed api fake-llm migrate; do
      if [[ -s "$RUN_DIR/$log.log" ]]; then
        echo "--- last lines of $RUN_DIR/$log.log" >&2
        tail -n 25 "$RUN_DIR/$log.log" >&2
      fi
    done
  fi
  if [[ -n "$db" ]]; then
    if [[ -n "${UPGRADE_KEEP:-}" ]]; then
      echo "scripts.upgrade.kept $db"
    else
      sql "DROP DATABASE IF EXISTS $db WITH (FORCE)" || echo "scripts.upgrade.drop-failed $db" >&2
    fi
  fi
  if [[ -d "$previous_dir" ]]; then
    git -C "$ROOT" worktree remove --force "$previous_dir" || true
  fi
  rm -rf "$scratch"
  exit "$code"
}
trap cleanup EXIT

# builds <dir> <target dir>: `server`, `migrate`, and `fake-llm` as debug builds, one package at a
# time (as scripts/smoke.sh builds them), so fake-llm's rmcp server features stay out of the server.
builds() {
  (
    cd "$1"
    export CARGO_TARGET_DIR="$2"
    cargo build -q --locked -p copilot-api --bin server
    cargo build -q --locked -p copilot-database --bin migrate
    cargo build -q --locked -p fake-llm
  )
}

# This commit's binaries.
if [[ -n "${UPGRADE_BIN_DIR:-}" ]]; then
  BIN="$(cd "$UPGRADE_BIN_DIR" && pwd)"
else
  builds "$ROOT" "$TARGET"
  BIN="$TARGET/debug"
fi
for binary in server migrate fake-llm; do
  [[ -x "$BIN/$binary" ]] || fail "no executable $BIN/$binary"
done

# --- 1. The previous release's data -----------------------------------------------------------

exclude=()
while read -r tag; do
  if [[ -n "$tag" ]]; then
    exclude+=(--exclude "$tag")
  fi
done < <(git tag --points-at HEAD)
previous="$(git describe --tags --abbrev=0 --match 'v[0-9]*' ${exclude[@]+"${exclude[@]}"} HEAD 2>/dev/null || true)"

if [[ -n "$previous" ]]; then
  echo "scripts.upgrade.from $previous to $(git rev-parse --short HEAD)"
  git worktree add --detach "$previous_dir" "$previous" >/dev/null
  # Its own target directory, so this commit's binaries are not replaced.
  builds "$previous_dir" "$TARGET/upgrade"
  seed_root="$previous_dir"
  seed_bin="$TARGET/upgrade/debug"
else
  notice "No release tag yet, so there is nothing to upgrade from: seeded with this commit's code to check the harness and a second migrate."
  seed_root="$ROOT"
  seed_bin="$BIN"
fi

# The release's own smoke test writes the data and keeps the database.
SMOKE_KEEP=1 SMOKE_BIN_DIR="$seed_bin" SMOKE_SECRET_KEY="$SECRET" SMOKE_FAKE_KEY="$FAKE_KEY" \
  SMOKE_API_PORT="$SEED_API_PORT" SMOKE_FAKE_PORT="$FAKE_PORT" SMOKE_ENV_LOADED=1 \
  bash "$seed_root/scripts/smoke.sh" >"$RUN_DIR/seed.log" 2>&1 || fail "the seeding smoke run (see $RUN_DIR/seed.log)"
db="$(sed -n 's/^scripts\.smoke\.kept //p' "$RUN_DIR/seed.log" | tail -n 1)"
[[ "$db" =~ ^copilot_smoke_[0-9a-f]+$ ]] || fail "the seeding smoke run kept no database (see $RUN_DIR/seed.log)"
# The connections and tool servers that release's smoke test made (a later smoke test may add
# more).
connections="$(sql "SELECT count(*) FROM providers" "$db")"
servers="$(sql "SELECT count(*) FROM tool_servers" "$db")"
pass "the ${previous:-current} release wrote its data to $db ($connections connections, $servers tool servers)"

# --- 2. The upgrade ---------------------------------------------------------------------------

DB_NAME="$db" ENVIRONMENT=local "$BIN/migrate" >"$RUN_DIR/migrate.log" 2>&1 ||
  fail "migrate on the previous release's database (see $RUN_DIR/migrate.log)"
expected="$(find "$ROOT/packages/database/migrations" -name '*.up.sql' | wc -l)"
applied="$(sql "SELECT count(*) FROM public._sqlx_migrations WHERE success" "$db")"
[[ "$applied" == "$expected" ]] || fail "$applied of $expected migrations applied after the upgrade"
pass "migrate upgraded $db ($applied migrations)"

# --- 3. This commit on the upgraded data ------------------------------------------------------

for url in "$API" "$FAKE"; do
  if curl -s -o /dev/null "$url"; then
    fail "something already answers on $url (set UPGRADE_API_PORT / UPGRADE_FAKE_PORT)"
  fi
done
FAKE_LLM_PORT="$FAKE_PORT" FAKE_LLM_KEY="$FAKE_KEY" FAKE_LLM_CHUNK_MS=5 \
  "$BIN/fake-llm" >"$RUN_DIR/fake-llm.log" 2>&1 &
fake_pid=$!
env -u WEB_DIR -u CORS_ALLOWED_ORIGINS \
  ENVIRONMENT=local HOST=127.0.0.1 PORT="$API_PORT" LOG_LEVEL=info \
  DB_HOST="$DB_HOST" DB_PORT="$DB_PORT" DB_USER="$DB_USER" DB_PASSWORD="$DB_PASSWORD" DB_NAME="$db" \
  SECRET_KEY="$SECRET" ALLOW_PRIVATE_NETWORK=all AUTH_REGISTRATION_OPEN=false \
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
wait_for "$API/health/ready" "$api_pid" || fail "the server did not get ready on the upgraded database"

token=""
body=""
# request <method> <path> [json]: the body of a successful answer in $body.
request() {
  local args=(-sS -o "$RUN_DIR/body" -w '%{http_code}' -X "$1" "$API/api$2") status
  [[ -n "$token" ]] && args+=(-H "authorization: Bearer $token")
  [[ $# -ge 3 ]] && args+=(-H 'content-type: application/json' --data "$3")
  status="$(curl "${args[@]}")"
  body="$(cat "$RUN_DIR/body")"
  [[ "$status" == 2* ]] || fail "$1 $2: HTTP $status: $body"
}

request POST /auth/login '{"username":"smoke","password":"smoke-password-123"}'
token="$(jq -er .data.token <<<"$body")"
request GET /auth/me
jq -e '.data.role == "administrator"' >/dev/null <<<"$body" || fail "the administrator lost the role: $body"
pass "the administrator signs in with the old password"

request GET /chats
chats="$(jq -r '.items[].id' <<<"$body")"
[[ "$(wc -l <<<"$chats")" -ge 4 ]] || fail "the four chats of the smoke run should remain: $body"
long=""
for chat in $chats; do
  request GET "/chats/$chat/messages"
  count="$(jq '.items | length' <<<"$body")"
  [[ "$count" -ge 2 ]] || fail "chat $chat lost its messages: $body"
  if [[ "$count" -ge 4 && -z "$long" ]]; then
    long="$chat"
  fi
done
[[ -n "$long" ]] || fail "no chat with both turns of the smoke run"
pass "every chat reads with its messages"

request GET /providers
providers="$(jq -r '.items[].id' <<<"$body")"
[[ "$(wc -l <<<"$providers")" == "$connections" ]] || fail "the $connections connections should remain: $body"
for provider in $providers; do
  request GET "/providers/$provider"
  jq -e '.data.key == "set"' >/dev/null <<<"$body" || fail "a stored key no longer opens: $body"
done
request GET /tool-servers
jq -e --argjson servers "$servers" '.items | length == $servers' >/dev/null <<<"$body" ||
  fail "the $servers tool servers should remain: $body"
pass "connections (keys readable) and the tool servers remain"

code="$(curl -sS -N --max-time 60 -o "$RUN_DIR/stream" -w '%{http_code}' -X POST "$API/api/chats/$long/messages" \
  -H "authorization: Bearer $token" -H 'content-type: application/json' --data '{"text":"hello again"}')"
[[ "$code" == 200 ]] || fail "a new turn in an old chat: HTTP $code: $(cat "$RUN_DIR/stream")"
reply="$(sed -n 's/^data: \{0,1\}//p' "$RUN_DIR/stream" | jq -rs '[.[] | select(.type == "text.delta") | .text] | join("")')"
[[ "$reply" == "You said: hello again" ]] || fail "a new turn in an old chat streamed: $reply"
pass "a new turn in an old chat streams through the old connection"

if [[ -n "$previous" ]]; then
  pass "$previous upgrades to $(git rev-parse --short HEAD)"
else
  pass "the harness, with this commit on both sides"
fi
