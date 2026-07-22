import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AppSettings,
  BootstrapPayload,
  CacheStats,
  ClearRestorePointsResult,
  CatalogQuery,
  CatalogRelease,
  CatalogSummary,
  CatalogUpdateStatus,
  LegacyVerbInfo,
  OperationEvent,
  OperationPlan,
  OperationRecord,
  OperationRequest,
  Recipe,
  RecipeListItem,
  RestorePoint,
  SystemReport,
  UUID,
  WinePrefix,
} from "../types";
import { mockBackend } from "./mock";

const isTauri = typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) return invoke<T>(command, args);
  return mockBackend.invoke<T>(command, args);
}

export const api = {
  bootstrap: () => call<BootstrapPayload>("bootstrap"),
  installCompatibilityHost: () => call<SystemReport>("install_compatibility_host"),
  listPrefixes: () => call<WinePrefix[]>("list_prefixes"),
  registerPrefix: (request: { path: string; name: string; runtime: string | null }) =>
    call<WinePrefix[]>("register_prefix", { request }),
  createPrefix: (request: {
    path: string;
    name: string;
    architecture: "win32" | "win64";
    runtime: string | null;
  }) => call<WinePrefix[]>("create_prefix", { request }),
  unregisterPrefix: (path: string) =>
    call<WinePrefix[]>("unregister_prefix", { path }),
  searchCatalog: (query: CatalogQuery) =>
    call<RecipeListItem[]>("catalog_search", { query }),
  getRecipe: (id: string) => call<Recipe>("get_recipe", { id }),
  importManualFile: (recipeId: string, fileId: string, path: string) =>
    call<string>("import_manual_file", { recipeId, fileId, path }),
  planOperation: (request: OperationRequest) =>
    call<OperationPlan>("plan_operation", { request }),
  startOperation: (plan: OperationPlan) => call<UUID>("start_operation", { plan }),
  cancelOperation: (operationId: UUID) =>
    call<void>("cancel_operation", { operationId }),
  respondToPrompt: (response: {
    operation_id: UUID;
    prompt_id: UUID;
    choice_id: string;
  }) => call<void>("respond_to_prompt", { response }),
  operationHistory: () => call<OperationRecord[]>("operation_history"),
  clearOperationHistory: () => call<OperationRecord[]>("clear_operation_history"),
  saveSettings: (settings: AppSettings) =>
    call<AppSettings>("save_settings", { settings }),
  clearCache: () => call<CacheStats>("clear_cache"),
  listRestorePoints: (prefixId?: UUID) =>
    call<RestorePoint[]>("list_restore_points", { prefixId: prefixId ?? null }),
  createRestorePoint: (prefixId: UUID) =>
    call<RestorePoint>("create_restore_point", { prefixId }),
  clearRestorePoints: () =>
    call<ClearRestorePointsResult>("clear_restore_points"),
  restorePrefix: (restorePointId: UUID) =>
    call<void>("restore_prefix", { restorePointId }),
  launchPrefixTool: (request: { prefix_id: UUID; tool: string; file?: string }) =>
    call<void>("launch_prefix_tool", { request }),
  openPath: (path: string) => call<void>("open_path", { path }),
  openUrl: (url: string) => call<void>("open_url", { url }),
  trashPrefix: (prefixId: UUID, confirmation: string) =>
    call<WinePrefix[]>("trash_prefix", { prefixId, confirmation }),
  selectLegacyVerb: async () => {
    if (!isTauri) return "/home/user/Downloads/custom.verb";
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Winetricks verb", extensions: ["verb"] }],
    });
    return typeof selected === "string" ? selected : null;
  },
  selectInstaller: async () => {
    if (!isTauri) return "/home/user/Downloads/setup.exe";
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Windows installer", extensions: ["exe", "msi"] }],
    });
    return typeof selected === "string" ? selected : null;
  },
  selectManualFile: async (filename: string) => {
    if (!isTauri) return `/home/user/Downloads/${filename}`;
    const extension = filename.includes(".") ? filename.split(".").at(-1)?.toLowerCase() : undefined;
    const selected = await open({
      multiple: false,
      directory: false,
      filters: extension && /^[a-z0-9]+$/.test(extension)
        ? [{ name: filename, extensions: [extension] }]
        : undefined,
    });
    return typeof selected === "string" ? selected : null;
  },
  inspectLegacyVerb: (path: string) =>
    call<LegacyVerbInfo>("inspect_legacy_verb", { path }),
  runLegacyVerb: (request: {
    prefix_id: UUID;
    path: string;
    trusted: boolean;
    options: OperationRequest["options"];
  }) => call<void>("run_legacy_verb", { request }),
  checkCatalogUpdate: () => call<CatalogUpdateStatus>("check_catalog_update"),
  installCatalogUpdate: (release: CatalogRelease) =>
    call<CatalogSummary>("install_catalog_update", { release }),
  rollbackCatalog: () => call<CatalogSummary>("rollback_catalog_version"),
  onOperationEvent: async (callback: (event: OperationEvent) => void): Promise<UnlistenFn> => {
    if (isTauri) {
      return listen<OperationEvent>("operation-event", ({ payload }) => callback(payload));
    }
    return mockBackend.listen(callback);
  },
  onCatalogUpdated: async (callback: (release: CatalogRelease) => void): Promise<UnlistenFn> => {
    if (isTauri) {
      return listen<CatalogRelease>("catalog-updated", ({ payload }) => callback(payload));
    }
    return () => undefined;
  },
};
