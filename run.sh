#!/usr/bin/env bash
# AiSmartGuy — Linux launcher.
#
# Two reasons this exists rather than running the binary directly.
#
# 1. THE SCHEMA SHIM. GNOME 50 moved `antialiasing`, `hinting` and `rgba-order`
#    out of org.gnome.settings-daemon.plugins.xsettings into a `.deprecated`
#    schema. WebKitGTK 2.52.3 still reads `antialiasing` from the original id,
#    and a missing GSettings key is FATAL, so every Tauri app on this box
#    aborts at startup:
#
#      GLib-GIO-ERROR: Settings schema
#      'org.gnome.settings-daemon.plugins.xsettings'
#      does not contain a key named 'antialiasing'
#
#    schema-shim/ redefines that id as a superset so the lookup succeeds.
#    Nothing outside this folder is touched and no root is needed — see
#    schema-shim/*.xml for the full story and the test for when it can be
#    dropped.
#
# 2. VS CODE SNAP CONTAMINATION. The editor's terminal exports variables that
#    follow every child process:
#
#      ELECTRON_RUN_AS_NODE   starts an Electron child as a bare Node runtime
#      GSETTINGS_SCHEMA_DIR   points at the snap's own partial schema set
#      XDG_DATA_HOME          points INSIDE the snap sandbox
#                             (/home/<user>/snap/code/<rev>/.local/share)
#
#    That last one is the nastiest, because nothing crashes. Tauri derives
#    `app_local_data_dir()` from XDG_DATA_HOME, so launched from a VS Code
#    terminal this app quietly keeps its model library, saved settings and
#    output reports inside the snap — a different, empty directory that also
#    changes whenever VS Code updates its revision. Models put in the real
#    ~/.local/share location are simply invisible, with no error to explain it.
#
#    Unsetting is right rather than hardcoding: XDG_DATA_HOME then falls back to
#    the spec default of ~/.local/share, and a genuine system-wide override
#    still works when the app is started from the launcher instead.
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

unset ELECTRON_RUN_AS_NODE

# 3. XWAYLAND, DELIBERATELY. This used to `unset GDK_BACKEND` on the grounds
#    that VS Code's snap leaks GDK_BACKEND=x11 and that forcing a Wayland app
#    onto XWayland is contamination. For THIS app it is the fix, and the
#    difference was measured rather than guessed:
#
#    Under Wayland, GTK3 must use client-side decorations (mutter advertises no
#    zxdg_decoration_manager_v1, and GTK3 does not implement it anyway). GTK
#    then draws a ~26px drop shadow around the window and lays the titlebar out
#    against the SURFACE width while pointer events arrive in WINDOW
#    coordinates. The close button's hit area ends up ~26px right of where it
#    is painted, so the ✕ does nothing — a double-click on it MAXIMIZES,
#    because GTK sees titlebar background there. Maximizing zeroes the shadow
#    inset, which is why the button starts working only after the operator
#    enlarges the window. Verified from a WAYLAND_DEBUG=1 pointer trace:
#      unmaximized geometry (26,23,960,847): clicks at surface x 962-969 -> nothing
#      maximized   geometry (0,0,1853,1048): click at surface x 1822.9   -> CloseRequested
#
#    On XWayland the window gets SERVER-SIDE decorations (no _GTK_FRAME_EXTENTS
#    property; mutter draws the titlebar), so there is no client shadow and no
#    offset. Confirmed on hardware: one click on ✕ -> CloseRequested -> Destroyed.
#
#    Three things come with it, all improvements or neutral:
#      * the ✕ works without maximizing first
#      * a minimize button exists (mutter's titlebar; under Wayland CSD the
#        GNOME button-layout 'appmenu:close' means there is none at all)
#      * center() actually works — a Wayland client may not position itself,
#        which is why the window used to open half off the bottom edge
#      * the splash loses a drop shadow it never asked for and that GTK wrongly
#        declared as part of its window geometry. This is the glitch going
#        away; do not "restore" it.
#
#    Safe for this app: no screen capture (unlike SOC Ultralight, where an X11
#    fallback silently captures black) and no HiDPI concern at 1920x1080 @1x.
#    Remove this line to go back to native Wayland and the broken ✕.
export GDK_BACKEND=x11

# Scrub EVERY variable that points into a snap, not a hand-picked few.
#
# The original list was GDK_BACKEND + three XDG_* vars. A `env | grep -i snap`
# from a VS Code terminal on this box returns TWENTY-ONE, and the ones that
# were being missed sit squarely in GTK's own load path:
#
#   GTK_PATH             GTK theme/module engines
#   GTK_IM_MODULE_FILE   input-method module cache  <- in the INPUT path
#   GIO_MODULE_DIR       GIO modules
#   GDK_PIXBUF_MODULE_FILE / GDK_PIXBUF_MODULEDIR   image loaders
#   LOCPATH              snap locales built against a different glibc
#
# LOCPATH is the loudest: it drags /snap/core20's glibc into the process and
# produces `symbol lookup error: ... undefined symbol: __libc_pthread_init`.
# The rest fail quietly, which is worse.
#
# XDG_DATA_DIRS is filtered rather than unset — it legitimately holds system
# paths that must survive; only the snap entries are removed.
#
# BY VALUE, over every exported variable, and on EVERY element of a colon
# list (2026-09-27 owner review). The old two loops caught only values that
# START with /snap/, plus a fixed name list that did not include
# GSETTINGS_SCHEMA_DIR -- which VS Code exports under $HOME/snap/. The shim
# below overwrites it only when gschemas.compiled exists, and that file is
# git-ignored, so in a fresh clone the snap's schema dir survived. PATH is
# exempt: /snap/bin on it is legitimate and shadows nothing.
while IFS= read -r -d '' _entry; do
    _name="${_entry%%=*}"
    _value="${_entry#*=}"
    case "$_name" in
        PATH|XDG_DATA_DIRS) continue ;;
    esac
    [[ "$_name" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || continue
    case ":$_value" in
        *:/snap/*|*":${HOME:-/nonexistent}/snap/"*) unset "$_name" ;;
    esac
done < <(env -0)
unset _entry _name _value

if [[ -n "${XDG_DATA_DIRS:-}" ]]; then
    _clean=""
    IFS=':' read -ra _parts <<< "$XDG_DATA_DIRS"
    for _p in "${_parts[@]}"; do
        case "$_p" in
            ""|/snap/*|"${HOME:-/nonexistent}"/snap/*) continue ;;
        esac
        _clean="${_clean:+$_clean:}$_p"
    done
    export XDG_DATA_DIRS="${_clean:-/usr/local/share:/usr/share}"
    unset _clean _parts _p
fi

SHIM="$PWD/schema-shim"
if [[ -f "$SHIM/gschemas.compiled" ]]; then
    # Overwrite rather than append: the inherited value may be the VS Code
    # snap's schema dir, which is exactly what must not be searched first.
    export GSETTINGS_SCHEMA_DIR="$SHIM"
else
    echo "[aismartguy] schema-shim not compiled — run: glib-compile-schemas schema-shim/"
    echo "[aismartguy] launching anyway; expect a GLib-GIO-ERROR abort on GNOME 50."
fi

APP="./target/release/aismartguy-app"
[[ -x "$APP" ]] || APP="./target/debug/aismartguy-app"
if [[ ! -x "$APP" ]]; then
    echo "[aismartguy] no binary found — build first:"
    echo "[aismartguy]   cargo build --manifest-path src-tauri/Cargo.toml"
    exit 1
fi

echo "[aismartguy] launching $APP"
exec "$APP" "$@"
