# Release verification

Bettertricks 1.0 is built from the same catalog, core, CLI, and desktop sources exercised by the
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
| AppImage | `Bettertricks_1.0_amd64.AppImage` | `f9cdd462561648ce185db6ea7b75de8e636525c706ba3bf8efd20d83f6677673` |
| Debian | `Bettertricks_1.0_amd64.deb` | `80d8ba1f08e9fac32950c58f11c237f6add0f6924355aaf90f88dca8520aa0d6` |
| RPM | `Bettertricks-1.0-1.x86_64.rpm` | `ad819063be52bc3acae40a64b7a58a8b02323e76b7aed562d4e0f4bc6e5ba14e` |

`target/release/publish/SHA256SUMS` records the same digests. CI generates it only after every
release gate passes against the exact 1.0 artifacts and uploads it alongside the packages.
Verify all three from the repository root with:

```sh
(cd target/release/publish && sha256sum --check SHA256SUMS)
```
