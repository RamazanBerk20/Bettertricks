import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");
const fontRoot = join(root, "catalog", "native", "fonts");
const generatedMarker = "# Generated from audited Winetricks font translations. Do not edit by hand.";

test("ships the audited font catalog without claiming the two unverified ports", () => {
  const generated = readdirSync(fontRoot)
    .filter((filename) => filename.endsWith(".toml"))
    .filter((filename) => readFileSync(join(fontRoot, filename), "utf8").startsWith(generatedMarker))
    .sort();

  assert.equal(generated.length, 40);
  assert(!generated.includes("micross.toml"));
  assert(!generated.includes("allfonts.toml"));
});

test("keeps nested extraction, Unicode aliases, and shared sources explicit", () => {
  const calibri = readFileSync(join(fontRoot, "calibri.toml"), "utf8");
  assert.match(calibri, /type = "extract_path"/);
  assert.match(calibri, /cache_path = "PowerPointViewer\/PowerPointViewer\.exe"/);

  const aliases = readFileSync(join(fontRoot, "fakejapanese.toml"), "utf8");
  assert.match(aliases, /alias = "メイリオ", replacement = "Source Han Sans"/);

  const sourceHan = readFileSync(join(fontRoot, "sourcehansans.toml"), "utf8");
  assert.equal(sourceHan.match(/\[\[verify\]\]/g)?.length, 1);

  const droid = readFileSync(join(fontRoot, "droid.toml"), "utf8");
  assert.match(droid, /raw\.githubusercontent\.com\/android\/platform_frameworks_base\/feef9887e8f8eb6f64fc1b4552c02efb5755cdc1/);
});
