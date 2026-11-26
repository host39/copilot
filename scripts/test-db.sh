#!/usr/bin/env bash
# Runs the tests that need Postgres (marked `#[ignore]`). `make test-db` calls this with
# `apps/api/.env.local` loaded.
#
# DATABASE_URL points at the `postgres` maintenance database on the DB_* server. `#[sqlx::test]`
# connects there and creates a throwaway database per test, so tests never touch DB_NAME (the
# `copilot` database that `make migrate` and the dev server use).
#
# Usage: scripts/with-env.sh apps/api/.env.local bash scripts/test-db.sh [cargo test args...]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for name in DB_HOST DB_PORT DB_USER DB_PASSWORD; do
  if [[ -z "${!name:-}" ]]; then
    echo "scripts.test-db.env-missing $name (run \`make setup\` or set it in apps/api/.env.local)" >&2
    exit 1
  fi
done

# Percent-encodes one URL component byte by byte.
encode() {
  local LC_ALL=C value="$1" out="" char index
  for ((index = 0; index < ${#value}; index++)); do
    char="${value:index:1}"
    case "$char" in
      [A-Za-z0-9._~-]) out+="$char" ;;
      *) printf -v char '%%%02X' "'$char" && out+="$char" ;;
    esac
  done
  printf '%s' "$out"
}

export DATABASE_URL="postgres://$(encode "$DB_USER"):$(encode "$DB_PASSWORD")@${DB_HOST}:${DB_PORT}/postgres"

exec cargo test --workspace "$@" -- --ignored
