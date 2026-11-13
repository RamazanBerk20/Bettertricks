# Changelog

All notable Bettertricks changes are documented here.

## 1.1.0 — 2026-11-13

Upgrade notes (behaviour changes for existing users):

- No downgrade: once an operation has been recorded as `interrupted` (an operation left running
  by a crash or quit), a 1.0 binary can no longer read the activity history in that database.
- Changes to Steam, Lutris, Bottles and Heroic prefixes are now blocked (`runtime_unknown`) when
  the launcher's Wine/Proton runtime cannot be determined, until a runtime is chosen (Add prefix →
  Existing prefix with a runtime) or the change is forced. Running system Wine against such a
  prefix can upgrade and break it.
- Only one operation runs per prefix at a time, across the desktop app and the CLI; a second one is
  refused while the prefix is busy.
- Restore points are kept without limit by default, as in 1.0; a per-prefix limit can be chosen
  in Settings → Restore points.

Added:

- Interrupted operations: operations left running by a crash or quit are reconciled at startup and
  shown as `interrupted` instead of staying "running" forever.
- `ops list` and `ops clear` accept `--json` in the CLI.
- Catalog manifests carry the pinned Winetricks compatibility host (tag, URL, SHA-256), so signed
  catalog updates can move the managed host without a new app release.
- Signed catalog revisions with a rollback pin that survives a newer bundled catalog.
- AppStream metadata is now installed with the AppImage, Debian and RPM packages.
- The AppImage, Debian and RPM packages now include the licence and NOTICE (`/usr/share/doc/bettertricks/copyright` in the deb, `/usr/share/licenses/bettertricks/` otherwise).
- A quiet GitHub Sponsors link in the README and repository funding file.

Changed:

- Prefix locking is a cross-process lock that fails closed instead of a per-process lock.
- The engine, database, CLI and UI share one operation-state model.
- Failed or partial downloads are quarantined in the cache and other mirrors are tried before
  failing.
- Dependency updates.

Fixed:

- Cancel now stops the Wine processes an operation started (whole process groups), instead of only
  abandoning the operation.
- Launcher runtimes are validated before use rather than trusted from metadata.

Security:

- The managed Winetricks host pin is now read from the signed catalog manifest; see SECURITY.md.

Not tested with real installs: this release was verified with unit, integration and fault-injection
tests and the CI smoke suites, but not against real Steam, Lutris, Bottles or Heroic installations,
nor with a real Wine run on the release machine.

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
