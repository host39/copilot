#!/usr/bin/env bash
# Scaffolds an empty reversible migration pair for sqlx:
#   migrations/<UTC timestamp>_<name>.up.sql
#   migrations/<UTC timestamp>_<name>.down.sql
#
# Usage: bash packages/database/scripts/generate-migration.sh <name>   (or `make migrations name=<name>`)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIGRATIONS_DIR="$ROOT_DIR/migrations"

name="${1:-}"
if [[ -z "$name" ]]; then
  echo "Usage: make migrations name=<snake_case_name>" >&2
  exit 1
fi
if [[ ! "$name" =~ ^[a-z][a-z0-9_]*$ ]]; then
  echo "database.migrations.invalid-name $name (use snake_case, for example create_widgets)" >&2
  exit 1
fi

timestamp="$(date -u +%Y%m%d%H%M%S)"
up="$MIGRATIONS_DIR/${timestamp}_${name}.up.sql"
down="$MIGRATIONS_DIR/${timestamp}_${name}.down.sql"

mkdir -p "$MIGRATIONS_DIR"
printf -- '-- %s: apply. Order columns per .ai/rules/postgres-column-alignment.md.\n' "$name" >"$up"
printf -- '-- %s: revert everything the .up.sql file does.\n' "$name" >"$down"

echo "Created:"
echo "  ${up#"$ROOT_DIR/"}"
echo "  ${down#"$ROOT_DIR/"}"
