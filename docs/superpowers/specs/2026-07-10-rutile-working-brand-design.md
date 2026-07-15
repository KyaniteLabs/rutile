# Rutile Working Brand Design

> **Status: Implemented historical design decision.** Rutile remains the user-facing name in 0.2.0 while technical `feathermark` identifiers remain intentionally stable.

**Status:** Approved by the user on 2026-07-10 with “just do rutile for now” and “go.”

## Goal

Present the completed local-first Markdown writing studio to users as **Rutile**, endorsed as **by Kyanite**, without destabilizing the verified FeatherMark 0.1.0 implementation.

## Scope

This is a reversible working-brand pass. It changes current user-facing product copy only:

- native window titles and status/error titles;
- the starter document and editor accessibility label;
- macOS bundle display name and local artifact display names;
- Linux archive display name and Debian/RPM human-readable summaries;
- the README product heading and description.

The canonical endorsement line is **“A local-first writing studio by Kyanite.”**

## Stability Boundary

The following remain `feathermark` in this pass:

- Rust workspace, crate, module, and binary identifiers;
- custom URL schemes, JavaScript bridge names, schemas, environment variables, and test-control protocols;
- macOS bundle identifier and Linux application/package IDs;
- executable paths, privileged-runner names, service names, filesystem paths, and repository/worktree names;
- historical plans, evidence, handoffs, artifact manifests, and already-built 0.1.0 packages.

These identifiers are migration-sensitive or are historical facts. A full technical rename requires a separate release migration after the public name is final.

## Implementation

Add one small application brand module that owns the product name, endorsement line, starter document, accessibility label, and title formatting. Both native shells consume this module instead of repeating product-facing literals. Packaging retains its existing technical IDs while emitting Rutile display names and descriptions.

No icon, color-system, URL, or legal/trademark change is included because Rutile is explicitly a working name “for now” and the repository has no current icon asset pipeline.

## Verification

- Brand contract unit tests assert exact approved copy and title formatting.
- macOS and Linux product tests continue to pass.
- Packaging tests assert Rutile display names while asserting that technical `feathermark` identifiers remain unchanged.
- A final reference audit distinguishes permitted internal/historical `FeatherMark` references from unintended product-facing references.
- `cargo test --workspace --all-targets --locked` and platform-appropriate build checks must pass before completion is claimed.
