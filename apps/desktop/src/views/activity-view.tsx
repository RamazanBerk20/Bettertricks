import { useMemo, useState } from "react";
import clsx from "clsx";
import * as Dialog from "@radix-ui/react-dialog";
import {
  ArrowClockwise,
  CaretDown,
  CheckCircle,
  ClockCounterClockwise,
  FileText,
  MagnifyingGlass,
  Package,
  SpinnerGap,
  Trash,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import type { OperationRecord, OperationState } from "../types";
import { titleCase } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { Badge, Button, EmptyState, StatusBadge } from "../components/common";

export function ActivityView({
  operations,
  onRefresh,
  onClearActivity,
  onRetryRecipes,
}: {
  operations: OperationRecord[];
  onRefresh: () => void;
  onClearActivity: () => Promise<void>;
  onRetryRecipes: (prefixId: string, recipeIds: string[]) => void;
}) {
  const { t, formatNumber, intlLocale } = useI18n();
  const [filter, setFilter] = useState<"all" | "running" | "failed" | "succeeded">("all");
  const [search, setSearch] = useState("");
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const visible = useMemo(() => operations.filter((operation) => {
    const stateMatch = filter === "all" || operation.state === filter || (filter === "running" && ["preflight", "waiting_for_user"].includes(operation.state));
    const haystack = `${operation.prefix_name} ${operation.recipes.join(" ")} ${operation.message ?? ""} ${(operation.failures ?? []).map((failure) => `${failure.recipe_title} ${failure.message}`).join(" ")}`.toLocaleLowerCase(intlLocale);
    return stateMatch && haystack.includes(search.toLocaleLowerCase(intlLocale));
  }), [operations, filter, search, intlLocale]);

  const running = operations.filter((operation) => ["running", "preflight", "waiting_for_user"].includes(operation.state)).length;
  const failed = operations.filter((operation) => operation.state === "failed").length;
  const hasClearableOperations = operations.some((operation) => ["succeeded", "failed", "cancelled"].includes(operation.state));

  const clearActivity = async () => {
    setClearing(true);
    try {
      await onClearActivity();
      setClearOpen(false);
    } catch {
      // The app-level toast preserves the backend error while the dialog stays open.
    } finally {
      setClearing(false);
    }
  };

  return (
    <div className="page-scroll activity-page">
      <header className="page-header standard-header">
        <div>
          <div className="heading-meta"><Badge tone="neutral">{t("Operation journal")}</Badge><span>{t("Stored locally")}</span></div>
          <h1>{t("Activity")}</h1>
          <p>{t("Follow active work, revisit diagnostics, and confirm what changed in each prefix.")}</p>
        </div>
        <div className="header-actions activity-header-actions">
          <Button onClick={() => setClearOpen(true)} disabled={!hasClearableOperations}><Trash /> {t("Clear activity")}</Button>
          <Button onClick={onRefresh}><ArrowClockwise /> {t("Refresh")}</Button>
        </div>
      </header>

      <section className="activity-summary">
        <SummaryItem icon={<ClockCounterClockwise />} label={t("Operations")} value={formatNumber(operations.length)} detail={t("Recorded locally")} />
        <SummaryItem icon={<Package />} label={t("Running")} value={formatNumber(running)} detail={running ? t("Work in progress") : t("Queue is clear")} accent={running > 0} />
        <SummaryItem icon={<WarningCircle />} label={t("Needs review")} value={formatNumber(failed)} detail={failed ? t("Open logs for details") : t("No recent failures")} warning={failed > 0} />
        <SummaryItem icon={<CheckCircle />} label={t("Completed")} value={formatNumber(operations.filter((item) => item.state === "succeeded").length)} detail={t("Successful operations")} />
      </section>

      <section className="activity-journal section-block">
        <div className="journal-toolbar">
          <div className="segmented-control" role="group" aria-label={t("Activity status")}>
            {(["all", "running", "failed", "succeeded"] as const).map((value) => (
              <button aria-pressed={filter === value} className={clsx(filter === value && "active")} onClick={() => setFilter(value)} key={value}>{activityFilterLabel(value, t)}</button>
            ))}
          </div>
          <label className="compact-search"><MagnifyingGlass /><input aria-label={t("Filter activity")} value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("Filter activity")} /></label>
        </div>

        {visible.length ? (
          <div className="operation-table" role="table" aria-label={t("Operation history")}>
            <div className="operation-table-head" role="row"><span role="columnheader">{t("Operation")}</span><span role="columnheader">{t("Prefix")}</span><span role="columnheader">{t("Progress")}</span><span role="columnheader">{t("Started")}</span><span role="columnheader">{t("Status")}</span><span role="columnheader" aria-label={t("Details")} /></div>
            {visible.map((operation) => <OperationRow operation={operation} onRetryRecipes={onRetryRecipes} key={operation.id} />)}
          </div>
        ) : (
          <EmptyState icon={<FileText size={25} />} title={t("No matching activity")} body={t("Completed and active operations will remain available here for troubleshooting.")} />
        )}
      </section>

      <Dialog.Root open={clearOpen} onOpenChange={(open) => { if (!clearing) setClearOpen(open); }}>
        <Dialog.Portal>
          <Dialog.Overlay className="dialog-overlay" />
          <Dialog.Content className="dialog-content danger-dialog">
            <Dialog.Close className="dialog-close" disabled={clearing} aria-label={t("Close")}><X /></Dialog.Close>
            <div className="dialog-icon danger"><Trash weight="fill" /></div>
            <Dialog.Title>{t("Clear activity history?")}</Dialog.Title>
            <Dialog.Description>{t("Completed, failed, and cancelled records will be removed. Active operations stay visible.")}</Dialog.Description>
            <div className="dialog-actions">
              <Dialog.Close asChild><Button disabled={clearing}>{t("Cancel")}</Button></Dialog.Close>
              <Button variant="danger" disabled={clearing} onClick={() => void clearActivity()}>
                {clearing ? <SpinnerGap className="spin" /> : <Trash weight="fill" />} {t("Clear activity")}
              </Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}

function OperationRow({ operation, onRetryRecipes }: { operation: OperationRecord; onRetryRecipes: (prefixId: string, recipeIds: string[]) => void }) {
  const { t, formatNumber, formatRelativeTime } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const progress = operation.total_steps ? operation.current_step / operation.total_steps : 0;
  const failures = operation.failures ?? [];
  const retryIds = [...new Set((failures.length ? failures.map((failure) => failure.recipe_id) : operation.recipes))];
  return (
    <div className={clsx("operation-row-group", expanded && "expanded")} role="rowgroup">
      <div className="operation-row" role="row">
        <span className="operation-name" role="cell"><span className={`operation-state-icon state-${operation.state}`}>{stateIcon(operation.state)}</span><span><strong dir="auto">{operation.recipes.map(friendlyVerb).join(", ")}</strong><small dir="auto">{operation.message ?? t("Recipes: {count}", { count: formatNumber(operation.recipes.length) })}</small></span></span>
        <span role="cell"><strong dir="auto">{operation.prefix_name}</strong><small>{operation.prefix_id.slice(0, 8)}</small></span>
        <span className="table-progress" role="cell"><strong>{formatNumber(operation.current_step)} / {formatNumber(operation.total_steps)}</strong><span role="progressbar" aria-label={t("Operation progress for {prefix}", { prefix: operation.prefix_name })} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(Math.max(0, Math.min(1, progress)) * 100)}><i style={{ transform: `scaleX(${Math.max(0, Math.min(1, progress))})` }} /></span></span>
        <span role="cell"><strong>{formatRelativeTime(operation.started_at ?? operation.created_at)}</strong><small>{operation.finished_at ? t("Finished {time}", { time: formatRelativeTime(operation.finished_at) }) : t("In progress")}</small></span>
        <span role="cell"><StatusBadge state={operation.state} /></span>
        <span role="cell"><button className="operation-details-toggle" aria-label={expanded ? t("Hide details for {prefix} operation", { prefix: operation.prefix_name }) : t("Show details for {prefix} operation", { prefix: operation.prefix_name })} aria-expanded={expanded} onClick={() => setExpanded((value) => !value)}><CaretDown aria-hidden="true" /></button></span>
      </div>
      {expanded ? (
        <section className="operation-details" aria-label={t("Diagnostics for {prefix} operation", { prefix: operation.prefix_name })}>
          <div className="operation-details-heading">
            <span><FileText /><span><strong>{t("Operation diagnostics")}</strong><small dir="auto">{operation.message ?? t("No additional diagnostic message was recorded.")}</small></span></span>
            {operation.state === "failed" && retryIds.length ? <Button size="small" onClick={() => onRetryRecipes(operation.prefix_id, retryIds)}><ArrowClockwise /> {t("Retry all ({count})", { count: formatNumber(retryIds.length) })}</Button> : null}
          </div>
          {failures.length ? (
            <div className="operation-failures">
              {failures.map((failure) => (
                <article className={failure.kind === "skipped_dependency" ? "skipped" : undefined} key={failure.recipe_id}>
                  <WarningCircle weight="fill" />
                  <span><strong dir="auto">{failure.recipe_title}</strong><small dir="auto">{failure.message}</small></span>
                  <Badge tone={failure.kind === "skipped_dependency" ? "warning" : "danger"}>{failure.kind === "skipped_dependency" ? t("Skipped") : t("Failed")}</Badge>
                  <Button size="small" variant="ghost" aria-label={t("Retry {component}", { component: failure.recipe_title })} onClick={() => onRetryRecipes(operation.prefix_id, [failure.recipe_id])}><ArrowClockwise /> {t("Retry")}</Button>
                </article>
              ))}
            </div>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}

function SummaryItem({ icon, label, value, detail, accent, warning }: { icon: React.ReactNode; label: string; value: string; detail: string; accent?: boolean; warning?: boolean }) {
  return <div className={clsx("activity-summary-item", accent && "accent", warning && "warning")}><span>{icon}</span><div><small>{label}</small><strong>{value}</strong><p>{detail}</p></div></div>;
}

function activityFilterLabel(filter: "all" | "running" | "failed" | "succeeded", t: (key: string) => string) {
  if (filter === "all") return t("All");
  if (filter === "running") return t("Running");
  if (filter === "failed") return t("Failed");
  return t("Completed");
}

function stateIcon(state: OperationState) {
  if (state === "succeeded") return <CheckCircle weight="fill" />;
  if (state === "failed") return <WarningCircle weight="fill" />;
  if (["running", "preflight", "waiting_for_user"].includes(state)) return <ArrowClockwise weight="bold" />;
  return <ClockCounterClockwise />;
}

function friendlyVerb(verb: string) {
  const map: Record<string, string> = { vcrun2022: "Visual C++ 2022", d3dcompiler_47: "D3D compiler 47", corefonts: "Core fonts", win10: "Windows 10 mode" };
  return map[verb] ?? titleCase(verb);
}
