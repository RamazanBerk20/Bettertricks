# Winetricks parity ledger

The public `1.0` gate is behavioral parity with the selected stable Winetricks release, not
merely matching names in a menu.

## Current baseline

| Baseline | Catalog entries | Native | Exact upstream host | Broken upstream |
| --- | ---: | ---: | ---: | ---: |
| Winetricks 20260125 | 550 | 159 | 390 | 1 |

The native set includes all 118 settings entries: Windows version modes, font smoothing,
virtual desktops, Wine/X11 window behavior, graphics backends, input behavior, prefix themes,
MIME integration, sound drivers, Direct3D and shader controls, video-memory reporting,
debugger/crash behavior, filesystem isolation, diagnostics, runtime cleanup, user-provided
MIDI/PATH values, and DLL overrides. It also includes 40 of the 42 font entries: Microsoft
core and PowerPoint Viewer families, open CJK and Latin families, Unicode-safe font aliases,
and their complete CJK/core/PPT aggregates. These use Winetricks-compatible shared cache
paths, pinned HTTPS sources, exact SHA-256 values, typed extraction and registration, and
deterministic file verification. `micross` remains blocked on an untested 316 MiB XP SP3
source, so the upstream `allfonts` aggregate also remains blocked rather than overstating parity.
The deprecated `directx9` aggregate is also native: matching upstream, it only emits a warning
and directs users to individual DirectX components without changing the prefix.

The generated remainder contains upstream title, category, publisher/year where available,
media requirements, flags, and provenance. Its 390 non-broken entries are executable through
the compatibility host only when the installed Winetricks version exactly matches 20260125.
This is behavioral delegation, not a native-port claim. The operation engine still serializes
access to the prefix, defaults to a restore point, propagates cancellation, records activity,
and validates the catalog identity before passing a catalog-owned verb identifier as a single
process argument. The remaining entry is explicitly marked broken upstream and stays disabled;
diagnostic verbs are not misclassified.

## Maturity states

- `native`: typed steps exist and pass schema/release validation; execution is enabled.
- `metadata_only`: tracked parity record; runs through the exact matching Winetricks host with
  an explicit review warning, or fails closed when that host is absent or mismatched.
- `broken_upstream`: retained for compatibility and diagnostics; execution is disabled.

## Definition of a port

A verb counts as native only when all applicable items are complete:

1. dependencies and conflicts match upstream behavior;
2. architecture and Wine constraints are explicit;
3. every download has maintained URLs, exact SHA-256, and correct manual-download behavior;
4. extraction, registry, DLL override, process, and cleanup steps use typed primitives;
5. unattended behavior and prompts are represented;
6. installed detection and verification are deterministic where possible;
7. a disposable-prefix integration fixture covers success and expected failure; and
8. upstream tag and verb provenance are preserved.

## Release gates

The project must not call a release `1.0` until:

- every non-broken entry has either an audited native implementation or a version-matched
  upstream-host execution path, with the mode visible before execution;
- catalog and disposable-prefix integration tests run on supported Wine and Proton variants;
- signed catalog publication and rollback are exercised in CI;
- AppImage, Debian, and RPM artifacts pass clean-system smoke tests;
- accessibility, keyboard-only operation, light/dark themes, and reduced motion pass the
  documented audit in [accessibility.md](accessibility.md);
- destructive and recovery paths pass fault-injection tests; and
- compatibility output is compared with the pinned Winetricks baseline.

Bettertricks 1.0 enforces these gates in CI. The concrete runtime/package matrix, commands, and
artifact policy are recorded in [release.md](release.md); compatibility-hosted entries remain
separate from native maturity in every count and UI surface.

Run `pnpm catalog:validate` for the authoritative live counts. The application derives its
category and maturity counts from parsed recipes rather than trusting the generated manifest.
Catalog generation also reads Winetricks' dedicated automated- and manual-download lists;
`list-all` alone does not expose manual-download media.
