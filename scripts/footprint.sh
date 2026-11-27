#!/usr/bin/env bash
# Footprint budgets (roadmap P6.3): each check prints what it measured, adds it to the CI run
# summary, and fails when a size passes its budget. The budgets sit a little above the sizes
# measured when they were set, so growth needs a deliberate change to this file.
#
#   bash scripts/footprint.sh binaries target/release  # the release `server` and `migrate`
#   bash scripts/footprint.sh web apps/web/dist         # gzip of the JS and CSS index.html loads
#   bash scripts/footprint.sh image copilot:local       # the Copilot image
#
# The CI jobs run it after their builds: binaries in the Rust job, web in the lint job, and the
# image in image.yml.
set -euo pipefail

# Budgets in bytes. Measured at P6.1: server 30,867,112 and migrate 7,181,744 (CI's release
# profile), first load 152.8 kB gzip, and the image 44.5 MB as `docker image inspect` reports it
# on the runner.
SERVER_MAX=$((36 * 1024 * 1024))
MIGRATE_MAX=$((9 * 1024 * 1024))
WEB_FIRST_LOAD_MAX=$((170 * 1000))
IMAGE_MAX=$((55 * 1000 * 1000))

rows=()
failed=0

# Records one measurement against its budget.
check() {
  local name="$1" bytes="$2" max="$3" verdict="ok"
  if ((bytes > max)); then
    verdict="over budget"
    failed=1
    echo "::error::$name is $bytes bytes, over its budget of $max (scripts/footprint.sh)"
  fi
  echo "$name: $bytes bytes (budget $max, $verdict)"
  rows+=("| $name | $bytes | $max | $verdict |")
}

binaries() {
  local dir="$1"
  check "server binary" "$(stat -c %s "$dir/server")" "$SERVER_MAX"
  check "migrate binary" "$(stat -c %s "$dir/migrate")" "$MIGRATE_MAX"
}

# The scripts and stylesheets index.html loads before anything renders, gzip -9 each, as a
# browser would receive them compressed.
web() {
  local dist="$1" total=0 path bytes
  local assets
  assets="$(grep -oE '(src|href)="/[^"]+\.(js|css)"' "$dist/index.html" | sed -E 's/^(src|href)="\///; s/"$//')"
  [[ -n "$assets" ]] || {
    echo "::error::no scripts or stylesheets found in $dist/index.html"
    exit 1
  }
  while IFS= read -r path; do
    bytes="$(gzip -9c "$dist/$path" | wc -c)"
    echo "  $path: $bytes bytes gzip"
    total=$((total + bytes))
  done <<<"$assets"
  check "web first load (gzip)" "$total" "$WEB_FIRST_LOAD_MAX"
}

image() {
  check "image" "$(docker image inspect -f '{{.Size}}' "$1")" "$IMAGE_MAX"
}

case "${1:-}" in
  binaries) binaries "${2:?the folder with the release binaries}" ;;
  web) web "${2:?the web build folder}" ;;
  image) image "${2:?the image name}" ;;
  *)
    echo "usage: bash scripts/footprint.sh binaries|web|image <path or image>" >&2
    exit 2
    ;;
esac

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "| Footprint | Bytes | Budget | Result |"
    echo "| --- | --- | --- | --- |"
    printf '%s\n' "${rows[@]}"
  } >>"$GITHUB_STEP_SUMMARY"
fi
exit "$failed"
