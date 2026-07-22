import { memo } from "react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  Archive,
  ArrowRight,
  CheckCircle,
  DotsThree,
  FileArrowUp,
  FolderOpen,
  HardDrive,
  Info,
  LinkBreak,
  Package,
  Plus,
  ShieldCheck,
  TerminalWindow,
  Trash,
  WarningCircle,
  Wrench,
} from "@phosphor-icons/react";
import type { OperationRecord, RestorePoint, WinePrefix } from "../types";
import { shortPath, titleCase } from "../lib/format";
import { useI18n, type TranslationValues } from "../lib/i18n";
import { Badge, Button, EmptyState, PrefixSourceIcon, StatusBadge } from "../components/common";

interface PrefixViewProps {
  prefix: WinePrefix | null;
  operations: OperationRecord[];
  restorePoints: RestorePoint[];
  onBrowseCatalog: () => void;
  onOpenPath: (path: string) => void;
  onLaunchTool: (tool: string) => void;
  onCreateRestorePoint: () => void;
  onForgetPrefix: () => void;
  onRunInstaller: () => void;
  onTrashPrefix: () => void;
}

const INSTALLED_COMPONENT_PREVIEW_LIMIT = 12;

export const PrefixView = memo(function PrefixView({
  prefix,
  operations,
  restorePoints,
  onBrowseCatalog,
  onOpenPath,
  onLaunchTool,
  onCreateRestorePoint,
  onForgetPrefix,
  onRunInstaller,
  onTrashPrefix,
}: PrefixViewProps) {
  const { t, formatBytes, formatNumber, formatRelativeTime } = useI18n();
  if (!prefix) {
    return (
      <div className="centered-view">
        <EmptyState
          icon={<HardDrive size={26} />}
          title={t("No Wine prefix selected")}
          body={t("Add an existing prefix or create a fresh one to start managing Windows components.")}
        />
      </div>
    );
  }

  const prefixOperations = operations.filter((operation) => operation.prefix_id === prefix.id);
  const prefixRestores = restorePoints.filter((point) => point.prefix_id === prefix.id);
  const installedComponentPreview = prefix.installed_verbs
    .slice(-INSTALLED_COMPONENT_PREVIEW_LIMIT)
    .reverse();
  const installedComponentsAreTruncated = prefix.installed_verbs.length > installedComponentPreview.length;

  return (
    <div className="page-scroll prefix-page">
      <header className="page-header prefix-header">
        <div className="prefix-title-lockup">
          <span className="prefix-hero-icon"><PrefixSourceIcon source={prefix.source} size={26} /></span>
          <div>
            <div className="heading-meta">
              <Badge tone={prefix.managed ? "warning" : "neutral"}>
                {prefix.managed ? t("{source} managed", { source: sourceName(prefix, t) }) : sourceName(prefix, t)}
              </Badge>
              <span>{architectureName(prefix.architecture, t)}</span>
            </div>
            <h1 dir="auto">{prefix.name}</h1>
            <button className="path-button" onClick={() => onOpenPath(prefix.path)} title={prefix.path}>
              {shortPath(prefix.path, 70)} <FolderOpen size={14} />
            </button>
          </div>
        </div>
        <div className="header-actions">
          <Button variant="primary" onClick={onBrowseCatalog}>
            <Plus size={17} weight="bold" /> {t("Install components")}
          </Button>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <Button size="icon" aria-label={t("More prefix actions")}>
                <DotsThree size={21} weight="bold" />
              </Button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content className="menu-content" align="end" sideOffset={8}>
                <DropdownMenu.Item className="menu-item" onSelect={() => onOpenPath(prefix.path)}>
                  <FolderOpen /> {t("Open prefix folder")}
                </DropdownMenu.Item>
                <DropdownMenu.Item className="menu-item" onSelect={onCreateRestorePoint}>
                  <Archive /> {t("Create restore point")}
                </DropdownMenu.Item>
                {prefix.source === "manual" ? (
                  <DropdownMenu.Item className="menu-item" onSelect={onForgetPrefix}>
                    <LinkBreak /> {t("Forget without deleting")}
                  </DropdownMenu.Item>
                ) : null}
                <DropdownMenu.Separator className="menu-separator" />
                <DropdownMenu.Item className="menu-item danger" onSelect={onTrashPrefix}>
                  <Trash /> {t("Move prefix to Trash")}
                </DropdownMenu.Item>
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>
      </header>

      {prefix.managed ? (
        <div className="managed-notice">
          <Info size={19} weight="fill" />
          <div>
            <strong>{t("This prefix belongs to {source}", { source: sourceName(prefix, t) })}</strong>
            <span>{t("Close the game and launcher before applying changes. Bettertricks will leave launcher configuration untouched.")}</span>
          </div>
        </div>
      ) : null}

      <section className="prefix-stats" aria-label={t("Prefix summary")}>
        <Stat label={t("Runtime")} value={prefix.runtime_label ?? t("System Wine")} detail={prefix.runtime ? shortPath(prefix.runtime, 28) : t("Auto detected")} />
        <Stat label={t("Installed components")} value={formatNumber(prefix.installed_verbs.length)} detail={t("Winetricks compatible")} />
        <Stat label={t("Prefix size")} value={formatBytes(prefix.size_bytes)} detail={prefix.size_bytes ? t("On disk") : t("Scan on demand")} />
        <Stat label={t("Last changed")} value={formatRelativeTime(prefix.last_modified)} detail={prefix.exists ? t("Prefix available") : t("Not initialized")} />
      </section>

      <div className="content-columns">
        <div className="content-primary">
          <section className="section-block">
            <div className="section-heading">
              <div>
                <h2>{t("Prefix tools")}</h2>
                <p>{t("Open Wine's built-in utilities in this prefix.")}</p>
              </div>
            </div>
            <div className="tool-grid">
              <ToolButton icon={<Wrench />} title={t("Wine settings")} description={t("Graphics, drives, audio")} onClick={() => onLaunchTool("winecfg")} />
              <ToolButton icon={<TerminalWindow />} title={t("Registry editor")} description={t("Inspect registry keys")} onClick={() => onLaunchTool("regedit")} />
              <ToolButton icon={<HardDrive />} title={t("Task manager")} description={t("Processes and resources")} onClick={() => onLaunchTool("taskmgr")} />
              <ToolButton icon={<FolderOpen />} title={t("Wine Explorer")} description={t("Browse virtual drives")} onClick={() => onLaunchTool("explorer")} />
              <ToolButton icon={<Package />} title={t("Uninstaller")} description={t("Manage Windows apps")} onClick={() => onLaunchTool("uninstaller")} />
              <ToolButton icon={<TerminalWindow />} title={t("Command prompt")} description={t("Open Wine cmd.exe")} onClick={() => onLaunchTool("cmd")} />
              <ToolButton icon={<FileArrowUp />} title={t("Run installer")} description={t("Open an EXE or MSI safely")} onClick={onRunInstaller} />
            </div>
          </section>

          <section className="section-block installed-section">
            <div className="section-heading inline-heading">
              <div>
                <h2>{t("Installed components")}</h2>
                <p>
                  {installedComponentsAreTruncated
                    ? t("Showing the {shown} most recently recorded of {total}.", { shown: formatNumber(installedComponentPreview.length), total: formatNumber(prefix.installed_verbs.length) })
                    : t("Read from the prefix's Winetricks-compatible install log.")}
                </p>
              </div>
              <Button size="small" variant="ghost" onClick={onBrowseCatalog}>
                {installedComponentsAreTruncated ? t("View all {count}", { count: formatNumber(prefix.installed_verbs.length) }) : t("Browse all")} <ArrowRight className="directional-icon" />
              </Button>
            </div>
            {prefix.installed_verbs.length ? (
              <div className="installed-grid">
                {installedComponentPreview.map((verb) => (
                  <button key={verb} className="installed-item" onClick={onBrowseCatalog}>
                    <span className="installed-check"><CheckCircle size={18} weight="fill" /></span>
                    <span>
                      <strong dir="auto">{friendlyVerb(verb)}</strong>
                      <small>{verb}</small>
                    </span>
                    <ArrowRight className="directional-icon" size={15} />
                  </button>
                ))}
              </div>
            ) : (
              <EmptyState
                icon={<Package size={24} />}
                title={t("No components recorded")}
                body={t("Install a runtime, font, application, or compatibility setting from the catalog.")}
                action={<Button variant="primary" size="small" onClick={onBrowseCatalog}>{t("Browse components")}</Button>}
              />
            )}
          </section>
        </div>

        <aside className="content-secondary">
          <section className="side-panel health-panel">
            <div className="side-panel-title">
              <ShieldCheck size={19} />
              <h2>{t("Prefix health")}</h2>
            </div>
            <HealthRow ok={prefix.exists} label={t("Prefix structure")} detail={prefix.exists ? t("Registry and drive C found") : t("Initialization required")} />
            <HealthRow ok={Boolean(prefix.runtime_label)} label={t("Wine runtime")} detail={prefix.runtime_label ?? t("Runtime not resolved")} />
            <HealthRow ok={prefixRestores.length > 0} warning label={t("Recovery")} detail={prefixRestores.length ? t("Restore points: {count}", { count: formatNumber(prefixRestores.length) }) : t("No restore point yet")} />
            <Button variant="secondary" size="small" className="full-width" onClick={onCreateRestorePoint}>
              <Archive /> {t("Create restore point")}
            </Button>
          </section>

          <section className="side-panel recent-panel">
            <div className="side-panel-title">
              <h2>{t("Recent activity")}</h2>
            </div>
            {prefixOperations.length ? (
              <div className="mini-activity-list">
                {prefixOperations.slice(0, 4).map((operation) => (
                  <div className="mini-activity" key={operation.id}>
                    <div>
                      <strong dir="auto">{operation.recipes.map(friendlyVerb).join(", ")}</strong>
                      <small>{formatRelativeTime(operation.created_at)}</small>
                    </div>
                    <StatusBadge state={operation.state} />
                  </div>
                ))}
              </div>
            ) : (
              <p className="quiet-copy">{t("Operations for this prefix will appear here.")}</p>
            )}
          </section>
        </aside>
      </div>
    </div>
  );
});

