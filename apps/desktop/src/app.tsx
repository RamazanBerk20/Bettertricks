import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import clsx from "clsx";
import {
  ArrowClockwise,
  CheckCircle,
  HardDrive,
  Package,
  WarningCircle,
  Wine,
  X,
} from "@phosphor-icons/react";
import { api } from "./lib/api";
import { applyOperationEvent, entryFromLive, isTerminal, LIVE_STATES, type OperationStore } from "./lib/operations";
import { I18nProvider, useI18n } from "./lib/i18n";
import { normalizeLanguagePreference, resolveLanguagePreference } from "./lib/translations";
import type {
  AppSettings,
  BootstrapPayload,
  LiveOperation,
  OperationPlan,
  UUID,
  WinePrefix,
} from "./types";
import { Button, Skeleton } from "./components/common";
import { Sidebar, type AppView } from "./components/sidebar";
import { PrefixView } from "./views/prefix-view";
import { CatalogView } from "./views/catalog-view";
import { ActivityView } from "./views/activity-view";
import { SettingsView } from "./views/settings-view";
import {
  AddPrefixDialog,
  CommandPalette,
  LegacyVerbDialog,
  OperationDrawer,
  ReviewDialog,
  TrashPrefixDialog,
} from "./components/dialogs";

export function App() {
  return <I18nProvider><AppContent /></I18nProvider>;
}

