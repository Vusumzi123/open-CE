#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
game=${1:-${OCE_SKYRIM_SE:-}}

find_game() {
    local steam library

    for steam in \
        "$HOME/.steam/steam" \
        "$HOME/.local/share/Steam" \
        "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam"; do
        for library in \
            "$steam/steamapps/common/Skyrim Special Edition" \
            "$steam/SteamApps/common/Skyrim Special Edition"; do
            if [[ -f "$library/Data/Skyrim.esm" ]]; then
                printf '%s\n' "$library"
                return 0
            fi
        done

        [[ -f "$steam/steamapps/libraryfolders.vdf" ]] || continue
        while IFS= read -r library; do
            [[ -n "$library" ]] || continue
            library="$library/steamapps/common/Skyrim Special Edition"
            if [[ -f "$library/Data/Skyrim.esm" ]]; then
                printf '%s\n' "$library"
                return 0
            fi
        done < <(sed -n 's/.*"path"[[:space:]]*"\(.*\)".*/\1/p' "$steam/steamapps/libraryfolders.vdf")
    done

    return 1
}

if [[ -z "$game" ]]; then
    game=$(find_game || true)
fi

if [[ -z "$game" ]] || { [[ ! -f "$game/Skyrim.esm" ]] && [[ ! -f "$game/Data/Skyrim.esm" ]]; }; then
    printf 'Skyrim Special Edition not found.\n' >&2
    printf 'Usage: scripts/setup.sh [/path/to/Skyrim Special Edition]\n' >&2
    exit 1
fi

printf -v quoted '%q' "$game"
printf 'export OCE_SKYRIM_SE=%s\n' "$quoted" >"$root/.env"
printf 'Wrote %s/.env\n' "$root"
