import { useEffect, useMemo, useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import clsx from "clsx";
import {
  Archive,
  ArrowClockwise,
  ArrowRight,
  Check,
  CheckCircle,
  Command,
  DownloadSimple,
  FolderOpen,
  HardDrive,
  MagnifyingGlass,
  Package,
  Play,
  Plus,
  ShieldCheck,
  SpinnerGap,
  Trash,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import { api } from "../lib/api";
import { titleCase } from "../lib/format";
import { useI18n } from "../lib/i18n";
import type {
  AppSettings,
  LegacyVerbInfo,
  OperationEvent,
  OperationOptions,
  OperationPlan,
  PlannedDownload,
  UUID,
  WinePrefix,
} from "../types";
import type { AppView } from "./sidebar";
import { Badge, Button, ProgressBar, SelectMenu } from "./common";

export function AddPrefixDialog({
  open,
  onOpenChange,
  onComplete,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onComplete: (prefixes: WinePrefix[]) => void;
}) {
  const { t } = useI18n();
  const [mode, setMode] = useState<"existing" | "create">("existing");
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [architecture, setArchitecture] = useState<"win32" | "win64">("win64");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!name.trim() || !path.trim()) {
      setError(t("Enter both a display name and a prefix path."));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const prefixes = mode === "existing"
        ? await api.registerPrefix({ name: name.trim(), path: path.trim(), runtime: null })
        : await api.createPrefix({ name: name.trim(), path: path.trim(), architecture, runtime: null });
      onComplete(prefixes);
      onOpenChange(false);
      setName("");
      setPath("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content add-prefix-dialog" aria-describedby="add-prefix-description">
          <Dialog.Close className="dialog-close" aria-label={t("Close")}><X /></Dialog.Close>
          <div className="dialog-icon"><HardDrive size={22} /></div>
          <Dialog.Title>{t("Add a Wine prefix")}</Dialog.Title>
          <Dialog.Description id="add-prefix-description">{t("Connect an existing prefix or initialize a clean one with system Wine.")}</Dialog.Description>

          <div className="dialog-tabs" role="group" aria-label={t("Prefix setup method")}>
            <button aria-pressed={mode === "existing"} className={mode === "existing" ? "active" : ""} onClick={() => setMode("existing")}><FolderOpen /> {t("Existing prefix")}</button>
            <button aria-pressed={mode === "create"} className={mode === "create" ? "active" : ""} onClick={() => setMode("create")}><Plus /> {t("Create new")}</button>
          </div>

          <div className="form-stack">
            <label><span>{t("Display name")}</span><input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("My Windows app")} autoFocus /></label>
            <label><span>{t("Prefix directory")}</span><div className="input-with-icon"><input value={path} onChange={(event) => setPath(event.target.value)} placeholder="/home/user/.local/share/wineprefixes/my-app" /><FolderOpen /></div><small>{mode === "create" ? t("The directory must be empty or not exist yet.") : t("Choose a folder containing drive_c and registry files.")}</small></label>
            {mode === "create" ? (
              <fieldset><legend>{t("Architecture")}</legend><div className="choice-grid"><Choice active={architecture === "win64"} title={t("64-bit / WoW64")} body={t("Recommended for modern applications")} onClick={() => setArchitecture("win64")} /><Choice active={architecture === "win32"} title={t("32-bit")} body={t("For older applications and runtimes")} onClick={() => setArchitecture("win32")} /></div></fieldset>
            ) : null}
          </div>
          {error ? <InlineError message={error} /> : null}
          <div className="dialog-actions"><Dialog.Close asChild><Button>{t("Cancel")}</Button></Dialog.Close><Button variant="primary" onClick={submit} disabled={busy}>{busy ? <SpinnerGap className="spin" /> : mode === "create" ? <Plus /> : <ArrowRight className="directional-icon" />}{busy ? t("Working…") : mode === "create" ? t("Create prefix") : t("Add prefix")}</Button></div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function ReviewDialog({
  open,
  onOpenChange,
  prefix,
  recipeIds,
  settings,
  onStarted,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  prefix: WinePrefix | null;
  recipeIds: string[];
  settings: AppSettings;
  onStarted: (operationId: UUID, plan: OperationPlan) => void;
}) {
  const { t, formatBytes, formatNumber } = useI18n();
  const [options, setOptions] = useState<OperationOptions>(defaultOptions(settings, prefix));
  const [plan, setPlan] = useState<OperationPlan | null>(null);
  const [inputValues, setInputValues] = useState<Record<string, string>>({});
  const [restoreTouched, setRestoreTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [importing, setImporting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setOptions(defaultOptions(settings, prefix));
  }, [settings, prefix]);

  useEffect(() => {
    setInputValues({});
    setRestoreTouched(false);
    setPlan(null);
    setImporting(null);
  }, [open, prefix?.id, recipeIds.join("\0")]);

  useEffect(() => {
    if (!open || !prefix || recipeIds.length === 0) return;
    setBusy(true);
    setError(null);
    api.planOperation({ prefix_id: prefix.id, recipes: recipeIds, input_values: inputValues, options })
      .then((nextPlan) => {
        setPlan(nextPlan);
        if (nextPlan.restore_recommended && !restoreTouched) {
          setOptions((current) => current.create_restore_point ? current : { ...current, create_restore_point: true });
        }
        setInputValues((current) => Object.fromEntries(nextPlan.inputs.map((input) => [
          input.key,
          current[input.key] ?? input.value ?? "",
        ])));
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setBusy(false));
  }, [open, prefix, recipeIds.join("\0"), options]);

  const start = async () => {
    if (!plan || !prefix) return;
    const missing = plan.inputs.find((input) => input.required && !(inputValues[input.key] ?? input.value ?? "").trim());
    if (missing) {
      setError(t("{field} is required.", { field: missing.label }));
      return;
    }
    setBusy(true);
    try {
      const verifiedPlan = await api.planOperation({
        prefix_id: prefix.id,
        recipes: recipeIds,
        input_values: Object.fromEntries(plan.inputs.map((input) => [input.key, inputValues[input.key] ?? input.value ?? ""])),
        options,
      });
      const operationId = await api.startOperation(verifiedPlan);
      onStarted(operationId, verifiedPlan);
      onOpenChange(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const importManualDownload = async (download: PlannedDownload) => {
    if (!prefix) return;
    const key = `${download.recipe_id}:${download.file_id}`;
    setError(null);
    try {
      const path = await api.selectManualFile(download.filename);
      if (!path) return;
      setImporting(key);
      await api.importManualFile(download.recipe_id, download.file_id, path);
      const nextPlan = await api.planOperation({
        prefix_id: prefix.id,
        recipes: recipeIds,
        input_values: Object.fromEntries((plan?.inputs ?? []).map((input) => [
          input.key,
          inputValues[input.key] ?? input.value ?? "",
        ])),
        options,
      });
      setPlan(nextPlan);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setImporting(null);
    }
  };

  const openDownloadSource = async (url: string) => {
    setError(null);
    try {
      await api.openUrl(url);
    } catch (reason) {
      setError(String(reason));
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content review-dialog" aria-describedby="review-description">
          <Dialog.Close className="dialog-close" aria-label={t("Close")}><X /></Dialog.Close>
          <div className="review-heading">
            <div className="dialog-icon"><ShieldCheck size={22} /></div>
            <div><Dialog.Title>{t("Review changes")}</Dialog.Title><Dialog.Description id="review-description">{t("Confirm the execution order and recovery options before modifying {prefix}.", { prefix: prefix?.name ?? t("this prefix") })}</Dialog.Description></div>
          </div>

          <div className="review-body">
            {busy && !plan ? <div className="plan-loading"><SpinnerGap className="spin" /><span>{t("Resolving dependencies and checking conflicts…")}</span></div> : plan ? (
              <div className="review-layout">
              <div className="review-main" role="region" aria-label={t("Review details")} tabIndex={0}>
                {error ? <InlineError message={error} /> : null}
                {plan.warnings.map((warning) => <div className="plan-notice warning" key={`${warning.code}-${warning.recipe_id}`}><WarningCircle weight="fill" /><div><strong dir="auto">{warning.title}</strong><span dir="auto">{warning.message}</span></div></div>)}
                {plan.conflicts.map((conflict) => <div className="plan-notice danger" key={`${conflict.code}-${conflict.recipe_id}`}><WarningCircle weight="fill" /><div><strong dir="auto">{conflict.title}</strong><span dir="auto">{conflict.message}</span></div></div>)}
                {plan.inputs.length ? <section className="review-section"><div className="review-section-title"><h3>{t("Recipe information")}</h3><Badge tone="accent">{t("Fields: {count}", { count: formatNumber(plan.inputs.length) })}</Badge></div><div className="review-inputs">{plan.inputs.map((input) => <label key={input.key}><span><bdi>{input.label}</bdi>{input.required ? <em>{t("Required")}</em> : null}</span><input value={inputValues[input.key] ?? input.value ?? ""} onChange={(event) => setInputValues((current) => ({ ...current, [input.key]: event.target.value }))} placeholder={input.placeholder ?? undefined} aria-required={input.required} autoComplete="off" spellCheck={false} />{input.description ? <small dir="auto">{input.description}</small> : null}</label>)}</div></section> : null}
                <section className="review-section"><div className="review-section-title"><h3>{t("Execution plan")}</h3><Badge>{t("Recipes: {count}", { count: formatNumber(plan.resolved_recipes.length) })}</Badge></div><div className="review-steps">{plan.steps.length ? plan.steps.map((step, index) => <div className="review-step" key={`${step.recipe_id}-${step.step_index}`}><span>{formatNumber(index + 1)}</span><div><strong dir="auto">{step.label}</strong><small dir="auto">{step.recipe_title}</small></div>{step.destructive ? <Badge tone="warning">{t("Destructive")}</Badge> : <Check size={15} />}</div>) : <div className="review-step"><span>{formatNumber(1)}</span><div><strong>{t("Record compatibility metadata")}</strong><small dir="auto">{plan.resolved_recipes.join(", ")}</small></div></div>}</div></section>
                {plan.downloads.length ? (
                  <section className="review-section">
                    <div className="review-section-title"><h3>{t("Downloads")}</h3><Badge tone="accent">{t("Files: {count}", { count: formatNumber(plan.downloads.length) })}</Badge></div>
                    {plan.downloads.map((download) => {
                      const key = `${download.recipe_id}:${download.file_id}`;
                      const sourceUrl = download.urls[0];
                      return (
                        <div className="download-review" key={key}>
                          <DownloadSimple />
                          <span><strong>{download.filename}</strong><small>{download.cached ? t("Cached; checksum verified before use") : download.manual ? t("Choose the exact upstream file; Bettertricks verifies its checksum") : t("Checksum verified after download")}</small></span>
                          {download.cached ? <CheckCircle weight="fill" /> : download.manual ? (
                            <div className="download-review-actions">
                              {sourceUrl ? <Button size="small" variant="ghost" onClick={() => openDownloadSource(sourceUrl)}><ArrowRight className="directional-icon" /> {t("Open source")}</Button> : null}
                              <Button size="small" onClick={() => importManualDownload(download)} disabled={Boolean(importing)}>
                                {importing === key ? <SpinnerGap className="spin" /> : <FolderOpen />} {t("Choose file")}
                              </Button>
                            </div>
                          ) : null}
                        </div>
                      );
                    })}
                  </section>
                ) : null}
              </div>
              <aside className="review-options">
                <h3>{t("Safeguards")}</h3>
                <OptionCheck checked={options.create_restore_point} onChange={(checked) => { setRestoreTouched(true); setOptions((value) => ({ ...value, create_restore_point: checked })); }} icon={<Archive />} title={t("Create restore point")} body={t("Recommended before compatibility changes")} />
                <OptionCheck checked={options.verify} onChange={(checked) => setOptions((value) => ({ ...value, verify: checked }))} icon={<CheckCircle />} title={t("Verify after install")} body={t("Run available recipe checks")} />
                <OptionCheck checked={options.unattended} onChange={(checked) => setOptions((value) => ({ ...value, unattended: checked }))} icon={<Play />} title={t("Unattended mode")} body={t("Avoid optional installer prompts")} />
                {settings.show_advanced ? <><div className="review-options-divider" /><h3>{t("Advanced")}</h3><OptionCheck checked={options.force} onChange={(checked) => setOptions((value) => ({ ...value, force: checked }))} icon={<WarningCircle />} title={t("Force installation")} body={t("Ignore installed and conflict checks")} /><OptionCheck checked={options.no_clean} onChange={(checked) => setOptions((value) => ({ ...value, no_clean: checked }))} icon={<HardDrive />} title={t("Keep temporary files")} body={t("Useful while troubleshooting")} /></> : null}
                <div className="review-target"><small>{t("Target prefix")}</small><strong>{plan.prefix.name}</strong><code>{plan.prefix.path}</code></div>
              </aside>
              </div>
            ) : error ? <InlineError message={error} /> : null}
          </div>
          <div className="dialog-actions review-actions"><div className="review-summary"><strong>{t("Steps: {count}", { count: formatNumber(plan?.steps.length ?? 0) })}</strong><span>{plan?.downloads.some((download) => download.manual && !download.cached) ? t("Choose required manual files") : plan?.estimated_download_bytes ? formatBytes(plan.estimated_download_bytes) : plan?.downloads.length ? t("Download size determined at runtime") : t("No downloads")}</span></div><Dialog.Close asChild><Button>{t("Cancel")}</Button></Dialog.Close><Button variant="primary" onClick={start} disabled={busy || Boolean(importing) || !plan || plan.downloads.some((download) => download.manual && !download.cached) || plan.inputs.some((input) => input.required && !(inputValues[input.key] ?? input.value ?? "").trim()) || (plan.conflicts.length > 0 && !options.force)}>{busy ? <SpinnerGap className="spin" /> : <Play weight="fill" />} {t("Apply changes")}</Button></div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function LegacyVerbDialog({
  open,
  onOpenChange,
  prefixes,
  selectedPrefixId,
  onComplete,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  prefixes: WinePrefix[];
  selectedPrefixId: UUID | null;
  onComplete: () => void;
}) {
  const { t, formatBytes } = useI18n();
  const [path, setPath] = useState("");
  const [info, setInfo] = useState<LegacyVerbInfo | null>(null);
  const [prefixId, setPrefixId] = useState(selectedPrefixId ?? prefixes[0]?.id ?? "");
  const [trusted, setTrusted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) setPrefixId(selectedPrefixId ?? prefixes[0]?.id ?? "");
    else {
      setPath("");
      setInfo(null);
      setTrusted(false);
      setError(null);
    }
  }, [open, selectedPrefixId, prefixes]);

  const choose = async () => {
    setError(null);
    try {
      const selected = await api.selectLegacyVerb();
      if (!selected) return;
      setPath(selected);
      setTrusted(false);
      setBusy(true);
      setInfo(await api.inspectLegacyVerb(selected));
    } catch (reason) {
      setInfo(null);
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const run = async () => {
    if (!info || !prefixId || !trusted) return;
    setBusy(true);
    setError(null);
    try {
      await api.runLegacyVerb({
        prefix_id: prefixId,
        path: info.path,
        trusted,
        options: {
          force: false,
          unattended: false,
          verify: false,
          no_clean: false,
          isolate: false,
          torify: false,
          country: null,
          create_restore_point: true,
        },
      });
      onComplete();
      onOpenChange(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content legacy-dialog" aria-describedby="legacy-description">
          <Dialog.Close className="dialog-close" aria-label={t("Close")}><X /></Dialog.Close>
          <div className="dialog-icon"><Command size={22} /></div>
          <Dialog.Title>{t("Run a legacy .verb file")}</Dialog.Title>
          <Dialog.Description id="legacy-description">{t("Use the Winetricks compatibility host for a custom shell recipe that has not been converted to Bettertricks’ typed format.")}</Dialog.Description>

          <button className="legacy-file" onClick={choose} disabled={busy}>
            <span><FolderOpen size={20} /></span>
            <span><strong>{path ? path.split("/").at(-1) : t("Choose a .verb file")}</strong><small>{path || t("The file is inspected before it can run.")}</small></span>
            {busy && !info ? <SpinnerGap className="spin" /> : <ArrowRight className="directional-icon" />}
          </button>

          {info ? (
            <div className="legacy-inspection">
              <div className="legacy-inspection-title"><span><CheckCircle weight="fill" /></span><div><small>{t("Metadata found")}</small><strong>{info.title ?? titleCase(info.id)}</strong></div><Badge>{info.category}</Badge></div>
              <dl><div><dt>{t("Verb ID")}</dt><dd><code>{info.id}</code></dd></div><div><dt>{t("File size")}</dt><dd>{formatBytes(info.size_bytes)}</dd></div></dl>
              <div className="legacy-target">
                <span>{t("Target prefix")}</span>
                <SelectMenu
                  className="legacy-target-trigger"
                  value={prefixId}
                  options={prefixes.map((prefix) => ({ value: prefix.id, label: prefix.name }))}
                  label={t("Target prefix")}
                  onValueChange={setPrefixId}
                />
              </div>
              <div className="legacy-warning"><WarningCircle weight="fill" /><div><strong>{t("Shell code requires trust")}</strong><span dir="auto">{info.warning}</span></div></div>
              <OptionCheck checked={trusted} onChange={setTrusted} icon={<ShieldCheck />} title={t("I reviewed and trust this file")} body={t("Bettertricks creates a restore point before execution")} />
            </div>
          ) : null}
          {error ? <InlineError message={error} /> : null}
          <div className="dialog-actions"><Dialog.Close asChild><Button>{t("Cancel")}</Button></Dialog.Close><Button variant="primary" onClick={info ? run : choose} disabled={busy || (Boolean(info) && (!trusted || !prefixId))}>{busy ? <SpinnerGap className="spin" /> : info ? <Play weight="fill" /> : <FolderOpen />}{info ? t("Run legacy verb") : t("Choose file")}</Button></div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function TrashPrefixDialog({ open, onOpenChange, prefix, onConfirm }: { open: boolean; onOpenChange: (open: boolean) => void; prefix: WinePrefix | null; onConfirm: (confirmation: string) => Promise<void> }) {
  const { t } = useI18n();
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { if (!open) { setConfirmation(""); setError(null); } }, [open]);
  const submit = async () => { setBusy(true); setError(null); try { await onConfirm(confirmation); onOpenChange(false); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  return <Dialog.Root open={open} onOpenChange={onOpenChange}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="dialog-content danger-dialog"><Dialog.Close className="dialog-close" aria-label={t("Close")}><X /></Dialog.Close><div className="dialog-icon danger"><Trash size={22} /></div><Dialog.Title>{t("Move prefix to Trash?")}</Dialog.Title><Dialog.Description>{t("This moves the full prefix and its Windows applications to your desktop Trash. It can usually be restored until Trash is emptied.")}</Dialog.Description><div className="danger-path"><HardDrive /><code>{prefix?.path}</code></div><label className="confirmation-field"><span>{t("Type {prefix} to confirm", { prefix: prefix?.name ?? "" })}</span><input value={confirmation} onChange={(event) => setConfirmation(event.target.value)} /></label>{error ? <InlineError message={error} /> : null}<div className="dialog-actions"><Dialog.Close asChild><Button>{t("Cancel")}</Button></Dialog.Close><Button variant="danger" disabled={confirmation !== prefix?.name || busy} onClick={submit}>{busy ? <SpinnerGap className="spin" /> : <Trash />} {t("Move to Trash")}</Button></div></Dialog.Content></Dialog.Portal></Dialog.Root>;
}

export function CommandPalette({ open, onOpenChange, prefixes, onNavigate, onPrefix, onAddPrefix }: { open: boolean; onOpenChange: (open: boolean) => void; prefixes: WinePrefix[]; onNavigate: (view: AppView) => void; onPrefix: (id: UUID) => void; onAddPrefix: () => void }) {
  const { t, intlLocale } = useI18n();
  const [search, setSearch] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const actions = useMemo(() => [
    { id: "components", label: t("Browse components"), detail: t("Runtimes, fonts, apps, and settings"), icon: <Package />, run: () => onNavigate("catalog") },
    { id: "activity", label: t("Open activity"), detail: t("Operations and diagnostics"), icon: <ArrowRight className="directional-icon" />, run: () => onNavigate("activity") },
    { id: "add", label: t("Add a Wine prefix"), detail: t("Connect or create"), icon: <Plus />, run: onAddPrefix },
    ...prefixes.map((prefix) => ({ id: prefix.id, label: prefix.name, detail: prefix.path, icon: <HardDrive />, run: () => onPrefix(prefix.id) })),
  ].filter((action) => `${action.label} ${action.detail}`.toLocaleLowerCase(intlLocale).includes(search.toLocaleLowerCase(intlLocale))), [prefixes, search, onNavigate, onPrefix, onAddPrefix, t, intlLocale]);
  const choose = (run: () => void) => { run(); onOpenChange(false); setSearch(""); };
  useEffect(() => { setActiveIndex(0); }, [open, search]);
  const onSearchKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (!actions.length) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + direction + actions.length) % actions.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const action = actions[activeIndex];
      if (action) choose(action.run);
    }
  };
  return <Dialog.Root open={open} onOpenChange={onOpenChange}><Dialog.Portal><Dialog.Overlay className="dialog-overlay palette-overlay" /><Dialog.Content className="command-palette" aria-describedby="command-palette-description"><Dialog.Title className="sr-only">{t("Search or run a command")}</Dialog.Title><Dialog.Description className="sr-only" id="command-palette-description">{t("Search commands and Wine prefixes. Use the up and down arrow keys to select a result, then press Enter to run it.")}</Dialog.Description><label className="palette-search"><MagnifyingGlass /><input role="combobox" aria-label={t("Search commands and prefixes")} aria-autocomplete="list" aria-expanded={open} aria-controls="command-palette-results" aria-activedescendant={actions[activeIndex] ? `command-option-${activeIndex}` : undefined} value={search} onChange={(event) => setSearch(event.target.value)} onKeyDown={onSearchKeyDown} placeholder={t("Search prefixes or run a command")} autoFocus /><kbd>Esc</kbd></label><div className="palette-results" id="command-palette-results" role="listbox" aria-label={t("Commands and prefixes")}>{actions.length ? actions.map((action, index) => <button id={`command-option-${index}`} role="option" aria-selected={index === activeIndex} tabIndex={-1} className={index === activeIndex ? "active" : undefined} key={action.id} onMouseEnter={() => setActiveIndex(index)} onClick={() => choose(action.run)}><span>{action.icon}</span><span><strong>{action.label}</strong><small>{action.detail}</small></span>{index === activeIndex ? <kbd>Enter</kbd> : null}</button>) : <p>{t("No matching commands")}</p>}</div><div className="palette-footer"><span><Command /> {t("Bettertricks command palette")}</span><span><kbd>↑</kbd><kbd>↓</kbd> {t("Navigate")} · <kbd>Enter</kbd> {t("Run")}</span></div></Dialog.Content></Dialog.Portal></Dialog.Root>;
}

export function OperationDrawer({
  operationId,
  plan,
  events,
  onClose,
  onCancel,
  onRespond,
  onRetry,
}: {
  operationId: UUID | null;
  plan: OperationPlan | null;
  events: OperationEvent[];
  onClose: () => void;
  onCancel: () => void;
  onRespond: (promptId: UUID, choiceId: string) => Promise<void>;
  onRetry: (recipeIds: string[]) => void;
}) {
  const { t, formatNumber, formatTime } = useI18n();
  const [responding, setResponding] = useState(false);
  const [responseError, setResponseError] = useState<string | null>(null);
  const promptRef = useRef<HTMLElement>(null);
  const promptId = events.at(-1)?.prompt?.id;
  const failures = useMemo(() => {
    const byRecipe = new Map<string, NonNullable<OperationEvent["failure"]>>();
    for (const event of events) {
      if (event.failure) byRecipe.set(event.failure.recipe_id, event.failure);
    }
    return [...byRecipe.values()];
  }, [events]);
  useEffect(() => {
    setResponding(false);
    setResponseError(null);
    if (promptId) promptRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
  }, [promptId]);
  if (!operationId || !plan) return null;
  const latest = events.at(-1);
  const state = latest?.state ?? "planned";
  const finished = ["succeeded", "failed", "cancelled"].includes(state);
  const logs = events.filter((event) => event.log_line).map((event) => event.log_line);
  const retryableRecipeIds = failures.length
    ? failures.map((failure) => failure.recipe_id)
    : plan.requested_recipes;
  const respond = async (promptId: UUID, choiceId: string) => {
    setResponding(true);
    setResponseError(null);
    try {
      await onRespond(promptId, choiceId);
    } catch (reason) {
      setResponseError(String(reason));
      setResponding(false);
    }
  };
  return (
    <aside className="operation-drawer" aria-live="polite" aria-label={t("Current operation")}>
      <header>
        <div className={clsx("operation-drawer-icon", `state-${state}`)}>
          {state === "succeeded" ? <CheckCircle weight="fill" /> : state === "failed" ? <WarningCircle weight="fill" /> : <SpinnerGap className={!finished ? "spin" : ""} />}
        </div>
        <div><small>{finished ? t("Operation result") : t("Operation in progress")}</small><strong dir="auto">{latest?.title ?? t("Preparing changes")}</strong></div>
        <button onClick={onClose} aria-label={finished ? t("Close activity drawer") : t("Hide activity drawer")} title={finished ? t("Close") : t("Hide; the operation will continue in the background")}><X /></button>
      </header>
      <div className="drawer-progress">
        <ProgressBar value={latest?.progress ?? 0} label={t("{step} of {total} steps", { step: formatNumber(latest?.step ?? 0), total: formatNumber(latest?.total_steps ?? plan.steps.length) })} />
        <p dir="auto">{latest?.detail ?? t("Applying {recipes} to {prefix}", { recipes: plan.resolved_recipes.join(", "), prefix: plan.prefix.name })}</p>
      </div>
      {latest?.prompt ? (
        <section ref={promptRef} className="drawer-prompt" role="alertdialog" aria-labelledby={`prompt-${latest.prompt.id}`} aria-describedby={`prompt-message-${latest.prompt.id}`}>
          <strong id={`prompt-${latest.prompt.id}`} dir="auto">{latest.prompt.title}</strong>
          <p id={`prompt-message-${latest.prompt.id}`} dir="auto">{latest.prompt.message}</p>
          {responseError ? <small role="alert" dir="auto">{responseError}</small> : null}
          <div>{latest.prompt.choices.map((choice) => <Button key={choice.id} variant={choice.destructive ? "danger" : choice.id === "continue" ? "primary" : "secondary"} disabled={responding} onClick={() => respond(latest.prompt!.id, choice.id)}>{responding ? <SpinnerGap className="spin" /> : null}<bdi>{choice.label}</bdi></Button>)}</div>
        </section>
      ) : null}
      <div className="drawer-timeline">
        {failures.length ? (
          <section className="drawer-failures" aria-label={t("Component failures")}>
            <div className="drawer-section-heading"><strong>{t("Needs attention")}</strong><Badge tone="danger">{formatNumber(failures.length)}</Badge></div>
            {failures.map((failure) => (
              <article className={clsx("drawer-failure", failure.kind === "skipped_dependency" && "skipped")} key={failure.recipe_id}>
                <span><WarningCircle weight="fill" /></span>
                <div><strong dir="auto">{failure.recipe_title}</strong><small dir="auto">{failure.message}</small></div>
                {state === "failed" ? <Button size="small" aria-label={t("Retry {component}", { component: failure.recipe_title })} onClick={() => onRetry([failure.recipe_id])}><ArrowClockwise /> {t("Retry")}</Button> : null}
              </article>
            ))}
          </section>
        ) : null}
        <div className="drawer-events">
          {events.filter((event) => !event.log_line).slice(-10).map((event) => (
            <div className={clsx("drawer-event", event === latest && "current", event.failure && "failed")} key={event.sequence}>
              <span>{event.state === "succeeded" ? <Check /> : event.failure ? <WarningCircle weight="fill" /> : event.step ? formatNumber(event.step) : "·"}</span>
              <div><strong dir="auto">{event.title}</strong><small>{event.recipe_id ? titleCase(event.recipe_id) : formatTime(event.timestamp)}</small></div>
            </div>
          ))}
        </div>
      </div>
      {logs.length ? <details className="drawer-logs"><summary>{t("Diagnostic log")} <Badge>{formatNumber(logs.length)}</Badge></summary><pre>{logs.join("\n")}</pre></details> : null}
      <footer>
        {state === "failed" && retryableRecipeIds.length ? <Button onClick={() => onRetry(retryableRecipeIds)}><ArrowClockwise /> {t("Retry all ({count})", { count: formatNumber(retryableRecipeIds.length) })}</Button> : null}
        {finished ? <Button variant="primary" onClick={onClose}>{t("Done")}</Button> : latest?.prompt ? null : <Button variant="danger" onClick={onCancel}>{t("Cancel operation")}</Button>}
      </footer>
    </aside>
  );
}

function OptionCheck({ checked, onChange, icon, title, body }: { checked: boolean; onChange: (checked: boolean) => void; icon: React.ReactNode; title: string; body: string }) { return <button className={clsx("option-check", checked && "checked")} aria-pressed={checked} onClick={() => onChange(!checked)}><span className="option-check-icon">{icon}</span><span><strong>{title}</strong><small>{body}</small></span><i>{checked ? <Check weight="bold" /> : null}</i></button>; }
function Choice({ active, title, body, onClick }: { active: boolean; title: string; body: string; onClick: () => void }) { return <button type="button" className={clsx("choice-card", active && "active")} aria-pressed={active} onClick={onClick}><span>{active ? <Check weight="bold" /> : null}</span><strong>{title}</strong><small>{body}</small></button>; }
function InlineError({ message }: { message: string }) { return <div className="inline-error" role="alert"><WarningCircle weight="fill" /><span dir="auto">{message}</span></div>; }
function defaultOptions(settings: AppSettings, prefix: WinePrefix | null): OperationOptions { return { force: false, unattended: false, verify: true, no_clean: false, isolate: false, torify: false, country: null, create_restore_point: Boolean(prefix?.managed && settings.restore_before_managed_changes) }; }
