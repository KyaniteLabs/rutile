# Rutile Documentation Current-State Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every documentation surface clearly reflect either the live Rutile 0.2.0 codebase or an explicitly labeled historical snapshot.

**Architecture:** Treat `README.md`, `docs/architecture.md`, and `docs/handoff/current-state.md` as the canonical current-state set. Preserve evidence, research, specifications, and completed plans as immutable historical context, adding lifecycle banners where their old instructions could be mistaken for current work.

**Tech Stack:** Rust 1.88.0 workspace, native macOS Iced/Wry shell, native Linux GTK3/GtkSourceView/Wry shell, Cargo, cargo-fuzz, xtask packaging/evidence tooling.

## Global Constraints

- Product-facing name is `Rutile`. (Historical constraint: at the time of this 0.2.0 documentation pass, migration-sensitive crate, binary, package, protocol, and path identifiers were held at `rutile`; PR #57 later unified them to Rutile.)
- Current release is `0.2.0` at tag `v0.2.0`; historical release evidence remains unchanged.
- No product-code behavior changes are authorized by this documentation pass.
- Existing untracked `.omx/` state must remain untouched.

---

### Task 1: Canonical current-state documentation

**Files:**
- Modify: `README.md`
- Create: `docs/architecture.md`
- Modify: `docs/handoff/current-state.md`

**Interfaces:**
- Consumes: workspace manifests, shell implementations, core public API, release handoff, and evidence bundle.
- Produces: one user/developer entry point, one architecture reference, and one resumable repository-state handoff.

- [x] **Step 1:** Replace 0.1.0 status claims with the live 0.2.0 release state.
- [x] **Step 2:** Document implemented features, platform boundaries, exact build commands, architecture, security invariants, and known debt.
- [x] **Step 3:** Record the current branch/commit/tag relationship and distinguish tracked source from local artifacts.

### Task 2: Documentation lifecycle labels and corrections

**Files:**
- Modify: `DESIGN-SYSTEM.md`
- Modify: `docs/handoff/cheap-model-prompt.md`
- Modify: `docs/handoff/continuation-plan.md`
- Modify: `docs/handoff/local-beta-0.1.0.md`
- Modify: `docs/handoff/local-beta-0.2.0.md`
- Modify: `docs/evidence/local-beta-0.1.0/evidence-debt.md`
- Modify: `docs/evidence/local-beta-0.1.0/debt-closure/closure-summary.md`
- Modify: `docs/evidence/local-beta-0.1.0/debt-closure/macos-runtime-audit.txt`
- Modify: `docs/evidence/local-beta-0.1.1/verification-summary.md`
- Modify: `docs/evidence/local-beta-0.2.0/verification-summary.md`
- Modify: `docs/plan/build-plan.md`
- Modify: `docs/plan/ralplan-dr.md`
- Modify: `docs/research/build-vs-adopt.md`
- Modify: `docs/research/landscape.md`
- Modify: `docs/superpowers/specs/2026-07-10-rutile-next-SPEC.md`
- Modify: `docs/superpowers/specs/2026-07-10-rutile-working-brand-design.md`
- Modify: `docs/superpowers/plans/2026-07-10-rutile-end-to-end-completion.md`
- Modify: `docs/superpowers/plans/2026-07-10-rutile-0.2-plan.md`
- Modify: `docs/superpowers/plans/2026-07-10-rutile-working-brand.md`
- Modify: `docs/runner-provisioning.md`
- Modify: `fuzz/README.md`
- Modify: `crates/rutile-app/src/app.rs` (Rustdoc-only correction)

**Interfaces:**
- Consumes: canonical current-state documentation from Task 1.
- Produces: unambiguous current, historical, superseded, and unprovisioned labels without rewriting historical evidence.

- [x] **Step 1:** Mark completed plans, old handoffs, and research snapshots as historical.
- [x] **Step 2:** Correct the design-system implementation status and 0.2.0 tag claim.
- [x] **Step 3:** Document all four live fuzz targets and the current runner-provisioning boundary.
- [x] **Step 4:** Remove private machine/path metadata and repair strict-Rustdoc warnings without changing behavioral code.

### Task 3: Verification and review

**Files:**
- Verify: all modified Markdown files and repository source/configuration.

**Interfaces:**
- Consumes: Tasks 1-2 documentation.
- Produces: link-clean, formatting-clean, test-backed documentation with a reviewed diff.

- [x] **Step 1:** Run a local Markdown-link and stale-current-claim audit.
- [x] **Step 2:** Run `cargo fmt --check`, `cargo test --workspace --all-targets --locked`, and strict workspace clippy.
- [x] **Step 3:** Review `git diff --check`, the full documentation diff, and repository status.
- [x] **Step 4:** Commit only the documentation files from this plan.
