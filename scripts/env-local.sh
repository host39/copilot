#!/usr/bin/env bash
# Creates the local env files from their samples when they are missing, then gives the API a
# generated SECRET_KEY when its key is blank or absent. Existing values are never changed.
#
#   apps/api/.env.sample  -> apps/api/.env.local
#   apps/web/.env.example -> apps/web/.env.local
#
# `make setup`, `scripts/worktree.sh`, and CI run it.
#
# Usage: bash scripts/env-local.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

API_ENV="apps/api/.env.local"
WEB_ENV="apps/web/.env.local"

[[ -f "$API_ENV" ]] || cp apps/api/.env.sample "$API_ENV"
[[ -f "$WEB_ENV" ]] || cp apps/web/.env.example "$WEB_ENV"

# 32 random bytes in standard base64, like `openssl rand -base64 32`.
key="$(head -c 32 /dev/urandom | base64 | tr -d '\n')"

if grep -q '^SECRET_KEY=..*' "$API_ENV"; then
  exit 0
elif grep -q '^SECRET_KEY=' "$API_ENV"; then
  sed -i.bak "s|^SECRET_KEY=.*|SECRET_KEY=$key|" "$API_ENV" && rm -f "$API_ENV.bak"
else
  printf '\nSECRET_KEY=%s\n' "$key" >>"$API_ENV"
fi
echo "scripts.env-local.secret-key-generated $API_ENV" >&2
