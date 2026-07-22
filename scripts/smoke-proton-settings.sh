#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
PROTON_TAG=GE-Proton11-1
PROTON_ARCHIVE="$PROTON_TAG.tar.gz"
PROTON_URL="https://github.com/GloriousEggroll/proton-ge-custom/releases/download/$PROTON_TAG/$PROTON_ARCHIVE"
PROTON_SHA512=d5792f4ac81d3832f5fe40467090c67d561b780c6a4236e76f8b59cb1d4ca25c82df91018e79d156bb267a67224a41f0d621a1e6cbbeec79040cc60275dc9e5a
PROTON_CACHE=${BETTERTRICKS_PROTON_CACHE:-${XDG_CACHE_HOME:-/tmp}/bettertricks-proton-smoke}
ARCHIVE_PATH="$PROTON_CACHE/$PROTON_ARCHIVE"
RUNTIME_ROOT="$PROTON_CACHE/$PROTON_TAG"

mkdir -p -- "$PROTON_CACHE"
if [[ ! -f "$ARCHIVE_PATH" ]] || [[ "$(sha512sum "$ARCHIVE_PATH" | cut -d ' ' -f 1)" != "$PROTON_SHA512" ]]; then
  PARTIAL_PATH="$ARCHIVE_PATH.partial"
  curl --fail --show-error --location --retry 3 "$PROTON_URL" --output "$PARTIAL_PATH"
  echo "$PROTON_SHA512  $PARTIAL_PATH" | sha512sum --check --strict
  mv -- "$PARTIAL_PATH" "$ARCHIVE_PATH"
fi

if [[ ! -x "$RUNTIME_ROOT/files/bin/wine" ]]; then
  tar -xzf "$ARCHIVE_PATH" -C "$PROTON_CACHE"
fi
test -x "$RUNTIME_ROOT/files/bin/wine"
test -x "$RUNTIME_ROOT/files/bin/wineserver"

BETTERTRICKS_SMOKE_RUNTIME_DIR="$RUNTIME_ROOT/files/bin" \
  bash "$PROJECT_ROOT/scripts/smoke-wine-settings.sh"

echo "$PROTON_TAG passed the Bettertricks disposable-prefix smoke suite."
