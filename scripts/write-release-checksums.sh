#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPOSITORY_ROOT"

BUNDLE_ROOT=${1:-target/release/bundle}
PUBLISH_ROOT=${2:-target/release/publish}
VERSION=$(node -p "JSON.parse(require('node:fs').readFileSync('apps/desktop/package.json', 'utf8')).version")
PUBLIC_VERSION=$(node -e 'const [major, minor, patch] = process.argv[1].split("."); console.log(patch === "0" ? `${major}.${minor}` : process.argv[1])' "$VERSION")

ARTIFACTS=(
  "appimage/Bettertricks_${VERSION}_amd64.AppImage"
  "deb/Bettertricks_${VERSION}_amd64.deb"
  "rpm/Bettertricks-${VERSION}-1.x86_64.rpm"
)

for artifact in "${ARTIFACTS[@]}"; do
  if [[ ! -f "$BUNDLE_ROOT/$artifact" ]]; then
    echo "Missing release artifact: $BUNDLE_ROOT/$artifact" >&2
    exit 1
  fi
done

mkdir -p "$PUBLISH_ROOT"

PUBLISHED_ARTIFACTS=(
  "Bettertricks_${PUBLIC_VERSION}_amd64.AppImage"
  "Bettertricks_${PUBLIC_VERSION}_amd64.deb"
  "Bettertricks-${PUBLIC_VERSION}-1.x86_64.rpm"
)

install -m 0755 "$BUNDLE_ROOT/${ARTIFACTS[0]}" "$PUBLISH_ROOT/${PUBLISHED_ARTIFACTS[0]}"
install -m 0644 "$BUNDLE_ROOT/${ARTIFACTS[1]}" "$PUBLISH_ROOT/${PUBLISHED_ARTIFACTS[1]}"
install -m 0644 "$BUNDLE_ROOT/${ARTIFACTS[2]}" "$PUBLISH_ROOT/${PUBLISHED_ARTIFACTS[2]}"

temporary_manifest=$(mktemp "$PUBLISH_ROOT/.SHA256SUMS.XXXXXX")
trap 'rm -f "$temporary_manifest"' EXIT

(
  cd "$PUBLISH_ROOT"
  sha256sum "${PUBLISHED_ARTIFACTS[@]}"
) > "$temporary_manifest"

chmod 0644 "$temporary_manifest"
mv "$temporary_manifest" "$PUBLISH_ROOT/SHA256SUMS"
trap - EXIT

(
  cd "$PUBLISH_ROOT"
  sha256sum --check --strict SHA256SUMS
)
