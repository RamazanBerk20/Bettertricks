import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import {
  maturityForTitle,
  normalizeWinetricksOutput,
  WINETRICKS_BASELINE,
} from "./catalog-metadata.mjs";

const root = resolve(import.meta.dirname, "..");
const outputRoot = join(root, "catalog", "generated");
const nativeRoot = join(root, "catalog", "native");
const winetricks = process.env.WINETRICKS ?? "winetricks";
const supportedCategories = ["apps", "benchmarks", "dlls", "fonts", "settings"];
const commandEnvironment = {
  ...process.env,
  LANG: "C",
  LC_ALL: "C",
  WINETRICKS_SUPER_QUIET: "1",
};

const versionOutput = execFileSync(winetricks, ["--version"], { encoding: "utf8" });
const upstreamTag = versionOutput.match(/\b\d{8}\b/)?.[0] ?? "unknown";
if (upstreamTag !== WINETRICKS_BASELINE) {
  throw new Error(
    `Catalog generation requires Winetricks ${WINETRICKS_BASELINE}; found ${upstreamTag}.`,
  );
}
const list = normalizeWinetricksOutput(execFileSync(winetricks, ["list-all"], {
  encoding: "utf8",
  env: commandEnvironment,
}), process.env.HOME);
const downloadIds = verbList(execFileSync(winetricks, ["list-download"], {
  encoding: "utf8",
  env: commandEnvironment,
}));
const manualDownloadIds = verbList(execFileSync(winetricks, ["list-manual-download"], {
  encoding: "utf8",
  env: commandEnvironment,
}));

const nativeIds = new Set();
const nativeCategoryCounts = new Map();
if (existsSync(nativeRoot)) {
  for (const category of supportedCategories) {
    const directory = join(nativeRoot, category);
    if (!existsSync(directory)) continue;
    for (const filename of readdirSync(directory)) {
      if (filename.endsWith(".toml")) {
        nativeIds.add(basename(filename, ".toml"));
        nativeCategoryCounts.set(category, (nativeCategoryCounts.get(category) ?? 0) + 1);
      }
    }
  }
}

rmSync(outputRoot, { recursive: true, force: true });
mkdirSync(outputRoot, { recursive: true });

let category;
let count = 0;
const categoryCounts = new Map();
for (const rawLine of list.split(/\r?\n/)) {
  const line = rawLine.trimEnd();
  const heading = line.match(/^===== ([a-z]+) =====$/);
  if (heading) {
    category = supportedCategories.includes(heading[1])
      ? heading[1]
      : undefined;
    continue;
  }
  if (!category || !line.trim()) continue;

  const firstSpace = line.search(/\s/);
  if (firstSpace < 1) continue;
  const id = line.slice(0, firstSpace);
  if (nativeIds.has(id)) continue;

  let remainder = line.slice(firstSpace).trim();
  const flagsMatch = remainder.match(/\s+\[([^\]]+)]$/);
  const flags = flagsMatch ? flagsMatch[1].split(",").map((flag) => flag.trim()) : [];
  if (flagsMatch) remainder = remainder.slice(0, flagsMatch.index).trim();

  let title = remainder;
  let publisher;
  let year;
  const metadataMatch = remainder.match(/^(.*) \(([^()]*)\s*,\s*([^()]*)\)$/);
  if (metadataMatch) {
    title = metadataMatch[1].trim();
    publisher = metadataMatch[2].trim() || undefined;
    year = metadataMatch[3].trim() || undefined;
  }

  const media = manualDownloadIds.has(id)
    ? "manual_download"
    : downloadIds.has(id)
      ? "download"
      : "none";
  if (media !== "none" && !flags.includes(media)) flags.push(media);
  const maturity = maturityForTitle(title);
  const description = descriptionFor(category, title);
  const toml = [
    "# Generated from upstream Winetricks metadata. Do not edit by hand.",
    "schema = 1",
    `id = ${string(id)}`,
    `category = ${string(category)}`,
    `title = ${string(title)}`,
    publisher ? `publisher = ${string(publisher)}` : undefined,
    year ? `year = ${string(year)}` : undefined,
    `description = ${string(description)}`,
    `media = ${string(media)}`,
    `maturity = ${string(maturity)}`,
    `tags = [${flags.map(string).join(", ")}]`,
    "",
    "[source]",
    `upstream_tag = ${string(upstreamTag)}`,
    `upstream_verb = ${string(id)}`,
    "",
  ].filter((line) => line !== undefined).join("\n");

  const directory = join(outputRoot, category);
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, `${id}.toml`), toml);
  count += 1;
  categoryCounts.set(category, (categoryCounts.get(category) ?? 0) + 1);
}

const manifest = {
  schema: 1,
  version: `winetricks-${upstreamTag}`,
  upstreamTag,
  generatedAt: `${upstreamTag.slice(0, 4)}-${upstreamTag.slice(4, 6)}-${upstreamTag.slice(6, 8)}T00:00:00.000Z`,
  generatedRecipes: count,
  nativeRecipes: nativeIds.size,
  categories: Object.fromEntries(supportedCategories.map((name) => [
    name,
    (categoryCounts.get(name) ?? 0) + (nativeCategoryCounts.get(name) ?? 0),
  ])),
};
writeFileSync(join(root, "catalog", "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);

process.stdout.write(`Generated ${count} metadata recipes from Winetricks ${upstreamTag}; ${nativeIds.size} native overrides.\n`);

function string(value) {
  return JSON.stringify(String(value));
}

function verbList(output) {
  return new Set(output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => /^[a-z0-9_=]+$/.test(line)));
}

function descriptionFor(category, title) {
  switch (category) {
    case "apps": return `Install ${title} in the selected Wine prefix.`;
    case "benchmarks": return `Install the ${title} benchmark in the selected Wine prefix.`;
    case "dlls": return `Install or configure ${title} for Windows application compatibility.`;
    case "fonts": return `Install ${title} in the selected Wine prefix.`;
    case "settings": return title.endsWith(".") ? title : `${title}.`;
    default: return title;
  }
}
