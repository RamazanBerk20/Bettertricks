import { useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import * as Switch from "@radix-ui/react-switch";
import {
  Archive,
  ArrowClockwise,
  CheckCircle,
  Code,
  Database,
  DownloadSimple,
  FolderOpen,
  HardDrive,
  Moon,
  ShieldCheck,
  SpinnerGap,
  Sun,
  Trash,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import type { AppSettings, BootstrapPayload, ThemePreference, UUID } from "../types";
import { hasVersionToken, shortPath } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { normalizeLanguagePreference, SUPPORTED_LOCALES } from "../lib/translations";
import { Badge, Button, SelectMenu } from "../components/common";

const LANGUAGE_OPTIONS = SUPPORTED_LOCALES.map(({ id, label }) => ({ value: id, label }));

interface SettingsViewProps {
  data: BootstrapPayload;
  settings: AppSettings;
  onSettingsChange: (settings: AppSettings) => void;
  onClearCache: () => void;
  onClearRestorePoints: () => Promise<void>;
  onOpenPath: (path: string) => void;
  onOpenLegacyVerb: () => void;
  onCheckCatalog: () => void;
  onRollbackCatalog: () => void;
  onInstallCompatibilityHost: () => void;
  compatibilityHostInstalling: boolean;
  onRestorePrefix: (restorePointId: UUID) => void;
}

export function SettingsView({ data, settings, onSettingsChange, onClearCache, onClearRestorePoints, onOpenPath, onOpenLegacyVerb, onCheckCatalog, onRollbackCatalog, onInstallCompatibilityHost, compatibilityHostInstalling, onRestorePrefix }: SettingsViewProps) {
  const { t, formatBytes, formatNumber, formatRelativeTime } = useI18n();
  const [clearRestoreOpen, setClearRestoreOpen] = useState(false);
  const [clearingRestore, setClearingRestore] = useState(false);
  const languageOptions = [{ value: "system", label: t("System") }, ...LANGUAGE_OPTIONS];
  const dependencyReady = (item: BootstrapPayload["system"]["dependencies"][number]) => item.available && (item.id !== "winetricks" || hasVersionToken(item.version, data.catalog.upstream_tag));
  const missingOptional = data.system.dependencies.filter((item) => !item.required && !dependencyReady(item));
  const legacyHostAvailable = data.system.dependencies.find((item) => item.id === "winetricks")?.available ?? false;
  const clearRestorePoints = async () => {
    setClearingRestore(true);
    try {
      await onClearRestorePoints();
      setClearRestoreOpen(false);
    } catch {
      // The app-level toast reports the backend error while this dialog stays open.
    } finally {
      setClearingRestore(false);
    }
  };
  return (
    <div className="page-scroll settings-page">
      <header className="page-header standard-header">
        <div>
          <div className="heading-meta"><Badge tone="neutral">{t("Local preferences")}</Badge><span>{t("No account required")}</span></div>
          <h1>{t("Settings")}</h1>
          <p>{t("Control appearance, recovery defaults, catalog updates, and the local Wine environment.")}</p>
        </div>
      </header>

      <div className="settings-layout">
        <nav className="settings-index" aria-label={t("Settings sections")}>
          <a href="#appearance">{t("Appearance")}</a>
          <a href="#safety">{t("Safety")}</a>
          <a href="#recovery">{t("Recovery")}</a>
          <a href="#catalog">{t("Catalog")}</a>
          <a href="#system">{t("System")}</a>
          <a href="#storage">{t("Storage")}</a>
          <a href="#advanced">{t("Advanced")}</a>
        </nav>

        <div className="settings-content">
          <SettingsSection id="appearance" title={t("Appearance")} description={t("Match your desktop or choose a fixed color mode.")}>
            <div className="theme-options" role="group" aria-label={t("Color theme")}>
              <ThemeOption value="system" current={settings.theme} icon={<HardDrive />} label={t("System")} onSelect={(theme) => onSettingsChange({ ...settings, theme })} />
              <ThemeOption value="light" current={settings.theme} icon={<Sun />} label={t("Light")} onSelect={(theme) => onSettingsChange({ ...settings, theme })} />
              <ThemeOption value="dark" current={settings.theme} icon={<Moon />} label={t("Dark")} onSelect={(theme) => onSettingsChange({ ...settings, theme })} />
            </div>
            <SettingRow title={t("Language")} description={t("Change the interface language immediately. Catalog content and diagnostic output keep their source language.")}>
              <SelectMenu
                className="language-trigger"
                centered
                value={normalizeLanguagePreference(settings.language)}
                options={languageOptions}
                label={t("Interface language")}
                onValueChange={(language) => onSettingsChange({ ...settings, language: normalizeLanguagePreference(language) })}
              />
            </SettingRow>
            <SettingRow title={t("Reduce motion")} description={t("Disable non-essential movement and use instant state changes.")}>
              <Toggle checked={settings.reduced_motion} onChange={(checked) => onSettingsChange({ ...settings, reduced_motion: checked })} label={t("Reduce motion")} />
            </SettingRow>
          </SettingsSection>

          <SettingsSection id="safety" title={t("Safety and recovery")} description={t("Keep risky changes understandable and recoverable.")}>
            <SettingRow title={t("Protect managed prefixes")} description={t("Recommend a restore point before changing Steam, Lutris, Bottles, or Heroic prefixes.")}>
              <Toggle checked={settings.restore_before_managed_changes} onChange={(checked) => onSettingsChange({ ...settings, restore_before_managed_changes: checked })} label={t("Protect managed prefixes")} />
            </SettingRow>
            <div className="settings-callout success">
              <ShieldCheck size={21} weight="fill" />
              <div><strong>{t("Recovery-first deletion is active")}</strong><span>{t("Prefixes are moved to the desktop Trash. Permanent deletion is never the default action.")}</span></div>
            </div>
          </SettingsSection>

          <SettingsSection id="recovery" title={t("Restore points")} description={t("Recover a prefix after its current directory has been moved aside or to Trash.")}>
            <div className="recovery-actions">
              <Button size="small" onClick={() => setClearRestoreOpen(true)} disabled={!data.restore_points.length}>
                <Trash /> {t("Clear restore points")}
              </Button>
            </div>
            {data.restore_points.length ? (
              <div className="dependency-list">
                {data.restore_points.map((point) => {
                  const targetExists = data.prefixes.some((prefix) => prefix.path === point.prefix_path && prefix.exists);
                  return (
                    <div className="dependency-row" key={point.id}>
                      <span className="dependency-ok"><Archive weight="fill" /></span>
                      <div><strong>{point.prefix_name}</strong><span>{formatRelativeTime(point.created_at)} · {formatBytes(point.size_bytes)}</span></div>
                      <Button size="small" onClick={() => onRestorePrefix(point.id)} disabled={targetExists} title={targetExists ? t("Move the current prefix to Trash before restoring") : undefined}>{t("Restore")}</Button>
                    </div>
                  );
                })}
              </div>
            ) : <p className="quiet-copy">{t("No restore points have been created yet.")}</p>}
            <p className="settings-footnote">{t("Restore never overwrites a live prefix. Move the current directory to Trash first, then restore the saved copy here.")}</p>
          </SettingsSection>

          <SettingsSection id="catalog" title={t("Recipe catalog")} description={t("Signed recipe data can update independently from the application.")}>
            <SettingRow title={t("Automatic catalog updates")} description={t("Currently using Winetricks {version} metadata.", { version: data.catalog.upstream_tag })}>
              <Toggle checked={settings.catalog_auto_update} onChange={(checked) => onSettingsChange({ ...settings, catalog_auto_update: checked })} label={t("Automatic catalog updates")} />
            </SettingRow>
            <div className="catalog-version-row">
              <span className="catalog-version-icon"><DownloadSimple size={21} /></span>
              <div className="catalog-version-copy"><strong>{data.catalog.version}</strong><span>{t("Native recipes: {native}; tracked ports: {tracked}", { native: formatNumber(data.catalog.native_count), tracked: formatNumber(data.catalog.recipe_count - data.catalog.native_count) })}</span></div>
              <Badge tone={data.catalog_signed ? "success" : "neutral"}><CheckCircle weight="fill" /> {data.catalog_signed ? t("Signature valid") : t("Bundled catalog")}</Badge>
              <div className="catalog-version-actions">{data.catalog_rollback_available ? <Button size="small" variant="ghost" onClick={onRollbackCatalog}>{t("Roll back")}</Button> : null}<Button size="small" onClick={onCheckCatalog}><ArrowClockwise /> {t("Check now")}</Button></div>
            </div>
          </SettingsSection>

          <SettingsSection id="system" title={t("System integration")} description={t("Bettertricks uses the Wine and helper tools installed by your distribution.")}>
            <div className="dependency-list">
              {data.system.dependencies.map((dependency) => (
                <div className="dependency-row" key={dependency.id}>
                  <span className={dependencyReady(dependency) ? "dependency-ok" : dependency.required || dependency.id === "winetricks" && dependency.available ? "dependency-error" : "dependency-optional"}>
                    {dependencyReady(dependency) ? <CheckCircle weight="fill" /> : <WarningCircle weight="fill" />}
                  </span>
                  <div><strong>{dependency.label}</strong><span>{dependency.id === "winetricks" && dependency.available && !dependencyReady(dependency) ? t("Expected {expected}; found {found}", { expected: data.catalog.upstream_tag, found: dependency.version ?? t("unknown") }) : dependency.version ?? dependency.remediation ?? t("Optional")}</span></div>
                  <div className="dependency-tail">
                    <code>{dependency.path ? shortPath(dependency.path, 34) : dependency.required ? t("Missing") : t("Optional")}</code>
                    {dependency.id === "winetricks" && !dependencyReady(dependency) ? (
                      <Button size="small" onClick={onInstallCompatibilityHost} disabled={compatibilityHostInstalling}>
                        <DownloadSimple /> {compatibilityHostInstalling ? t("Installing…") : t("Install verified host")}
                      </Button>
                    ) : null}
                  </div>
                </div>
              ))}
            </div>
            {missingOptional.length ? <p className="settings-footnote">{t("Optional helpers not ready: {count}. Relevant recipes explain how to enable them.", { count: formatNumber(missingOptional.length) })}</p> : null}
          </SettingsSection>

          <SettingsSection id="storage" title={t("Storage")} description={t("Downloads are shared with Winetricks to avoid duplicate files.")}>
            <div className="storage-meter">
              <span className="storage-icon"><Database size={22} /></span>
              <div><small>{t("Download cache")}</small><strong>{formatBytes(data.cache.size_bytes)}</strong><span>{t("Files: {count}", { count: formatNumber(data.cache.file_count) })}</span></div>
              <div className="storage-actions">
                <Button size="small" variant="ghost" onClick={() => onOpenPath(data.cache.path)}><FolderOpen /> {t("Open")}</Button>
                <Button size="small" onClick={onClearCache}>{t("Clear cache")}</Button>
              </div>
            </div>
            <code className="storage-path">{data.cache.path}</code>
          </SettingsSection>

          <SettingsSection id="advanced" title={t("Advanced controls")} description={t("Expose low-level flags, raw paths, and recipe diagnostics.")}>
            <SettingRow title={t("Show advanced options")} description={t("Add force, no-clean, verification, Tor, and raw log controls to operation review.")}>
              <Toggle checked={settings.show_advanced} onChange={(checked) => onSettingsChange({ ...settings, show_advanced: checked })} label={t("Show advanced options")} />
            </SettingRow>
            <SettingRow title={t("Run a legacy .verb file")} description={legacyHostAvailable ? t("Inspect a custom Winetricks shell recipe, create a restore point, then run it through the optional compatibility host.") : t("Install Winetricks to enable the isolated compatibility path for custom .verb files.")}>
              <Button size="small" onClick={onOpenLegacyVerb} disabled={!legacyHostAvailable}><Code /> {t("Choose file")}</Button>
            </SettingRow>
          </SettingsSection>
        </div>
      </div>

      <Dialog.Root open={clearRestoreOpen} onOpenChange={(open) => { if (!clearingRestore) setClearRestoreOpen(open); }}>
        <Dialog.Portal>
          <Dialog.Overlay className="dialog-overlay" />
          <Dialog.Content className="dialog-content danger-dialog">
            <Dialog.Close className="dialog-close" disabled={clearingRestore} aria-label={t("Close")}><X /></Dialog.Close>
            <div className="dialog-icon danger"><Trash weight="fill" /></div>
            <Dialog.Title>{t("Clear restore points?")}</Dialog.Title>
            <Dialog.Description>{t("Saved prefix snapshots will be permanently deleted. Restore points in use by active operations will be kept.")}</Dialog.Description>
            <div className="dialog-actions">
              <Dialog.Close asChild><Button disabled={clearingRestore}>{t("Cancel")}</Button></Dialog.Close>
              <Button variant="danger" disabled={clearingRestore} onClick={() => void clearRestorePoints()}>
                {clearingRestore ? <SpinnerGap className="spin" /> : <Trash weight="fill" />} {t("Clear restore points")}
              </Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}

function SettingsSection({ id, title, description, children }: { id: string; title: string; description: string; children: React.ReactNode }) {
  return <section className="settings-section" id={id}><div className="settings-section-heading"><h2>{title}</h2><p>{description}</p></div><div className="settings-section-body">{children}</div></section>;
}

function SettingRow({ title, description, children }: { title: string; description: string; children: React.ReactNode }) {
  return <div className="setting-row"><div><strong>{title}</strong><span>{description}</span></div>{children}</div>;
}

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (checked: boolean) => void; label: string }) {
  return <Switch.Root className="switch-root" checked={checked} onCheckedChange={onChange} aria-label={label}><Switch.Thumb className="switch-thumb" /></Switch.Root>;
}

function ThemeOption({ value, current, icon, label, onSelect }: { value: ThemePreference; current: ThemePreference; icon: React.ReactNode; label: string; onSelect: (value: ThemePreference) => void }) {
  return <button className={current === value ? "theme-option active" : "theme-option"} aria-pressed={current === value} onClick={() => onSelect(value)}><span>{icon}</span><strong>{label}</strong>{current === value ? <CheckCircle weight="fill" /> : null}</button>;
}
