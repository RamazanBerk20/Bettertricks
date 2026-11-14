# Release verification

Bettertricks 1.2.0 is built from the same catalog, core, CLI, and desktop sources exercised by the
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
| AppImage | clean Ubuntu 24.04 container | self-extraction, bundled CLI/catalog query, desktop/AppRun presence, no bundled libwayland |
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

### AppImage build and GUI check

`pnpm build` runs `scripts/repack-appimage.sh` after the Tauri bundler (CI runs it as its own
step). The script removes `libwayland-*.so*` from `target/release/bundle/appimage/Bettertricks.AppDir`
and repacks the AppImage with Tauri's cached `linuxdeploy-plugin-appimage`. The AppImage is built
on Ubuntu 24.04, so its libwayland-client is 1.22; on a host with a newer Mesa (for example
Arch with wayland 1.26) it shadows the host copy, the host's `libEGL_mesa.so.0` fails to load
(`undefined symbol: wl_fixes_interface`), and WebKitGTK aborts with
`Could not create surfaceless EGL display: EGL_BAD_ALLOC`. libwayland is backwards compatible, so
the host copy serves the bundled GTK. `pnpm smoke:packages` fails if libwayland is bundled again.

No CI job starts the AppImage GUI: the CI runner's Mesa matches the bundle, so it could not
reproduce the failure. Before each release, start the AppImage on a current rolling distribution
(Arch, Fedora) under Wayland and confirm the window shows the sidebar and the top bar.

## Release artifacts

The Tauri build produces these verified x86-64 release files:

| Format | File | SHA-256 |
| --- | --- | --- |
| AppImage | `Bettertricks_1.2_amd64.AppImage` | `a2326e42dcc0ead1185f124df6c216d65b1eab2d38a6b703ec6cf659cae9fd8d` |
| Debian | `Bettertricks_1.2_amd64.deb` | `5eda3f27bf96d054cae265d6c4aa59296fdc87b889377142674e887264ad71a6` |
| RPM | `Bettertricks-1.2-1.x86_64.rpm` | `7773ed4c94b185a906b6ba18eaf6023aa636b5b7cb2a79d5ee404ef054a9a2c7` |

`target/release/publish/SHA256SUMS` records the same digests. CI generates it only after every
release gate passes against the exact 1.2.0 artifacts and uploads it alongside the packages.
Verify all three from the repository root with:

```sh
(cd target/release/publish && sha256sum --check SHA256SUMS)
```
