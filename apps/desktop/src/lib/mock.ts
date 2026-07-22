import type {
  AppSettings,
  BootstrapPayload,
  CatalogQuery,
  LegacyVerbInfo,
  OperationEvent,
  OperationPlan,
  OperationRecord,
  OperationRequest,
  Recipe,
  RecipeListItem,
  RestorePoint,
  WinePrefix,
} from "../types";

const now = new Date();
const hourAgo = new Date(now.getTime() - 3_600_000).toISOString();
const dayAgo = new Date(now.getTime() - 86_400_000).toISOString();

const prefixes: WinePrefix[] = [
  {
    id: "6a943796-a760-4a35-a3d0-a4d18c680e91",
    name: "Default prefix",
    path: "/home/user/.wine",
    source: "default_wine",
    architecture: "wow64",
    runtime: "/usr/bin/wine",
    runtime_label: "Wine 11.13",
    managed: false,
    exists: true,
    installed_verbs: ["corefonts", "vcrun2022", "dxvk", "win10", "d3dcompiler_47"],
    size_bytes: 2_934_702_080,
    last_modified: hourAgo,
  },
  {
    id: "6d7fa6fa-f455-4aec-b3fe-672a691ac8d2",
    name: "Baldur's Gate 3",
    path: "/home/user/.local/share/Steam/steamapps/compatdata/1086940/pfx",
    source: "steam",
    architecture: "wow64",
    runtime: null,
    runtime_label: "Proton Experimental",
    managed: true,
    exists: true,
    installed_verbs: ["vcrun2019", "dotnet48", "dxvk"],
    size_bytes: 1_784_430_592,
    last_modified: dayAgo,
  },
  {
    id: "2b254e76-69d9-4818-94da-540fc6ae8ee0",
    name: "Affinity Photo",
    path: "/home/user/.local/share/bottles/bottles/Affinity",
    source: "bottles",
    architecture: "win64",
    runtime: null,
    runtime_label: "Caffe 9.7",
    managed: true,
    exists: true,
    installed_verbs: ["dotnet48", "corefonts", "win11"],
    size_bytes: 4_013_416_448,
    last_modified: dayAgo,
  },
];

const seedRecipes: Array<Partial<RecipeListItem> & Pick<RecipeListItem, "id" | "title" | "category">> = [
  { id: "vcrun2022", title: "Visual C++ 2015-2022 runtime", category: "dlls", publisher: "Microsoft", year: "2022", tags: ["runtime", "visual c++"] },
  { id: "dxvk", title: "DXVK Vulkan translation layer", category: "dlls", publisher: "DXVK Project", year: "2024", tags: ["directx", "vulkan", "gaming"] },
  { id: "corefonts", title: "Microsoft core fonts", category: "fonts", publisher: "Microsoft", year: "2008", tags: ["fonts", "office"] },
  { id: "dotnet48", title: ".NET Framework 4.8", category: "dlls", publisher: "Microsoft", year: "2019", tags: ["runtime", ".net"] },
  { id: "d3dcompiler_47", title: "Direct3D compiler 47", category: "dlls", publisher: "Microsoft", year: "2010", tags: ["directx", "gaming"] },
  { id: "win10", title: "Windows 10 compatibility mode", category: "settings", tags: ["windows version", "compatibility"], maturity: "native" },
  { id: "win11", title: "Windows 11 compatibility mode", category: "settings", tags: ["windows version", "compatibility"], maturity: "native" },
  { id: "fontsmooth=rgb", title: "RGB font smoothing", category: "settings", tags: ["fonts", "display"], maturity: "native" },
  { id: "set_mididevice", title: "Set MIDImap device", category: "settings", tags: ["midi", "sound"], maturity: "native" },
  { id: "xact", title: "Microsoft XACT audio engine", category: "dlls", publisher: "Microsoft", year: "2010", tags: ["audio", "gaming"] },
  { id: "physx", title: "NVIDIA PhysX runtime", category: "dlls", publisher: "NVIDIA", year: "2021", tags: ["gaming", "physics"] },
  { id: "7zip", title: "7-Zip 24.09", category: "apps", publisher: "Igor Pavlov", year: "2024", tags: ["utility", "archive"] },
  { id: "unigine_heaven", title: "Unigine Heaven benchmark", category: "benchmarks", publisher: "Unigine", year: "2010", tags: ["benchmark", "graphics"], media: "manual_download" },
];

