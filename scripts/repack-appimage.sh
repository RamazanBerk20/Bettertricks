#!/usr/bin/env bash
# Drop the build distribution's libwayland from the AppImage and repack it.
# A bundled libwayland-client shadows the host's, and the host's Mesa libEGL_mesa needs newer
# symbols (wl_fixes_interface, wl_display_create_queue_with_name, ...). Mesa then fails to load
# and WebKitGTK aborts with "Could not create surfaceless EGL display: EGL_BAD_ALLOC".
# libwayland keeps its ABI backwards compatible, so the host copy always serves the bundled GTK.
set -euo pipefail

REPOSITORY_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
VERSION=$(node -p "JSON.parse(require('node:fs').readFileSync('$REPOSITORY_ROOT/apps/desktop/package.json', 'utf8')).version")
BUNDLE_DIR=${1:-"$REPOSITORY_ROOT/target/release/bundle/appimage"}
APPDIR="$BUNDLE_DIR/Bettertricks.AppDir"
APPIMAGE="$BUNDLE_DIR/Bettertricks_${VERSION}_amd64.AppImage"
PLUGIN="${XDG_CACHE_HOME:-$HOME/.cache}/tauri/linuxdeploy-plugin-appimage.AppImage"

for required in "$APPDIR/usr/lib" "$APPIMAGE" "$PLUGIN"; do
  test -e "$required" || { echo "missing $required (run the Tauri AppImage build first)" >&2; exit 1; }
done

rm -f -- "$APPDIR"/usr/lib/libwayland-*.so*

repacked="${APPIMAGE%.AppImage}.repacked.AppImage"
rm -f -- "$repacked"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 LDAI_OUTPUT="$repacked" "$PLUGIN" --appdir "$APPDIR"
mv -f -- "$repacked" "$APPIMAGE"
