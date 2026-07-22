#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if [[ -n "${BETTERTRICKS_SMOKE_RUNTIME_DIR:-}" ]]; then
  for runtime_tool in wine wineserver; do
    test -x "$BETTERTRICKS_SMOKE_RUNTIME_DIR/$runtime_tool" || {
      echo "Missing Proton/Wine runtime tool: $BETTERTRICKS_SMOKE_RUNTIME_DIR/$runtime_tool" >&2
      exit 1
    }
  done
  export PATH="$BETTERTRICKS_SMOKE_RUNTIME_DIR:$PATH"
fi

SMOKE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/bettertricks-wine-smoke.XXXXXX")
SMOKE_DATA="$SMOKE_ROOT/data"
SMOKE_CONFIG="$SMOKE_ROOT/config"
SMOKE_STATE="$SMOKE_ROOT/state"
SMOKE_CACHE="$SMOKE_ROOT/cache"
SMOKE_PREFIX="$SMOKE_DATA/wineprefixes/smoke"

cleanup() {
  WINEPREFIX="$SMOKE_PREFIX" wineserver -k >/dev/null 2>&1 || true
  if [[ "$SMOKE_ROOT" == "${TMPDIR:-/tmp}/bettertricks-wine-smoke."* ]]; then
    rm -rf -- "$SMOKE_ROOT"
  fi
}
trap cleanup EXIT

for command in wine wineserver; do
  command -v "$command" >/dev/null || {
    echo "Missing required Wine command: $command" >&2
    exit 1
  }
done
if [[ -z "${BETTERTRICKS_SMOKE_RUNTIME_DIR:-}" ]]; then
  command -v wineboot >/dev/null || {
    echo "Missing required Wine command: wineboot" >&2
    exit 1
  }
fi

run_wineboot() {
  if [[ -n "${BETTERTRICKS_SMOKE_RUNTIME_DIR:-}" ]] && [[ ! -x "$BETTERTRICKS_SMOKE_RUNTIME_DIR/wineboot" ]]; then
    wine wineboot "$@"
  else
    wineboot "$@"
  fi
}

echo "Testing runtime: $(wine --version) ($(command -v wine))"

mkdir -p -- "$SMOKE_PREFIX" "$SMOKE_CONFIG" "$SMOKE_STATE" "$SMOKE_CACHE"
export WINEPREFIX="$SMOKE_PREFIX"
export WINEARCH=win64
export WINEDEBUG=-all
export WINEDLLOVERRIDES="mscoree,mshtml="
export XDG_DATA_HOME="$SMOKE_DATA"
export XDG_CONFIG_HOME="$SMOKE_CONFIG"
export XDG_STATE_HOME="$SMOKE_STATE"
export XDG_CACHE_HOME="$SMOKE_CACHE"
export BETTERTRICKS_CATALOG="$PROJECT_ROOT/catalog"

run_wineboot -u
wineserver -w

cargo build --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" -p bettertricks
BETTERTRICKS="$PROJECT_ROOT/target/debug/bettertricks"

"$BETTERTRICKS" fontsmooth=rgb graphics=x11 native_oleaut32 native_mdac hosts prefix=smoke
"$BETTERTRICKS" --input 'set_mididevice.device=FluidSynth MIDI' set_mididevice prefix=smoke
"$BETTERTRICKS" --input 'set_userpath.paths=/opt/bettertricks/bin' set_userpath prefix=smoke

wine reg query 'HKCU\Control Panel\Desktop' /v FontSmoothing | tr -d '\r' | grep -F '2' >/dev/null
wine reg query 'HKCU\Software\Wine\Drivers' /v Graphics | tr -d '\r' | grep -F 'x11' >/dev/null
wine reg query 'HKCU\Software\Wine\DllOverrides' /v '*oleaut32' | tr -d '\r' | grep -F 'native,builtin' >/dev/null
wine reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Multimedia\MIDIMap' /v CurrentInstrument | tr -d '\r' | grep -F 'FluidSynth MIDI' >/dev/null
wine reg query 'HKCU\Environment' /v PATH | tr -d '\r' | grep -F 'bettertricks' >/dev/null
test -f "$SMOKE_PREFIX/drive_c/windows/system32/drivers/etc/hosts"
test -f "$SMOKE_PREFIX/drive_c/windows/system32/drivers/etc/services"

"$BETTERTRICKS" alldlls=builtin prefix=smoke
wine reg query 'HKCU\Software\Wine\DllOverrides' /v '*d3d11' | tr -d '\r' | grep -F 'builtin' >/dev/null

