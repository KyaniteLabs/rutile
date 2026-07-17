# Rutile Current-State Handoff

> **Status: Current.** Reconciled with `main` on 2026-07-17 (0.2.2). Supersedes
> the 0.2.0-era version previously at this path (that list is folded into the
> debt section below, split resolved / partial / remains).

## BLUF

> **Safety update (2026-07-16):** 0.2.1 is withdrawn from dogfood. Its macOS
> event loop continuously redrew at idle (97.7–99.2% CPU). The deterministic
> idle redraw loop is fixed in 0.2.2; the separate one-time observation of
> approximately 10 GB RSS was not reproduced; a 180-second RSS/CPU soak now
> gates native verification.

Rutile **0.2.2 (internal/preview)** is merged on `main` (`74df96c`), tagged
`v0.2.2`, and published as Forgejo **pre-release 354** — a macOS arm64 DMG +
`.app.zip`, ad-hoc signed, **unnotarized**. It is `preview_authorized`, NOT
`publication_authorized`: the 14 external release-prerequisite blockers
(`release/evidence/release-prerequisite-preflight-v1.json`, `ready:false`)
remain out of scope by design. The unsafe 0.2.1 release 353 remains warning-
flagged and must not be run unattended.

The 0.2.0 packages are **quarantined** (`release/quarantine-v1.json`) — the
2026-07-12 audit found `test-control` in the Linux binaries and builder paths in
both. They are historical evidence only; **0.2.2 supersedes them** (path-remapped
via `xtask reproducible-build`, leak-audited by the artifact inspector).

This release also lands the **Wave-3 preview-tier publication pipeline** (PR #36,
`ec51f53`): bounded zip + DMG readers, a release-authority ed25519
preview-authorization (`preview_authorized`), provenance generation, and a
`publication_authorized` flag that stays `false` until the full attestation bar
is met. It went through an adversarial triple-check (see debt §C).

## Repository and release state

