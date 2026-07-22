import type { ButtonHTMLAttributes, ReactNode } from "react";
import clsx from "clsx";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  CaretDown,
  Check,
  CheckCircle,
  CloudArrowDown,
  Flask,
  GearSix,
  LinuxLogo,
  Package,
  SteamLogo,
  WarningCircle,
  Wine,
} from "@phosphor-icons/react";
import type {
  OperationState,
  PrefixSource,
  RecipeMaturity,
  VerbCategory,
} from "../types";
import { useI18n } from "../lib/i18n";

export function Button({
  variant = "secondary",
  size = "medium",
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "small" | "medium" | "icon";
}) {
  return (
    <button
      className={clsx("button", `button-${variant}`, `button-${size}`, className)}
      {...props}
    >
      {children}
    </button>
  );
}

export function Badge({
  tone = "neutral",
  children,
  className,
}: {
  tone?: "neutral" | "success" | "warning" | "danger" | "accent";
  children: ReactNode;
  className?: string;
}) {
  return <span className={clsx("badge", `badge-${tone}`, className)}>{children}</span>;
}

export interface SelectMenuOption {
  value: string;
  label: string;
}

export function SelectMenu({
  value,
  options,
  label,
  onValueChange,
  centered = false,
  className,
}: {
  value: string;
  options: SelectMenuOption[] | readonly SelectMenuOption[];
  label: string;
  onValueChange: (value: string) => void;
  centered?: boolean;
  className?: string;
}) {
  const selected = options.find((option) => option.value === value) ?? options[0];
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <button
          type="button"
          className={clsx("select-trigger", centered && "select-trigger-centered", className)}
          aria-label={label}
        >
          <span className="select-trigger-label" dir="auto">{selected?.label}</span>
          <CaretDown className="select-trigger-caret" weight="bold" aria-hidden="true" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          className="menu-content select-menu-content"
          align="end"
          sideOffset={6}
          collisionPadding={12}
        >
          <DropdownMenu.RadioGroup value={value}>
            {options.map((option) => (
              <DropdownMenu.RadioItem
                className="select-menu-item"
                value={option.value}
                onSelect={() => onValueChange(option.value)}
                key={option.value}
              >
                <span className="select-menu-check" aria-hidden="true"><Check weight="bold" /></span>
                <span className="select-menu-label" dir="auto">{option.label}</span>
              </DropdownMenu.RadioItem>
            ))}
          </DropdownMenu.RadioGroup>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

export function StatusBadge({ state }: { state: OperationState }) {
  const { t } = useI18n();
  const map: Record<OperationState, { tone: Parameters<typeof Badge>[0]["tone"]; label: string }> = {
    planned: { tone: "neutral", label: t("Planned") },
    preflight: { tone: "accent", label: t("Preflight") },
    running: { tone: "accent", label: t("Running") },
    waiting_for_user: { tone: "warning", label: t("Needs attention") },
    cancelling: { tone: "warning", label: t("Cancelling") },
    succeeded: { tone: "success", label: t("Complete") },
    failed: { tone: "danger", label: t("Failed") },
    cancelled: { tone: "neutral", label: t("Cancelled") },
  };
  return <Badge tone={map[state].tone}>{map[state].label}</Badge>;
}

export function CategoryIcon({ category, size = 18 }: { category: VerbCategory; size?: number }) {
  if (category === "apps") return <Package size={size} weight="duotone" />;
  if (category === "benchmarks") return <Flask size={size} weight="duotone" />;
  if (category === "fonts") return <span className="font-icon" aria-hidden="true">Aa</span>;
  if (category === "settings") return <GearSix size={size} weight="duotone" />;
  return <CloudArrowDown size={size} weight="duotone" />;
}

export function PrefixSourceIcon({ source, size = 18 }: { source: PrefixSource; size?: number }) {
  if (source === "steam") return <SteamLogo size={size} weight="fill" />;
  if (source === "default_wine" || source === "wine_prefixes") return <Wine size={size} weight="duotone" />;
  if (source === "manual") return <LinuxLogo size={size} weight="duotone" />;
  return <Package size={size} weight="duotone" />;
}

export function MaturityBadge({ maturity }: { maturity: RecipeMaturity }) {
  const { t } = useI18n();
  if (maturity === "native") {
    return (
      <Badge tone="success">
        <CheckCircle size={13} weight="fill" /> {t("Native")}
      </Badge>
    );
  }
  if (maturity === "broken_upstream") {
    return (
      <Badge tone="danger">
        <WarningCircle size={13} weight="fill" /> {t("Broken upstream")}
      </Badge>
    );
  }
  return <Badge>{t("Cataloged")}</Badge>;
}

export function ProgressBar({ value, label }: { value: number; label?: string }) {
  const { t } = useI18n();
  const safe = Math.max(0, Math.min(1, value));
  return (
    <div
      className="progress-wrap"
      role="progressbar"
      aria-label={label ?? t("Progress")}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(safe * 100)}
    >
      <div className="progress-track">
        <div className="progress-value" style={{ transform: `scaleX(${safe})` }} />
      </div>
      {label ? <span>{label}</span> : null}
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  body,
  action,
}: {
  icon: ReactNode;
  title: string;
  body: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">{icon}</div>
      <h3>{title}</h3>
      <p>{body}</p>
      {action}
    </div>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div className={clsx("skeleton", className)} aria-hidden="true" />;
}
