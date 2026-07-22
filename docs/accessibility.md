# Accessibility audit

Bettertricks treats keyboard and assistive-technology behavior as release functionality, not
visual polish. The current code audit and automated regressions cover the desktop shell, catalog,
settings, operation review, command palette, and attended-operation prompt.

## Keyboard and focus

- A first-focus skip link moves directly to the main view.
- Every action uses a native button, link, input, select, summary, or Radix keyboard primitive.
- The command palette implements the ARIA combobox/listbox pattern, wraps Up/Down selection, and
  runs the active result with Enter. Its instructions are exposed through the dialog description.
- Dialogs and menus trap or restore focus through Radix. When an attended operation prompt
  appears, focus moves to its first response.
- The operation review exposes its bounded, keyboard-focusable scroll area as a named region while
  keeping safeguards and confirmation actions visible around long plans.
- Active operation monitoring can be hidden without cancellation and reopened from a named sidebar
  button; a newly attended prompt brings the monitor back so its focused choices are not missed.
- All interactive elements retain a visible two-pixel focus indicator; palette selection has a
  persistent visual highlight while focus remains in the search field.

## Semantics and announcements

- Main and prefix navigation expose current-page state. Toggle, theme, filter, and selection
  controls expose pressed or checked state.
- Recipe inputs retain programmatic labels and required state. Inline failures and failed-operation
  toasts are alerts; routine status messages use polite live regions.
- Progress is clamped and announced from 0 through 100. Activity history exposes table, row,
  header, cell, and per-operation progress semantics. Failed-operation details are expandable;
  individual retry controls include the component name, and retry-all reports its item count.
- Decorative skeletons and visual-only icons are hidden where they would add noise.

## Visual accessibility

Light and dark themes use shared semantic color tokens. `scripts/accessibility-regressions.test.mjs`
checks every primary, secondary, muted, semantic, and button text pair at WCAG AA 4.5:1 or better.
Theme-aware foreground tokens prevent the dark theme's lighter action colors from reducing button
contrast. Both the operating-system `prefers-reduced-motion` setting and the in-app setting reduce
animations and transitions to effectively instantaneous changes.

The application sets the document language for every supported locale. Arabic additionally sets
right-to-left direction, mirrors directional controls and progress origins, and keeps technical
paths and keyboard tokens left-to-right. Locale switching updates translated accessible names and
number/date formatting without restarting the app.

Run the automated audit with:

```sh
node --test scripts/accessibility-regressions.test.mjs
pnpm --filter @bettertricks/desktop test
```

The clean Debian-package smoke additionally starts the built WebKitGTK application under Xvfb and
a fresh D-Bus session, discovers it through Linux AT-SPI, requires the main window and named
navigation controls, counts exposed buttons, and moves assistive-technology focus to one of them:

```sh
pnpm smoke:packages:clean
```

The automated AT-SPI probe validates the platform bridge and focusability on the oldest supported
Ubuntu release. An exploratory Orca pass remains recommended whenever labels, navigation order, or
attended-operation prompts change because speech phrasing is a human review judgment.
