#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# baxters-aismartguy — build the .deb from a release build of this tree.
#
# Tauri, but a simple one: tauri.conf.json sets frontendDist to ../frontend and
# has NO beforeBuildCommand, so the frontend is static files. There is no npm
# step and nothing to build but the Rust binary.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
HERE="$PWD"

. "$HOME/workspace/baxters-repo/packaging/bxdeb.sh"

PKG=baxters-aismartguy
APPDIR=aismartguy
CONTROL="$HERE/packaging/DEBIAN/control"

VERSION="$(bx_control_version "$CONTROL")"
bx_assert_control "$CONTROL"
CRATE_VERSION="$(bx_cargo_version "$HERE/src-tauri" aismartguy-app)"
bx_assert_versions_agree "$VERSION" "$CRATE_VERSION" "the aismartguy-app crate"

BIN="$HERE/target/release/aismartguy-app"
bx_assert_binary_fresh "$BIN" "$HERE/src-tauri/src" "$HERE/crates" "$HERE/frontend" \
                       "$HERE/src-tauri/Cargo.toml" "$HERE/Cargo.lock"

STAGE="$(mktemp -d)"; trap 'rm -rf "$STAGE"' EXIT
DEST="$STAGE/opt/baxters/$APPDIR"
mkdir -p "$DEST"

cp -a packaging/DEBIAN "$STAGE/DEBIAN"
cp -a packaging/usr/. "$STAGE/usr/"

bx_install_required "$BIN"                  "$DEST/aismartguy-app" 0755
bx_install_required "$HERE/packaging/run.sh" "$DEST/run.sh" 0755
# The frontend is the app's UI, not a build artefact: tauri serves it from disk.
bx_install_required "$HERE/frontend"        "$DEST/frontend"
for item in LICENSE README.md; do
    [ -e "$HERE/$item" ] && bx_install_required "$HERE/$item" "$DEST/$item"
done

for pair in "src-tauri/icons/32x32.png:32x32" "src-tauri/icons/128x128.png:128x128" \
            "src-tauri/icons/128x128@2x.png:256x256" "src-tauri/icons/icon.png:512x512"; do
    src="${pair%%:*}"; dim="${pair##*:}"
    [ -e "$HERE/$src" ] || { echo "FATAL: missing icon $src" >&2; exit 1; }
    mkdir -p "$STAGE/usr/share/icons/hicolor/$dim/apps"
    cp -a "$HERE/$src" "$STAGE/usr/share/icons/hicolor/$dim/apps/$PKG.png"
done

bx_write_mit_copyright "$STAGE/usr/share/doc/$PKG/copyright" "AiSmartGuy"
find "$DEST" -name '*.bak' -delete 2>/dev/null || true
chmod -R go-w "$STAGE"
chmod 0755 "$DEST/run.sh" "$DEST/aismartguy-app"
bx_assert_not_group_writable "$STAGE"
bx_assert_copyright "$STAGE" "$PKG"
bx_assert_desktop "$STAGE" "$STAGE/usr/share/applications/$PKG.desktop"

# --- assertions on the STAGED tree -----------------------------------------
[ -x "$DEST/aismartguy-app" ] || { echo "FATAL: binary not in payload" >&2; exit 1; }
# tauri.conf.json points frontendDist at ../frontend. If index.html is missing
# the window opens blank with no error anywhere -- the defect this check exists
# for, and the same shape as a desktop Exec pointing at nothing.
[ -s "$DEST/frontend/index.html" ] || {
    echo "FATAL: frontend/index.html not in payload -- the window would open blank" >&2; exit 1; }
echo "  frontend: $(find "$DEST/frontend" -type f | wc -l) file(s)"

OUT="$HERE/dist/${PKG}_${VERSION}_amd64.deb"
bx_build_deb "$STAGE" "$OUT"
echo "built $OUT ($(du -h "$OUT" | cut -f1))"