function AppContent() {
  const { t, setLocale } = useI18n();
  const [data, setData] = useState<BootstrapPayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [fatalError, setFatalError] = useState<string | null>(null);
  const [view, setView] = useState<AppView>("prefix");
  const [selectedPrefixId, setSelectedPrefixId] = useState<UUID | null>(null);
  const [selectedRecipes, setSelectedRecipes] = useState<Set<string>>(new Set());
  const [addPrefixOpen, setAddPrefixOpen] = useState(false);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [trashOpen, setTrashOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [legacyVerbOpen, setLegacyVerbOpen] = useState(false);
  const [activeOperationId, setActiveOperationId] = useState<UUID | null>(null);
  const [activePlan, setActivePlan] = useState<OperationPlan | null>(null);
  const [operationDrawerOpen, setOperationDrawerOpen] = useState(false);
  const [operationStore, setOperationStore] = useState<OperationStore>(new Map());
  const [livePlans, setLivePlans] = useState<Map<UUID, OperationPlan>>(new Map());
  const activeOperationRef = useRef<UUID | null>(null);
  activeOperationRef.current = activeOperationId;
  const [installingCompatibilityHost, setInstallingCompatibilityHost] = useState(false);
  const [toast, setToast] = useState<{ tone: "success" | "error" | "info"; message: string } | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const payload = await api.bootstrap();
      const locale = resolveLanguagePreference(payload.settings.language);
      setLocale(locale);
      setData(payload);
      setSelectedPrefixId((current) => current && payload.prefixes.some((prefix) => prefix.id === current) ? current : payload.prefixes[0]?.id ?? null);
      attachLiveOperations(payload.live_operations ?? []);
      setFatalError(null);
    } catch (reason) {
      setFatalError(String(reason));
    } finally {
      setLoading(false);
    }
  }, [setLocale]);

  /** Re-attaches to operations the backend still runs; the prompt state lives in the backend. */
  const attachLiveOperations = useCallback((live: LiveOperation[]) => {
    if (!live.length) return;
    setLivePlans((current) => new Map([...current, ...live.map((operation) => [operation.plan.id, operation.plan] as const)]));
    setOperationStore((current) => {
      const next = new Map(current);
      for (const operation of live) if (!next.has(operation.plan.id)) next.set(operation.plan.id, entryFromLive(operation));
      return next;
    });
    const focus = live.find((operation) => operation.prompt) ?? live[0];
    if (!activeOperationRef.current) {
      setActiveOperationId(focus.plan.id);
      setActivePlan(focus.plan);
      if (focus.prompt) setOperationDrawerOpen(true);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  useEffect(() => {
    let unlisten: undefined | (() => void);
    api.onOperationEvent((event) => {
      setOperationStore((current) => applyOperationEvent(current, event, activeOperationRef.current));
      if (isTerminal(event.state)) {
        setLivePlans((current) => {
          if (!current.has(event.operation_id)) return current;
          const next = new Map(current);
          next.delete(event.operation_id);
          return next;
        });
        setToast({ tone: event.state === "succeeded" ? "success" : "error", message: event.title });
        void refreshOperationalData();
      }
    }).then((cleanup) => { unlisten = cleanup; });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlisten: undefined | (() => void);
    api.onCatalogUpdated((release) => {
      setToast({ tone: "success", message: t("Signed catalog {version} installed", { version: release.version }) });
      void load();
    }).then((cleanup) => { unlisten = cleanup; });
    return () => unlisten?.();
  }, [load, t]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
      if (event.key === "/" && !(event.target instanceof HTMLInputElement) && !(event.target instanceof HTMLTextAreaElement)) {
        event.preventDefault();
        setView("catalog");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  useEffect(() => {
    if (!data) return;
    applyTheme(data.settings);
  }, [data?.settings]);

  useEffect(() => {
    if (!data || normalizeLanguagePreference(data.settings.language) !== "system") return;
    const syncSystemLanguage = () => setLocale(resolveLanguagePreference("system"));
    syncSystemLanguage();
    window.addEventListener("languagechange", syncSystemLanguage);
    return () => window.removeEventListener("languagechange", syncSystemLanguage);
  }, [data?.settings.language, setLocale]);

  useEffect(() => {
    if (!toast) return;
    const timeout = window.setTimeout(() => setToast(null), 4200);
    return () => window.clearTimeout(timeout);
  }, [toast]);

  const selectedPrefix = useMemo(
    () => data?.prefixes.find((prefix) => prefix.id === selectedPrefixId) ?? null,
    [data?.prefixes, selectedPrefixId],
  );
  const activeEntry = activeOperationId ? operationStore.get(activeOperationId) ?? null : null;
  const activeOperationEvent = activeEntry?.latest ?? undefined;
  const activeOperationFinished = isTerminal(activeEntry?.state);
  const runningCount = data?.operations.filter((operation) => LIVE_STATES.has(operation.state)).length ?? 0;
  const compatibilityHostReady = data?.system.compatibility_host?.state === "ready";

  useEffect(() => {
    if (activeEntry?.prompt) setOperationDrawerOpen(true);
  }, [activeEntry?.prompt?.id]);

  // A prompt raised by any live operation (not only the active one) must reach the user.
  useEffect(() => {
    for (const [id, entry] of operationStore) {
      if (entry.prompt && id !== activeOperationId && livePlans.has(id)) {
        setActiveOperationId(id);
        setActivePlan(livePlans.get(id) ?? null);
        setOperationDrawerOpen(true);
        break;
      }
    }
  }, [operationStore, activeOperationId, livePlans]);

  const openOperation = useCallback((operationId: UUID) => {
    const plan = livePlans.get(operationId);
    if (!plan) return;
    setActiveOperationId(operationId);
    setActivePlan(plan);
    setOperationDrawerOpen(true);
  }, [livePlans]);

  const cancelOperation = useCallback((operationId: UUID) => {
    api.cancelOperation(operationId).catch((reason) => setToast({ tone: "error", message: String(reason) }));
  }, []);

  async function refreshOperationalData() {
    try {
      const [prefixes, operations] = await Promise.all([api.listPrefixes(), api.operationHistory()]);
      setData((current) => current ? { ...current, prefixes, operations } : current);
    } catch {
      // The live operation result remains visible even if a background refresh fails.
    }
  }

  const updateSettings = useCallback((settings: AppSettings) => {
    setData((current) => current ? { ...current, settings } : current);
    applyTheme(settings);
    const locale = resolveLanguagePreference(settings.language);
    setLocale(locale);
    api.saveSettings(settings).catch((reason) => setToast({ tone: "error", message: String(reason) }));
  }, [setLocale]);

  const clearActivity = useCallback(async () => {
    try {
      const operations = await api.clearOperationHistory();
      const remainingIds = new Set(operations.map((operation) => operation.id));
      setData((current) => current ? { ...current, operations } : current);
      setOperationStore((current) => new Map([...current].filter(([id]) => remainingIds.has(id) || id === activeOperationId)));
      setToast({ tone: "success", message: t("Activity history cleared") });
    } catch (reason) {
      setToast({ tone: "error", message: t("Could not clear activity history: {error}", { error: String(reason) }) });
      throw reason;
    }
  }, [activeOperationId, t]);

  const browseCatalog = useCallback(() => setView("catalog"), []);
  const openPath = useCallback((path: string) => {
    api.openPath(path).catch((reason) => setToast({ tone: "error", message: String(reason) }));
  }, []);
  const openTrashPrefixDialog = useCallback(() => setTrashOpen(true), []);

  const handlePrefixComplete = (prefixes: WinePrefix[], path: string) => {
    setData((current) => current ? { ...current, prefixes } : current);
    const normalized = path.replace(/\/+$/, "");
    const added = prefixes.find((prefix) => prefix.path === normalized) ?? prefixes.find((prefix) => prefix.path.endsWith(normalized.split("/").at(-1) ?? ""));
    if (added) setSelectedPrefixId(added.id);
    setView("prefix");
    setToast({ tone: "success", message: t("Wine prefix is ready") });
  };

  const toggleRecipe = useCallback((id: string) => {
    setSelectedRecipes((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const setRecipeSelection = useCallback((ids: string[], selected: boolean) => {
    setSelectedRecipes((current) => {
      const next = new Set(current);
      for (const id of ids) {
        if (selected) next.add(id); else next.delete(id);
      }
      return next;
    });
  }, []);

  const prepareRetry = useCallback((prefixId: UUID, recipeIds: string[]) => {
    const prefix = data?.prefixes.find((candidate) => candidate.id === prefixId);
    const uniqueRecipeIds = [...new Set(recipeIds)].filter(Boolean);
    if (!prefix) {
      setToast({ tone: "error", message: t("The target Wine prefix is no longer available.") });
      return;
    }
    if (!uniqueRecipeIds.length) {
      setToast({ tone: "error", message: t("There are no failed components to retry.") });
      return;
    }
    setSelectedPrefixId(prefix.id);
    setSelectedRecipes(new Set(uniqueRecipeIds));
    setOperationDrawerOpen(false);
    setReviewOpen(true);
    setToast({ tone: "info", message: t("Reviewing components before retry: {count}", { count: uniqueRecipeIds.length }) });
  }, [data?.prefixes, t]);

  const launchTool = useCallback(async (tool: string) => {
    if (!selectedPrefix) return;
    try {
      await api.launchPrefixTool({ prefix_id: selectedPrefix.id, tool });
      setToast({ tone: "info", message: t("Opened {tool} in {prefix}", { tool, prefix: selectedPrefix.name }) });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  }, [selectedPrefix, t]);

  const runInstaller = useCallback(async () => {
    if (!selectedPrefix) return;
    try {
      const file = await api.selectInstaller();
      if (!file) return;
      await api.launchPrefixTool({ prefix_id: selectedPrefix.id, tool: "installer", file });
      setToast({ tone: "info", message: t("Opened installer in {prefix}", { prefix: selectedPrefix.name }) });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  }, [selectedPrefix, t]);

  const forgetPrefix = useCallback(async () => {
    if (!selectedPrefix || selectedPrefix.source !== "manual") return;
    if (!window.confirm(t("Forget {prefix}? Its files will remain on disk.", { prefix: selectedPrefix.name }))) return;
    try {
      const forgottenName = selectedPrefix.name;
      const prefixes = await api.unregisterPrefix(selectedPrefix.path);
      setData((current) => current ? { ...current, prefixes } : current);
      setSelectedPrefixId(prefixes[0]?.id ?? null);
      setToast({ tone: "success", message: t("{prefix} forgotten; its files were not deleted", { prefix: forgottenName }) });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  }, [selectedPrefix, t]);

  const createRestorePoint = useCallback(async () => {
    if (!selectedPrefix) return;
    setToast({ tone: "info", message: t("Creating a restore point for {prefix}", { prefix: selectedPrefix.name }) });
    try {
      const point = await api.createRestorePoint(selectedPrefix.id);
      setData((current) => current ? { ...current, restore_points: [point, ...current.restore_points] } : current);
      setToast({ tone: "success", message: t("Restore point created") });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  }, [selectedPrefix, t]);

  const clearCache = async () => {
    if (!window.confirm(t("Clear the shared Winetricks download cache? Installers can be downloaded again later."))) return;
    try {
      const cache = await api.clearCache();
      setData((current) => current ? { ...current, cache } : current);
      setToast({ tone: "success", message: t("Download cache cleared") });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  };

  const clearRestorePoints = useCallback(async () => {
    try {
      const result = await api.clearRestorePoints();
      setData((current) => current ? { ...current, restore_points: result.restore_points } : current);
      setToast({
        tone: "success",
        message: result.protected
          ? t("Restore points cleared; {count} in use were kept.", { count: result.protected })
          : t("Restore points cleared"),
      });
    } catch (reason) {
      setToast({ tone: "error", message: t("Could not clear restore points: {error}", { error: String(reason) }) });
      throw reason;
    }
  }, [t]);

  const checkCatalog = async () => {
    setToast({ tone: "info", message: t("Checking the signed catalog channel") });
    try {
      const status = await api.checkCatalogUpdate();
      if (!status.available) {
        setToast({ tone: "info", message: status.message });
        return;
      }
      setToast({ tone: "info", message: t("Installing signed catalog {version}", { version: status.available.version }) });
      const catalog = await api.installCatalogUpdate(status.available);
      setData((current) => current ? { ...current, catalog, catalog_signed: true, catalog_rollback_available: true } : current);
      setToast({ tone: "success", message: t("Catalog {version} installed", { version: catalog.version }) });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  };

  const rollbackCatalog = async () => {
    if (!window.confirm(t("Roll back to the previously installed recipe catalog?"))) return;
    try {
      const catalog = await api.rollbackCatalog();
      await load();
      setToast({ tone: "success", message: t("Rolled back to catalog {version}", { version: catalog.version }) });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  };

  const installCompatibilityHost = async () => {
    const baseline = data?.catalog.upstream_tag;
    if (!baseline) return;
    setInstallingCompatibilityHost(true);
    setToast({ tone: "info", message: t("Installing verified Winetricks {version}", { version: baseline }) });
    try {
      const system = await api.installCompatibilityHost();
      setData((current) => current ? { ...current, system } : current);
      setToast({ tone: "success", message: t("Verified Winetricks {version} is ready", { version: baseline }) });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    } finally {
      setInstallingCompatibilityHost(false);
    }
  };

  const deleteRestorePoint = async (restorePointId: UUID) => {
    if (!window.confirm(t("Permanently delete this snapshot? This cannot be undone."))) return;
    try {
      const restorePoints = await api.deleteRestorePoint(restorePointId);
      setData((current) => current ? { ...current, restore_points: restorePoints } : current);
      setToast({ tone: "success", message: t("Snapshot deleted") });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  };

  const restoreSnapshot = async (restorePointId: UUID) => {
    if (!window.confirm(t("Restore this snapshot? The current prefix will be moved to Trash first."))) return;
    try {
      await api.restorePrefix(restorePointId, true);
      await load();
      setToast({ tone: "success", message: t("Wine prefix restored") });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  };

  const restorePrefix = async (restorePointId: UUID) => {
    if (!window.confirm(t("Restore this saved prefix? The original target directory must not exist."))) return;
    try {
      await api.restorePrefix(restorePointId);
      await load();
      setToast({ tone: "success", message: t("Wine prefix restored") });
    } catch (reason) {
      setToast({ tone: "error", message: String(reason) });
    }
  };

  const trashPrefix = async (confirmation: string) => {
    if (!selectedPrefix) return;
    const prefixes = await api.trashPrefix(selectedPrefix.id, confirmation);
    setData((current) => current ? { ...current, prefixes } : current);
    setSelectedPrefixId(prefixes[0]?.id ?? null);
    setToast({ tone: "success", message: t("{prefix} moved to Trash", { prefix: selectedPrefix.name }) });
  };

  if (loading && !data) return <LoadingScreen />;
  if (fatalError || !data) return <FatalError error={fatalError ?? t("Bettertricks could not initialize")} onRetry={load} />;

  return (
    <div className={clsx("app-shell", data.settings.reduced_motion && "reduce-motion")} data-operation-state={activeEntry?.state ?? "idle"}>
      <a className="skip-link" href="#main-content">{t("Skip to content")}</a>
      <Sidebar
        view={view}
        onViewChange={setView}
        prefixes={data.prefixes}
        selectedPrefixId={selectedPrefixId}
        onPrefixChange={setSelectedPrefixId}
        onAddPrefix={() => setAddPrefixOpen(true)}
        onCommandPalette={() => setPaletteOpen(true)}
        catalog={data.catalog}
        system={data.system}
        runningCount={runningCount}
        operationStatus={activeOperationId && activePlan && !operationDrawerOpen ? {
          title: activeOperationEvent?.title ?? t("Preparing changes"),
          detail: activeOperationFinished
            ? t("View operation result")
            : t("{percent}% · {current} of {total}", { percent: Math.round(Math.max(0, Math.min(1, activeOperationEvent?.progress ?? 0)) * 100), current: activeOperationEvent?.step ?? 0, total: activeOperationEvent?.total_steps ?? activePlan.steps.length }),
          state: activeEntry?.state ?? "planned",
        } : null}
        onOpenOperation={() => setOperationDrawerOpen(true)}
      />
      <main className="main-surface" id="main-content" tabIndex={-1}>
        {!data.system.ready ? <SystemBanner data={data} onOpenSettings={() => setView("settings")} /> : null}
        {view === "prefix" ? (
          <PrefixView
            prefix={selectedPrefix}
            operations={data.operations}
            restorePoints={data.restore_points}
            onBrowseCatalog={browseCatalog}
            onOpenPath={openPath}
            onLaunchTool={launchTool}
            onCreateRestorePoint={createRestorePoint}
            onForgetPrefix={forgetPrefix}
            onRunInstaller={runInstaller}
            onTrashPrefix={openTrashPrefixDialog}
          />
        ) : view === "catalog" ? (
          <CatalogView
            prefix={selectedPrefix}
            catalog={data.catalog}
            compatibilityHostReady={compatibilityHostReady}
            compatibilityHostInstalling={installingCompatibilityHost}
            selectedRecipes={selectedRecipes}
            onToggleRecipe={toggleRecipe}
            onSetRecipeSelection={setRecipeSelection}
            onInstallCompatibilityHost={installCompatibilityHost}
            onReview={() => setReviewOpen(true)}
          />
        ) : view === "activity" ? (
          <ActivityView
            operations={data.operations}
            liveOperationIds={new Set(livePlans.keys())}
            restorePoints={data.restore_points}
            onRefresh={refreshOperationalData}
            onClearActivity={clearActivity}
            onRetryRecipes={prepareRetry}
            onOpenOperation={openOperation}
            onCancelOperation={cancelOperation}
            onRestoreSnapshot={restoreSnapshot}
            onDeleteSnapshot={deleteRestorePoint}
          />
        ) : (
          <SettingsView
            data={data}
            settings={data.settings}
            onSettingsChange={updateSettings}
            onClearCache={clearCache}
            onClearRestorePoints={clearRestorePoints}
            onOpenPath={(path) => api.openPath(path).catch((reason) => setToast({ tone: "error", message: String(reason) }))}
            onOpenLegacyVerb={() => setLegacyVerbOpen(true)}
            onCheckCatalog={checkCatalog}
            onRollbackCatalog={rollbackCatalog}
            onInstallCompatibilityHost={installCompatibilityHost}
            compatibilityHostInstalling={installingCompatibilityHost}
            onRestorePrefix={restorePrefix}
            onDeleteRestorePoint={deleteRestorePoint}
          />
        )}
      </main>

      <AddPrefixDialog open={addPrefixOpen} onOpenChange={setAddPrefixOpen} onComplete={handlePrefixComplete} runtimes={data.system.runtimes} />
      <ReviewDialog
        open={reviewOpen}
        onOpenChange={setReviewOpen}
        prefix={selectedPrefix}
        recipeIds={[...selectedRecipes]}
        settings={data.settings}
        onStarted={(operationId, plan) => {
          setActiveOperationId(operationId);
          setActivePlan(plan);
          setLivePlans((current) => new Map(current).set(operationId, plan));
          setOperationDrawerOpen(true);
          setSelectedRecipes(new Set());
        }}
      />
      <TrashPrefixDialog open={trashOpen} onOpenChange={setTrashOpen} prefix={selectedPrefix} onConfirm={trashPrefix} />
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        prefixes={data.prefixes}
        onNavigate={setView}
        onPrefix={(id) => { setSelectedPrefixId(id); setView("prefix"); }}
        onAddPrefix={() => setAddPrefixOpen(true)}
      />
      <LegacyVerbDialog
        open={legacyVerbOpen}
        onOpenChange={setLegacyVerbOpen}
        prefixes={data.prefixes}
        selectedPrefixId={selectedPrefixId}
        onComplete={() => setToast({ tone: "success", message: t("Legacy verb completed; a restore point was created first") })}
      />
      {operationDrawerOpen ? (
        <OperationDrawer
          operationId={activeOperationId}
          plan={activePlan}
          entry={activeEntry}
          onClose={() => {
            setOperationDrawerOpen(false);
            if (activeOperationFinished) {
              setActiveOperationId(null);
              setActivePlan(null);
            }
          }}
          onCancel={() => activeOperationId && cancelOperation(activeOperationId)}
          onRespond={(promptId, choiceId) => activeOperationId
            ? api.respondToPrompt({ operation_id: activeOperationId, prompt_id: promptId, choice_id: choiceId })
            : Promise.reject(new Error("No active operation"))}
          onRetry={(recipeIds) => {
            if (activePlan) prepareRetry(activePlan.prefix.id, recipeIds);
          }}
        />
      ) : null}
      {toast ? <Toast tone={toast.tone} message={toast.message} onClose={() => setToast(null)} /> : null}
    </div>
  );
}

function LoadingScreen() {
  const { t } = useI18n();
  return (
    <div className="loading-shell" role="status" aria-live="polite">
      <aside><div className="loading-brand"><img className="brand-mark" src="/icon.png" alt="" aria-hidden="true" draggable={false} /><Skeleton className="loading-brand-copy" /></div>{Array.from({ length: 5 }).map((_, index) => <Skeleton className="loading-nav" key={index} />)}</aside>
      <main><div className="loading-heading"><Skeleton className="loading-eyebrow" /><Skeleton className="loading-title" /><Skeleton className="loading-subtitle" /></div><div className="loading-stats">{Array.from({ length: 4 }).map((_, index) => <Skeleton key={index} />)}</div><Skeleton className="loading-content" /></main>
      <div className="loading-status"><Wine size={19} weight="fill" /><span>{t("Inspecting Wine and prefixes…")}</span></div>
    </div>
  );
}

function FatalError({ error, onRetry }: { error: string; onRetry: () => void }) {
  const { t } = useI18n();
  return <div className="fatal-screen" role="alert"><div className="fatal-mark"><WarningCircle size={27} weight="fill" /></div><h1>{t("Bettertricks could not start")}</h1><p>{error}</p><Button variant="primary" onClick={onRetry}><ArrowClockwise /> {t("Try again")}</Button><small>{t("Your Wine prefixes were not changed.")}</small></div>;
}

function SystemBanner({ data, onOpenSettings }: { data: BootstrapPayload; onOpenSettings: () => void }) {
  const { t } = useI18n();
  const missing = data.system.dependencies.filter((dependency) => dependency.required && !dependency.available);
  return <div className="system-banner" role="status"><WarningCircle weight="fill" /><span><strong>{t("Setup needs attention")}</strong><small>{t("Required tools missing: {tools}.", { tools: missing.map((item) => item.label).join(", ") })}</small></span><Button size="small" onClick={onOpenSettings}>{t("Review setup")}</Button></div>;
}

function Toast({ tone, message, onClose }: { tone: "success" | "error" | "info"; message: string; onClose: () => void }) {
  const { t } = useI18n();
  return <div className={clsx("toast", `toast-${tone}`)} role={tone === "error" ? "alert" : "status"} aria-atomic="true"><span>{tone === "success" ? <CheckCircle weight="fill" /> : tone === "error" ? <WarningCircle weight="fill" /> : <Package weight="fill" />}</span><strong dir="auto">{message}</strong><button onClick={onClose} aria-label={t("Dismiss")}><X /></button></div>;
}

function applyTheme(settings: AppSettings) {
  document.documentElement.dataset.theme = settings.theme;
  document.documentElement.style.colorScheme = settings.theme === "system" ? "light dark" : settings.theme;
}
