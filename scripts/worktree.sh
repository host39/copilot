#!/usr/bin/env bash
# Creates the worktree for one Phase 2 track (see docs/design/tracks/README.md):
#
#   ../copilot-<track> on branch claude/copilot-p2-<track>, branched from the p1-frozen tag
#
# then installs the pnpm dependencies, copies the env samples, and gives the worktree its own API
# port (the lowest free one from 3001 up, skipping ports other worktrees already use). The web
# client and `make test-bruno` follow that port, so tracks run their servers side by side.
#
# Usage: bash scripts/worktree.sh <track> [base]   (base defaults to p1-frozen)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIRST_PORT=3001

track="${1:-}"
base="${2:-p1-frozen}"

if [[ ! "$track" =~ ^[a-z][a-z0-9-]*$ ]]; then
  echo "Usage: bash scripts/worktree.sh <track> [base]   (track: lowercase, for example a)" >&2
  exit 1
fi

dir="$(dirname "$ROOT")/copilot-$track"
branch="claude/copilot-p2-$track"

if [[ -e "$dir" ]]; then
  echo "scripts.worktree.exists $dir" >&2
  exit 1
fi
if ! git -C "$ROOT" rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
  echo "scripts.worktree.base-not-found $base (fetch it with \`git fetch --tags\`)" >&2
  exit 1
fi
if git -C "$ROOT" rev-parse --verify --quiet "refs/heads/$branch" >/dev/null; then
  echo "scripts.worktree.branch-exists $branch (resume it with \`git worktree add $dir $branch\`)" >&2
  exit 1
fi

# PORT values already set in the API env file of any existing worktree, the main one included.
used_ports() {
  git -C "$ROOT" worktree list --porcelain |
    sed -n 's/^worktree //p' |
    while IFS= read -r path; do
      sed -n 's/^PORT=//p' "$path/apps/api/.env.local" 2>/dev/null || true
    done
}

port=$FIRST_PORT
used="$(used_ports)"
while grep -qx "$port" <<<"$used"; do
  port=$((port + 1))
done

git -C "$ROOT" worktree add "$dir" -b "$branch" "$base"
cd "$dir"

pnpm install --frozen-lockfile

cp apps/api/.env.sample apps/api/.env.local
cp apps/web/.env.example apps/web/.env.local

# Sets KEY=value in an env file, replacing an existing line or appending one.
set_var() {
  local file="$1" key="$2" value="$3"
  if grep -q "^$key=" "$file"; then
    sed -i.bak "s|^$key=.*|$key=$value|" "$file" && rm -f "$file.bak"
  else
    printf '%s=%s\n' "$key" "$value" >>"$file"
  fi
}

set_var apps/api/.env.local PORT "$port"
set_var apps/web/.env.local PUBLIC_API_URL "http://localhost:$port"

cat <<EOF
Worktree ready: $dir
  branch:   $branch (from $base)
  API port: $port (apps/api/.env.local)
  web:      PUBLIC_API_URL=http://localhost:$port (apps/web/.env.local)
  Bruno:    make test-bruno starts this worktree's API and tests http://localhost:$port
Next: cd $dir, then create docs/design/tracks/$track.md from the template in docs/design/tracks/README.md.
EOF
