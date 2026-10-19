#!/usr/bin/env bash
# Guards the sqlx migrations embedded by `sqlx::migrate!()`.
#
# sqlx records a checksum per applied version in `_sqlx_migrations` and refuses
# to start when the embedded file for that version differs. So:
#   1. every version number must be unique (two branches both adding "0006_*"
#      is how revision 26 broke on know-me), and
#   2. a migration that has reached the base branch must never be modified,
#      renamed or deleted; schema changes go in a new, higher-numbered file.
#
# Usage: scripts/check-migrations.sh [BASE_REF]
# With BASE_REF (e.g. origin/main) the immutability check runs against it.
# Set ALLOW_MIGRATION_CHANGES=1 to skip the immutability check for a reviewed
# renumbering of migrations that never reached production.
set -euo pipefail

dir="crates/flint-gate-core/migrations"
status=0

dupes="$(find "$dir" -maxdepth 1 -name '*.sql' -exec basename {} \; \
  | sed -n 's/^\([0-9]\{1,\}\)_.*/\1/p' | sort | uniq -d)"
if [ -n "$dupes" ]; then
  for v in $dupes; do
    echo "::error::duplicate migration version $v:" >&2
    find "$dir" -maxdepth 1 -name "${v}_*.sql" >&2
  done
  status=1
fi

base="${1:-}"
if [ -n "$base" ]; then
  if [ "${ALLOW_MIGRATION_CHANGES:-0}" = "1" ]; then
    echo "ALLOW_MIGRATION_CHANGES=1: skipping immutability check against $base"
  else
    changed="$(git diff --name-status -M "$base"...HEAD -- "$dir" | grep -v '^A' || true)"
    if [ -n "$changed" ]; then
      echo "::error::migrations already on $base were modified, renamed or deleted; add a new migration instead:" >&2
      echo "$changed" >&2
      status=1
    fi
  fi
fi

[ "$status" -eq 0 ] && echo "migrations OK"
exit "$status"
