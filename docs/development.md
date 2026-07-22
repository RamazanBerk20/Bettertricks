# Development

## Devcontainer

The repository includes an Ubuntu 24.04 devcontainer with Rust stable, Node 24, pnpm 11,
Wine 32/64-bit, WebKitGTK, Tauri build libraries, and Winetricks helper tools.
It installs the exact Winetricks 20260125 compatibility host with a pinned SHA-256, so tracked
recipes can exercise their upstream-host path without weakening native maturity accounting.
Container-only `node_modules` and Cargo `target` volumes keep its Node and Rust versions from
invalidating host build caches.

In VS Code, run **Dev Containers: Reopen in Container**. From a shell:

```sh
devcontainer up --workspace-folder .
devcontainer exec --workspace-folder . pnpm check
devcontainer exec --workspace-folder . pnpm test
```

`pnpm dev:web` exposes the mock-backed browser preview on port 1420. A real Tauri window
needs access to a display server. For headless host checks, use
`xvfb-run -a pnpm --filter @bettertricks/desktop tauri dev`.

## Linux host dependencies

Install Rust stable, Node 24, pnpm 11, Wine, WebKitGTK 4.1 development headers, OpenSSL
headers, `libayatana-appindicator`, `librsvg`, `patchelf`, `cabextract`, 7-Zip, unzip, tar
with zstd support, and `gio`. Exact package names differ by distribution; the devcontainer
Dockerfile is the authoritative Ubuntu list.

Then run:

```sh
corepack enable
corepack prepare pnpm@11.3.0 --activate
pnpm install
pnpm check
pnpm test
```

## Running

```sh
pnpm dev                       # real Tauri app
pnpm dev:web                   # browser preview with mock backend
cargo run -p bettertricks -- list-all
cargo run -p bettertricks -- --install-compatibility-host
BETTERTRICKS_CATALOG=./catalog cargo run -p bettertricks -- --dry-run win10
BETTERTRICKS_CATALOG=./catalog cargo run -p bettertricks -- \
  --manual-file 'recipe.file=/path/to/download.exe'
```

The browser mock is intentionally realistic but never starts Wine or changes local prefixes.
Use the Tauri app or CLI for integration testing.

## Localization workflow

Desktop UI messages use their English source text as stable keys through `useI18n`. The translated
JSON dictionaries live in `apps/desktop/src/lib/locales`; every supported language must contain
the same keys and preserve every `{placeholder}` exactly. The localization test extracts messages
from the TypeScript source and fails on missing, stale, empty, or placeholder-incompatible entries.

Use locale-aware formatters from `useI18n` for numbers, byte sizes, relative dates, and times.
Avoid formatting those values directly with an implicit system locale. Arabic is the sole RTL
locale today; prefer CSS logical properties so new controls mirror without language-specific
layout code. Recipe titles/descriptions from signed catalogs, filesystem paths, product names,
and backend diagnostic lines deliberately remain source data rather than UI translations.

## Catalog workflow

`pnpm catalog:generate` first writes audited native settings and font translations, then
reads the installed Winetricks `list-all`, `list-download`, and `list-manual-download` output
and regenerates parity metadata. Generation never
turns an upstream entry into an executable recipe unless an audited native definition exists.
Generation is pinned to Winetricks 20260125, removes machine-specific home paths, and produces
byte-stable output. Validation checks manifest totals, category counts, dependency integrity,
provenance, native-action types, and automated-download HTTPS/SHA-256 requirements.

After editing recipes:

```sh
pnpm catalog:generate
pnpm catalog:validate
cargo test -p bettertricks-core
pnpm smoke:wine
```

A native recipe must include upstream provenance, exact SHA-256 values for every automated or
manual download, architecture constraints, and deterministic detection or verification where the
verb permits it. Never lower `maturity` to `native` merely to improve the parity count.

Metadata-only recipes are passed to a managed or system Winetricks process only after
`winetricks --version` contains the catalog's exact upstream tag as a whitespace-delimited token.
The Settings and catalog-detail screens can install the checksum-pinned managed host for the
bundled baseline; the CLI exposes the same flow through `--install-compatibility-host`. The
identifier comes from the validated catalog and is
passed as one process argument, never through a shell. A missing or mismatched host fails before
prefix initialization. The hosted path is a compatibility guarantee, not permission to label the
recipe native.

## Signing a catalog

The release tool accepts a 32-byte Ed25519 private seed as raw bytes or 64 hexadecimal
characters. Keep it outside the repository and CI logs.

```sh
cargo run -p bettertricks-catalog-tool -- bundle \
  --catalog catalog \
  --output dist/catalog-winetricks-20260125.tar.zst \
  --url https://updates.example.org/catalog-winetricks-20260125.tar.zst \
  --signing-key /secure/path/catalog-ed25519.key

cargo run -p bettertricks-catalog-tool -- index \
  --release dist/catalog-winetricks-20260125.tar.release.json \
  --output dist/index.json
```

Configure a development client with the HTTPS index URL and the 32-byte public key encoded
as 64 hexadecimal characters:

```sh
export BETTERTRICKS_CATALOG_INDEX=https://updates.example.org/index.json
export BETTERTRICKS_CATALOG_PUBLIC_KEY=0123456789abcdef...
pnpm dev
```

## Verification before a change is ready

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --filter @bettertricks/desktop check
pnpm --filter @bettertricks/desktop test
pnpm --filter @bettertricks/desktop build
pnpm smoke:packages
pnpm smoke:packages:clean
pnpm smoke:catalog-update
pnpm smoke:compatibility-host
```

The wildcard Node test command includes accessibility color-token and reduced-motion regressions.
Component tests cover the skip link, semantic navigation and selection state, keyboard command
palette, prompt focus, progress values, and live error handling.

For recipe-engine changes, additionally exercise a disposable prefix. Never use a personal
or launcher-managed prefix as an integration fixture.

`pnpm smoke:wine` always covers settings and DLL registry behavior. It also accepts
pre-cached, checksum-verified archives for the larger font paths without making every CI run
download them:

```sh
BETTERTRICKS_SMOKE_FONT_ARCHIVE=/path/to/andale32.exe pnpm smoke:wine
BETTERTRICKS_SMOKE_LUCIDA_ARCHIVE=/path/to/eurofixi.exe pnpm smoke:wine
BETTERTRICKS_SMOKE_POWERPOINT_ARCHIVE=/path/to/PowerPointViewer.exe pnpm smoke:wine
BETTERTRICKS_SMOKE_SOURCE_HAN_ARCHIVE=/path/to/SourceHanSans.ttc.zip pnpm smoke:wine
```

`pnpm smoke:proton` downloads the pinned GE-Proton release once into
`$BETTERTRICKS_PROTON_CACHE` (or the XDG cache), verifies its published SHA-512, and repeats the
same disposable-prefix suite with Proton's Wine tools first on `PATH`.