const recipes: RecipeListItem[] = seedRecipes.map((recipe, index) => ({
  id: recipe.id,
  category: recipe.category,
  title: recipe.title,
  publisher: recipe.publisher ?? null,
  year: recipe.year ?? null,
  description: recipe.description ?? description(recipe.category, recipe.title),
  media: recipe.media ?? (recipe.category === "settings" ? "none" : "download"),
  maturity: recipe.maturity ?? "metadata_only",
  tags: recipe.tags ?? [],
  installed: index < 5,
  cached: index === 0 || index === 2 || index === 4,
  compatible: recipe.id !== "physx",
  compatibility_reason: recipe.id === "physx" ? "Not supported by the selected Wine runtime" : null,
}));

const operations: OperationRecord[] = [
  {
    id: "c3e24777-c89f-47a6-b173-4962df82a697",
    prefix_id: prefixes[0].id,
    prefix_name: prefixes[0].name,
    recipes: ["vcrun2022", "d3dcompiler_47"],
    state: "succeeded",
    created_at: hourAgo,
    started_at: hourAgo,
    finished_at: new Date(new Date(hourAgo).getTime() + 84_000).toISOString(),
    current_step: 8,
    total_steps: 8,
    message: "Operation complete",
    failures: [],
  },
];

const restorePointTemplate: RestorePoint = {
  id: "4ee659a4-58d6-4686-8a7b-9fb49436523d",
  prefix_id: prefixes[0].id,
  prefix_name: prefixes[0].name,
  prefix_path: prefixes[0].path,
  storage_path: "/home/user/.local/state/bettertricks/backups/default/4ee659a4",
  method: "reflink",
  created_at: dayAgo,
  size_bytes: 247_463_936,
  operation_id: null,
};
const restorePoints: RestorePoint[] = [{ ...restorePointTemplate }];

let settings: AppSettings = {
  theme: "system",
  language: "system",
  catalog_auto_update: true,
  restore_before_managed_changes: true,
  show_advanced: false,
  reduced_motion: false,
  custom_wine_binary: null,
};

const listeners = new Set<(event: OperationEvent) => void>();
const importedManualFiles = new Set<string>();

export const mockBackend = {
  async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    await delay(command === "bootstrap" ? 420 : 100);
    switch (command) {
      case "bootstrap":
        return bootstrap() as T;
      case "install_compatibility_host":
        return bootstrap().system as T;
      case "list_prefixes":
      case "register_prefix":
      case "create_prefix":
      case "trash_prefix":
        return [...prefixes] as T;
      case "unregister_prefix": {
        const index = prefixes.findIndex((prefix) => prefix.path === String(args?.path));
        if (index >= 0) prefixes.splice(index, 1);
        return [...prefixes] as T;
      }
      case "catalog_search":
        return search((args?.query ?? {}) as CatalogQuery) as T;
      case "get_recipe":
        return fullRecipe(String(args?.id)) as T;
      case "import_manual_file":
        importedManualFiles.add(`${String(args?.recipeId)}:${String(args?.fileId)}`);
        return String(args?.path) as T;
      case "plan_operation":
        return makePlan(args?.request as OperationRequest) as T;
      case "start_operation": {
        const plan = args?.plan as OperationPlan;
        simulateOperation(plan);
        return plan.id as T;
      }
      case "operation_history":
        return [...operations] as T;
      case "clear_operation_history": {
        const active = operations.filter((operation) => !["succeeded", "failed", "cancelled"].includes(operation.state));
        operations.splice(0, operations.length, ...active);
        return [...operations] as T;
      }
      case "save_settings":
        settings = args?.settings as AppSettings;
        return settings as T;
      case "clear_cache":
        return { path: "/home/user/.cache/winetricks", file_count: 0, size_bytes: 0 } as T;
      case "list_restore_points":
        return [...restorePoints] as T;
      case "create_restore_point": {
        const prefix = prefixes.find((item) => item.id === String(args?.prefixId)) ?? prefixes[0];
        const point = {
          ...restorePointTemplate,
          id: crypto.randomUUID(),
          prefix_id: prefix.id,
          prefix_name: prefix.name,
          prefix_path: prefix.path,
          created_at: new Date().toISOString(),
        };
        restorePoints.unshift(point);
        return point as T;
      }
      case "clear_restore_points": {
        const cleared = restorePoints.length;
        restorePoints.splice(0, restorePoints.length);
        return { cleared, protected: 0, restore_points: [] } as T;
      }
      case "restore_prefix":
      case "open_url":
        return undefined as T;
      case "inspect_legacy_verb":
        return {
          path: String(args?.path),
          id: "custom",
          category: "dlls",
          title: "Custom compatibility component",
          size_bytes: 4_820,
          warning: "Legacy .verb files are shell programs and run with your user permissions. Only continue for code you have reviewed and trust.",
        } satisfies LegacyVerbInfo as T;
      case "run_legacy_verb":
        return undefined as T;
      case "check_catalog_update":
        return { configured: false, signed: false, rollback_available: false, available: null, message: "No signed catalog update channel is configured." } as T;
      case "rollback_catalog_version":
      case "install_catalog_update":
        return bootstrap().catalog as T;
      default:
        return undefined as T;
    }
  },
  listen(callback: (event: OperationEvent) => void) {
    listeners.add(callback);
    return () => listeners.delete(callback);
  },
};

