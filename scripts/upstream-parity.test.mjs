import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");
const winetricks = process.env.WINETRICKS ?? "winetricks";
const baseline = "20260125";
const environment = {
  ...process.env,
  LANG: "C",
  LC_ALL: "C",
  WINETRICKS_SUPER_QUIET: "1",
};

test("matches the complete pinned Winetricks catalog and media lists", () => {
  const version = run(["--version"]);
  assert.match(version, new RegExp(`\\b${baseline}\\b`));

  const upstream = parseListAll(run(["list-all"]));
  const automatic = verbList(run(["list-download"]));
  const manual = verbList(run(["list-manual-download"]));
  const catalog = loadCatalog();

  assert.equal(catalog.size, 550);
  assert.deepEqual([...catalog.keys()].sort(), [...upstream.keys()].sort());
  for (const [id, recipe] of catalog) {
    assert.equal(recipe.category, upstream.get(id), `${id} category differs from Winetricks`);
    const expectedMedia = manual.has(id) ? "manual_download" : automatic.has(id) ? "download" : "none";
    assert.equal(recipe.media, expectedMedia, `${id} media differs from Winetricks`);
    assert.equal(recipe.upstreamTag, baseline, `${id} uses another upstream baseline`);
    assert.equal(recipe.upstreamVerb, id, `${id} points at another upstream verb`);
  }
});

function run(arguments_) {
  return execFileSync(winetricks, arguments_, {
    encoding: "utf8",
    env: environment,
    maxBuffer: 8 * 1024 * 1024,
  });
}

function parseListAll(output) {
  const categories = new Set(["apps", "benchmarks", "dlls", "fonts", "settings"]);
  const recipes = new Map();
  let category;
  for (const raw of output.split(/\r?\n/)) {
    const heading = raw.trim().match(/^===== ([a-z]+) =====$/);
    if (heading) {
      category = categories.has(heading[1]) ? heading[1] : undefined;
      continue;
    }
    const id = raw.trim().split(/\s+/, 1)[0];
    if (!category || !/^[a-z0-9_=]+$/.test(id)) continue;
    assert.ok(!recipes.has(id), `Winetricks listed ${id} more than once`);
    recipes.set(id, category);
  }
  return recipes;
}

function verbList(output) {
  return new Set(output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => /^[a-z0-9_=]+$/.test(line)));
}

function loadCatalog() {
  const recipes = new Map();
  for (const source of ["native", "generated"]) {
    const sourceRoot = join(root, "catalog", source);
    for (const category of readdirSync(sourceRoot)) {
      const directory = join(sourceRoot, category);
      for (const filename of readdirSync(directory)) {
        if (!filename.endsWith(".toml")) continue;
        const toml = readFileSync(join(directory, filename), "utf8");
        const id = field(toml, "id");
        assert.ok(!recipes.has(id), `Catalog defines ${id} more than once`);
        recipes.set(id, {
          category: field(toml, "category"),
          media: field(toml, "media"),
          upstreamTag: sourceField(toml, "upstream_tag"),
          upstreamVerb: sourceField(toml, "upstream_verb"),
        });
      }
    }
  }
  return recipes;
}

function field(toml, name) {
  const match = toml.match(new RegExp(`^${name} = "([^"]+)"$`, "m"));
  assert.ok(match, `Recipe is missing ${name}`);
  return match[1];
}

function sourceField(toml, name) {
  const source = toml.split(/^\[source\]$/m)[1] ?? "";
  return field(source, name);
}
