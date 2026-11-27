#!/bin/sh
# Backs up a compose install of Copilot (docs/self-hosting.md, "Backups"): one `pg_dump -Fc` of
# the `copilot` database, which is the whole backup because file bytes live in Postgres. Keep
# SECRET_KEY (deploy/.env) with the dumps, or the stored provider keys cannot be read after a
# restore.
#
#   deploy/backup.sh [directory]
#
# The dump goes to the directory (default: BACKUP_DIR, else `backups` next to deploy/) as
# copilot-<UTC time>.dump, readable by its owner only. It is written under a temporary name,
# checked with `pg_restore --list`, and only then renamed, so a failed or partial dump never
# looks like a backup. Dumps older than BACKUP_KEEP_DAYS days (default 14) are deleted afterwards;
# the newest dump always stays. COMPOSE_FILE and an override file work as for any compose command.
#
# Run it from cron or a systemd timer, and copy the directory off the machine. It needs only
# Docker with Compose v2 and a POSIX shell.
set -eu

deploy="$(cd "$(dirname "$0")" && pwd)"
dir="${1:-${BACKUP_DIR:-$deploy/../backups}}"
keep="${BACKUP_KEEP_DAYS:-14}"

case "$keep" in
  '' | *[!0-9]*)
    echo "deploy.backup.failed BACKUP_KEEP_DAYS must be a whole number of days" >&2
    exit 1
    ;;
esac

compose() {
  if [ -n "${COMPOSE_FILE:-}" ]; then
    docker compose "$@"
  else
    docker compose -f "$deploy/compose.yml" "$@"
  fi
}

umask 077
mkdir -p "$dir"
dir="$(cd "$dir" && pwd)"
name="copilot-$(date -u +%Y%m%dT%H%M%SZ).dump"
partial="$dir/.$name.partial"
trap 'rm -f "$partial"' EXIT

compose exec -T postgres pg_dump -U copilot -Fc copilot >"$partial"
if ! compose exec -T postgres pg_restore --list <"$partial" >/dev/null; then
  echo "deploy.backup.failed the dump is not readable by pg_restore" >&2
  exit 1
fi
mv "$partial" "$dir/$name"
echo "deploy.backup.written $dir/$name $(wc -c <"$dir/$name") bytes"

# Retention: older dumps go, never the one just written.
find "$dir" -maxdepth 1 -type f -name 'copilot-*.dump' ! -name "$name" -mtime +"$keep" \
  -print -exec rm -f {} + | sed 's/^/deploy.backup.deleted /'
