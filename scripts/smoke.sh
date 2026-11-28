#!/usr/bin/env bash
# Smoke test: Copilot's API against the fake model server (tools/fake-llm), over HTTP only.
#
# It creates a throwaway database (copilot_smoke_<random>) on the DB_* server, migrates it,
# starts fake-llm and the API on ports of their own, and drives the API the way a person would:
#
#   1. register the first account with the setup token;
#   2. add an OpenAI-compatible connection (the `openai` preset on Chat Completions, with the
#      fake's base URL) to the fake server, refresh its models, enable fake-chat, chat "hello" (streamed "You said:
#      hello" with usage), then "failed 403" (a PROVIDER_AUTH error), and read the saved
#      messages back; a `custom` connection's "hello" completes without usage (none was asked);
#   3. add the fake MCP server as a tool server, Test it, ask for `tool echo {...}`, approve the
#      call, and check the result and "Tool said: ..."; then the same over stdio, with
#      `fake-llm --stdio` as a stdio tool server that the API starts for the Test and the run;
#   4. an `openai` preset connection on its default API, Responses: the text flow, a refusal,
#      an approved tool round whose encrypted reasoning the fake checks on replay, and a decision
#      after the chat moved to another model (the reasoning must not follow it);
#   5. the same text flow over an Anthropic connection to the fake server.
#
# Both servers stop and the database is dropped when the script exits (SMOKE_KEEP=1 keeps the
# database and prints its name as `scripts.smoke.kept <name>`; scripts/check-upgrade.sh keeps it
# as the previous release's data). The dev database (DB_NAME in apps/api/.env.local) is never used.
#
# Settings: DB_HOST, DB_PORT, DB_USER, DB_PASSWORD (the shell, then apps/api/.env.local, then
# localhost:5432 and postgres/postgres); SMOKE_API_PORT (3091) and SMOKE_FAKE_PORT (4091);
# SMOKE_BIN_DIR, a folder with prebuilt `server`, `migrate`, and `fake-llm` binaries (CI),
# otherwise this checkout's debug builds; SMOKE_SECRET_KEY and SMOKE_FAKE_KEY, the server's
# SECRET_KEY and the fake server's key (random otherwise), so a kept database's sealed keys can be
# opened again. Needs curl, jq, and psql.
#
# Usage: bash scripts/smoke.sh   (or `make smoke`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENV_FILE="apps/api/.env.local"
if [[ -z "${SMOKE_ENV_LOADED:-}" && -f "$ENV_FILE" ]]; then
  export SMOKE_ENV_LOADED=1
  exec bash scripts/with-env.sh "$ENV_FILE" bash "$0" "$@"
fi

RUN_DIR="$ROOT/target/smoke"
mkdir -p "$RUN_DIR"

export DB_HOST="${DB_HOST:-localhost}"
export DB_PORT="${DB_PORT:-5432}"
export DB_USER="${DB_USER:-postgres}"
export DB_PASSWORD="${DB_PASSWORD:-postgres}"
API_PORT="${SMOKE_API_PORT:-3091}"
FAKE_PORT="${SMOKE_FAKE_PORT:-4091}"
API="http://127.0.0.1:$API_PORT"
FAKE="http://127.0.0.1:$FAKE_PORT"
FAKE_KEY="${SMOKE_FAKE_KEY:-smoke-key-$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')}"
SECRET_KEY_VALUE="${SMOKE_SECRET_KEY:-$(head -c 32 /dev/urandom | base64 | tr -d '\n')}"
SETUP_TOKEN="smoke-setup-$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"
DB="copilot_smoke_$(head -c 6 /dev/urandom | od -An -tx1 | tr -d ' \n')"

fail() {
  echo "scripts.smoke.failed $*" >&2
  exit 1
}

pass() {
  echo "scripts.smoke.passed $*"
}

for tool in curl jq psql; do
  command -v "$tool" >/dev/null || fail "missing $tool (install it to run the smoke test)"
done