function bootstrap(): BootstrapPayload {
  return {
    system: {
      ready: true,
      os: "CachyOS Linux",
      architecture: "x86_64",
      desktop: "KDE",
      dependencies: [
        dependency("wine", "Wine", true, "wine-11.13"),
        dependency("wineserver", "Wine server", true, "wine-11.13"),
        dependency("cabextract", "Cabinet extraction", true, "1.11"),
        dependency("7z", "7-Zip extraction", true, "24.09"),
        dependency("unzip", "ZIP extraction", true, "6.00"),
        dependency("gzip", "Gzip extraction", true, "1.13"),
        dependency("xz", "XZ extraction", true, "5.6.1"),
        dependency("tar", "Archive restore points", true, "1.35"),
        dependency("zstd", "Compressed restore points", true, "1.5.5"),
        dependency("aria2c", "Parallel downloads", false, null),
        dependency("btrfs", "Btrfs restore points", true, "6.15"),
        dependency("winetricks", "Winetricks compatibility host", true, "20260125"),
      ],
      runtimes: [
        { id: "system", label: "System Wine 11.13", wine_binary: "/usr/bin/wine", wineserver_binary: "/usr/bin/wineserver", version: "wine-11.13", source: "system" },
        { id: "proton", label: "Proton Experimental", wine_binary: "/steam/Proton/files/bin/wine", wineserver_binary: null, version: "Proton 10", source: "steam" },
      ],
      data_directory: "/home/user/.local/share/bettertricks",
      cache_directory: "/home/user/.cache/winetricks",
      state_directory: "/home/user/.local/state/bettertricks",
    },
    prefixes: [...prefixes],
    catalog: {
      version: "winetricks-20260125",
      upstream_tag: "20260125",
      recipe_count: 550,
      native_count: 159,
      metadata_only_count: 390,
      categories: { apps: 59, benchmarks: 8, dlls: 323, fonts: 42, settings: 118 },
    },
    settings,
    cache: { path: "/home/user/.cache/winetricks", file_count: 84, size_bytes: 1_483_456_512 },
    operations: [...operations],
    restore_points: [...restorePoints],
    catalog_signed: false,
    catalog_rollback_available: false,
  };
}

function search(query: CatalogQuery): RecipeListItem[] {
  const term = query.search?.trim().toLowerCase() ?? "";
  return recipes
    .map((recipe) => ({
      ...recipe,
      installed: prefixes.find((prefix) => prefix.id === query.prefix_id)?.installed_verbs.includes(recipe.id) ?? recipe.installed,
    }))
    .filter((recipe) => !query.category || recipe.category === query.category)
    .filter((recipe) => !query.media || recipe.media === query.media)
    .filter((recipe) => !query.installed_only || recipe.installed)
    .filter((recipe) => !query.cached_only || recipe.cached)
    .filter((recipe) => !query.compatible_only || recipe.compatible)
    .filter((recipe) => !term || `${recipe.id} ${recipe.title} ${recipe.publisher ?? ""} ${recipe.tags.join(" ")}`.toLowerCase().includes(term));
}

function fullRecipe(id: string): Recipe {
  const item = recipes.find((recipe) => recipe.id === id) ?? recipes[0];
  const steps: Recipe["steps"] = item.maturity === "native"
    ? item.id.startsWith("win")
      ? [{ type: "windows_version", version: item.id }]
      : [{ type: "native_action", action: "font_smoothing" }]
    : [];
  const inputs: Recipe["inputs"] = item.id === "set_mididevice" ? [{
    id: "device",
    label: "MIDI device",
    description: "The exact Windows MIDI output device name.",
    placeholder: "Microsoft GS Wavetable Synth",
    environment: "MIDI_DEVICE",
    required: true,
  }] : [];
  return {
    schema: 1,
    id: item.id,
    category: item.category,
    title: item.title,
    publisher: item.publisher,
    year: item.year,
    description: item.description,
    media: item.media,
    maturity: item.maturity,
    tags: item.tags,
    dependencies: item.id === "vcrun2022" ? ["remove_mono"] : [],
    conflicts: [],
    constraints: {
      architectures: [],
      min_wine: null,
      max_wine: null,
      new_wow64_supported: true,
      broken_reason: item.compatibility_reason,
      bug_url: null,
    },
    files: item.maturity !== "native" || item.media === "none" ? [] : [{ id: "installer", filename: `${item.id}.exe`, cache_path: null, urls: item.media === "manual_download" ? [] : ["https://example.invalid/installer"], sha256: "5d41402abc4b2a76b9719d911017c5925d41402abc4b2a76b9719d911017c592", manual: item.media === "manual_download" }],
    inputs,
    detect: [],
    steps,
    verify: [],
    source: { upstream_tag: "20260125", upstream_commit: null, upstream_verb: item.id },
  };
}

