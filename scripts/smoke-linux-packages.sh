#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
version=$(node -p "require('$root/apps/desktop/src-tauri/tauri.conf.json').version")
deb=${1:-"$root/target/release/bundle/deb/Bettertricks_${version}_amd64.deb"}
rpm_package=${2:-"$root/target/release/bundle/rpm/Bettertricks-${version}-1.x86_64.rpm"}
appimage=${3:-"$root/target/release/bundle/appimage/Bettertricks_${version}_amd64.AppImage"}
binary="$root/target/release/bettertricks-desktop"

for command in ar bsdtar ldd node rg tar; do
  command -v "$command" >/dev/null || { echo "missing package smoke dependency: $command" >&2; exit 1; }
done
for artifact in "$deb" "$rpm_package" "$binary"; do
  test -f "$artifact" || { echo "missing release artifact: $artifact" >&2; exit 1; }
done

if ldd "$binary" | rg -q "not found"; then
  echo "desktop binary has unresolved shared libraries" >&2
  exit 1
fi

deb_control_member=$(ar t "$deb" | rg '^control\.tar')
deb_data_member=$(ar t "$deb" | rg '^data\.tar')
deb_control=$(ar p "$deb" "$deb_control_member" | tar -xzOf - control)
deb_files=$(ar p "$deb" "$deb_data_member" | tar -tzf -)
rg -q '^Package: bettertricks$' <<<"$deb_control"
rg -q '^Depends: .*wine' <<<"$deb_control"
for dependency in cabextract p7zip-full unzip gzip tar xz-utils zstd; do
  rg -q "^Depends: .*${dependency}" <<<"$deb_control"
done
rg -q '^usr/bin/bettertricks-desktop$' <<<"$deb_files"
rg -q '^usr/bin/bettertricks$' <<<"$deb_files"
rg -q '^usr/lib/Bettertricks/catalog/manifest\.json$' <<<"$deb_files"
rg -q '^usr/share/applications/Bettertricks\.desktop$' <<<"$deb_files"
rg -q '^usr/share/icons/.+/bettertricks-desktop\.png$' <<<"$deb_files"
if rg -q '/_up_/' <<<"$deb_files"; then
  echo "Debian package contains an unstable _up_ resource path" >&2
  exit 1
fi

rpm_files=$(bsdtar -tf "$rpm_package")
rg -q '^\./usr/bin/bettertricks-desktop$' <<<"$rpm_files"
rg -q '^\./usr/bin/bettertricks$' <<<"$rpm_files"
rg -q '^\./usr/lib/Bettertricks/catalog/manifest\.json$' <<<"$rpm_files"
rg -q '^\./usr/share/applications/Bettertricks\.desktop$' <<<"$rpm_files"
rg -q '^\./usr/share/icons/.+/bettertricks-desktop\.png$' <<<"$rpm_files"
if rg -q '/_up_/' <<<"$rpm_files"; then
  echo "RPM package contains an unstable _up_ resource path" >&2
  exit 1
fi

if command -v rpm >/dev/null; then
  rpm_requires=$(rpm -qpR "$rpm_package")
  rg -q '^wine' <<<"$rpm_requires"
  for dependency in cabextract p7zip unzip gzip tar xz zstd; do
    rg -q "^${dependency}" <<<"$rpm_requires"
  done
  rg -q 'webkit2gtk' <<<"$rpm_requires"
fi

deb_recipe_count=$(rg -c '\.toml$' <<<"$deb_files")
rpm_recipe_count=$(rg -c '\.toml$' <<<"$rpm_files")
test "$deb_recipe_count" -eq 550
test "$rpm_recipe_count" -eq 550

cli_root=$(mktemp -d /tmp/bettertricks-cli.XXXXXX)
extract_root=
cleanup() {
  rm -rf -- "$cli_root"
  if [[ -n "$extract_root" ]]; then rm -rf -- "$extract_root"; fi
}
trap cleanup EXIT
ar p "$deb" "$deb_data_member" | tar -xzf - -C "$cli_root"
XDG_CONFIG_HOME="$cli_root/config" \
XDG_DATA_HOME="$cli_root/data" \
XDG_STATE_HOME="$cli_root/state" \
XDG_CACHE_HOME="$cli_root/cache" \
  "$cli_root/usr/bin/bettertricks" --json settings list-all \
  | node -e 'let input = ""; process.stdin.on("data", chunk => input += chunk).on("end", () => { const recipes = JSON.parse(input); if (recipes.length === 118) return; throw new Error(`expected 118 settings, got ${recipes.length}`); });'

if [[ -n "$appimage" ]]; then
  test -f "$appimage" || { echo "missing AppImage artifact: $appimage" >&2; exit 1; }
  appimage=$(cd "$(dirname "$appimage")" && pwd)/$(basename "$appimage")
  extract_root=$(mktemp -d /tmp/bettertricks-appimage.XXXXXX)
  chmod +x "$appimage"
  (cd "$extract_root" && "$appimage" --appimage-extract >/dev/null)
  appimage_files=$(rg --files "$extract_root/squashfs-root")
  rg -q '/usr/bin/bettertricks-desktop$' <<<"$appimage_files"
  rg -q '/usr/bin/bettertricks$' <<<"$appimage_files"
  rg -q '/usr/lib/Bettertricks/catalog/manifest\.json$' <<<"$appimage_files"
  appimage_recipe_count=$(rg -c '\.toml$' <<<"$appimage_files")
  test "$appimage_recipe_count" -eq 550
fi

echo "Linux package smoke checks passed: 550 recipes in every supplied artifact."