if [[ -n "${BETTERTRICKS_SMOKE_FONT_ARCHIVE:-}" ]]; then
  test "$(sha256sum "$BETTERTRICKS_SMOKE_FONT_ARCHIVE" | cut -d ' ' -f 1)" = "0524fe42951adc3a7eb870e32f0920313c71f170c859b5f770d82b4ee111e970"
  mkdir -p -- "$SMOKE_CACHE/winetricks/corefonts"
  cp -- "$BETTERTRICKS_SMOKE_FONT_ARCHIVE" "$SMOKE_CACHE/winetricks/corefonts/andale32.exe"
  "$BETTERTRICKS" andale prefix=smoke
  test -f "$SMOKE_PREFIX/drive_c/windows/Fonts/andalemo.ttf"
  wine reg query 'HKLM\Software\Microsoft\Windows NT\CurrentVersion\Fonts' /v 'Andale Mono (TrueType)' | tr -d '\r' | grep -F 'andalemo.ttf' >/dev/null
fi

if [[ -n "${BETTERTRICKS_SMOKE_LUCIDA_ARCHIVE:-}" ]]; then
  test "$(sha256sum "$BETTERTRICKS_SMOKE_LUCIDA_ARCHIVE" | cut -d ' ' -f 1)" = "41f272a33521f6e15f2cce9ff1e049f2badd5ff0dc327fc81b60825766d5b6c7"
  mkdir -p -- "$SMOKE_CACHE/winetricks/lucida"
  cp -- "$BETTERTRICKS_SMOKE_LUCIDA_ARCHIVE" "$SMOKE_CACHE/winetricks/lucida/eurofixi.exe"
  "$BETTERTRICKS" --verify lucida prefix=smoke
  test -f "$SMOKE_PREFIX/drive_c/windows/Fonts/lucon.ttf"
  wine reg query 'HKLM\Software\Microsoft\Windows NT\CurrentVersion\Fonts' /v 'Lucida Console (TrueType)' | tr -d '\r' | grep -F 'lucon.ttf' >/dev/null
fi

if [[ -n "${BETTERTRICKS_SMOKE_POWERPOINT_ARCHIVE:-}" ]]; then
  test "$(sha256sum "$BETTERTRICKS_SMOKE_POWERPOINT_ARCHIVE" | cut -d ' ' -f 1)" = "249473568eba7a1e4f95498acba594e0f42e6581add4dead70c1dfb908a09423"
  mkdir -p -- "$SMOKE_CACHE/winetricks/PowerPointViewer"
  cp -- "$BETTERTRICKS_SMOKE_POWERPOINT_ARCHIVE" "$SMOKE_CACHE/winetricks/PowerPointViewer/PowerPointViewer.exe"
  "$BETTERTRICKS" --verify calibri prefix=smoke
  test -f "$SMOKE_PREFIX/drive_c/windows/Fonts/calibri.ttf"
  wine reg query 'HKLM\Software\Microsoft\Windows NT\CurrentVersion\Fonts' /v 'Calibri (TrueType)' | tr -d '\r' | grep -F 'calibri.ttf' >/dev/null
fi

if [[ -n "${BETTERTRICKS_SMOKE_SOURCE_HAN_ARCHIVE:-}" ]]; then
  test "$(sha256sum "$BETTERTRICKS_SMOKE_SOURCE_HAN_ARCHIVE" | cut -d ' ' -f 1)" = "6f59118a9adda5a7fe4e9e6bb538309f7e1d3c5411f9a9d32af32a79501b7e4f"
  mkdir -p -- "$SMOKE_CACHE/winetricks/sourcehansans"
  cp -- "$BETTERTRICKS_SMOKE_SOURCE_HAN_ARCHIVE" "$SMOKE_CACHE/winetricks/sourcehansans/SourceHanSans.ttc.zip"
  "$BETTERTRICKS" --verify fakejapanese prefix=smoke
  test -f "$SMOKE_PREFIX/drive_c/windows/Fonts/sourcehansans.ttc"
  wine reg query 'HKCU\Software\Wine\Fonts\Replacements' /v 'メイリオ' | tr -d '\r' | grep -F 'Source Han Sans' >/dev/null
fi

"$BETTERTRICKS" sandbox prefix=smoke
test ! -L "$SMOKE_PREFIX/dosdevices/z:"
grep -Fx 'disable' "$SMOKE_PREFIX/.update-timestamp" >/dev/null

echo "Disposable Wine prefix passed native settings smoke tests."
