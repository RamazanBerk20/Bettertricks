# Changelog

All notable Bettertricks changes are documented here.

## 1.0 — 2026-07-23

- Ships a recovery-first Tauri desktop application and scripting-friendly Rust CLI for Wine,
  Steam/Proton, Lutris, Bottles, and Heroic prefixes.
- Tracks all 550 user-visible Winetricks 20260125 entries: 159 audited native recipes, 390
  non-broken recipes through a checksum-verified compatibility host, and one upstream-broken entry
  retained but disabled.
- Implements every settings verb and 40 of 42 font verbs with typed native operations, pinned
  downloads, manual-file checksum import, Unicode registry support, and Winetricks cache sharing.
- Adds previewable operation plans, prefix locking, cancellation, structured diagnostics, per-item
  failure isolation, individual or bulk retry, and clearable activity history.
- Adds reflink or compressed restore points, fail-safe restoration, clearable recovery history, and
  Trash-based prefix removal with protection for managed prefixes.
- Resolves Steam compatdata AppIDs to locally installed game names when manifests are available.
- Keeps the component catalog usable during installs, supports filtered Select all and global
  Deselect all, and separates row actions from the scrollbar hit area.
- Makes long review plans independently scrollable while safeguards, headings, and confirmation
  actions remain visible.
- Improves overview startup and scrolling with cached prefix discovery, memoized catalog rows,
  stale-response guards, off-screen paint skipping, and reduced compositing work.
- Localizes the interface in English, Turkish, Spanish, Italian, French, German, Russian, Arabic,
  Simplified Chinese, Japanese, and Korean, with a system-language option and RTL Arabic layout.
- Adds an original minimalist Wine-glass, gear, and B application icon plus audited control,
  typography, switch, dropdown, and selection alignment.
- Adds deterministic Ed25519 catalog bundles, signed update activation, retained-version rollback,
  traversal/link/special-file rejection, and a pinned managed Winetricks host.
- Completes keyboard, contrast, reduced-motion, semantic, focus, and Linux AT-SPI accessibility
  checks.
- Adds an Ubuntu 24.04 devcontainer, continuous integration, Wine and Proton integration tests, and
  clean Debian, Fedora RPM, and AppImage release verification.