- Branch: `main` (`e98f0cb`)
- Conditional source-pane redraw hardening: PR #44 (merge `e98f0cb`)
- Idle-loop fix merge: `68a16cb` (PR #41)
- Release merge: `74df96c` (squash of `release/0.2.2`, PR #42)
- Pipeline merge: `ec51f53` (PR #36)
- Release tag: `v0.2.2` → `74df96c` (annotated); build source commit `6d8f53c`
- Forgejo pre-release: id 354 (`/releases/tag/v0.2.2`, `prerelease:true`); assets
  `Rutile-0.2.2-macos-arm64.dmg` + `.app.zip` + manifests + provenance + 2 preview-authorizations
- Workspace/crate version: `0.2.2`
- Rust toolchain: `1.88.0`, edition 2024
- **Release-authority key**: public pinned at `release/keys/release-authority-v1.pub.hex`
  (`8a178c0c…`); **secret is operator-owned, off-repo (0600), never committed.**

## Product state

Implemented across the native shells (macOS Iced/wry/tiny-skia, Linux GTK3/wry):
native source editing + system-webview preview; incremental edits, IME,
undo/redo, **atomic save with permission preservation**, dirty-close handling,
**external-file conflict detection on both shells**; revisioned two-way
source/preview scrolling; formatting commands + smart list continuation;
find/replace, counts, reading-time, **content-preserving smart-paste**,
**serialized autosave/recovery with corrupt-candidate reporting**, session
restore; **navigation-hardened self-contained HTML export**; bounded rendering,
protocol, navigation, and link-safety contracts. **macOS NSAccessibility bridge**
shipped (CY-A11Y-001): window/editor-text/toolbar/notice exposed to the NSAccessibility/AX tree.

## Verification and artifacts

- **0.2.2 preview e2e** (production path): `reproducible-build` → signed
  `package local macos` → `artifact inspect --mode package` =
  `accepted + preview_authorized:true, publication_authorized:false`, zero
  findings on both DMG and `.app.zip`.
- **Packaged soak:** 180 seconds idle, 140.0 MiB baseline/peak RSS and 0.0% final
  CPU. The same gate rejects shipped 0.2.1 at 97.7% final idle CPU.
- **Native QA:** 50-cycle lifecycle smoke passes; `hdiutil verify` reports VALID;
  `codesign --verify --deep --strict` passes.
- **W0-C bar** green for 0.2.2: `cargo fmt --check` / `cargo clippy --workspace
  --all-targets --locked -- -D warnings` / `cargo test -p xtask --all-targets`.
- Forgejo CI workflows present: `.forgejo/workflows/{verify,release}.yml`.
- 0.2.0 receipts remain at `docs/handoff/local-beta-0.2.0.md` + `docs/evidence/local-beta-0.2.0/`
  (historical; their packages are quarantined).

## Known debt

### A. Product-behavior debt — re-verified 2026-07-17 against 0.2.2 code

**Resolved by the remediation** (each backed by file:line evidence + tests):
- ✅ macOS save semantics (untitled Cmd-S, save-failure exit, swallowed Cmd
  shortcuts, external-change overwrite) — all four fixed.
- ✅ Atomic-replace permission preservation (`files.rs` copies existing mode;
  `MetadataCopyFailed` variant; 0600 tests). *Minor: xattr/gid not copied.*
- ✅ Autosave record/prune serialization (fs2 lock) + corrupt-candidate / prune /
  session-persistence failure surfacing.
- ✅ Export hardening (allowlist inspector rejects `<base>`/`<form>`/`<meta
  http-equiv>`; CSP) + content-preserving smart-paste (allowlist HTML→Markdown,
  bounded, raw-text fallback).
- ✅ Compile-contract fixture hermeticity (`crates/feathermark-app/tests/compile_contracts.rs`)
  via injected vendored `pulldown-cmark` `[patch.crates-io]`.

**Partial**:
- ◐ **macOS recovery/close prompt**: edit-during-recovery is blocked (MAC-004);
  the prompt is now an AXStaticText node. **Remains**: no accessible action-button
  dialog on macOS (decisions are Cmd+Y/Esc shortcut-only). GTK is fully modal.
- ◐ **Linux transient title errors**: critical failures now surface as durable
  `SurfaceNotice`; interactive Ctrl+S now calls `report_save_failure`.
  **Remains**: ~24 other product-facing error sites still route to the transient
  window title. *(Test-control title receipts are not product debt.)*
- ◐ **Document-open/association**: Linux CLI open + macOS `CFBundleDocumentTypes`/
  `UTExportedTypeDeclarations` + Open/Save UI + drag/drop + Linux desktop/mime
  all wired. **Remains**: the macOS `application:openURLs:` AppKit delegate for
  second-launch Finder opens is deferred (winit owns the delegate; see
  `open_events.rs:1-6`).

**Remains**:
- ⬜ **gid/xattr preservation on atomic save** (`files.rs` copies existing mode, not gid or extended attributes).

### B. Accessibility — validation debt + residual gaps (Wave 4)
- **Resolved**: ED-A11Y-0 (attestation schema exists).
- Interactive VoiceOver / AT-SPI / keyboard-only validation (ED-A11Y-1/2/3) +
  zero-trap/zero-unlabeled sweep (ED-A11Y-4) — **evidence-debt**, need real platforms.
- Codeable residual a11y gaps (open, not contingent on a full AccessKit
  migration): find/replace pseudo-fields, pseudo-dialog focus traps,
  per-character editor navigation, ARIA live-region auto-announce.

### C. Publication-pipeline review findings (2026-07-16 triple-check, PR #36/#37)
**Resolved**:
- ✅ `source_tree_clean` and `test-control` rejection now enforced.
- ✅ Sibling evidence pattern scan (`scan_bytes` over `.provenance`/`.preview-auth`/`.manifest`).
- ✅ `RUSTUP_HOME`/sysroot remapping.

**Remains**:
- **[MED]** Codesign-aware binding chain: bind `build_input_sha256` to provenance
  `candidate_sha256`, and the packaged-executable hash to the inspected
  executable — never a direct pre-sign/post-sign comparison.
- **[MED]** `generate_with_reproducible_env` injects the reproducibility env it
  then measures (self-fulfilling for an ad-hoc build; true in practice since the
  operator runs `reproducible-build`, but unverifiable by the tool).

### D. External / by-design out of scope (the full-public-release bar)
The **14 release-prerequisite blockers** (`ready:false`): trusted verifier;
Forgejo runner lock; Apple Developer ID sign + notarization; Linux GPG; macOS
arm64 + Linux x86_64_x11 runner capabilities + clean-install hosts; protected-tag
owner-approval process; formal release-authority gate; artifact-retention policy;
independent-builder reproduction. Plus deferred **platform coverage**: Linux
artifacts, Intel macOS, native Wayland, RPM install verification. The built-but-
not-wired **runner-verification subsystem** (`xtask/src/runner/*` +
`CaptureVerifyMatrix`) needs the provisioned runners to complete the closed
release-candidate pipeline.

## Documentation authority

1. `README.md` — entry point, features, build/run commands, limits, doc map.
2. `docs/architecture.md` — implemented ownership and runtime boundaries.
3. This file — live operational state and open debt.
4. `docs/handoff/local-beta-0.2.0.md` + `docs/evidence/local-beta-0.2.0/` — immutable 0.2.0 receipts (quarantined packages).
5. `docs/wayfinder/MAP.md` — the 0.2.1 release Wayfinder map.
6. `DESIGN-SYSTEM.md` — current visual rules.

Everything in `docs/plan/`, `docs/research/`, versioned old handoffs, and dated
`docs/superpowers/` files is provenance unless its header says **Current**.

## Safe next step

Genuine remaining **in-repo** work: the macOS recovery action-button dialog (§A
partial); the ~24 Linux transient-title error sites (§A partial); gid/xattr
preservation on atomic save (§A remains); the §B codeable residual a11y gaps
(find/replace, focus, per-character navigation, announcements); and the §C
codesign-aware binding chain + reproducibility-assertion boundary.

**External/blocked boundaries** (not codeable in-repo): the 14
release-prerequisite blockers (§D) need provisioning + external authority;
ED-A11Y-1..4 real AT receipts need real platforms (VoiceOver/AT-SPI/keyboard);
and the macOS second-launch `openURLs:` delegate is blocked on winit owning the
AppKit delegate. `publication_authorized` stays `false`; do not cut/tag a
*public* release without all prerequisite evidence.
