#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
SMOKE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/bettertricks-managed-host.XXXXXX")

cleanup() {
  if [[ "$SMOKE_ROOT" == "${TMPDIR:-/tmp}/bettertricks-managed-host."* ]]; then
    rm -r -- "$SMOKE_ROOT"
  fi
}
trap cleanup EXIT

mkdir -p -- "$SMOKE_ROOT/data" "$SMOKE_ROOT/config" "$SMOKE_ROOT/state" "$SMOKE_ROOT/cache"
export XDG_DATA_HOME="$SMOKE_ROOT/data"
export XDG_CONFIG_HOME="$SMOKE_ROOT/config"
export XDG_STATE_HOME="$SMOKE_ROOT/state"
export XDG_CACHE_HOME="$SMOKE_ROOT/cache"
export BETTERTRICKS_CATALOG="$PROJECT_ROOT/catalog"

cargo build --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" -p bettertricks
BETTERTRICKS="$PROJECT_ROOT/target/debug/bettertricks"
HOST_PATH="$XDG_DATA_HOME/bettertricks/compatibility-hosts/winetricks-20260125"

"$BETTERTRICKS" --install-compatibility-host
test -f "$HOST_PATH"
test ! -L "$HOST_PATH"
test "$(stat -c '%a' "$HOST_PATH")" = 755
test "$(sha256sum "$HOST_PATH" | cut -d ' ' -f 1)" = 431f82fc74000e6c864409f1d8fb495d696c03928808e3e8acffc45179312a7b
"$HOST_PATH" --version | grep -F '20260125' >/dev/null

first_mtime=$(stat -c '%Y' "$HOST_PATH")
"$BETTERTRICKS" --install-compatibility-host
test "$(stat -c '%Y' "$HOST_PATH")" = "$first_mtime"

echo "Managed compatibility host passed download, integrity, baseline, and idempotence checks."
