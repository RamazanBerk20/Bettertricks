#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
VERSION=$(node -p "require('$PROJECT_ROOT/apps/desktop/src-tauri/tauri.conf.json').version")
DEB=${1:-"$PROJECT_ROOT/target/release/bundle/deb/Bettertricks_${VERSION}_amd64.deb"}
RPM=${2:-"$PROJECT_ROOT/target/release/bundle/rpm/Bettertricks-${VERSION}-1.x86_64.rpm"}
APPIMAGE=${3:-"$PROJECT_ROOT/target/release/bundle/appimage/Bettertricks_${VERSION}_amd64.AppImage"}

command -v docker >/dev/null || { echo "Docker is required for clean-system package smoke tests" >&2; exit 1; }
for artifact in "$DEB" "$RPM" "$APPIMAGE"; do
  test -f "$artifact" || { echo "missing release artifact: $artifact" >&2; exit 1; }
done

DEB_DIR=$(cd -- "$(dirname -- "$DEB")" && pwd)
RPM_DIR=$(cd -- "$(dirname -- "$RPM")" && pwd)
APPIMAGE_DIR=$(cd -- "$(dirname -- "$APPIMAGE")" && pwd)
DEB_NAME=$(basename -- "$DEB")
RPM_NAME=$(basename -- "$RPM")
APPIMAGE_NAME=$(basename -- "$APPIMAGE")

docker run --rm \
  --volume "$DEB_DIR:/artifacts:ro" \
  --volume "$PROJECT_ROOT/scripts:/bettertricks-scripts:ro" \
  --env "DEB_NAME=$DEB_NAME" \
  ubuntu:24.04 bash -euo pipefail -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -qq --yes --no-install-recommends "/artifacts/$DEB_NAME" at-spi2-core dbus-x11 python3-pyatspi xauth xvfb
    test "$(bettertricks --json settings list-all | grep -o '"'"'"id"'"'"' | wc -l)" -eq 118
    ! ldd /usr/bin/bettertricks-desktop | grep -q "not found"
    SMOKE_ROOT=$(mktemp -d /tmp/bettertricks-a11y.XXXXXX)
    mkdir -p "$SMOKE_ROOT/data" "$SMOKE_ROOT/config" "$SMOKE_ROOT/state" "$SMOKE_ROOT/cache"
    export XDG_DATA_HOME="$SMOKE_ROOT/data"
    export XDG_CONFIG_HOME="$SMOKE_ROOT/config"
    export XDG_STATE_HOME="$SMOKE_ROOT/state"
    export XDG_CACHE_HOME="$SMOKE_ROOT/cache"
    export NO_AT_BRIDGE=0
    export GTK_A11Y=always
    export GDK_BACKEND=x11
    export WEBKIT_DISABLE_COMPOSITING_MODE=1
    dbus-run-session -- xvfb-run -a bash -euo pipefail -c '"'"'
      bettertricks-desktop >/tmp/bettertricks-desktop.log 2>&1 &
      desktop_pid=$!
      trap "kill $desktop_pid >/dev/null 2>&1 || true" EXIT
      python3 /bettertricks-scripts/smoke-accessibility-bridge.py
    '"'"'
  '

docker run --rm \
  --volume "$RPM_DIR:/artifacts:ro" \
  --env "RPM_NAME=$RPM_NAME" \
  fedora:44 bash -euo pipefail -c '
    dnf install -q -y --setopt=install_weak_deps=False "/artifacts/$RPM_NAME"
    test "$(bettertricks --json settings list-all | grep -o '"'"'"id"'"'"' | wc -l)" -eq 118
    ! ldd /usr/bin/bettertricks-desktop | grep -q "not found"
  '

docker run --rm \
  --volume "$APPIMAGE_DIR:/artifacts:ro" \
  --env "APPIMAGE_NAME=$APPIMAGE_NAME" \
  ubuntu:24.04 bash -euo pipefail -c '
    cd /tmp
    "/artifacts/$APPIMAGE_NAME" --appimage-extract >/dev/null
    test "$(./squashfs-root/usr/bin/bettertricks --json settings list-all | grep -o '"'"'"id"'"'"' | wc -l)" -eq 118
    test -x ./squashfs-root/AppRun
    test -x ./squashfs-root/usr/bin/bettertricks-desktop
  '

echo "Debian, RPM, and AppImage artifacts passed clean-system install/runtime checks."
