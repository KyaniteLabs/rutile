# Rutile architecture

## Parts and data flow

```text
native source editor -> revisioned edit transactions -> shared AppState reducer
 -> effects executed by platform shell -> document/file/autosave/session core
document snapshot -> render scheduler -> sanitized semantic HTML -> private preview scheme
preview events -> bounded typed protocol/revision checks -> scroll/link/app state
export request -> trusted renderer + self-contained template -> export inspector -> HTML
```

`rutile-types` is the leaf for revisions/interaction/safe-link values; `rutile-protocol` defines preview frames; `rutile-core` owns documents, files, rendering, formatting/search, export and recovery; `rutile-app` owns reducer/session/scheduler/preview/native adapters. `xtask` owns fixtures, packaging/runners and evidence—not the product UI. [docs/architecture.md](docs/architecture.md) gives the larger map.

[Document](crates/rutile-core/src/document.rs) uses a Rope, revisioned transactions and bounded history. Shared tab/session logic parks inactive ropes while keeping per-slot metadata; platform shells execute returned effects and refresh adapters after swaps. [main.rs](crates/rutile-app/src/main.rs) selects target-specific shells; the default headless build is not the desktop product.

[PreviewHost](crates/rutile-app/src/preview_host.rs) serves fixed internal assets and a staged revisioned page through `rutile://preview/v1/`, not a local HTTP server. Its restrictive CSP denies network/media/frames/forms and navigation; typed IPC rejects invalid/stale frames. [Export](crates/rutile-core/src/export.rs) reuses the trusted renderer but emits a distinct inline-CSS/no-JavaScript template, rewriting validated link carriers to export links and inspecting the final page.

## Risky paths and failure signatures

- **Persistence/recovery:** atomic replacement, external-change decisions, snapshot retention and journal/session handling protect different paths. A saved active document does not prove parked tabs restore correctly. Recovery/GC can delete/compact state; never launch against a real writing profile just for a docs probe.
- **Untrusted Markdown/IPC:** raw HTML must not bypass typed rendering or safe-link/CSP boundaries. Source-byte/revision mismatch can cause incorrect scroll, stale preview or wrong-document effects even when HTML looks fine. Frozen renderer/security/safe-link files remain untouched.
- **Adapter divergence:** macOS tabs/palette behavior is not Linux chrome parity. Linux discards the positional path; native focus/IME/shortcut and webview behavior require platform-specific operator recordings. Portable tests cannot attest them.
- **Export boundary:** self-containment validation does not imply image support, perfect print/accessibility fidelity or safety of every future renderer change. Current images become alt text; verify actual recipient HTML separately.
- **Release/evidence:** source version, public tag, draft preview artifacts, signed publication authorization and production readiness are separate. Dated handoffs contain inconsistent visibility statements; unsigned quality probes and historical receipts must not become current release authorization.

## Bounded docs verification

Read purpose, repo rules/current handoff/architecture, workspace and actual entrypoint/document/preview/export implementation. `cargo test --locked -p rutile-types --lib` passed two tests using isolated cache/target paths. Native Linux prerequisites were present, but no native app launch, lifecycle gate, real-file edit/recovery, packaging or release operation ran. No user-side native acceptance is claimed. Docs rollback is reverting the commit; canonical Forgejo reconciliation remains separate.
