export const WINETRICKS_BASELINE = "20260125";
export const WINETRICKS_BUILTIN_DLL_COUNT = 720;
export const WINETRICKS_BUILTIN_DLL_SHA256 = "5b0c10cdd6b4eff0e913f63187c9c664d6f698aa5b18688b366fe1545001d03b";

export function normalizeWinetricksOutput(value, homeDirectory) {
  if (!homeDirectory || homeDirectory === "/") return value;
  return value.split(homeDirectory).join("$HOME");
}

export function maturityForTitle(title) {
  return /\(broken in wine\)/i.test(title) ? "broken_upstream" : "metadata_only";
}

export function parseBuiltinDllOverrides(source) {
  const lines = source.split(/\r?\n/);
  const functionStart = lines.findIndex((line) => line.trim() === "w_override_all_dlls()");
  if (functionStart < 0) throw new Error("Winetricks source has no w_override_all_dlls function.");

  const invocation = lines.findIndex(
    (line, index) => index > functionStart && line.trim() === "w_override_dlls builtin \\",
  );
  if (invocation < 0) throw new Error("Winetricks source has no builtin DLL override invocation.");

  const libraries = [];
  for (const rawLine of lines.slice(invocation + 1)) {
    const line = rawLine.trim();
    if (!line) break;
    if (!line.endsWith("\\")) {
      throw new Error("Builtin DLL override list lost its expected line continuation.");
    }
    libraries.push(...line.slice(0, -1).trim().split(/\s+/));
  }
  if (!libraries.length || new Set(libraries).size !== libraries.length) {
    throw new Error("Builtin DLL override list is empty or contains duplicates.");
  }
  return libraries;
}
