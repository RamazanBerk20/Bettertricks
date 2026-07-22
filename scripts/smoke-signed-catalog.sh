#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
SMOKE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/bettertricks-signed-catalog.XXXXXX")

cleanup() {
  if [[ "$SMOKE_ROOT" == "${TMPDIR:-/tmp}/bettertricks-signed-catalog."* ]]; then
    rm -r -- "$SMOKE_ROOT"
  fi
}
trap cleanup EXIT

SIGNING_KEY="$SMOKE_ROOT/catalog.key"
BUNDLE_A="$SMOKE_ROOT/catalog-a.tar.zst"
BUNDLE_B="$SMOKE_ROOT/catalog-b.tar.zst"
printf '%s\n' '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f' > "$SIGNING_KEY"

cargo run --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" -p bettertricks-catalog-tool -- bundle \
  --catalog "$PROJECT_ROOT/catalog" \
  --output "$BUNDLE_A" \
  --url https://updates.example.invalid/catalog-winetricks-20260125.tar.zst \
  --signing-key "$SIGNING_KEY"
cargo run --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" -p bettertricks-catalog-tool -- bundle \
  --catalog "$PROJECT_ROOT/catalog" \
  --output "$BUNDLE_B" \
  --url https://updates.example.invalid/catalog-winetricks-20260125.tar.zst \
  --signing-key "$SIGNING_KEY"

cmp "$BUNDLE_A" "$BUNDLE_B"
cmp "$SMOKE_ROOT/catalog-a.tar.release.json" "$SMOKE_ROOT/catalog-b.tar.release.json"

cargo run --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" -p bettertricks-catalog-tool -- index \
  --release "$SMOKE_ROOT/catalog-a.tar.release.json" \
  --output "$SMOKE_ROOT/index.json"

node - "$BUNDLE_A" "$SMOKE_ROOT/catalog-a.tar.release.json" "$SMOKE_ROOT/index.json" <<'NODE'
const fs = require("node:fs");
const crypto = require("node:crypto");
const [bundlePath, releasePath, indexPath] = process.argv.slice(2);
const release = JSON.parse(fs.readFileSync(releasePath, "utf8"));
const index = JSON.parse(fs.readFileSync(indexPath, "utf8"));
const digest = crypto.createHash("sha256").update(fs.readFileSync(bundlePath)).digest("hex");
if (release.version !== "winetricks-20260125") throw new Error("unexpected signed version");
if (release.upstream_tag !== "20260125") throw new Error("unexpected signed upstream baseline");
if (release.recipe_count !== 550) throw new Error(`expected 550 recipes, got ${release.recipe_count}`);
if (release.sha256 !== digest) throw new Error("signed descriptor checksum does not match bundle");
if (!/^[0-9a-f]{128}$/.test(release.signature)) throw new Error("invalid Ed25519 signature encoding");
if (index.schema !== 1 || index.releases.length !== 1) throw new Error("invalid release index");
NODE

cargo test --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml" -p bettertricks-core \
  catalog_update::tests::verifies_activates_and_rolls_back_a_signed_bundle -- --exact

echo "Signed catalog publication is deterministic; activation and rollback passed."
