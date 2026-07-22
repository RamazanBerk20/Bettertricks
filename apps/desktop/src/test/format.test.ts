import { describe, expect, it, vi } from "vitest";

import { formatBytes, formatRelativeTime, hasVersionToken, shortPath, titleCase } from "../lib/format";

describe("display formatting", () => {
  it("formats storage, identifiers, and long paths", () => {
    expect(formatBytes(1_610_612_736)).toBe("1.5 GB");
    expect(titleCase("d3dcompiler_47")).toBe("D3dcompiler 47");
    expect(shortPath("/home/user/.local/share/wineprefixes/my-application", 20)).toBe("…/share/wineprefixes/my-application");
  });

  it("formats recent timestamps relative to now", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-21T12:00:00Z"));
    expect(formatRelativeTime("2026-07-21T11:30:00Z")).toBe("30m ago");
    vi.useRealTimers();
  });

  it("matches compatibility-host versions as complete tokens", () => {
    expect(hasVersionToken("winetricks 20260125", "20260125")).toBe(true);
    expect(hasVersionToken("winetricks 20260125-next", "20260125")).toBe(false);
    expect(hasVersionToken(undefined, "20260125")).toBe(false);
  });
});
