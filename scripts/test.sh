#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if [[ -f "$root/.env" ]]; then
    # shellcheck disable=SC1091
    source "$root/.env"
else
    printf 'note: %s/.env not found; real-data tests will skip\n' "$root" >&2
fi

cd "$root"
exec cargo test "$@"
