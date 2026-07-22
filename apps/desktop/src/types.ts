export type UUID = string;

export type VerbCategory = "apps" | "benchmarks" | "dlls" | "fonts" | "settings";
export type MediaKind = "none" | "download" | "manual_download";
export type RecipeMaturity = "native" | "metadata_only" | "broken_upstream";
export type PrefixArchitecture = "win32" | "win64" | "wow64" | "unknown";
export type PrefixSource =
  | "default_wine"
  | "wine_prefixes"
  | "steam"
  | "lutris"
  | "bottles"
  | "heroic"
  | "manual";
export type OperationState =
  | "planned"
  | "preflight"
  | "running"
  | "waiting_for_user"
  | "cancelling"
  | "succeeded"
  | "failed"
  | "cancelled";
export type RecipeFailureKind = "failed" | "skipped_dependency";
export type ThemePreference = "system" | "light" | "dark";

export interface WinePrefix {
  id: UUID;
  name: string;
  path: string;
  source: PrefixSource;
  architecture: PrefixArchitecture;
  runtime: string | null;
  runtime_label: string | null;
  managed: boolean;
  exists: boolean;
  installed_verbs: string[];
  size_bytes: number | null;
  last_modified: string | null;
}

export interface WineRuntime {
  id: string;
  label: string;
  wine_binary: string;
  wineserver_binary: string | null;
  version: string | null;
  source: string;
}

export interface DependencyCheck {
  id: string;
  label: string;
  required: boolean;
  available: boolean;
  path: string | null;
  version: string | null;
  remediation: string | null;
}

export interface SystemReport {
  ready: boolean;
  os: string;
  architecture: string;
  desktop: string | null;
  dependencies: DependencyCheck[];
  runtimes: WineRuntime[];
  data_directory: string;
  cache_directory: string;
  state_directory: string;
}

export interface CatalogSummary {
  version: string;
  upstream_tag: string;
  recipe_count: number;
  native_count: number;
  metadata_only_count: number;
  categories: Record<string, number>;
}

export interface LegacyVerbInfo {
  path: string;
  id: string;
  category: VerbCategory;
  title: string | null;
  size_bytes: number;
  warning: string;
}

export interface CatalogRelease {
  version: string;
  upstream_tag: string;
  url: string;
  sha256: string;
  signature: string;
  recipe_count: number;
}

export interface CatalogUpdateStatus {
  configured: boolean;
  signed: boolean;
  rollback_available: boolean;
  available: CatalogRelease | null;
  message: string;
}

export interface RecipeListItem {
  id: string;
  category: VerbCategory;
  title: string;
  publisher: string | null;
  year: string | null;
  description: string | null;
  media: MediaKind;
  maturity: RecipeMaturity;
  tags: string[];
  installed: boolean;
  cached: boolean;
  compatible: boolean;
  compatibility_reason: string | null;
}

export interface RecipeFile {
  id: string;
  filename: string;
  cache_path: string | null;
  urls: string[];
  sha256: string | null;
  manual: boolean;
}

export interface RecipeStep {
  type: string;
  [key: string]: unknown;
}

export interface RecipeInput {
  id: string;
  label: string;
  description: string | null;
  placeholder: string | null;
  environment: string | null;
  required: boolean;
}

export interface Recipe extends Omit<RecipeListItem, "installed" | "cached" | "compatible" | "compatibility_reason"> {
  schema: number;
  dependencies: string[];
  conflicts: string[];
  constraints: {
    architectures: PrefixArchitecture[];
    min_wine: string | null;
    max_wine: string | null;
    new_wow64_supported: boolean | null;
    broken_reason: string | null;
    bug_url: string | null;
  };
  files: RecipeFile[];
  inputs: RecipeInput[];
  detect: Array<{ path: string; kind: string }>;
  steps: RecipeStep[];
  verify: RecipeStep[];
  source: {
    upstream_tag: string;
    upstream_commit: string | null;
    upstream_verb: string;
  };
}

export interface CatalogQuery {
  search: string | null;
  category: VerbCategory | null;
  media: MediaKind | null;
  installed_only: boolean;
  cached_only: boolean;
  compatible_only: boolean;
  prefix_id: UUID | null;
}

export interface OperationOptions {
  force: boolean;
  unattended: boolean;
  verify: boolean;
  no_clean: boolean;
  isolate: boolean;
  torify: boolean;
  country: string | null;
  create_restore_point: boolean;
}

export interface OperationRequest {
  prefix_id: UUID;
  recipes: string[];
  input_values: Record<string, string>;
  options: OperationOptions;
}

export interface PlannedInput {
  key: string;
  recipe_id: string;
  id: string;
  label: string;
  description: string | null;
  placeholder: string | null;
  required: boolean;
  value: string | null;
}

export interface PlannedStep {
  recipe_id: string;
  recipe_title: string;
  step_index: number;
  label: string;
  destructive: boolean;
}

export interface PlanIssue {
  code: string;
  title: string;
  message: string;
  recipe_id: string | null;
}

export interface PlannedDownload {
  recipe_id: string;
  file_id: string;
  filename: string;
  urls: string[];
  cached: boolean;
  manual: boolean;
}

export interface OperationPlan {
  id: UUID;
  prefix: WinePrefix;
  requested_recipes: string[];
  resolved_recipes: string[];
  steps: PlannedStep[];
  inputs: PlannedInput[];
  downloads: PlannedDownload[];
  conflicts: PlanIssue[];
  warnings: PlanIssue[];
  restore_recommended: boolean;
  estimated_download_bytes: number | null;
  options: OperationOptions;
}

export interface OperationPrompt {
  id: UUID;
  level: "info" | "warning" | "confirmation";
  title: string;
  message: string;
  choices: Array<{ id: string; label: string; destructive: boolean }>;
}

export interface RecipeFailure {
  recipe_id: string;
  recipe_title: string;
  kind: RecipeFailureKind;
  message: string;
}

export interface OperationEvent {
  operation_id: UUID;
  sequence: number;
  state: OperationState;
  step: number;
  total_steps: number;
  recipe_id: string | null;
  title: string;
  detail: string | null;
  progress: number | null;
  prompt: OperationPrompt | null;
  log_line: string | null;
  failure: RecipeFailure | null;
  timestamp: string;
}

export interface OperationRecord {
  id: UUID;
  prefix_id: UUID;
  prefix_name: string;
  recipes: string[];
  state: OperationState;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  current_step: number;
  total_steps: number;
  message: string | null;
  failures: RecipeFailure[];
}

export interface RestorePoint {
  id: UUID;
  prefix_id: UUID;
  prefix_name: string;
  prefix_path: string;
  storage_path: string;
  method: "btrfs" | "reflink" | "archive";
  created_at: string;
  size_bytes: number | null;
  operation_id: UUID | null;
}

export interface ClearRestorePointsResult {
  cleared: number;
  protected: number;
  restore_points: RestorePoint[];
}

export interface CacheStats {
  path: string;
  file_count: number;
  size_bytes: number;
}

export interface AppSettings {
  theme: ThemePreference;
  language: string;
  catalog_auto_update: boolean;
  restore_before_managed_changes: boolean;
  show_advanced: boolean;
  reduced_motion: boolean;
  custom_wine_binary: string | null;
}

export interface BootstrapPayload {
  system: SystemReport;
  prefixes: WinePrefix[];
  catalog: CatalogSummary;
  settings: AppSettings;
  cache: CacheStats;
  operations: OperationRecord[];
  restore_points: RestorePoint[];
  catalog_signed: boolean;
  catalog_rollback_available: boolean;
}
