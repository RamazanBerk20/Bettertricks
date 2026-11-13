# Release verification

Bettertricks 1.1 is built from the same catalog, core, CLI, and desktop sources exercised by the
release checks below. The selected compatibility baseline is Winetricks 20260125.

## Scope

| Catalog state | Count | Execution behavior |
| --- | ---: | --- |
| Audited native | 159 | Typed Rust recipe engine |
| Exact upstream host | 390 | Checksum-verified Winetricks 20260125 host |
| Broken upstream | 1 | Visible but blocked |
| Total | 550 | Compared with the pinned upstream source |

Compatibility-hosted entries are deliberately not reported as native. The managed host is the
exact upstream 20260125 script, pinned to SHA-256
`431f82fc74000e6c864409f1d8fb495d696c03928808e3e8acffc45179312a7b`.

## Release matrix

| Area | Environment | Verification |
| --- | --- | --- |
| Core/UI | Rust stable, Node 24, pnpm 11 | format, Clippy, typecheck, unit/component/regression suites |
| Baseline parity | Winetricks 20260125 | all IDs, categories, media flags, maturity, and provenance compared |
| Wine integration | Ubuntu 24.04 Wine 9.0 | disposable win64 prefix and typed native settings/DLL operations |
| Proton integration | GE-Proton11-1, Wine 11.0 staging | the same disposable-prefix suite through Proton's Wine tools |
| Signed catalogs | deterministic `tar.zst` plus Ed25519 | publish twice, compare bytes, verify, activate, and roll back |
| Managed host | isolated XDG directories | HTTPS download, size/SHA/mode/version checks, atomic publish, idempotence |
| Debian | clean Ubuntu 24.04 container | dependency install, CLI catalog query, desktop linkage, AT-SPI launch |
| RPM | clean Fedora 44 container | dependency install, CLI catalog query, desktop linkage |
| AppImage | clean Ubuntu 24.04 container | self-extraction, bundled CLI/catalog query, desktop/AppRun presence |
| Accessibility | WebKitGTK under Xvfb/D-Bus | 65-node AT-SPI tree, 19 buttons, named navigation, focus movement |
| Interaction/performance | release WebKitGTK under Xvfb | divider-owned 13-step review scrolling, fixed safeguards/actions, hide/reopen during a live run, concurrent catalog selection, bulk selection, cached discovery, and offscreen paint guards |
| Recovery | core fault-injection tests | corrupt payloads, symlink escape, failed staging, and no-overwrite publish |

## Reproducing the gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
pnpm test
pnpm catalog:validate
pnpm smoke:catalog-update
pnpm smoke:compatibility-host
xvfb-run -a pnpm smoke:wine
xvfb-run -a pnpm smoke:proton
pnpm build
pnpm smoke:packages
pnpm smoke:packages:clean
pnpm release:checksums
```

`pnpm smoke:packages:clean` intentionally installs the built Debian and RPM packages with their
declared dependencies in fresh distribution containers. It is slower than the content smoke and
catches package-manager or runtime-linkage failures that archive inspection cannot.

## Release artifacts

The Tauri build produces these verified x86-64 release files:

| Format | File | SHA-256 |
| --- | --- | --- |
| AppImage | `Bettertricks_1.1_amd64.AppImage` | `4e456a6d7b6f458e182362dac35165dc6a2a4b9a2fa43ae879ae46172a8c6f51` |
| Debian | `Bettertricks_1.1_amd64.deb` | `37db41e3fe4bf58acee5ef56d8082e27af3bd6d56dc583f681d2cd1dbfca1faf` |
| RPM | `Bettertricks-1.1-1.x86_64.rpm` | `45bc2c35210e52f669b8280d05768b46895ca48f76fefffd5d8faa3d421e4210` |

`target/release/publish/SHA256SUMS` records the same digests. CI generates it only after every
release gate passes against the exact 1.1 artifacts and uploads it alongside the packages.
Verify all three from the repository root with:

```sh
(cd target/release/publish && sha256sum --check SHA256SUMS)
```