function Stat({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <div className="stat-block">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{detail}</small>
    </div>
  );
}

function ToolButton({ icon, title, description, onClick }: { icon: React.ReactNode; title: string; description: string; onClick: () => void }) {
  return (
    <button className="tool-button" onClick={onClick}>
      <span className="tool-icon">{icon}</span>
      <span>
        <strong>{title}</strong>
        <small>{description}</small>
      </span>
      <ArrowRight className="directional-icon" size={15} />
    </button>
  );
}

function HealthRow({ ok, warning = false, label, detail }: { ok: boolean; warning?: boolean; label: string; detail: string }) {
  return (
    <div className="health-row">
      <span className={ok ? "health-ok" : warning ? "health-warning" : "health-error"}>
        {ok ? <CheckCircle weight="fill" /> : <WarningCircle weight="fill" />}
      </span>
      <span><strong>{label}</strong><small>{detail}</small></span>
    </div>
  );
}

function sourceName(prefix: WinePrefix, t: (key: string, values?: TranslationValues) => string) {
  if (prefix.source === "default_wine") return t("Default Wine prefix");
  if (prefix.source === "wine_prefixes") return t("Wine prefix");
  if (prefix.source === "manual") return t("Manually added");
  return titleCase(prefix.source);
}

function architectureName(architecture: WinePrefix["architecture"], t: (key: string, values?: TranslationValues) => string) {
  if (architecture === "wow64") return t("64-bit WoW64");
  if (architecture === "win64") return t("64-bit");
  if (architecture === "win32") return t("32-bit");
  return t("Unknown architecture");
}

function friendlyVerb(verb: string) {
  const names: Record<string, string> = {
    corefonts: "Microsoft core fonts",
    vcrun2022: "Visual C++ 2015-2022",
    vcrun2019: "Visual C++ 2015-2019",
    dxvk: "DXVK",
    dotnet48: ".NET Framework 4.8",
    d3dcompiler_47: "Direct3D compiler 47",
    win10: "Windows 10 mode",
    win11: "Windows 11 mode",
  };
  return names[verb] ?? titleCase(verb);
}