function makePlan(request: OperationRequest): OperationPlan {
  const prefix = prefixes.find((item) => item.id === request.prefix_id) ?? prefixes[0];
  const selected = request.recipes.map(fullRecipe);
  const steps = selected.flatMap((recipe) => recipe.maturity === "metadata_only" ? [{
    recipe_id: recipe.id,
    recipe_title: recipe.title,
    step_index: 0,
    label: `Run ${recipe.id} through Winetricks`,
    destructive: true,
  }] : recipe.steps.map((step, index) => ({
      recipe_id: recipe.id,
      recipe_title: recipe.title,
      step_index: index,
      label: step.type === "windows_version" ? `Set Windows version to ${String(step.version)}` : "Update font smoothing",
      destructive: false,
    })));
  const inputs = selected.flatMap((recipe) => recipe.inputs.map((input) => {
    const key = `${recipe.id}.${input.id}`;
    return {
      key,
      recipe_id: recipe.id,
      id: input.id,
      label: input.label,
      description: input.description,
      placeholder: input.placeholder,
      required: input.required,
      value: request.input_values[key] ?? null,
    };
  }));
  return {
    id: crypto.randomUUID(),
    prefix,
    requested_recipes: request.recipes,
    resolved_recipes: request.recipes,
    steps,
    inputs,
    downloads: selected.flatMap((recipe) => recipe.files.map((file) => ({ recipe_id: recipe.id, file_id: file.id, filename: file.filename, urls: file.urls, cached: importedManualFiles.has(`${recipe.id}:${file.id}`), manual: file.manual }))),
    conflicts: [],
    warnings: [
      ...compatibilityHostWarnings(selected),
      ...(prefix.managed ? [{ code: "managed_prefix", title: "Managed prefix", message: "Close the game and launcher before applying changes.", recipe_id: null }] : []),
    ],
    restore_recommended: prefix.managed || selected.some((recipe) => recipe.maturity === "metadata_only"),
    estimated_download_bytes: null,
    options: request.options,
  };
}

function compatibilityHostWarnings(selected: Recipe[]) {
  const hosted = selected.filter((recipe) => recipe.maturity === "metadata_only");
  if (hosted.length === 0) return [];
  const upstreamTag = hosted[0].source.upstream_tag;
  return [{
    code: "winetricks_compatibility_host",
    title: hosted.length === 1
      ? `${hosted[0].title} uses the Winetricks compatibility host`
      : `${hosted.length} selected recipes use the Winetricks compatibility host`,
    message: hosted.length === 1
      ? `This tracked recipe runs through locally installed Winetricks ${upstreamTag}.`
      : `These tracked recipes run through locally installed Winetricks ${upstreamTag}.`,
    recipe_id: hosted.length === 1 ? hosted[0].id : null,
  }];
}

function simulateOperation(plan: OperationPlan) {
  const total = Math.max(plan.steps.length, 3);
  let step = 0;
  const operationId = plan.id;
  const tick = () => {
    step += 1;
    const done = step >= total;
    const event: OperationEvent = {
      operation_id: operationId,
      sequence: step,
      state: done ? "succeeded" : "running",
      step,
      total_steps: total,
      recipe_id: plan.resolved_recipes[Math.min(step - 1, plan.resolved_recipes.length - 1)] ?? null,
      title: done ? "Operation complete" : plan.steps[step - 1]?.label ?? "Preparing Wine prefix",
      detail: done ? `Applied ${plan.resolved_recipes.length} recipe(s)` : plan.prefix.name,
      progress: step / total,
      prompt: null,
      log_line: null,
      failure: null,
      timestamp: new Date().toISOString(),
    };
    listeners.forEach((listener) => listener(event));
    if (!done) window.setTimeout(tick, 650);
  };
  window.setTimeout(tick, 250);
}

function dependency(id: string, label: string, available: boolean, version: string | null) {
  const required = new Set(["wine", "wineserver", "cabextract", "7z", "unzip", "gzip", "xz", "tar", "zstd"]).has(id);
  return { id, label, required, available, path: available ? `/usr/bin/${id}` : null, version, remediation: available ? null : `Install ${label}` };
}

function description(category: string, title: string) {
  if (category === "settings") return `${title} in the selected Wine prefix.`;
  if (category === "dlls") return `Install or configure ${title} for Windows application compatibility.`;
  return `Install ${title} in the selected Wine prefix.`;
}

function delay(milliseconds: number) {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}
