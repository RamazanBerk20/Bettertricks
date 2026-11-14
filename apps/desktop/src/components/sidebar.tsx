import clsx from "clsx";
import {
  ArrowsClockwise,
  ClockCounterClockwise,
  GearSix,
  House,
  Package,
  Plus,
  Pulse,
} from "@phosphor-icons/react";
import type { CatalogSummary, SystemReport, UUID, WinePrefix } from "../types";
import { useI18n, type TranslationValues } from "../lib/i18n";
import { PrefixSourceIcon } from "./common";

export type AppView = "prefix" | "catalog" | "activity" | "settings";

interface SidebarProps {
  view: AppView;
  onViewChange: (view: AppView) => void;
  prefixes: WinePrefix[];
  selectedPrefixId: UUID | null;
  onPrefixChange: (id: UUID) => void;
  onAddPrefix: () => void;
  catalog: CatalogSummary;
  system: SystemReport;
  runningCount: number;
}

export function Sidebar({
  view,
  onViewChange,
  prefixes,
  selectedPrefixId,
  onPrefixChange,
  onAddPrefix,
  catalog,
  system,
  runningCount,
}: SidebarProps) {
  const { t, formatNumber } = useI18n();
  return (
    <aside className="sidebar">
      {/* The brand row continues the window bar, so it moves the window too. */}
      <div className="brand-row" data-tauri-drag-region="deep">
        <img className="brand-mark" src="/icon.png" alt="" aria-hidden="true" draggable={false} />
        <div className="brand-copy">
          <strong>Bettertricks</strong>
          <span>{t("Wine, made manageable")}</span>
        </div>
      </div>

      <nav className="primary-nav" aria-label={t("Main navigation")}>
        <NavButton active={view === "prefix"} icon={<House />} label={t("Overview")} onClick={() => onViewChange("prefix")} />
        <NavButton active={view === "catalog"} icon={<Package />} label={t("Components")} meta={formatNumber(catalog.recipe_count)} onClick={() => onViewChange("catalog")} />
        <NavButton active={view === "activity"} icon={<ClockCounterClockwise />} label={t("Activity")} meta={runningCount ? formatNumber(runningCount) : undefined} onClick={() => onViewChange("activity")} />
        <NavButton active={view === "settings"} icon={<GearSix />} label={t("Settings")} onClick={() => onViewChange("settings")} />
      </nav>

      <div className="sidebar-section-header">
        <span>{t("Wine prefixes")}</span>
        <button className="icon-button subtle" onClick={onAddPrefix} aria-label={t("Add Wine prefix")}>
          <Plus size={15} />
        </button>
      </div>
      <nav className="prefix-nav" aria-label={t("Wine prefixes")}>
        {prefixes.map((prefix) => (
          <button
            key={prefix.id}
            aria-current={selectedPrefixId === prefix.id ? "page" : undefined}
            className={clsx("prefix-nav-item", selectedPrefixId === prefix.id && "active")}
            onClick={() => {
              onPrefixChange(prefix.id);
              onViewChange("prefix");
            }}
          >
            <span className="prefix-nav-icon"><PrefixSourceIcon source={prefix.source} /></span>
            <span className="prefix-nav-copy">
              <strong dir="auto">{prefix.name}</strong>
              <span>{sourceLabel(prefix, t)}</span>
            </span>
          </button>
        ))}
      </nav>

      <div className="sidebar-footer">
        <div className={clsx("system-summary", system.ready ? "ready" : "warning")}>
          <span className="system-summary-icon">
            {system.ready ? <Pulse size={16} weight="bold" /> : <ArrowsClockwise size={16} />}
          </span>
          <span>
            <strong>{system.ready ? t("System ready") : t("Setup needed")}</strong>
            <small>{system.runtimes[0]?.version ?? t("Wine not found")}</small>
          </span>
        </div>
      </div>
    </aside>
  );
}

function NavButton({
  active,
  icon,
  label,
  meta,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  meta?: string | number;
  onClick: () => void;
}) {
  return (
    <button className={clsx("nav-button", active && "active")} onClick={onClick} aria-current={active ? "page" : undefined}>
      <span className="nav-icon">{icon}</span>
      <span>{label}</span>
      {meta !== undefined ? <small>{meta}</small> : null}
    </button>
  );
}

function sourceLabel(prefix: WinePrefix, t: (key: string, values?: TranslationValues) => string) {
  if (prefix.source === "default_wine") return t("Default Wine");
  if (prefix.source === "wine_prefixes") return t("Wine prefix");
  if (prefix.source === "steam") return "Steam / Proton";
  if (prefix.source === "lutris") return "Lutris";
  if (prefix.source === "bottles") return "Bottles";
  if (prefix.source === "heroic") return "Heroic";
  return t("Added manually");
}
