#!/usr/bin/env bash
# Runs the Bruno collection against the API. Starts the API with `apps/api/.env.local` when it is
# not already listening, and stops it afterwards.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BASE_URL="${BASE_URL:-http://localhost:3000}"
RUN_DIR="apps/api/tests/bruno/.run"
mkdir -p "$RUN_DIR"

api_pid=""
cleanup() {
  if [[ -n "$api_pid" ]]; then
    kill "$api_pid" 2>/dev/null || true
    wait "$api_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if ! curl -sf "$BASE_URL/health" >/dev/null; then
  cargo build -q -p copilot-api --bin server
  bash scripts/with-env.sh apps/api/.env.local target/debug/server >"$RUN_DIR/api.log" 2>&1 &
  api_pid=$!

  for _ in $(seq 1 50); do
    curl -sf "$BASE_URL/health" >/dev/null && break
    sleep 0.2
  done

  if ! curl -sf "$BASE_URL/health" >/dev/null; then
    echo "scripts.test-bruno.api-not-ready $BASE_URL" >&2
    tail -n 20 "$RUN_DIR/api.log" >&2 || true
    exit 1
  fi
fi

cd apps/api/tests/bruno
"$ROOT/node_modules/.bin/bru" run --env development
