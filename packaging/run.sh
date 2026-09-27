#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# AiSmartGuy — packaged launcher.
#
# DISPLAY BACKEND: GDK_BACKEND is deliberately NOT set. This is a Tauri app on
# WebKitGTK, which runs natively on Wayland; forcing XWayland would add a
# translation layer to a path that already works.
#
# The dev run.sh looks in target/release then target/debug. Installed there is
# no target/ at all -- the binary sits beside this script -- so the installed
# path is checked FIRST. Preferring a dev build over the packaged one is how
# "stale builds lie" bites, and it has cost this estate a debugging round before.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"

unset ELECTRON_RUN_AS_NODE

_strip_snap_list() {
    local IFS=':' out=() part
    for part in $1; do
        [[ "$part" == */snap/* ]] || out+=("$part")
    done
    local joined; printf -v joined '%s:' "${out[@]}"
    printf '%s' "${joined%:}"
}
[[ "${PATH:-}" == */snap/* ]] && PATH="$(_strip_snap_list "$PATH")" && export PATH
[[ "${XDG_DATA_DIRS:-}" == */snap/* ]] \
    && XDG_DATA_DIRS="$(_strip_snap_list "$XDG_DATA_DIRS")" && export XDG_DATA_DIRS
while IFS='=' read -r _name _value; do
    case "$_name" in
        PATH|XDG_DATA_DIRS) continue ;;
        *) [[ "$_value" == */snap/* ]] && unset "$_name" ;;
    esac
done < <(env)

APP=""
for candidate in "$PWD/aismartguy-app" "$PWD/target/release/aismartguy-app"; do
    [[ -x "$candidate" ]] && { APP="$candidate"; break; }
done
if [[ -z "$APP" ]]; then
    echo "[aismartguy] no binary found beside this launcher."
    exit 1
fi
exec "$APP" "$@"