for url in "$API" "$FAKE"; do
  if curl -s -o /dev/null "$url"; then
    fail "something already answers on $url (stop it, or set SMOKE_API_PORT / SMOKE_FAKE_PORT)"
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
    if [[ -n "${SMOKE_KEEP:-}" ]]; then
      echo "scripts.smoke.kept $DB"
    else
      sql "DROP DATABASE IF EXISTS $DB WITH (FORCE)" || echo "scripts.smoke.drop-failed $DB" >&2
    fi
  fi
  exit "$code"
}
trap cleanup EXIT

# Binaries: prebuilt (CI) or this checkout's debug builds.
if [[ -n "${SMOKE_BIN_DIR:-}" ]]; then
  BIN="$(cd "$SMOKE_BIN_DIR" && pwd)"
else
  # One build per package, as `make test-bruno`, `make migrate`, and `make fake-llm` build them,
  # so their artifacts are reused and fake-llm's rmcp server features stay out of the server.
  cargo build -q -p copilot-api --bin server
  cargo build -q -p copilot-database --bin migrate
  cargo build -q -p fake-llm
  target="${CARGO_TARGET_DIR:-target}"
  [[ "$target" == /* ]] || target="$ROOT/$target"
  BIN="$target/debug"
fi
for binary in server migrate fake-llm; do
  [[ -x "$BIN/$binary" ]] || fail "no executable $BIN/$binary"
done

sql "CREATE DATABASE $DB" || fail "could not create the database $DB on $DB_HOST:$DB_PORT"
created=yes
DB_NAME="$DB" ENVIRONMENT=local "$BIN/migrate" >"$RUN_DIR/migrate.log" 2>&1 ||
  fail "migrate (see $RUN_DIR/migrate.log)"
pass "database $DB migrated"

FAKE_LLM_PORT="$FAKE_PORT" FAKE_LLM_KEY="$FAKE_KEY" FAKE_LLM_CHUNK_MS=5 \
  "$BIN/fake-llm" >"$RUN_DIR/fake-llm.log" 2>&1 &
fake_pid=$!

env -u WEB_DIR -u CORS_ALLOWED_ORIGINS \
  ENVIRONMENT=local HOST=127.0.0.1 PORT="$API_PORT" LOG_LEVEL=info \
  DB_HOST="$DB_HOST" DB_PORT="$DB_PORT" DB_USER="$DB_USER" DB_PASSWORD="$DB_PASSWORD" DB_NAME="$DB" \
  SECRET_KEY="$SECRET_KEY_VALUE" \
  ALLOW_PRIVATE_NETWORK=all AUTH_REGISTRATION_OPEN=false AUTH_SETUP_TOKEN="$SETUP_TOKEN" \
  MCP_STDIO_ENABLED=true MCP_STDIO_DIR="$RUN_DIR/mcp-stdio" \
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
wait_for "$API/health/ready" "$api_pid" || fail "the API did not get ready on $API"
pass "fake-llm on $FAKE and the API on $API"

# --- HTTP helpers ---------------------------------------------------------------------------

token=""
status=""
body=""

# request <method> <path> [json]: the API's answer in $status and $body.
request() {
  local args=(-sS -o "$RUN_DIR/body" -w '%{http_code}' -X "$1" "$API/api$2")
  [[ -n "$token" ]] && args+=(-H "authorization: Bearer $token")
  [[ $# -ge 3 ]] && args+=(-H 'content-type: application/json' --data "$3")
  status="$(curl "${args[@]}")"
  body="$(cat "$RUN_DIR/body")"
}

# expect <status> <what>: the last request answered <status>.
expect() {
  [[ "$status" == "$1" ]] || fail "$2: expected HTTP $1, got $status: $body"
}

# field <jq filter>: one value of the last body (a failure ends the script where it is used as a
# command or an assignment).
field() {
  jq -er "$1" <<<"$body" || {
    echo "scripts.smoke.failed no $1 in: $body" >&2
    return 1
  }
}

events=""

# stream <path> <json>: POSTs a turn and reads its server-sent events (the data of each) into
# $events as a JSON array.
stream() {
  local code
  code="$(curl -sS -N --max-time 60 -o "$RUN_DIR/stream" -w '%{http_code}' -X POST "$API/api$1" \
    -H "authorization: Bearer $token" -H 'content-type: application/json' --data "$2")"
  [[ "$code" == 200 ]] || fail "POST $1: expected a 200 stream, got $code: $(cat "$RUN_DIR/stream")"
  events="$(sed -n 's/^data: \{0,1\}//p' "$RUN_DIR/stream" | jq -cs .)" ||
    fail "POST $1: unreadable events: $(cat "$RUN_DIR/stream")"
}

# check <what> <jq filter>: the filter holds for $events.
check() {
  jq -e "$2" >/dev/null <<<"$events" || fail "$1: $events"
  pass "$1"
}

# text: the streamed text of the last stream.
text() {
  jq -r '[.[] | select(.type == "text.delta") | .text] | join("")' <<<"$events"
}

# --- 1. The first account -------------------------------------------------------------------

request GET /auth/setup
expect 200 "setup status"
[[ "$(field .data.needsSetup)" == true ]] || fail "a fresh database should need setup: $body"
request POST /auth/register \
  "{\"username\":\"smoke\",\"password\":\"smoke-password-123\",\"displayName\":\"Smoke\",\"setupToken\":\"$SETUP_TOKEN\"}"
expect 201 "register with the setup token"
token="$(field .data.token)"
pass "registered the administrator with the setup token"

# connect <kind> <preset> <name> <base URL> [api]: adds a connection (on `api` when given),
# refreshes it, enables fake-chat, and sets $model to fake-chat's id, $connected_api to the API it
# uses (`messages` for Anthropic), and $connected_provider to the connection's id.
model=""
connected_api=""
connected_provider=""
connect() {
  local kind="$1" preset="$2" name="$3" base="$4" api="${5:-}" provider choice=""
  [[ -n "$api" ]] && choice=",\"api\":\"$api\""
  request POST /providers \
    "{\"kind\":\"$kind\",\"preset\":\"$preset\",\"name\":\"$name\",\"baseUrl\":\"$base\",\"apiKey\":\"$FAKE_KEY\"$choice}"
  expect 201 "add the $kind connection"
  [[ -z "$api" || "$(field .data.api)" == "$api" ]] || fail "the connection should use $api: $body"
  api="$(jq -r '.data.api // "messages"' <<<"$body")"
  connected_api="$api"
  provider="$(field .data.id)"
  connected_provider="$provider"
  request POST "/providers/$provider/refresh"
  expect 200 "refresh the $kind models"
  [[ "$(field .data.added)" == 3 ]] || fail "$kind refresh should add three models: $body"
  request GET "/providers/$provider/models"
  expect 200 "list the $kind models"
  model="$(field '.items[] | select(.remoteId == "fake-chat") | .id')"
  if [[ "$api" == chat_completions ]]; then
    field '.items[] | select(.remoteId == "fake-audio") | .capabilities | select(.audioIn and .audioOut)' >/dev/null
    field '.items[] | select(.remoteId == "fake-reasoner") | .capabilities | select(.reasoning)' >/dev/null
  elif [[ "$api" == responses ]]; then
    # The Responses API takes no audio: the flags are locked off whatever the list reports.
    field '.items[] | select(.remoteId == "fake-audio") | select((.capabilities.audioIn or .capabilities.audioOut) | not) | .locks[] | select(.capability == "audioIn")' >/dev/null
    field '.items[] | select(.remoteId == "fake-reasoner") | .capabilities | select(.reasoning)' >/dev/null
  else
    field '.items[] | select(.remoteId == "fake-audio") | .capabilities | select((.audioIn or .audioOut) | not)' >/dev/null
    field '.items[] | select(.remoteId == "fake-reasoner") | .capabilities | select(.reasoning)' >/dev/null
  fi
  request PATCH "/models/$model" '{"enabled":true}'
  expect 200 "enable fake-chat ($kind)"
  [[ "$(field .data.enabled)" == true ]] || fail "fake-chat should be enabled: $body"
  pass "$kind connection ($preset preset, $api): three models with their capabilities, fake-chat enabled"
}

# chat <kind>: a chat on $model; "hello", then "failed 403", then the saved messages.
chat() {
  local kind="$1" id
  request POST /chats "{\"modelId\":\"$model\"}"
  expect 201 "create a $kind chat"
  id="$(field .data.id)"

  stream "/chats/$id/messages" '{"text":"hello"}'
  [[ "$(text)" == "You said: hello" ]] || fail "$kind hello: streamed $(text)"
  check "$kind: hello streams You said: hello and ends complete with usage" '
    (map(.type) | first == "message.started" and last == "done")
    and ([.[] | select(.type == "text.delta")] | length >= 1)
    and any(.[]; .type == "message.finished" and .status == "complete"
      and .finishReason == "stop" and (.usage.outputTokens // 0) > 0)'

  stream "/chats/$id/messages" '{"text":"failed 403"}'
  check "$kind: failed 403 ends with PROVIDER_AUTH" '
    any(.[]; .type == "error" and .code == "PROVIDER_AUTH"
      and .message == "Key rejected: open provider settings.")
    and any(.[]; .type == "message.finished" and .status == "error")
    and (map(.type) | last == "done")'

  request GET "/chats/$id/messages"
  expect 200 "read the $kind messages"
  jq -e '[.items[] | [.role, .status]] == [["user","complete"],["assistant","complete"],["user","complete"],["assistant","error"]]' \
    >/dev/null <<<"$body" || fail "$kind saved messages: $body"
  pass "$kind: both turns are saved (an answer, then an error)"
}

# --- 2. OpenAI-compatible ---------------------------------------------------------------------

# The `openai` preset with the fake's base URL on Chat Completions, so Copilot asks for streamed
# usage (`stream_options.include_usage`) and the fake sends it, as OpenAI does.
connect openai openai "Fake OpenAI" "$FAKE/v1" chat_completions
openai_model="$model"
chat openai

# A `custom` connection asks for no usage, so the fake sends none: the reply still completes.
connect openai custom "Fake compatible" "$FAKE/v1"
request POST /chats "{\"modelId\":\"$model\"}"
expect 201 "create a custom chat"
stream "/chats/$(field .data.id)/messages" '{"text":"hello"}'
[[ "$(text)" == "You said: hello" ]] || fail "custom hello: streamed $(text)"
check "custom: hello completes without usage (none was asked for)" '
  any(.[]; .type == "message.finished" and .status == "complete" and .finishReason == "stop"
    and ((.usage.outputTokens // 0) == 0))
  and (map(.type) | last == "done")'

# --- 3. Tools ---------------------------------------------------------------------------------

request POST /tool-servers "{\"name\":\"Fake tools\",\"url\":\"$FAKE/mcp\",\"approval\":\"ask\"}"
expect 201 "add the tool server"
server="$(field .data.id)"
request POST "/tool-servers/$server/test"
expect 200 "test the tool server"
[[ "$(field '[.data.server.tools[].name] | sort | join(",")')" == "add,echo" ]] ||
  fail "the tool server should list add and echo: $body"
pass "tool server: Test lists echo and add"

request POST /chats "{\"modelId\":\"$openai_model\",\"toolOverrides\":[{\"serverId\":\"$server\",\"enabled\":true}]}"
expect 201 "create a chat with the tool server"
tool_chat="$(field .data.id)"
stream "/chats/$tool_chat/messages" '{"text":"tool echo {\"text\":\"smoke check\"}"}'
check "tool echo asks for approval" '
  any(.[]; .type == "tool.requested" and .toolName == "echo" and .needsApproval
    and .arguments == {"text": "smoke check"} and .serverId != null)
  and any(.[]; .type == "message.finished" and .status == "awaiting_approval")'
message="$(jq -er '.[] | select(.type == "tool.requested") | .messageId' <<<"$events")"
call="$(jq -er '.[] | select(.type == "tool.requested") | .callId' <<<"$events")"

stream "/chats/$tool_chat/messages/$message/decisions" "{\"decisions\":[{\"callId\":\"$call\",\"decision\":\"approve\"}]}"
check "the approved call runs and the model reads its result" '
  any(.[]; .type == "tool.finished" and .status == "ok"
    and .output == [{"type": "text", "text": "smoke check"}])
  and any(.[]; .type == "message.finished" and .status == "complete")'
[[ "$(text)" == "Tool said: smoke check" ]] || fail "after the tool: streamed $(text)"
pass "the answer after the tool is Tool said: smoke check"

# A stdio server: the API starts `fake-llm --stdio` itself (the first account is the
# administrator, and the server runs with MCP_STDIO_ENABLED), with nothing of its own environment.
program="$(jq -n --arg command "$BIN/fake-llm" '{
  name: "Fake stdio", transport: "stdio",
  stdio: {command: $command, args: ["--stdio"], env: {FAKE_STDIO: "1"}}
}')"
request POST /tool-servers "$program"
expect 201 "add the stdio tool server"
stdio_server="$(field .data.id)"
[[ "$(field '.data.stdio.envNames | join(",")')" == "FAKE_STDIO" ]] ||
  fail "the stdio server should show its variable names only: $body"
request POST "/tool-servers/$stdio_server/test"
expect 200 "test the stdio tool server"
[[ "$(field '[.data.server.tools[].name] | sort | join(",")')" == "add,echo" ]] ||
  fail "the stdio server should list add and echo: $body"
pass "stdio tool server: Test starts fake-llm --stdio and lists echo and add"

request POST /chats "{\"modelId\":\"$openai_model\",\"toolOverrides\":[{\"serverId\":\"$stdio_server\",\"enabled\":true}]}"
expect 201 "create a chat with the stdio tool server"
stdio_chat="$(field .data.id)"
stream "/chats/$stdio_chat/messages" '{"text":"tool echo {\"text\":\"over stdio\"}"}'
check "stdio: tool echo asks for approval" '
  any(.[]; .type == "tool.requested" and .toolName == "echo" and .needsApproval)
  and any(.[]; .type == "message.finished" and .status == "awaiting_approval")'
message="$(jq -er '.[] | select(.type == "tool.requested") | .messageId' <<<"$events")"
call="$(jq -er '.[] | select(.type == "tool.requested") | .callId' <<<"$events")"
stream "/chats/$stdio_chat/messages/$message/decisions" "{\"decisions\":[{\"callId\":\"$call\",\"decision\":\"approve\"}]}"
check "stdio: the approved call runs in the program and the model reads its result" '
  any(.[]; .type == "tool.finished" and .status == "ok"
    and .output == [{"type": "text", "text": "over stdio"}])
  and any(.[]; .type == "message.finished" and .status == "complete")'
[[ "$(text)" == "Tool said: over stdio" ]] || fail "after the stdio tool: streamed $(text)"
for key in mcp.stdio.started mcp.stdio.stopped; do
  grep -q "$key" "$RUN_DIR/api.log" || fail "the API log should show $key"
done
pass "stdio: the answer after the tool is Tool said: over stdio, and the process ended with the run"

# --- 4. The Responses API ---------------------------------------------------------------------

# The `openai` preset without a chosen API uses Responses: typed events, usage in the terminal
# event, and stateless replay. The fake refuses a replayed reasoning item whose encrypted content
# it did not give, so the tool round below also checks that Copilot sends it back verbatim.
connect openai openai "Fake OpenAI Responses" "$FAKE/v1"
[[ "$connected_api" == responses ]] || fail "the openai preset should default to the Responses API"
responses_model="$model"
chat responses

request POST /chats "{\"modelId\":\"$responses_model\"}"
expect 201 "create a responses chat"
stream "/chats/$(field .data.id)/messages" '{"text":"refuse"}'
check "responses: refuse ends refused with the model's explanation" '
  any(.[]; .type == "message.finished" and .status == "refused" and .finishReason == "refusal"
    and .refusal.explanation == "I can'"'"'t help with that.")
  and (map(.type) | last == "done")'

request POST /chats "{\"modelId\":\"$responses_model\",\"toolOverrides\":[{\"serverId\":\"$server\",\"enabled\":true}]}"
expect 201 "create a responses chat with the tool server"
tool_chat="$(field .data.id)"
stream "/chats/$tool_chat/messages" '{"text":"think tool echo {\"text\":\"over responses\"}"}'
check "responses: think streams a reasoning summary, then the call asks for approval" '
  any(.[]; .type == "reasoning.delta")
  and any(.[]; .type == "tool.requested" and .toolName == "echo" and .needsApproval
    and .arguments == {"text": "over responses"})
  and any(.[]; .type == "message.finished" and .status == "awaiting_approval")'
message="$(jq -er '.[] | select(.type == "tool.requested") | .messageId' <<<"$events")"
call="$(jq -er '.[] | select(.type == "tool.requested") | .callId' <<<"$events")"
stream "/chats/$tool_chat/messages/$message/decisions" "{\"decisions\":[{\"callId\":\"$call\",\"decision\":\"approve\"}]}"
check "responses: the approved call runs and the replayed round reads its result" '
  any(.[]; .type == "tool.finished" and .status == "ok")
  and any(.[]; .type == "message.finished" and .status == "complete")'
[[ "$(text)" == "Tool said: over responses" ]] || fail "responses after the tool: streamed $(text)"
grep -q 'fake-llm.responses.reasoning-replayed' "$RUN_DIR/fake-llm.log" ||
  fail "the round after the approval should replay the encrypted reasoning"
pass "responses: the encrypted reasoning went back verbatim (Tool said: over responses)"

# A decision after the chat moved to another model on the same connection: the paused answer's
# items (encrypted reasoning from fake-chat) must not reach fake-audio, which the fake refuses as
# OpenAI does ("Reasoning input items can only be provided to a reasoning ... model"), so the
# answer goes back rebuilt from its parts.
request GET "/providers/$connected_provider/models"
expect 200 "list the responses models"
audio_model="$(field '.items[] | select(.remoteId == "fake-audio") | .id')"
request PATCH "/models/$audio_model" '{"enabled":true}'
expect 200 "enable fake-audio (responses)"
request POST /chats "{\"modelId\":\"$responses_model\",\"toolOverrides\":[{\"serverId\":\"$server\",\"enabled\":true}]}"
expect 201 "create a responses chat that switches models"
switch_chat="$(field .data.id)"
stream "/chats/$switch_chat/messages" '{"text":"think tool echo {\"text\":\"after a switch\"}"}'
check "responses: the call before the switch asks for approval" '
  any(.[]; .type == "message.finished" and .status == "awaiting_approval")'
message="$(jq -er '.[] | select(.type == "tool.requested") | .messageId' <<<"$events")"
call="$(jq -er '.[] | select(.type == "tool.requested") | .callId' <<<"$events")"
request PATCH "/chats/$switch_chat" "{\"modelId\":\"$audio_model\"}"
expect 200 "switch the paused chat to fake-audio"
stream "/chats/$switch_chat/messages/$message/decisions" "{\"decisions\":[{\"callId\":\"$call\",\"decision\":\"approve\"}]}"
check "responses: the decision after a model switch completes on the new model" '
  any(.[]; .type == "tool.finished" and .status == "ok")
  and any(.[]; .type == "message.finished" and .status == "complete")'
[[ "$(text)" == "Tool said: after a switch" ]] || fail "responses after a model switch: streamed $(text)"
pass "responses: a model switch sends the paused answer rebuilt, without the other model's reasoning"

# --- 5. Anthropic -----------------------------------------------------------------------------

connect anthropic custom "Fake Anthropic" "$FAKE"
chat anthropic

pass "all steps"
