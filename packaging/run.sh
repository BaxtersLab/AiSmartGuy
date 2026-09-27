#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# AiSmartGuy — packaged launcher.
#
# DISPLAY BACKEND: GDK_BACKEND=x11, as in the dev run.sh (A1 section 3 rule 6).
# Under Wayland this window gets GTK client-side decorations, and GTK hit-tests
# the titlebar about 26px right of where it draws it (the CSD shadow inset), so
# the close button does nothing unless the window is maximized. Measured
# 2026-08-05 (handoffs.md), and reproduced with THIS launcher in the VM harness
# on 2026-09-27: two clicks on the drawn close button and one at +26px did
# nothing, the maximized one closed it; with GDK_BACKEND=x11 mutter draws the
# titlebar and one click closes it. An earlier version of this comment said
# Wayland "already works"; it did not. No screen capture here, so XWayland
# costs nothing.
#
# The dev run.sh looks in target/release then target/debug. Installed there is
# no target/ at all -- the binary sits beside this script -- so the installed
# path is checked FIRST. Preferring a dev build over the packaged one is how
# "stale builds lie" bites, and it has cost this estate a debugging round before.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"

unset ELECTRON_RUN_AS_NODE
export GDK_BACKEND=x11

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
