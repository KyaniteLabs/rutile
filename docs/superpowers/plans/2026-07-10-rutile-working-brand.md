# Rutile Working Brand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebrand the completed native beta’s current user-facing surfaces from FeatherMark to Rutile while preserving migration-sensitive technical identifiers.

**Architecture:** Centralize application-facing copy in `crates/feathermark-app/src/brand.rs`, consume it from both platform shells, and update only display metadata in the packaging layer. Keep crate, binary, protocol, schema, app ID, package ID, runner, path, and historical evidence names unchanged.

**Tech Stack:** Rust 1.88, edition 2024, Iced/Wry on macOS, GTK3/WebKitGTK on Linux, deterministic Rust xtask packaging.

## Global Constraints

- Product name: `Rutile`.
- Endorsement: `A local-first writing studio by Kyanite.`
- This is a reversible working-brand pass; do not rename technical identifiers.
- Do not rewrite historical evidence, handoffs, or existing artifact receipts.
- Add no new dependencies and no icon or color asset pipeline.

---

### Task 1: Application Brand Contract

**Files:**
- Create: `crates/feathermark-app/src/brand.rs`
- Modify: `crates/feathermark-app/src/lib.rs`
- Test: `crates/feathermark-app/src/brand.rs`

**Interfaces:**
- Produces: `PRODUCT_NAME`, `ENDORSEMENT`, `STARTER_DOCUMENT`, `SOURCE_EDITOR_LABEL`, and `status_title(&str) -> String`.
- Consumes: no platform dependencies.

- [ ] Add unit tests first for the exact approved strings and `Rutile — <status>` formatting.
- [ ] Run `cargo test -p feathermark-app --lib --locked` and verify the new tests fail because the brand contract is absent.
- [ ] Implement the constants and formatter, then export `pub mod brand` from `lib.rs`.
- [ ] Re-run `cargo test -p feathermark-app --lib --locked` and verify it passes.

### Task 2: Native Shell Display Copy

**Files:**
- Modify: `crates/feathermark-app/src/main.rs`
- Modify: `crates/feathermark-app/src/platform/macos/native.rs`
- Modify: `crates/feathermark-app/src/platform/linux_gtk.rs`
- Test: existing application unit and product tests.

**Interfaces:**
- Consumes: the Task 1 brand constants and `status_title` function.
- Produces: Rutile startup copy, titles, error/status titles, and accessibility label on both platforms.

- [ ] Add or extend tests around brand-derived starter copy and status titles before shell edits.
- [ ] Replace only user-visible `FeatherMark` literals with the brand contract; retain internal smoke filenames, environment variables, protocols, and bridge names.
- [ ] Run `cargo test -p feathermark-app --all-targets --locked` and verify it passes.

### Task 3: Package Display Metadata

**Files:**
- Modify: `xtask/src/local_package.rs`
- Modify: `xtask/src/local_package_cli.rs`
- Modify: `xtask/tests/local_package.rs`

**Interfaces:**
- Produces: `Rutile.app`, Rutile-named zip/DMG/archive artifacts, and Rutile human-readable package descriptions.
- Preserves: `feathermark` binary, bundle ID, package IDs, schemas, install paths, and RPM spec filename.

- [ ] Update packaging assertions first to expect Rutile display names and unchanged technical IDs.
- [ ] Run `cargo test -p xtask --test local_package --locked` and verify failures identify the old display names.
- [ ] Change macOS bundle/artifact display names, Linux archive display names, and Debian/RPM descriptions; keep executable/package IDs unchanged.
- [ ] Re-run `cargo test -p xtask --test local_package --locked` and verify it passes.

### Task 4: Product Documentation and Integrated Verification

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-07-10-rutile-working-brand-design.md` only if verification reveals a contradiction.

**Interfaces:**
- Produces: an accurate current-product README and a reference-audit receipt.

- [ ] Replace the README heading with `Rutile` and add the endorsement line plus a short note that internal identifiers remain `feathermark` during the working-name phase.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test --workspace --all-targets --locked`.
- [ ] Run a focused `rg` audit over current application and packaging sources; classify remaining FeatherMark references as technical or historical.
- [ ] Run the macOS feature build check: `cargo check -p feathermark-app --no-default-features --features macos-shell --locked`.
- [ ] Record the changed-file list and test receipts in the final handoff; do not push.

