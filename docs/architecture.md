# Architecture

Bettertricks separates desktop presentation from a reusable native engine. The desktop and
CLI resolve the same recipe model and call the same planner and operation runner.

```text
React desktop ── typed Tauri commands ─┐
                                      ├── Rust core ── Wine and helper processes
bettertricks CLI ─────────────────────┘       │
                                             ├── SQLite state
                                             ├── XDG cache and restore points
                                             └── versioned recipe catalogs
```

## Workspace

- `apps/desktop`: React 19 interface, browser mock backend, and Tauri 2 host.
- `crates/bettertricks-core`: domain model, discovery, catalog, planner, executor, recovery,
  update verification, legacy bridge, system inspection, and SQLite persistence.
- `crates/bettertricks-cli`: Winetricks-shaped command-line surface backed by the core.
- `crates/bettertricks-catalog-tool`: catalog validation, deterministic bundles, Ed25519
  signatures, and update index generation.
- `catalog/native`: audited executable recipes.
- `catalog/generated`: upstream parity metadata executed only through the exact matching
  Winetricks compatibility host.

## Prefix discovery

Discovery providers inspect the default `WINEPREFIX`, `$XDG_DATA_HOME/wineprefixes`, Steam
library manifests and `compatdata`, Lutris YAML, Bottles, Heroic, and manually registered
paths. Results are deduplicated by canonical path. Prefix IDs are deterministic UUIDv5
values, so launcher rediscovery does not break activity or recovery links.
Steam prefixes use the local `appmanifest_<appid>.acf` display name when available and fall
back to `Steam <appid>` for orphaned compatdata or non-library entries.

A prefix inspection reads architecture markers and `winetricks.log` without starting Wine.
Launcher-owned prefixes are marked managed and receive stronger preflight warnings.

## Recipes and planning

Recipes are TOML documents validated against schema version 1. Their steps are a closed Rust
enum: download, directory/file preparation and verification, copy/move/remove, pinned or
nested archive extraction, font installation and replacement, Wine process, registry import,
DLL override, Windows version, dependency call, notice, prompt, architecture-conditional
group, or audited native action. Windows-correct 32/64-bit system-directory expansion keeps
WoW64 placement explicit. Unknown actions fail closed. Required free-form values are declared
as typed recipe inputs and are
carried in the validated operation plan rather than interpolated into shell commands.

The planner resolves dependencies topologically, rejects cycles, detects installed or
selected conflicts, evaluates architecture constraints, lists downloads and cache hits, and
builds the exact execution order shown in the review dialog. A `metadata_only` recipe receives
an explicit compatibility-host step and warning. Execution requires a checksum-verified managed
host or system Winetricks whose version output contains the recipe catalog's exact upstream tag as
a whitespace-delimited token; identifiers are catalog-validated
and passed as one process argument. `broken_upstream` remains searchable but cannot execute.

## Operation lifecycle

Only one operation can mutate a prefix at a time. Other prefixes may run independently.
Every operation is persisted and emits sequenced events through Tauri:

```text
planned → preflight → running ⇄ waiting_for_user → succeeded
                       │   └─ recipe failure: record diagnostics, skip its dependants,
                       │      and continue independent recipes
                       └────────────────────────→ failed / cancelled
```

Preflight, cancellation, storage, and security-policy failures stop the operation. A recipe-level
installer or verification failure does not: Bettertricks stores the component, error, and bounded
process-output tail, skips only recipes that depend on it, and finishes every independent recipe.
The final operation is marked failed if any component failed or was dependency-skipped. The live
drawer and persisted Activity entry can prepare a reviewed retry for one component or all failures.

Downloads use HTTPS-capable Rust networking, optional Tor SOCKS routing, per-cache-path
coordination, operation-unique partial files, and SHA-256 verification before activation.
Manual downloads use the same 16 GiB safety cap and are copied into the cache through an
operation-unique staging file; a checksum mismatch removes the staging file and never activates it.
Recipes may intentionally share Winetricks cache paths only when their filename, checksum,
and manual-download policy agree. Process arguments are passed directly rather than through
a shell. Mutating and nested-extraction file steps are constrained to the prefix or
application cache. Registry imports automatically use UTF-16LE for non-ASCII font aliases.
Compatibility-hosted recipes use the same prefix lock, cancellation flag, activity events, and
installed log. Their review plans are destructive by default so recovery is recommended before
the external host starts.

## Recovery

Restore points prefer copy-on-write reflinks. Other filesystems use `tar.zst`. Records live
in SQLite while payloads live under the XDG state directory. Prefix deletion uses the
desktop Trash through `gio`; there is no permanent-delete command in the normal UI.
Restore reads reject symlinks and payloads outside Bettertricks' canonical backup root.
Corrupt extraction and copy failures remove staging data. Final publication uses Linux
`renameat2(RENAME_NOREPLACE)`, so a path that appears during restoration is never overwritten.

## Catalog updates

The bundled catalog is the offline trust baseline. An update channel is enabled only when
both `BETTERTRICKS_CATALOG_INDEX` and `BETTERTRICKS_CATALOG_PUBLIC_KEY` are configured.

Each release descriptor signs its version, upstream tag, HTTPS URL, SHA-256, and recipe
count with Ed25519. The updater then:

1. verifies the descriptor before download;
2. caps the bundle size and checks its SHA-256;
3. rejects archive traversal, links, and special files;
4. parses every recipe and checks the signed recipe count in a staging directory;
5. renames the validated directory into versioned storage; and
6. updates the active SQLite record transactionally.

Older catalogs remain available for rollback. The release tool creates deterministic
`tar.zst` bundles so signatures can be reproduced.

## Compatibility host and legacy `.verb` files

Audited native catalog operations never wrap Winetricks. Tracked catalog recipes can use a
named-verb compatibility path only through the exact catalog-matching host. Bettertricks can
install the supported host into its XDG data directory from a pinned HTTPS URL and verifies its
size, executable mode, SHA-256, and reported baseline before use; this behavior is
shown in both catalog detail and operation review and never changes their maturity to native.

Custom `.verb` support is a separate, higher-risk compatibility boundary because those files
are arbitrary shell programs. Bettertricks checks
the extension, canonical regular-file status, size, single matching `w_metadata` declaration,
and filename; shows an unavoidable trust warning; creates a restore point; then passes the
canonical path as one argument to the verified managed host or an optional system Winetricks host. No shell interpolation
is performed by Bettertricks. CLI execution additionally requires `--allow-legacy-verb`.

## Persistent data

The application follows XDG directories and stores only local data:

- configuration: preferences;
- data: installed catalog versions and checksum-verified compatibility hosts;
- state: SQLite, logs, and restore points;
- `$XDG_CACHE_HOME/winetricks`: the shared download cache.

SQLite uses WAL mode and stores settings, manual prefixes, operation summaries and structured
component failures, restore points, and catalog versions. Bettertricks does not collect or
transmit usage telemetry.
