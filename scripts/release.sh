#!/usr/bin/env bash
# Release helpers, used by the release jobs (image.yml publishes the image, native.yml the GitHub
# Release with the app bundles) and by whoever prepares a release.
#
#   bash scripts/release.sh version          # the release version; fails when its two sources differ
#   bash scripts/release.sh check v0.1.0     # fails unless the tag is v<version>
#   bash scripts/release.sh tags 0.1.0       # the image tags: 0.1.0, 0.1, and latest (not for 0.2.0-rc.1)
#   bash scripts/release.sh notes 0.1.0      # that version's CHANGELOG.md section, for the release body
#
# The version lives in the root Cargo.toml (`workspace.package.version`) and in
# apps/native/package.json (which Tauri reads); CHANGELOG.md explains the policy. `notes` falls
# back to the Unreleased section, with a warning, when the version has no section yet. Needs jq.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() {
  echo "scripts.release.failed $*" >&2
  exit 1
}

warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
    echo "::warning title=Release::$*" >&2
  else
    echo "scripts.release.warning $*" >&2
  fi
}

version() {
  local cargo native
  cargo="$(awk '
    /^\[workspace\.package\]/ { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && /^version[[:space:]]*=/ { gsub(/[" ]/, "", $0); sub(/^version=/, "", $0); print; exit }
  ' "$ROOT/Cargo.toml")"
  native="$(jq -r .version "$ROOT/apps/native/package.json")"
  [[ -n "$cargo" ]] || fail "no workspace.package.version in Cargo.toml"
  [[ "$cargo" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || fail "Cargo.toml version $cargo is not MAJOR.MINOR.PATCH"
  [[ "$cargo" == "$native" ]] ||
    fail "Cargo.toml says $cargo but apps/native/package.json says $native: set both to the release version"
  echo "$cargo"
}

check() {
  local tag="${1:?a tag, for example v0.1.0}" expected
  expected="v$(version)"
  [[ "$tag" == "$expected" ]] || fail "the tag $tag does not name this commit's version ($expected)"
  echo "$tag"
}

tags() {
  local version="${1:?a version, for example 0.1.0}"
  [[ "$version" =~ ^([0-9]+)\.([0-9]+)\.[0-9]+(-.+)?$ ]] || fail "$version is not MAJOR.MINOR.PATCH"
  echo "$version"
  # A pre-release (0.2.0-rc.1) moves neither the minor tag nor latest.
  if [[ -z "${BASH_REMATCH[3]}" ]]; then
    echo "${BASH_REMATCH[1]}.${BASH_REMATCH[2]}"
    echo latest
  fi
}

# The lines under `## [<name>]` up to the next `## ` heading, without leading or trailing blanks.
section() {
  awk -v heading="## [$1]" '
    index($0, heading) == 1 { inside = 1; next }
    inside && /^## / { exit }
    inside { lines[++count] = $0 }
    END {
      first = 1; while (first <= count && lines[first] ~ /^[[:space:]]*$/) first++
      last = count; while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
      for (i = first; i <= last; i++) print lines[i]
    }
  ' "$ROOT/CHANGELOG.md"
}

notes() {
  local version="${1:?a version, for example 0.1.0}" body
  body="$(section "$version")"
  if [[ -z "$body" ]]; then
    warn "CHANGELOG.md has no section for $version; the release notes use the Unreleased section"
    body="$(section Unreleased)"
  fi
  [[ -n "$body" ]] || fail "CHANGELOG.md has neither a $version nor an Unreleased section"
  printf '%s\n\n' "$body"
  echo "Upgrade steps: [docs/self-hosting.md](https://github.com/host39/copilot/blob/v$version/docs/self-hosting.md#upgrades). Image: \`ghcr.io/host39/copilot:$version\`. Check downloads with \`sha256sum -c SHA256SUMS\`."
}

command -v jq >/dev/null || fail "missing jq"
case "${1:-}" in
  version) version ;;
  check) check "${2:-}" ;;
  tags) tags "${2:-}" ;;
  notes) notes "${2:-}" ;;
  *) fail "usage: scripts/release.sh version | check <tag> | tags <version> | notes <version>" ;;
esac
