import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const cssUrl = new URL("../apps/desktop/src/styles.css", import.meta.url);

test("keeps core light and dark theme text at WCAG AA contrast", async () => {
  const css = await readFile(cssUrl, "utf8");
  const light = variables(block(css, /:root\s*\{/));
  const dark = variables(block(css, /:root\[data-theme="dark"\]\s*\{/));

  for (const [theme, colors] of [["light", light], ["dark", dark]]) {
    const surfaces = ["bg", "surface", "surface-raised", "surface-muted", "sidebar"];
    for (const foreground of ["text", "text-secondary", "text-muted"]) {
      for (const background of surfaces) {
        assertContrast(colors, foreground, background, 4.5, theme);
      }
    }
    assertContrast(colors, "accent-text", "accent-soft", 4.5, theme);
    assertContrast(colors, "success", "success-soft", 4.5, theme);
    assertContrast(colors, "warning", "warning-soft", 4.5, theme);
    assertContrast(colors, "danger", "danger-soft", 4.5, theme);
    assertContrast(colors, "on-accent", "accent", 4.5, theme);
    assertContrast(colors, "on-accent", "accent-hover", 4.5, theme);
    assertContrast(colors, "on-danger", "danger", 4.5, theme);
    assertContrast(colors, "on-danger", "danger-hover", 4.5, theme);
  }
});

test("honors operating-system and in-app reduced-motion preferences", async () => {
  const css = await readFile(cssUrl, "utf8");
  assert.match(css, /@media\s*\(prefers-reduced-motion:\s*reduce\)/);
  assert.match(css, /\.reduce-motion\s+\*,\s*\.reduce-motion\s+\*::before,\s*\.reduce-motion\s+\*::after/);
});

function block(css, selector) {
  const match = selector.exec(css);
  assert.ok(match, `Missing CSS block ${selector}`);
  const start = match.index + match[0].length;
  const end = css.indexOf("}", start);
  assert.notEqual(end, -1, `Unclosed CSS block ${selector}`);
  return css.slice(start, end);
}

function variables(css) {
  return Object.fromEntries([...css.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})\s*;/gi)].map((match) => [match[1], match[2]]));
}

function assertContrast(colors, foreground, background, minimum, theme) {
  assert.ok(colors[foreground], `Missing --${foreground} in ${theme} theme`);
  assert.ok(colors[background], `Missing --${background} in ${theme} theme`);
  const ratio = contrast(colors[foreground], colors[background]);
  assert.ok(ratio >= minimum, `${theme} --${foreground} on --${background} is ${ratio.toFixed(2)}:1; expected at least ${minimum}:1`);
}

function contrast(left, right) {
  const first = luminance(left);
  const second = luminance(right);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

function luminance(hex) {
  const channels = hex.slice(1).match(/../g).map((value) => Number.parseInt(value, 16) / 255);
  const linear = channels.map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}
