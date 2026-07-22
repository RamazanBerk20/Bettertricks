export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "Unknown";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} ${units[unit]}` : `${value.toFixed(1)} ${units[unit]}`;
}

export function formatRelativeTime(value: string | null): string {
  if (!value) return "Never";
  const difference = Date.now() - new Date(value).getTime();
  const minutes = Math.round(difference / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(new Date(value));
}

export function titleCase(value: string): string {
  return value
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function shortPath(path: string, length = 54): string {
  if (path.length <= length) return path;
  const parts = path.split("/");
  return `…/${parts.slice(-3).join("/")}`;
}

export function hasVersionToken(version: string | null | undefined, expected: string): boolean {
  return version?.split(/\s+/).includes(expected) ?? false;
}
