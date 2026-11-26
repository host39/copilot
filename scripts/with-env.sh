#!/usr/bin/env bash
# Runs a command with variables from an env file, like `node --env-file`.
# Variables already set in the shell win over the file.
#
# Usage: scripts/with-env.sh <env-file> <command> [args...]
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: scripts/with-env.sh <env-file> <command> [args...]" >&2
  exit 1
fi

env_file="$1"
shift

if [[ ! -f "$env_file" ]]; then
  echo "scripts.with-env.not-found $env_file (run \`make setup\` to create it)" >&2
  exit 1
fi

while IFS= read -r line || [[ -n "$line" ]]; do
  line="${line#"${line%%[![:space:]]*}"}"
  [[ -z "$line" || "$line" == \#* || "$line" != *=* ]] && continue
  key="${line%%=*}"
  value="${line#*=}"
  [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || continue
  if [[ "$value" =~ ^\"(.*)\"$ || "$value" =~ ^\'(.*)\'$ ]]; then
    value="${BASH_REMATCH[1]}"
  fi
  if [[ -z "${!key+x}" ]]; then
    export "$key=$value"
  fi
done <"$env_file"

exec "$@"
