import { useEffect, useState } from "react";
import clsx from "clsx";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CheckCircle, Command, CornersIn, CornersOut, Minus, SpinnerGap, WarningCircle, X } from "@phosphor-icons/react";
import type { OperationState, WinePrefix } from "../types";
import { isTauri } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { PrefixSourceIcon } from "./common";

type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>["startResizeDragging"]>[0];
const EDGES: ResizeDirection[] = ["North", "South", "East", "West", "NorthEast", "NorthWest", "SouthEast", "SouthWest"];

export interface BarOperationStatus {
  title: string;
  detail: string;
  state: OperationState;
}

interface WindowBarProps {
  /** The prefix the window is working in; null where the page itself already names it. */
  prefix: WinePrefix | null;
  onOpenPrefix: () => void;
  onCommandPalette: () => void;
  operationStatus: BarOperationStatus | null;
  onOpenOperation: () => void;
  onError: (message: string) => void;
}

/** The window's own top bar: working prefix, command field, live operation, window buttons. */
export function WindowBar({ prefix, onOpenPrefix, onCommandPalette, operationStatus, onOpenOperation, onError }: WindowBarProps) {
  const { t } = useI18n();
  return (
    // Stops Radix dialogs treating a bar click as "outside" and dismissing themselves.
    <header className="window-bar" data-tauri-drag-region="deep" onPointerDown={(event) => event.stopPropagation()}>
      <div className="window-bar-context">
        {prefix ? (
          <button className="bar-prefix" onClick={onOpenPrefix} title={prefix.path}>
            <span className="bar-prefix-icon"><PrefixSourceIcon source={prefix.source} size={15} /></span>
            <strong dir="auto">{prefix.name}</strong>{" "}
            {prefix.runtime_label ? <span className="bar-chip" dir="auto">{prefix.runtime_label}</span> : null}
          </button>
        ) : null}
      </div>
      <button className="command-trigger" onClick={onCommandPalette}>
        <Command size={16} />
        <span>{t("Search or run")}</span>
        <kbd>Ctrl K</kbd>
      </button>
      <div className="window-bar-status">
        {operationStatus ? (
          <button className={clsx("bar-operation", `state-${operationStatus.state}`)} onClick={onOpenOperation} aria-label={t("Show current operation: {operation}", { operation: operationStatus.title })}>
            <span className="bar-operation-icon">
              {operationStatus.state === "succeeded" ? <CheckCircle weight="fill" /> : operationStatus.state === "failed" || operationStatus.state === "cancelled" || operationStatus.state === "interrupted" ? <WarningCircle weight="fill" /> : <SpinnerGap className="spin" />}
            </span>
            <span className="bar-operation-copy"><strong dir="auto">{operationStatus.title}</strong>{" "}<small dir="auto">{operationStatus.detail}</small></span>
          </button>
        ) : null}
      </div>
      <WindowControls onError={onError} />
    </header>
  );
}

/** Drag area and window buttons for screens that have no app bar yet (loading, fatal error). */
export function WindowStrip() {
  if (!isTauri) return null;
  return <div className="window-strip" data-tauri-drag-region="deep"><WindowControls /></div>;
}

function WindowControls({ onError }: { onError?: (message: string) => void }) {
  const { t } = useI18n();
  const maximized = useMaximized();
  // The browser preview has no native window to control.
  if (!isTauri) return null;
  const run = (action: "minimize" | "toggleMaximize" | "close") => {
    // close() raises the same CloseRequested as the system button, so the exit hook still runs.
    getCurrentWindow()[action]().catch((reason) => onError?.(String(reason)));
  };
  const resizeLabel = maximized ? t("Restore window") : t("Maximize window");
  return (
    <div className="window-controls" role="group" aria-label={t("Window controls")}>
      <button onClick={() => run("minimize")} aria-label={t("Minimize window")} title={t("Minimize window")}><Minus size={16} /></button>
      <button onClick={() => run("toggleMaximize")} aria-label={resizeLabel} title={resizeLabel}>{maximized ? <CornersIn size={16} /> : <CornersOut size={16} />}</button>
      <button className="window-close" onClick={() => run("close")} aria-label={t("Close window")} title={t("Close window")}><X size={16} /></button>
    </div>
  );
}

/** Resize handles and the outline an undecorated window needs while it is not maximized. */
export function WindowFrame() {
  const maximized = useMaximized();
  useEffect(() => {
    if (!isTauri) return;
    document.documentElement.dataset.windowFrame = maximized ? "maximized" : "normal";
  }, [maximized]);
  if (!isTauri || maximized) return null;
  return (
    <>
      {EDGES.map((edge) => (
        <div
          key={edge}
          className="window-resize-edge"
          data-edge={edge}
          aria-hidden="true"
          onMouseDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            getCurrentWindow().startResizeDragging(edge).catch(() => undefined);
          }}
        />
      ))}
    </>
  );
}

function useMaximized() {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!isTauri) return;
    const window = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const sync = () => {
      window.isMaximized().then((value) => { if (!disposed) setMaximized(value); }).catch(() => undefined);
    };
    sync();
    window.onResized(sync).then((release) => { if (disposed) release(); else unlisten = release; }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  return maximized;
}
