import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");

test("preserves Winetricks manual-download media in generated metadata", () => {
  for (const path of [
    "catalog/generated/apps/foobar2000.toml",
    "catalog/generated/apps/protectionid.toml",
    "catalog/generated/apps/utorrent.toml",
    "catalog/generated/benchmarks/3dmark03.toml",
    "catalog/generated/benchmarks/3dmark06.toml",
    "catalog/generated/benchmarks/stalker_pripyat_bench.toml",
    "catalog/generated/benchmarks/unigine_heaven.toml",
    "catalog/generated/dlls/gdiplus_winxp.toml",
  ]) {
    const recipe = readFileSync(join(root, path), "utf8");
    assert.match(recipe, /^media = "manual_download"$/m, path);
    assert.match(recipe, /^tags = \["manual_download"\]$/m, path);
  }
});

test("ports the deprecated DirectX aggregate as the upstream no-op", () => {
  const recipe = readFileSync(join(root, "catalog/native/dlls/directx9.toml"), "utf8");
  assert.match(recipe, /^maturity = "native"$/m);
  assert.match(recipe, /^type = "notice"$/m);
  assert.doesNotMatch(recipe, /^\[\[files\]\]$/m);
});
