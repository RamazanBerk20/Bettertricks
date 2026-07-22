import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");

test("keeps circular control glyphs geometrically centered", () => {
  const styles = readFileSync(join(root, "apps/desktop/src/styles.css"), "utf8");
  assert.match(styles, /\.setting-row > div > span \{[^}]*margin-top: 4px;/s);
  assert.doesNotMatch(styles, /\.setting-row span \{/);
  assert.doesNotMatch(styles, /\.selection-tray span \{/);
  assert.doesNotMatch(styles, /\.catalog-version-row span \{/);
  assert.match(styles, /\.switch-thumb \{[^}]*inset-block-start: 2px;[^}]*inset-inline-start: 2px;[^}]*margin: 0;/s);
  assert.match(styles, /\.switch-thumb\[data-state="checked"\] \{[^}]*inset-inline-start: 16px;/s);
  assert.match(styles, /\.queue-toggle \{[^}]*display: grid;[^}]*padding: 0;[^}]*place-items: center;/s);
});

test("keeps component actions clear of the catalog scrollbar", () => {
  const styles = readFileSync(join(root, "apps/desktop/src/styles.css"), "utf8");
  assert.match(styles, /\.catalog-list \{[^}]*overflow-y: auto;[^}]*scrollbar-gutter: stable;/s);
  assert.match(styles, /\.queue-toggle \{[^}]*width: 31px;[^}]*height: 31px;[^}]*margin-inline: 9px 21px;/s);
});

test("keeps select popups inside the app visual and accessibility system", () => {
  const styles = readFileSync(join(root, "apps/desktop/src/styles.css"), "utf8");
  const common = readFileSync(join(root, "apps/desktop/src/components/common.tsx"), "utf8");
  const settings = readFileSync(join(root, "apps/desktop/src/views/settings-view.tsx"), "utf8");
  const dialogs = readFileSync(join(root, "apps/desktop/src/components/dialogs.tsx"), "utf8");
  assert.doesNotMatch(`${settings}\n${dialogs}`, /<select\b/);
  assert.match(common, /DropdownMenu\.RadioItem/);
  assert.match(styles, /\.select-trigger-centered \{[^}]*grid-template-columns: 16px minmax\(0, 1fr\) 16px;/s);
  assert.match(styles, /\.select-trigger-label \{[^}]*line-height: 1\.4;/s);
  assert.match(styles, /\.select-menu-content \{[^}]*max-height:[^;}]*--radix-dropdown-menu-content-available-height/s);
});

test("keeps shared layout edges and fixed-size indicators aligned", () => {
  const styles = readFileSync(join(root, "apps/desktop/src/styles.css"), "utf8");
  assert.match(styles, /\.settings-page \.page-header \{[^}]*max-width: 1160px;/s);
  assert.match(styles, /\.settings-layout \{[^}]*grid-template-columns: 170px minmax\(0, 1fr\);[^}]*max-width: 1160px;/s);
  assert.match(styles, /\.review-actions \{[^}]*padding-inline: 22px 18px;/s);
  assert.match(styles, /\.choice-card > span \{[^}]*justify-self: center;/s);
  assert.match(styles, /\.review-step > span:first-child \{[^}]*justify-self: center;/s);
  assert.match(styles, /\.option-check i \{[^}]*justify-self: center;/s);
  assert.match(styles, /\.drawer-event > span \{[^}]*justify-self: center;/s);
});

test("puts the bounded review scroller at the execution pane and skips offscreen catalog paint", () => {
  const styles = readFileSync(join(root, "apps/desktop/src/styles.css"), "utf8");
  const catalogView = readFileSync(join(root, "apps/desktop/src/views/catalog-view.tsx"), "utf8");
  assert.match(styles, /\.review-dialog \{[^}]*display: grid;[^}]*grid-template-rows: auto minmax\(0, 1fr\) auto;[^}]*height: min\(720px, calc\(100dvh - 42px\)\);/s);
  assert.match(styles, /\.review-body \{[^}]*overflow: hidden;/s);
  assert.match(styles, /\.review-main \{[^}]*overflow-y: auto;[^}]*scrollbar-gutter: stable;/s);
  assert.match(styles, /\.catalog-row \{[^}]*content-visibility: auto;[^}]*contain-intrinsic-size: auto 74px;/s);
  assert.match(catalogView, /const CatalogRow = memo\(function CatalogRow/);
  assert.doesNotMatch(styles, /backdrop-filter:/);
});
