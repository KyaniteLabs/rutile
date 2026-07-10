# FeatherMark Side-by-Side Editor Implementation Plan

> **BEST AVAILABLE PLAN — MAX ITERATIONS, ARCHITECT R5 SOUND, CRITIC R5 ITERATE, CONSENSUS NOT COMPLETE, NO EXECUTION AUTHORIZED.**
>
> **Planning state:** RALPLAN reached its five-iteration maximum. This artifact records the best available revision after terminal mechanical cleanup of the three Critic-r5 defects; that cleanup was not re-reviewed and does not convert the final Critic verdict into approval. This artifact authorizes no application implementation, commit, push, or release. A future execution handoff requires an approving Architect review followed by an approving Critic review of the same artifact SHA-256. During any later authorized execution, Task 1 remains a second stop gate: Tasks 2-8 may not begin until Task 1 evidence and ADR 0001 receive another Architect -> Critic approval.

**Goal:** Build a super-lightweight native Rust Markdown editor for macOS and Linux with source on the left, a literal browser-rendered HTML/CSS preview on the right, revisioned offset-based two-way scroll synchronization, read-only generated-HTML inspection, and a deny-by-default preview boundary.

**Architecture outcome:** FeatherMark will own a small toolkit-neutral Rust core. Native Wayland is a required Linux target; XWayland is useful diagnostic coverage but does not satisfy Linux support. Linux therefore uses a GTK3-owned window and source adapter; after `gtk::init` on that same main thread, it configures a Wry 0.55.1 `WebViewBuilder` and consumes it with `WebViewBuilderExtUnix::build_gtk(&container)`. macOS compares iced+Wry and egui/eframe+Wry against the same production contracts. No shell is approved until Task 1 proves the complete editor/renderer/webview seam.

**Pinned baseline:** Rust 1.88 / edition 2024; Wry 0.55.1; GTK3/gtk-rs and GtkSourceView 4 on Linux; iced 0.14 and egui/eframe 0.35 for the macOS selection spike; Ropey 1.6.1; pulldown-cmark 0.13.4; notify 8.2; serde/serde_json; url; blake3; tempfile; criterion; proptest; cargo-fuzz; Forgejo Actions-compatible CI.

**Evidence anchors:** `.omx/context/feathermark-side-by-side-20260709T210654Z.md`, `docs/research/build-vs-adopt.md`, `docs/research/landscape.md`, the [Wry 0.55.1 GTK builder trait](https://docs.rs/wry/0.55.1/wry/trait.WebViewBuilderExtUnix.html), [Ubuntu 24.04 WebKitGTK package](https://packages.ubuntu.com/noble/libwebkit2gtk-4.1-dev), [Fedora supported-release policy](https://docs.fedoraproject.org/en-US/releases/), [Fedora WebKitGTK 4.1 packages](https://packages.fedoraproject.org/pkgs/webkitgtk/webkit2gtk4.1-devel/), [Fedora GtkSourceView 4 packages](https://packages.fedoraproject.org/pkgs/gtksourceview4/gtksourceview4-devel/), and [Fedora GTK3 packages](https://packages.fedoraproject.org/pkgs/gtk3/gtk3-devel/).

## Global Constraints and Decisions

- Rust native desktop only. Electron, a browser-hosted application UI, bundled Chromium, and a Tauri/JS frontend are forbidden.
- macOS 13+ must support `aarch64-apple-darwin` and `x86_64-apple-darwin` through WKWebView.
- Linux must support native GTK/Wayland and GTK/X11 on Ubuntu 24.04 and Fedora 43 through WebKitGTK 4.1. Fedora 43 is the pinned supported baseline; the build packages are `gtk3-devel`, `gtksourceview4-devel`, and `webkit2gtk4.1-devel`, and runtime packages are `gtk3`, `gtksourceview4`, and `webkit2gtk4.1`. `GDK_BACKEND=x11` under a Wayland login is XWayland evidence only and cannot pass the native-Wayland row.
- The source and literal system-browser HTML/CSS preview remain side by side. Native Markdown widgets do not satisfy preview fidelity.
- The product is one Markdown document per window, not an IDE or notes system: no LSP, terminal, executable code, plugin marketplace, vault, workspace/project manager, Git UI, cloud, collaboration, AI, or telemetry.
- Raw Markdown HTML is escaped in v1. User images are represented by safe alt text; no `<img>` is generated and CSP is `img-src 'none'`.
- The preview receives no ambient network, navigation, popup, download, form, media, frame, filesystem, clipboard, or document-authored script authority.
- Source files are capped at 20 MiB before and after every edit. The 1 MiB and 5 MiB fixtures are performance gates, not the safety limit.
- Preserve the pre-existing README change. Each implementation task uses red-green-refactor and one reviewable commit, but only after both planning gates approve.

## Requirements Summary

1. Create, open, edit, undo/redo, save, save-as, and detect external changes for one UTF-8 Markdown file per window.
2. Render CommonMark plus tables, footnotes, strikethrough, and task lists into a complete in-memory HTML document with revisioned source-byte annotations.
3. Keep source and preview aligned in both directions without stale-revision application or feedback loops.
4. Show the exact generated HTML in a read-only native source view.
5. Stay responsive at 1 MiB and 5 MiB and reject an open or edit that would exceed 20 MiB without mutating the current document.
6. Pass absolute binary, startup, process-tree RSS, typing, preview, scroll, security, platform, packaging, and teardown gates.

## RALPLAN-DR Short Summary

### Principles

1. Hard platform and preview requirements beat single-toolkit convenience.
2. Prove the production seam, not a toy webview, before approving a shell.
3. One core source of truth, one bounded render pipeline, and revision on every asynchronous boundary.
4. Security is represented by typed values and generated output allowlists, not post-render cleanup.
5. Stop and re-review contradictory evidence; never weaken native Wayland, security, or absolute budgets silently.

### Top Decision Drivers

1. Correct main-thread ownership and lifecycle across GTK/Wayland, GTK/X11, and macOS AppKit/WKWebView.
2. Correct UTF-8/IME/history/viewport behavior through the actual Rope-backed `EditorAdapter` at 1 MiB and 5 MiB.
3. End-to-end security and resource budgets, including full-page preview transport and all WebKit child processes.

### Viable Options

| Option | Bounded upside | Bounded downside | Gate status |
|---|---|---|---|
| GTK3 + GtkSourceView 4 + Wry on Linux; iced+Wry on macOS | Native Wayland through Wry's GTK container; iced has first-party multiline/IME shaping | Two shell adapters; iced/macOS footprint and child-view seam unproved | Favored only if Task 1 passes every hard gate |
| GTK3 + GtkSourceView 4 + Wry on Linux; egui/eframe+Wry on macOS | Same native-Wayland path; proven small-editor category and direct macOS window access | Stock `TextEdit` cannot be the production Rope adapter; custom viewport/IME work may be larger | Required macOS comparator |
| Bounded Ferrite fork with Wry preview replacement | Reuses a working cross-platform editor, packaging, and editing behavior | Native-widget preview, IDE scope, security surface, native-Wayland ownership, and retained maintenance surface must be replaced or deleted | Measured in Task 1; contradiction triggers re-review, never automatic adoption |

A single iced/egui+winit Linux shell is invalidated for v1 because Wry documents its direct window path as X11-only; native Wayland requires the GTK container path. Porting Marco remains invalidated by a permanent unsupported macOS port. The shared-core BUILD decision survives a shell-spike failure, but the application shell does not: failure leaves FeatherMark with no approved implementation architecture.

## Approved Shape Versus Unapproved Shape

**Approved for planning:** a small shared core, Ropey document, pulldown-cmark authority, system webviews, escaped raw HTML, typed file/security boundaries, and native Wayland as a hard target.

**Not approved until Task 1 evidence:** the macOS toolkit, the exact native widget adapters, acceptable duplicate editor-buffer cost, full-page custom-scheme preview latency, and the release resource floor. If any candidate fails, implementation stops at Task 1 and the plan returns to Architect then Critic; BUILD does not imply permission to improvise a shell.

## Event-Loop and Thread Ownership

| Surface | Owner | Exact rule |
|---|---|---|
| Linux GTK/Wayland and GTK/X11 | GTK `Application`/GLib main context on the process main thread | Call `gtk::init` there; create the GTK window, `Paned`, GtkSourceView, and a shown `gtk::Fixed` container; configure `WebViewBuilder::new()` with the protocol, pending `RenderUrl`, IPC, navigation policy, and bounds; then call `WebViewBuilderExtUnix::build_gtk(builder, &container)`. Bounds, focus, navigation, and destruction stay on this thread. No winit, iced, or eframe loop exists in the Linux production binary. Worker results enter through a bounded GLib main-context channel. Drop order is WebView -> GTK child -> window -> application. |
| macOS iced candidate | iced/winit event loop on the process main thread | NSWindow/WKWebView creation and all Wry calls occur inside event-loop callbacks; workers use an event-loop proxy. Drop WebView before native window. |
| macOS egui candidate | eframe/winit event loop on the process main thread | Same AppKit/Wry ownership rule; no secondary Cocoa loop and no Wry call from a render worker. |
| Core document | UI thread | All accepted edits, history mutations, file-state transitions, and revision increments are serialized here. |
| Renderer | One named worker thread | Receives immutable snapshots through the single-flight scheduler; never touches a native widget or Wry. |
| File watcher/I/O | Bounded worker(s) | Emits typed results to the UI owner; it cannot mutate `Document` directly. |

Task 1 must run with GLib thread assertions and platform main-thread assertions enabled. A wrong-thread call, nested GTK loop, orphan process, or teardown deadlock disqualifies that shell.

## Production Core, Editor, Snapshot, and History Contracts

```rust
pub const MAX_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_UNDO_BYTES: usize = 64 * 1024 * 1024;
pub type Revision = u64;
pub type InteractionId = u64;
pub type AdapterCommitId = u64;
pub type CompositionId = u64;

pub struct Edit { pub byte_range: std::ops::Range<usize>, pub replacement: String }
pub enum TransactionKind { Typing, Delete, Paste, Cut, ImeCommit, Programmatic }
pub struct EditTransaction {
    pub base_revision: Revision,
    pub id: u64,
    pub kind: TransactionKind,
    pub edits: Vec<Edit>,
}
pub struct ChangeSet {
    pub before: Revision,
    pub after: Revision,
    pub edits: Vec<Edit>,
    pub changed_bytes_after: std::ops::Range<usize>,
}
#[derive(Clone)]
pub struct DocumentSnapshot { pub revision: Revision, rope: ropey::Rope }

pub struct HistoryEntry { transaction: EditTransaction, inverse: Vec<Edit>, charged_bytes: usize }
pub struct Document {
    rope: ropey::Rope,
    revision: Revision,
    undo: std::collections::VecDeque<HistoryEntry>,
    redo: std::collections::VecDeque<HistoryEntry>,
    undo_bytes: usize,
}
impl Document {
    pub fn new(text: &str) -> Result<Self, DocumentError>;
    pub fn revision(&self) -> Revision;
    pub fn len_bytes(&self) -> usize;
    pub fn snapshot(&self) -> DocumentSnapshot;
    pub fn apply(&mut self, tx: EditTransaction) -> Result<ChangeSet, EditError>;
    pub fn undo(&mut self) -> Option<ChangeSet>;
    pub fn redo(&mut self) -> Option<ChangeSet>;
    pub fn write_to<W: std::io::Write>(&self, sink: W) -> std::io::Result<()>;
}

pub struct ImeCommit {
    pub composition_id: CompositionId,
    pub base_revision: Revision,
    pub byte_range: std::ops::Range<usize>,
    pub replacement: String,
}
pub enum EditorCommit { Edit(EditTransaction), Ime(ImeCommit) }

pub enum EditorEvent {
    CommitRequested { adapter_commit_id: AdapterCommitId, commit: EditorCommit },
    CompositionStarted { id: CompositionId, base_revision: Revision, byte_range: std::ops::Range<usize> },
    CompositionUpdated { id: CompositionId, base_revision: Revision, preedit: String },
    CompositionCancelled { id: CompositionId, base_revision: Revision, reason: CompositionCancelReason },
    ViewportChanged { revision: Revision, top_visible_byte: usize, user: bool },
    SourcePainted { revision: Revision, frame_seq: u64 },
}
pub enum CompositionCancelReason { User, FocusLost, StaleRevision }
pub enum LocalCommitRejection { StaleRevision, InvalidEdit, TooLarge }
pub type EditorEventSink = Box<dyn FnMut(EditorEvent) + 'static>;
pub trait EditorAdapter {
    fn set_event_sink(&mut self, sink: EditorEventSink);
    fn install_open_snapshot(&mut self, snapshot: &DocumentSnapshot) -> Result<(), EditorError>;
    fn acknowledge_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        change: &ChangeSet,
    ) -> Result<(), EditorError>;
    fn reject_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        reason: LocalCommitRejection,
        authoritative: &DocumentSnapshot,
    ) -> Result<(), EditorError>;
    fn apply_external_change(&mut self, change: &ChangeSet) -> Result<(), EditorError>;
    fn top_visible_byte(&self, revision: Revision) -> Result<usize, StaleRevision>;
    fn scroll_to_byte(&mut self, revision: Revision, byte: usize, id: InteractionId)
        -> Result<(), EditorError>;
    fn set_read_only_generated(&mut self, revision: Revision, html: std::sync::Arc<str>)
        -> Result<(), EditorError>;
}
```

Contract rules:

- `Document` is the sole authoritative text/history state. `FileService` owns paths and I/O; `Document` has no `open(path)` or `save(path)` method.
- `snapshot()` is a cheap immutable shared-root clone and never flattens to `String` on the UI thread. A test asserts structural sharing. The one render worker may flatten the accepted snapshot once because pulldown-cmark consumes `&str`; no second full source copy may coexist in the render pipeline.
- Each adapter may keep at most one widget-owned mirror because native text widgets require it. Open/recovery may install one full snapshot; normal typing applies incremental `ChangeSet`s. Per-keystroke full-buffer get/set is forbidden and measured with an allocation counter.
- `set_event_sink` is installed before the widget accepts input. Native callbacks deliver every `EditorEvent` through that sink on the UI thread; polling widget text to discover edits is forbidden. Each local widget mutation gets one monotonically increasing `AdapterCommitId` and emits exactly one `CommitRequested`; `EditorCommit::Edit` carries its `EditTransaction`, and `EditorCommit::Ime` carries the sole accepted composition payload. Both carry the widget's pre-mutation authoritative `base_revision`.
- A local widget edit has already changed the mirror when `CommitRequested` is emitted. After `Document::apply` succeeds, the reducer calls `acknowledge_local_commit`; that method only retags the mirror to `change.after`, clears the pending id, and schedules `SourcePainted`—it must not apply the bytes a second time. Undo, redo, reload, and other core-originated changes use `apply_external_change` exactly once. Rejection calls `reject_local_commit`, restores the supplied authoritative snapshot, and emits no paint for the rejected revision.
- The Linux adapter disables GtkSourceView's native undo manager and routes undo/redo shortcuts to `Document`; the macOS adapter likewise disables or bypasses toolkit snapshot history. There is exactly one history owner.
- Every byte range must be ordered, in bounds, and on UTF-8 boundaries against `base_revision`. Transactions are all-or-nothing. A post-edit size greater than 20 MiB returns `EditError::TooLarge` without revision/history/widget mutation.
- IME preedit remains adapter-owned and does not enter Rope/history. `CompositionStarted` captures `base_revision`; every update and native commit callback must match both the active id and that revision. There is no `CompositionCommitted` event and no toolkit-specific document callback. An accepted native commit clears the visual preedit, applies the replacement once to the adapter mirror, allocates one `AdapterCommitId`, and emits exactly one `CommitRequested { adapter_commit_id, commit: EditorCommit::Ime(ImeCommit { composition_id, base_revision, byte_range, replacement }) }`. The reducer converts that payload to one `EditTransaction { base_revision, id: adapter_commit_id, kind: TransactionKind::ImeCommit, edits: vec![Edit { byte_range, replacement }] }`, invokes `Document::apply` exactly once, records exactly one history entry and revision, then invokes `acknowledge_local_commit` exactly once. The acknowledgement only clears the pending commit and retags the mirror; the next qualifying native layout callback emits exactly one `SourcePainted` for that revision. No composition event other than `CommitRequested` may call or cause `Document::apply`.
- The normative successful trace is `CompositionStarted(base=r)` -> zero or more `CompositionUpdated(base=r)` -> `CommitRequested(Ime(base=r))` -> one `Document::apply(r->r+1)` -> one `acknowledge_local_commit(after=r+1)` -> one `SourcePainted(revision=r+1)`. The conformance test records reducer/adapter calls and asserts this exact partial order, one mirror replacement, one Rope replacement, one history transaction, one revision increment, one acknowledgement, and one source paint; it also asserts that no `CompositionCommitted` symbol exists. Before any external `ChangeSet` or local commit at another revision, the adapter synchronously removes preedit and emits `CompositionCancelled { reason: StaleRevision }`; a late update or native commit for that id emits no `CommitRequested` and cannot mutate mirror, Rope, history, acknowledgement state, or paint counters. User/focus cancellation has the same zero-mutation rule. GTK, iced, and egui candidates must pass the identical Japanese success trace and revision-change-mid-preedit trace.
- Adjacent `Typing` commits coalesce only when they are contiguous, share direction/selection, have no intervening command/composition, and arrive within 500 ms. Delete, paste, cut, IME, newline, focus loss, selection change, save, and cursor relocation close the group.
- Undo accounting is `replaced_bytes + replacement_bytes + 96 bytes/edit`; evict oldest complete undo transactions until <=64 MiB. Redo is cleared by a new edit. Undo/redo each increment revision and emit a `ChangeSet`; they never restore a whole 5 MiB snapshot.
- Adapter viewport offsets are UTF-8 bytes for the exact revision. Stale viewport reports are rejected, never translated optimistically.
- `SourcePainted { revision, frame_seq }` is emitted only after the adapter's native layout/draw callback has displayed the mirror acknowledged for that exact accepted revision. Preedit-only draws, rejected edits, duplicate frame sequences, and stale revisions cannot satisfy typing latency or startup. The reducer records at most the first qualifying source paint per revision.

## Renderer, Backpressure, and Preview Transport

### Bounded renderer

`RenderScheduler` has exactly one running job and one replaceable pending slot. A 50 ms quiet period starts work. While a job runs, a newer snapshot atomically replaces the pending snapshot; skipped revisions are counted. A result is accepted only when `result.revision == Document::revision()`. Stale results and their HTML buffers are dropped without navigation. There is no unbounded queue.

- Source snapshot maximum: 20 MiB.
- Generated body maximum: 80 MiB; complete page maximum: 96 MiB. Overflow produces a typed native `PreviewTooLarge` state and no webview navigation.
- Scheduler test: 1,000 edits while rendering must produce queue depth <=1 pending, accept only the newest revision, and return to zero retained stale pages.
- Renderer metrics record debounce wait, queue wait, parse/generate, protocol response, and paint-ack phases separately.

### One authoritative Rust -> preview transport

The only document transport is a complete, immutable HTML response loaded by native Wry navigation from a revisioned custom-scheme URL. `evaluate_script`, `data:` URLs, POST bodies, DOM patch messages, and base64 document injection are forbidden.

```rust
pub struct RenderUrl { revision: Revision, nonce: [u8; 16] }
pub enum PreviewHostCommand {
    Navigate { revision: Revision, url: RenderUrl, page_bytes: usize },
    ScrollTo { revision: Revision, source_start: usize, interaction_id: InteractionId },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PreviewEventWireV1 {
    BridgeReady { v: u8, revision: Revision },
    Painted { v: u8, revision: Revision, frame_seq: u64 },
    Scroll { v: u8, revision: Revision, source_start: usize, interaction_id: InteractionId, user: bool },
    LinkActivated { v: u8, revision: Revision, normalized_url: String },
}
pub enum PreviewEventV1 {
    BridgeReady { revision: Revision },
    Painted { revision: Revision, frame_seq: u64 },
    Scroll { revision: Revision, source_start: usize, interaction_id: InteractionId, user: bool },
    LinkActivated { revision: Revision, target: SafeLinkTarget },
}
pub fn decode_preview_event(bytes: &[u8], loaded: Revision) -> Result<PreviewEventV1, PreviewEventError>;
```

`Revision` and `InteractionId` are declared once in `feathermark-types`. `SafeLinkTarget` and its complete canonical parser/serializer are also owned by `feathermark-types::safe_link`. Task 1A creates both `feathermark-types` and `feathermark-protocol`; Task 1B creates only `feathermark-core` document/editor implementation. `feathermark-protocol` depends on `feathermark-types`; `feathermark-core` and `feathermark-app` each depend on both and never depend on one another. This is the only permitted dependency direction for these contracts, so `PreviewEventV1` can contain `SafeLinkTarget` in Task 1 without a Task-3 promotion cycle.

- `Navigate` is encoded only by Wry's native `load_url` call. The protocol serves exact paths `/v1/document/{revision}/{nonce}`, `/v1/assets/preview.css`, and `/v1/assets/bridge.js`; every other method, host, path, query, fragment, stale revision, or nonce returns 404.
- Document response: `200`, `Content-Type: text/html; charset=utf-8`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, and the exact CSP below. CSS/JS responses use `text/css` and `text/javascript`, fixed compile-time bytes, and the same no-store/nosniff headers.
- Navigation policy allows only the app-initiated pending `RenderUrl`; redirects, user top-level navigation, new windows, downloads, and all `http`, `https`, `file`, `data`, `blob`, and other schemes are denied. Asset subrequests are protocol allowlist lookups, not navigation permission.
- The fixed bridge sends UTF-8 JSON IPC only. Maximum IPC is 1,024 bytes. The native boundary alone deserializes private `PreviewEventWireV1`; `v` must equal 1, unknown/duplicate fields fail, `revision` must equal the loaded document, offsets must be in that revision, and malformed messages have no side effects. `decode_preview_event` converts the private wire enum to the public trusted enum; no reducer or app message can receive `PreviewEventWireV1`.
- For `LinkActivated`, `decode_preview_event` reparses `normalized_url` with `SafeLinkTarget::parse_wire`, requires exact equality with that type's canonical serialization, and only then emits `PreviewEventV1::LinkActivated { target }`. A plain `String`, merely bridge-normalized URL, encoded alternative, mixed-case noncanonical spelling, or validation failure never crosses the native webview boundary.
- `ScrollTo` is the only bounded control exception: Rust serializes a fixed schema under 256 bytes into one audited bridge function. It contains no document text/HTML/URL and is rejected by JS when revision is stale. No other `evaluate_script` use is allowed.
- After `DOMContentLoaded`, the bridge waits for two nested `requestAnimationFrame` callbacks and sends `Painted`. Rust records the acknowledgement receipt on its monotonic host clock. A late `Painted` for a non-current revision is recorded as stale and cannot satisfy latency.
- Cross-platform protocol tests run on WKWebView and WebKitGTK under Ubuntu X11, Ubuntu native Wayland, and Fedora native Wayland; they assert MIME, origin, CSP, 404 behavior, revision/nonce rejection, paint acknowledgement, and no custom-scheme body discrepancy.

Exact CSP, delivered as an HTTP-equivalent response header and duplicated in a meta tag for inspection:

```text
default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none';
font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none';
child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none';
base-uri 'none'; form-action 'none'; navigate-to 'none'
```

## Typed Generated-HTML and URL Policy

Rendering constructs a typed `SafeNode` tree and serializes it; it never renders arbitrary tag/attribute strings and never sanitizes after the fact.

- Allowed elements: `main`, `section`, `div`, `p`, `h1`-`h6`, `blockquote`, `pre`, `code`, `em`, `strong`, `del`, `ul`, `ol`, `li`, `table`, `thead`, `tbody`, `tr`, `th`, `td`, `hr`, `br`, `sup`, `a`, and `span`.
- Allowed attributes are exact: source wrappers may use `id`, fixed-enum `class`, `data-source-start`, `data-source-end`, `data-source-revision`, and `data-source-kind`; `ol` may use a positive decimal `start`; code may use one validated `language-[A-Za-z0-9_-]{1,32}` class; alignment uses fixed classes only; task spans may use `role="checkbox"` and `aria-checked="true|false"`; links may use `role="link"`, `tabindex="0"`, and `data-feathermark-url` containing `SafeLinkTarget`. No `style`, `href`, `src`, `srcset`, `id` outside generated wrappers, generic `data-*`, generic `aria-*`, or `on*` attribute exists.
- Markdown raw HTML becomes escaped text. Markdown images become `<span class="image-alt">[image: escaped alt text]</span>`; destinations are discarded, `<img>` is impossible, and `img-src` stays `'none'`.
- Task 1A implements `SafeLinkTarget` once in `feathermark-types::safe_link`; Task 3 consumes that type and does not recreate or promote it. It has private fields and accepts only an absolute `http`, `https`, or `mailto` URL of at most 768 serialized UTF-8 bytes after HTML entity decoding, Unicode/control/whitespace rejection, one `url`-crate parse, lowercase scheme normalization, and serialization round-trip. `SafeLinkTarget::parse_wire` performs the same validation on IPC and requires that the incoming bytes already equal `as_canonical_str()`. It rejects userinfo, scheme-relative URLs, backslashes, embedded NUL/control characters, invalid percent escapes, and every other scheme. The page emits no `href`; activation is parsed again natively into `SafeLinkTarget` for display/copy only. v1 never opens it or performs a request.
- Tests cover mixed-case schemes, leading/trailing whitespace, HTML entities, `%66ile:`, double encoding, `javascript:`, `data:`, `blob:`, `file:`, UNC paths, protocol-relative URLs, CRLF insertion, SVG/MathML/raw HTML, CSS `url()`/`@import`, event handlers, and malformed UTF-8 inputs.

## Revisioned `SourceBlock` Model and Scroll Contract

```rust
pub enum SourceBlockKind {
    Heading, Paragraph, CodeBlock, TableRow, FootnoteDefinition,
    ThematicBreak, LeafFallback, Continuation,
}
pub struct SourceBlock {
    pub revision: Revision,
    pub start: usize,
    pub end: usize,
    pub ordinal: u32,
    pub depth: u16,
    pub kind: SourceBlockKind,
    pub dom_id: String,
    pub segment_index: u16,
    pub segment_count: u16,
}
```

Construction rules:

1. Consume pulldown-cmark `into_offset_iter()` events and maintain a checked container stack. Event ranges are half-open source byte ranges. Opening/closing pairs take the union of their own event ranges and descendant ranges.
2. Anchors are deepest visible leaves: headings, paragraphs, fenced/indented code blocks, table rows, footnote definitions, and thematic breaks. Lists, items, block quotes, tables, and footnote sections are containers and do not emit an anchor when they contain a valid descendant leaf. An empty/malformed container emits one `LeafFallback` from the checked union.
3. A code block is opaque even when its text resembles Markdown. For overlapping candidates choose greater depth, then smaller byte span, then earlier event ordinal. Final anchors are sorted by `(start, end, original_event_ordinal)` and assigned contiguous `ordinal`; duplicates collapse deterministically.
4. `start <= end <= snapshot.len_bytes()` and both ends must be UTF-8 boundaries. Invalid ranges fail the render with `RenderError::InvalidSourceRange`; they are never clamped silently. Empty documents receive one `LeafFallback` at `0..0`.
5. A leaf larger than 32 KiB is **replaced**, not supplemented, by consecutive nonempty segments that exactly partition `[start,end)` with no gaps/overlap. For each target `min(previous_cut + 32 KiB, end)`, choose the greatest UTF-8 line boundary in `(previous_cut, target]`; if none exists, choose the greatest Unicode-scalar boundary in that interval, and if none can advance use the next scalar boundary after `previous_cut`. The first replacement keeps the original leaf kind; later replacements use `Continuation`. All carry zero-based `segment_index`, common `segment_count`, and the original depth/event ordinal for sorting. The unsplit anchor emits no ordinal or source wrapper.
6. After replacement and duplicate collapse, sort by `(start, end, original_event_ordinal, segment_index)` and assign contiguous global ordinals. Every segment receives exactly one element with `dom_id = "sb-{revision}-{ordinal}"` and the exact revision/start/end/ordinal attributes, using this type-aware placement table:

   | Kind | Normative DOM placement | Geometry source |
   |---|---|---|
   | `Heading`, `Paragraph`, `FootnoteDefinition` | Partition the generated inline-child stream at the segment's source boundaries; place a non-nesting sibling `<span class="source-segment">` around each partition inside the single semantic owner. Inline nodes that cross a cut are cloned only as formatting shells; text is partitioned once and remains in document order. | Minimum nonempty text rect in that segment; when the segment contains syntax but no visible glyph, its own span border-box. |
   | `CodeBlock` | Emit sibling `<span class="source-segment code-segment">` children inside the one `<code>` element; escaped code text is partitioned at the exact UTF-8 cuts and CSS preserves whitespace. | Minimum text rect, or that span's border-box for an empty final line. |
   | `TableRow` | Keep one `<tr>`. Partition cell content by source range and put each segment wrapper inside the first cell whose event range intersects the segment start; if only row delimiters intersect, put the wrapper at the start of the first cell. Wrappers remain ordered by `segment_index`. | Wrapper text rect, or that wrapper's border-box when the segment contains only delimiters. |
   | `ThematicBreak` | Put the source attributes and id directly on the generated `<hr>`. It is never wrapped in a span. | `<hr>` border-box top. |
   | `LeafFallback` | Put the source attributes and id on the fallback `<div class="source-fallback">`; an empty document owns one zero-text fallback element. | Fallback border-box top. |
   | `Continuation` | Use the placement rule of the original leaf kind retained in a private `placement_kind`; `Continuation` changes the public kind/ordinal, not DOM legality. | The geometry source for `placement_kind`. |

   A valid visible semantic owner with a syntax-only or zero-glyph segment therefore uses its own border box and is not a render failure. A missing owner, detached wrapper, `display:none`, non-finite rectangle, or wrapper order that differs from ordinal order is `RenderError::InvalidAnchorGeometry`; borrowing a preceding/following anchor's rectangle, synthesizing CSS top offsets, or moving a wrapper outside its semantic owner is forbidden.
7. Every `PreviewEventV1::Scroll` and every `ScrollTo` carries the document revision; stale messages are rejected. After layout the bridge records `viewport_height = document.documentElement.clientHeight`, `content_height = document.documentElement.scrollHeight`, `preview_max_y = max(0, content_height - viewport_height)`, and each finite anchor top `T_k = clamp(scrollY + geometry_top_k, 0, preview_max_y)`. Equal tops choose the greater ordinal. For a user preview position `y`, use `y_c = clamp(y, 0, preview_max_y)` and report `j = max { k | T_k <= y_c + 1px }`, or ordinal zero when the set is empty.

Mapping and acceptance formulas:

- Define `no_scroll = source_max_top == 0 || preview_max_y == 0`. This boolean is the first branch in both production mappings, both independent oracles, sample grading, and endpoint assertions; no bottom/EOF or anchor rule may run before it.
- For source byte `b`, source-to-preview oracle `A(b)` is the replacement block containing `b`; if none contains it, choose greatest `start <= b`, otherwise the first block. Let `source_max_top` be the adapter-reported greatest byte that can occupy the source viewport top and `preview_max_y` the bridge value above. If `no_scroll`, source-to-preview commands exactly `0`; else if `b >= source_max_top`, it commands exactly `preview_max_y`; otherwise it commands `clamp(T_{A(b)}, 0, preview_max_y)`. `EditorAdapter::scroll_to_byte(revision, document_len, id)` is the normative request for editor EOF only when `!no_scroll`; the adapter then clamps it to `source_max_top` after layout.
- Reverse direction uses a separately hand-authored and SHA-256-pinned `tests/fixtures/scroll-oracle.json`; it lists expected `(ordinal,start,end)` for each fixture and is not generated by or linked to production `SourceBlock`/mapping code. For browser position `y`, let `y_c = clamp(y, 0, preview_max_y)`. If `no_scroll`, the expected request and settled editor top are exactly `0`; else if `y_c >= preview_max_y - 1px`, the expected request is document EOF (`len_bytes`) and the adapter must settle at `source_max_top`; otherwise `P(y_c) = oracle[j].start`, where `j = max { k | measured_T_k <= y_c + 1px }`, ties choose greater `k`, and `j=0` when the set is empty. Only after the `no_scroll` branch does grading map the actual editor top byte `q` to the independent oracle ordinal containing `q` (same containing/greatest-prior rule), never by calling production `A`.
- Source-to-preview samples are `b_i = floor(i * (len_bytes - 1) / 99)` for `i=0..99` (all zero for an empty document). Preview-to-source samples are `y_i = floor(i * max_scroll_y / 99)` for `i=0..99` (all zero when no scroll range). Error is the absolute difference between actual and independently expected oracle ordinals: `E_i = abs(actual_ordinal_i - expected_ordinal_i)`.
- Under `no_scroll`, every sample in both directions has expected command `0`, both panes must settle at `0`, and `E_i` is accepted as zero only when those exact command and settled offsets are zero; any nonzero command or settled offset is a hard failure before ordinal grading. The closed `scroll-short` scenario contains all three cases: `(source_max_top=0, preview_max_y=0)`, `(source_max_top=0, preview_max_y>0)`, and `(source_max_top>0, preview_max_y=0)`. The empty document additionally requires `len_bytes=0` and fallback ordinal zero. Only when `!no_scroll` do long-document endpoint rules apply: the final source sample must settle the preview at `preview_max_y`, and the final preview sample must request `len_bytes` and settle the editor at `source_max_top`; failure of either endpoint is a hard scroll failure even when the 95% ordinal threshold would otherwise pass.
- Pass requires at least 95 of 100 samples with `E_i <= 1`, all 100 with `E_i <= 2`, and nondecreasing actual ordinals for increasing samples. The remaining 5% therefore cannot jump farther than two anchors or reverse direction.
- A user scroll creates a fresh `interaction_id`. Programmatic movement on the other pane retains that id. Echoes with the same id are suppressed for `max(150 ms, two preview frames)` capped at 500 ms; a true user event always gets a new id. A ping-pong failure is any second programmatic direction reversal with the same id, or any command emitted after the settle window without new user input. Required result: zero failures over 100 alternating gestures.

## Measurable Acceptance Criteria

Percentiles use nearest rank: sort all retained samples and select index `ceil(0.95*n)-1`. Five warmups are recorded but excluded; no measured sample or outlier is discarded. A failed/missing sample fails the gate.

```rust
pub enum AppEvent {
    Interactive { revision: Revision },
}
```

`Interactive { revision }` is the sole startup marker. The reducer emits it once per process, and only when editor input is enabled, the adapter has delivered a non-stale `SourcePainted` for the current revision, and the host has accepted `PreviewEventV1::Painted` for that same revision after two animation frames. `BridgeReady`, `ControlReady`, window visibility, DOMContentLoaded, and either paint alone are not startup completion.

| Area | Exact gate |
|---|---|
| Executable size | Pre-package `release-size.json` measures only the stripped, hash-locked executable: <=25 MiB on macOS arm64/x86_64 and Ubuntu/Fedora x86_64; package fields are forbidden |
| Package size | After final package creation/signing/hashing and before installed smoke, hash-bound `package-size.json` proves each `.dmg`, `.deb`, and `.rpm` <=20 MiB excluding system WebKitGTK/WKWebView |
| Startup | On the stripped/hash-locked release candidate, 5 excluded launches and 20 measured launches each start from a separately restored, identical powered-off runner snapshot: candidate installed, no FeatherMark data/cache directory, FeatherMark never launched in that snapshot, no fixture/page pre-touch, network disabled, and OS/system-webview caches only at the pristine snapshot's baseline. `t0` is harness monotonic timestamp immediately before spawn; `t1` is external receipt of app `Interactive { revision }`. p95 <=500 ms macOS, <=750 ms Linux |
| Idle memory | On the same release candidate, at 10 s after `Interactive`, enumerate the associated process set defined below; sample summed RSS once/sec for 5 s and gate the maximum. <=180 MiB macOS; <=220 MiB Linux |
| Typing | On the instrumented `test-control` binary, 1,000 committed single-scalar edits after 100 warmups. Each next edit waits for the prior revision's accepted `SourcePainted` and then 10 ms, so none is coalesced. `t0` is control-command receipt before `Document::apply`; `t1` is `SourcePainted` for that accepted revision. p95 <=16 ms at 1 MiB and <=25 ms at 5 MiB |
| Preview | On the same paced instrumented run, each next edit waits for the prior revision's accepted `PreviewPainted` and then 10 ms; all 1,000 revisions must have one sample. `t0` is accepted core revision, `t1` is host receipt of current-revision `Painted` after two animation frames. p95 <=100 ms at 1 MiB and <=200 ms at 5 MiB |
| Backpressure | Under 1,000 zero-delay edits: running<=1, pending<=1, accepted final revision=current, zero stale navigations, zero retained stale pages |
| Scroll | Both directions apply `no_scroll = source_max_top == 0 || preview_max_y == 0` first, including both asymmetric short-document cases, then satisfy the 100-sample formula and long-document endpoints; zero ping-pong failures over 100 alternating gestures |
| File safety | Atomic same-directory save; injected failure before rename preserves original; dirty external change never auto-overwrites |
| Security | Local HTTP/HTTPS sentinel, DNS log, and file sentinel record zero access; all navigation/popup/download/form/image/script/protocol bypass cases remain blocked |
| Lifecycle | 50 create/focus/resize/hide/show/destroy cycles per GUI session; zero crash, deadlock, wrong-thread assertion, orphan child, or retained WebKit process after 5 s |
| Platforms | Automated and GUI-control gates pass on both macOS architectures, Ubuntu X11, Ubuntu native Wayland, and Fedora 43 native Wayland; Japanese IME trace passes each shell |

## Reproducible Metrics and GUI-Control Specification

### Locked reference runners and scenario classes

| Runner id | Immutable hardware/display/session contract | Pristine snapshot name |
|---|---|---|
| `fm-macos-arm64-v1` | Apple M1 8-core, 16 GiB RAM, 2560x1600 at 60 Hz/1x, native `aarch64`; the Task-1A lock pins exact macOS product/build and WKWebView versions | `fm-macos-arm64-v1-pristine-{candidate_sha12}` |
| `fm-macos-x86_64-v1` | Intel Core i7-9750H 6-core, 16 GiB RAM, 1920x1080 at 60 Hz/1x, native `x86_64`; the Task-1A lock pins exact macOS product/build and WKWebView versions | `fm-macos-x86_64-v1-pristine-{candidate_sha12}` |
| `fm-ubuntu-x11-v1` | Intel Core i5-8500 6-core, 16 GiB RAM, 1920x1080 at 60 Hz/1x, Ubuntu 24.04 GNOME Xorg, `XDG_SESSION_TYPE=x11`, `WAYLAND_DISPLAY` unset | `fm-ubuntu-x11-v1-pristine-{candidate_sha12}` |
| `fm-ubuntu-wayland-v1` | Intel Core i5-8500 6-core, 16 GiB RAM, 1920x1080 at 60 Hz/1x, Ubuntu 24.04 GNOME Wayland, `XDG_SESSION_TYPE=wayland`, live `WAYLAND_DISPLAY`, `DISPLAY` unset | `fm-ubuntu-wayland-v1-pristine-{candidate_sha12}` |
| `fm-fedora-wayland-v1` | Intel Core i5-8500 6-core, 16 GiB RAM, 1920x1080 at 60 Hz/1x, Fedora 43 GNOME Wayland, `XDG_SESSION_TYPE=wayland`, live `WAYLAND_DISPLAY`, `DISPLAY` unset | `fm-fedora-wayland-v1-pristine-{candidate_sha12}` |

**Revision 3 amendment — approved for software implementation by the Critic against Architect SHA-256 `99a9ecb70af5b98636fe364b3933be14bb484cc44ad774ef49a49529a66d400e`. Task 1A completion remains externally gated.**

Task 1A may create `xtask/runner-lock-v1.json` only after exact five-runner provisioning. Normal/release `xtask` builds compile as typed `Unprovisioned` when both production manifests are absent; the exact command fails before output/network. If one manifest exists or either is invalid, build fails. When valid, `build.rs` embeds five independently reviewed roots, dispatch pins, installed-probe hashes, and per-row enrollment snapshot/provider/base-image commitments. No test feature, environment, CLI, capture file, generated key, or lock field can select/override production trust.

`runner capture-verify-matrix` treats `--capture-dir` as output-only. It dispatches five enrollment probes, commits over manifests/ordered identities/enrollment exchanges, then dispatches five fresh `post_lock` probes signing that commitment. The final lock embeds all ten exchanges and verifies offline against compiled roots. Missing, extra, duplicate, replayed, stale, wrong-purpose/run/challenge/commitment, noncanonical, unsigned, substituted, or mismatched evidence fails.

```bash
cargo build --release --locked -p xtask --bin xtask
target/release/xtask runner capture-verify-matrix --runners fm-macos-arm64-v1,fm-macos-x86_64-v1,fm-ubuntu-x11-v1,fm-ubuntu-wayland-v1,fm-fedora-wayland-v1 --capture-dir target/runner-captures --out xtask/runner-lock-v1.json
```

Both Task-1A probes equal the independently pinned enrollment snapshot/provider/base-image commitment and share one boot/session. Later pre-spawn expectations derive from the verified candidate/release manifest and runner id. Every application/candidate launch consumes a fresh five-second single-use `VerifiedRunner` through centralized app launch. Identity, launcher-measured probe hash, lock/manifest hash, snapshot, boot/session relation, challenge, and freshness are checked before spawn.

The scenario registry is closed and lives at `xtask/scenarios-v1.toml`: `ime-success`, `ime-stale`, `paced-latency`, `coalescing-stress`, `scroll-empty`, `scroll-short`, `scroll-long`, `security`, `lifecycle`, `product-functional`, `release-startup`, `release-idle-rss`, `release-size`, `release-teardown`, and `package-smoke`. The first ten require `build_kind="instrumented"`; the four `release-*` scenarios require `build_kind="release-candidate"`; `package-smoke` requires `build_kind="installed"`. `release-size` declares `measurement="stripped-executable-bytes"` and forbids package path, package bytes, and package hash fields. Unknown scenario names fail. `ime-success` and `ime-stale` run only against instrumented candidates. `scroll-short` executes the symmetric and both asymmetric `no_scroll` cases before any ordinal/end-point case. `package-smoke` deliberately excludes IME: it covers create/open/fixed ASCII edit/preview/bidirectional sync/save/reopen/close/uninstall, because macOS Accessibility and Linux AT-SPI are not declared as native input-method drivers.

Every raw JSON record includes schema version, git commit, dirty flag, Rust/toolchain, target triple, release profile/features, candidate executable SHA-256, package SHA-256 when applicable, runner id, runner-lock SHA-256, pristine snapshot id, CPU model/core count, RAM, OS/kernel, display session and environment, GTK/WebKitGTK or WKWebView version, monitor scale/refresh, fixture SHA-256/bytes, wall-clock capture time, monotonic clock kind, warmups, ordered raw samples, skipped/stale counts, and recursive PID/RSS samples. Instrumented artifacts live at `target/metrics/{commit}/{runner_id}/{scenario}.json`; release and installed-package artifacts use the unique closed fan-in paths defined below. `target/release/xtask metrics assert --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --dir target/metrics/gate` performs nearest-rank assertions and rejects missing runner/scenario rows.

Linux process trees are read from one generation of `/proc/{pid}/stat` + `status`, recursively rooted at the launched PID. macOS uses `proc_pidinfo` to collect the app's resource-coalition members (covering WKWebView XPC helpers) plus recursive descendants; the raw PID/PPID/coalition/RSS rows are retained. RSS is summed in KiB before MiB conversion. Name-based WebKit matching and subtracting an idle baseline are forbidden.

There are exactly two builds per candidate, isolated by `CARGO_TARGET_DIR`. The **instrumented** build enables only `test-control`; it owns correctness, IME, source/preview latency, scroll, backpressure, focus/resize, security, and lifecycle gates, but its bytes/RSS/startup are never compared with release budgets. The **release-like build output** uses `--no-default-features` plus only its shell feature and is input to the closed release-candidate pipeline below; the unstripped build output is never measured or packaged. `metrics assert` rejects a record whose `build_kind`, executable hash, runner, or scenario does not match the registry and release manifest.

The GUI harness exists only in the instrumented build and communicates over inherited stdin/stdout NDJSON pipes—never TCP, a webview endpoint, or a release IPC surface. Commands are versioned, `request_id`-correlated, <=64 KiB, and include `OpenFixture`, `Edit`, `BeginComposition`, `UpdateComposition`, `CommitComposition`, `CancelComposition`, `SetSourceViewport`, `SetPreviewViewport`, `FocusEditor`, `FocusPreview`, `Resize`, `HideShow`, and `Close`. Events include `ControlReady`, `EditAccepted`, revisioned `SourcePainted`, `PreviewPainted`, app `Interactive`, `FocusChanged`, `BoundsChanged`, `Closed`, and typed errors. The harness waits for correlated events with a 5 s timeout and kills the process group on failure.

The release candidate has no command pipe. For startup measurement only, the launcher passes one inherited write-only file descriptor using `--interactive-marker-fd <fd>`; on `AppEvent::Interactive`, the app writes exactly one `{"type":"interactive","v":1,"revision":N}\n` record and closes it. The fd accepts no input, is not a webview/IPC surface, and the same code is present but dormant in the packaged executable. `xtask gui installed` otherwise uses macOS Accessibility APIs or Linux AT-SPI to launch the hash-verified installed binary, type the registry's fixed ASCII edit, inspect pane roles/titles, invoke Save As to a temporary path, reopen it, and close. Its action trace and resulting file hash are stored with package evidence; it never reports an IME result.

Latency and coalescing are separate reproducible scenarios. `paced-latency` sends 100 warmups and then 1,000 edits, waiting after each for both revision-matched source and preview paints plus 10 ms; missing, duplicate, stale, or coalesced revisions fail. `coalescing-stress` sends 1,000 edits with zero delay, expects `running<=1` and `pending<=1`, and samples throughput/backpressure only—its per-edit latency is never folded into the paced p95.

### Closed release-candidate pipeline

The order is normative and irreversible: build -> strip -> assemble -> macOS sign -> hash candidate -> create pristine snapshots -> measure candidate, including executable-only `release-size.json` -> exact five-runner release fan-in/global assertion -> package without executable mutation -> hash package -> emit and pass hash-bound `package-size.json` -> install -> verify installed executable/package hashes -> package smoke -> exact five-runner package fan-in/global assertion. A command after candidate hashing that changes the candidate executable invalidates all measurements.

```bash
# Linux: GNU strip output is the candidate; the build output is retained only as input evidence.
target/release/xtask release prepare-linux --input target/release-like/product-linux/release/feathermark --strip-tool /usr/bin/strip --candidate target/release-candidate/linux/feathermark --manifest target/release-candidate/linux/release-manifest.json

# macOS: copy into the final bundle, strip before signing, sign once, then hash the signed embedded executable.
test -n "$FEATHERMARK_CODESIGN_IDENTITY"
target/release/xtask release prepare-macos --input target/release-like/product-macos/release/feathermark --strip-tool /usr/bin/strip --bundle target/release-candidate/macos/FeatherMark.app --codesign-identity "$FEATHERMARK_CODESIGN_IDENTITY" --codesign-options runtime --timestamp --manifest target/release-candidate/macos/release-manifest.json
codesign --verify --deep --strict --verbose=2 target/release-candidate/macos/FeatherMark.app
```

`prepare-linux` executes `/usr/bin/strip --strip-unneeded -o <candidate> <input>`, marks the candidate executable, hashes input and candidate, and records both paths. `prepare-macos` creates the final `.app`, copies the input to `Contents/MacOS/feathermark`, executes `/usr/bin/strip -x` on that embedded executable, then executes `/usr/bin/codesign --force --deep --strict --options runtime --timestamp --sign <identity> <bundle>` exactly once; only after signing does it hash the embedded executable and bundle file manifest. Both commands write schema `feathermark.release-manifest.v1`, fail dirty/missing inputs, and make the candidate path/hash immutable inputs to subsequent commands.

For each release-matrix row, `metrics release-row` creates the powered-off snapshot named by the runner table after installing the row's exact candidate but before FeatherMark has ever launched. The snapshot contains no FeatherMark data/cache/preferences/recent-document state, no fixture/page pre-touch, disabled network, and only baseline OS/system-webview caches. `release-startup` restores that named snapshot before every one of 5 warmups and 20 retained launches. `release-idle-rss` and `release-teardown` each restore it once per retained trial. Pre-package `release-size` reads only the stripped, hash-verified executable and writes only `executable_path`, `executable_bytes`, and `executable_sha256` size evidence; the schema rejects package path, bytes, hash, or aggregate bundle size. A snapshot whose embedded candidate hash differs from the release manifest is unusable.

Task 7 creates this complete `xtask/release-matrix-v1.toml`; these are the only product release rows:

```toml
[[row]]
runner = "fm-macos-arm64-v1"
candidate = "target/release-candidate/macos/FeatherMark.app/Contents/MacOS/feathermark"
manifest = "target/release-candidate/macos/release-manifest.json"
out = "target/metrics/gate/fm-macos-arm64-v1"
[[row]]
runner = "fm-macos-x86_64-v1"
candidate = "target/release-candidate/macos/FeatherMark.app/Contents/MacOS/feathermark"
manifest = "target/release-candidate/macos/release-manifest.json"
out = "target/metrics/gate/fm-macos-x86_64-v1"
[[row]]
runner = "fm-ubuntu-x11-v1"
candidate = "target/release-candidate/linux/feathermark"
manifest = "target/release-candidate/linux/release-manifest.json"
out = "target/metrics/gate/fm-ubuntu-x11-v1"
[[row]]
runner = "fm-ubuntu-wayland-v1"
candidate = "target/release-candidate/linux/feathermark"
manifest = "target/release-candidate/linux/release-manifest.json"
out = "target/metrics/gate/fm-ubuntu-wayland-v1"
[[row]]
runner = "fm-fedora-wayland-v1"
candidate = "target/release-candidate/linux/feathermark"
manifest = "target/release-candidate/linux/release-manifest.json"
out = "target/metrics/gate/fm-fedora-wayland-v1"
```

On each locked runner, the workflow sets `FORGEJO_RUNNER_NAME` from the immutable runner label. The identical closed command validates that value against the five-row lock, selects exactly one row, and derives `snapshot = "{runner}-pristine-{first_12_hex(candidate_sha256)}"` from the release manifest; a zero-row, multi-row, or unknown-runner match fails. Each runner then packs one uniquely named archive:

```bash
target/release/xtask metrics release-row --runner "$FORGEJO_RUNNER_NAME" --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --scenarios release-startup,release-idle-rss,release-size,release-teardown
target/release/xtask evidence pack-release --runner "$FORGEJO_RUNNER_NAME" --input "target/metrics/gate/$FORGEJO_RUNNER_NAME" --out "target/evidence/release/release-$FORGEJO_RUNNER_NAME.tar.zst"
```

The first command must create exactly `release-startup.json`, `release-idle-rss.json`, `release-size.json`, and `release-teardown.json`; `release-size.json` is executable-only. The second archive contains those four files plus `evidence-manifest.json`, whose runner id, runner-lock hash, release-manifest hash, candidate executable hash, file hashes, and archive path are self-consistent. The five Forgejo matrix jobs upload exactly `feathermark-release-fm-macos-arm64-v1`, `feathermark-release-fm-macos-x86_64-v1`, `feathermark-release-fm-ubuntu-x11-v1`, `feathermark-release-fm-ubuntu-wayland-v1`, and `feathermark-release-fm-fedora-wayland-v1`; each contains only its like-named `release-<runner>.tar.zst`.

The release fan-in job downloads those five artifacts into `target/evidence/release-fan-in/` and runs exactly one global assertion command with five explicit, non-glob archive arguments:

```bash
target/release/xtask metrics assert-global --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --archive target/evidence/release-fan-in/release-fm-macos-arm64-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-macos-x86_64-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-ubuntu-x11-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-ubuntu-wayland-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-fedora-wayland-v1.tar.zst
```

`metrics assert-global` rejects any missing, extra, duplicate, renamed, wrong-runner, instrumented, installed, or hash-mismatched archive/record; requires all four exact release scenarios for each of the five exact rows; forbids package fields in every `release-size.json`; checks every executable SHA-256 against the matching release manifest; and applies the executable/startup/RSS/lifecycle budgets once across the complete matrix. Packaging cannot begin until this one global assertion exits zero.

## Exact File and Responsibility Map

| Path | Responsibility / creation order |
|---|---|
| `Cargo.toml`, `rust-toolchain.toml`, `deny.toml` | Task 1A workspace, pinned toolchain/components, dependency policy |
| `xtask/{build.rs,runner-trust-roots-v1.json,runner-dispatch-v1.toml,src/runner/**,src/bin/{feathermark-runner-probe,feathermark-runner-launcher}.rs}` plus launcher service definitions | Task 1A typed config, roots/dispatch/enrollment commitments, probe and root measured launcher, ten-exchange lock, offline/current verification; real rows require external provisioning |
| `xtask/src/{app_launch,tool_process,candidate,gui,metrics,release,package,installed_smoke}.rs` | Task 1A creates capability launch boundary; Task 1C wires all real launchers; only audited tool process may spawn non-app tools |
| `tests/fixtures/*.md`, `tests/corpus/` | Task 1A creates before any spike/test consumes them |
| `crates/feathermark-types/src/{lib,safe_link}.rs` | Task 1A sole owner of `Revision`, `InteractionId`, and canonical `SafeLinkTarget`; leaf crate with no workspace dependencies |
| `crates/feathermark-core/src/{lib,document,editor_contract}.rs` | Task 1B production document/snapshot/history/editor contracts; depends on `feathermark-types` |
| `crates/feathermark-core/src/{render,security,scroll,files}.rs` | Tasks 2-5; only `FileService` loads/saves paths |
| `crates/feathermark-core/benches/{edit,render,scroll}.rs` | Package-owned Criterion benches; no root `benches/` |
| `crates/feathermark-protocol/src/lib.rs` | Task 1A sole owner of versioned render URL, host command, preview events, metrics, and GUI-control schemas; depends only on `feathermark-types` |
| `spikes/feathermark-spike-support/src/{scheduler,transport,typed_render,scroll}.rs` | Task 1C first real spike-local render/block and seam behavior; never production dependency |
| `spikes/linux-gtk-wry/` | Task 1C first-class GTK/X11 + native GTK/Wayland production seam |
| `spikes/macos-iced-wry/`, `spikes/macos-egui-wry/` | Task 1C macOS shell comparison only |
| `spikes/ferrite-wry-slice/README.md`, `patches/`, `metrics/` | Task 1D pinned, bounded fork comparator evidence; no vendored fork in product workspace |
| `fuzz/{Cargo.toml,Cargo.lock,README.md,rust-toolchain.toml,fuzz_targets/*.rs,corpus/*}` | Task 1A real protocol target plus reserved non-evidence corpora; Task 1C real render/block evidence; Task 3 unchanged retarget |
| `crates/feathermark-app/src/{main,app,render_scheduler,preview_host}.rs` | Task 6 creates the app crate, reducer, scheduler, and sole Wry protocol host |
| `crates/feathermark-app/src/platform/{linux_gtk,macos}.rs` | Tasks 6-7 create Linux GTK and only the Task-1-approved macOS implementation; no losing macOS production module is created |
| `crates/feathermark-app/assets/{preview.css,bridge.js}` | Fixed audited assets; HTML itself is generated per revision |
| `crates/feathermark-app/tests/{protocol,security,e2e}.rs` | Host boundary and complete workflows |
| `docs/decisions/0001-shell-feasibility.md` | Task 1 evidence/selection/stop decision |
| `docs/decisions/0002-release-budgets.md` | Task 8 release measurements/exceptions |
| `.forgejo/workflows/{ci,gui,package}.yml` | Test, named GUI matrix, artifacts, packaging |

### Task-1 Spike Ownership and Promotion

Task 1A owns production `Revision`/`SafeLinkTarget` in the leaf `feathermark-types` crate and all preview/GUI/metric schemas in `feathermark-protocol`. Task 1B owns only `Document`/`EditorAdapter` implementation in `feathermark-core`. The dependency graph is `feathermark-types <- {feathermark-core, feathermark-protocol} <- feathermark-app`; core and protocol do not depend on each other. The runnable scheduler, custom-scheme host, typed-node renderer, and two-way mapper are deliberately spike-local in `feathermark-spike-support`; Tasks 2-6 may not pretend those implementations already exist in production.

Promotion is explicit and reviewable: Task 3 moves only typed-node/source-block implementation to `feathermark-core::{security,render}` and imports the already-owned `feathermark_types::SafeLinkTarget`; Task 4 moves the mapper to `feathermark-core::scroll`; Task 6 moves the scheduler and Wry transport to the explicitly created `feathermark-app::{render_scheduler,preview_host}` files. Each promotion starts by copying the spike's black-box tests unchanged, proving them red because the production module is absent, then moving the minimum implementation with `git mv` or a patch that preserves blame, rerunning those tests, and deleting the corresponding spike-local module only after green. Production crates may not import any path or package under `spikes/`; `cargo tree -p feathermark-app` and `rg 'feathermark-spike-support|spikes/' crates/` must both prove that boundary. Code that is not promoted is deleted after ADR 0001 rather than maintained twice.

Task 3 retargets byte-identical render/block harness logic/corpora from spike support to core and passes before deleting spike owner. Fuzz may depend on spike support for Task-1 evidence; production may not.

## Task 1: Bootstrap and Prove the Production Editor/GTK/Wry Seam (XL, mandatory stop gate)

### 1A — Bootstrap before use

- [ ] Create the root workspace with leaf `feathermark-types`, core, protocol, xtask, three minimal compiling spike members, and the spike-only `feathermark-spike-support` member; create exact-size deterministic `small`, `unicode`, `one-mib`, `five-mib`, `hostile`, `nested`, and `giant-block` fixtures before compiling spike tests. `feathermark-app` is not a member until Task 6, and no production crate depends on `feathermark-spike-support`.
- [ ] Bootstrap exact tools before their first use: `rustup toolchain install 1.88.0 --profile minimal --component rustfmt,clippy`; `rustup toolchain install nightly-2026-07-01 --profile minimal --component llvm-tools-preview`; `cargo install --locked cargo-deny --version 0.20.2`; `cargo install --locked cargo-audit --version 0.22.2`; `cargo install --locked cargo-fuzz --version 0.13.2`; `cargo install --locked tokei --version 12.1.2`. Record `rustc +1.88.0 -Vv`, `cargo deny --version`, `cargo audit --version`, `cargo fuzz --version`, and `tokei --version`; any missing binary or version other than cargo-deny 0.20.2, cargo-audit 0.22.2, cargo-fuzz 0.13.2, or tokei 12.1.2 fails bootstrap.
- [ ] Create pinned `fuzz/` and real strengthened `preview_event` successful-result oracles while retaining deterministic protocol semantic tests. Its input is exactly an eight-byte little-endian `u64` loaded revision followed by one newline-terminated NDJSON frame. Migrate the old raw-JSON seed and commit `bridge_ready_rev0`, `painted_rev1`, `scroll_rev2`, and `link_https_rev3` success seeds plus prefixed `stale_revision`, `malformed_json`, and four-byte `short_prefix` error seeds; the pinned receipt must show all four success variants decoded. Build/run with `-runs=10000 -seed=1`. Remove no-op render/block harnesses/bins; retain their exact corpus directories unchanged as reserved Task-1A non-evidence seed data, document in `fuzz/README.md`, and update `Cargo.lock`. Task 1C alone makes them evidence.
- [ ] Create and test `Revision`, `InteractionId`, and the complete canonical `SafeLinkTarget` parser/serializer in `feathermark-types`; then create the NDJSON preview/GUI-control/metric schemas in `feathermark-protocol` and the xtask driver. First command: `cargo test -p feathermark-types -p feathermark-protocol -p xtask`; expected red until URL canonicalization, framing, maximum sizes, timeouts, and fixture checks exist. `cargo tree -p feathermark-protocol --edges normal` must show `feathermark-types` and must not show `feathermark-core` or any spike package. After those tests pass and before any runner, comparator, fixture, GUI, metric, release, or package subcommand is invoked, run `cargo build --release --locked -p xtask --bin xtask` and require executable `target/release/xtask`; every later plan command invokes that built path rather than `cargo run`.
- [ ] Add typed provisioned/unprovisioned build state, sealed production/test providers, root measured-probe launcher, deterministic-CBOR exchanges, enrollment commitment, self-contained ten-exchange lock, offline/current verification, and centralized capability-consuming application launch. Linux executes the measured descriptor with `fexecve`; macOS copies the no-follow opened and hashed signed probe into a unique root-only `0500` file, rechecks identical hash/designated requirement/cdhash, and executes that exact copy with SDK-supported `posix_spawn`. Publish a lock only as an affirmative pair of normal lock plus permanent hash-binding committed record: both files and parent transitions are fsynced, `open_committed_runner_lock` requires the pair, and post-commit diagnostic cleanup cannot revoke authorization. Unprovisioned release build succeeds but exact capture fails before I/O. Hermetic tests cannot satisfy production lock.
- [ ] After fixtures/contracts/xtask are green but before 1B implementation, create the comparator's separate Git repository with `target/release/xtask comparator scaffold create --fixtures tests/fixtures --contracts crates/feathermark-types,crates/feathermark-protocol --xtask xtask --out target/comparator/shared-scaffold --lock spikes/ferrite-wry-slice/scaffold-lock.json`. The command copies only `fixtures/`, `contracts/`, and `xtask/` into an empty repository, rejects symlinks and every other top-level path, commits with its recorded deterministic author/timestamp, and writes the full 40-hex `commit_sha`, full 40-hex `tree_sha`, sorted `git ls-tree -r --full-tree` rows, and SHA-256 of that listing to the lock. Run `target/release/xtask comparator scaffold verify --repo target/comparator/shared-scaffold --lock spikes/ferrite-wry-slice/scaffold-lock.json`; it must recompute `git rev-parse HEAD^{commit}`, `git rev-parse HEAD^{tree}`, the sorted tree listing/hash, assert a clean worktree, and assert the three-path allowlist. Commit the lock before either 16-hour lane starts; any later fixture/contract/xtask change requires a new lock and restarts both clocks.
- [ ] Green command: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-targets`; this must succeed before 1B.

### 1B — Core document/editor implementation only, not toy textareas

- [ ] Implement only the `feathermark-core` document/snapshot/history and `EditorAdapter` contracts above, with String-oracle property tests, structural-sharing assertion, allocation counter, 20 MiB post-edit rejection, 64 MiB whole-transaction eviction, IME trace, and top-visible-byte contract. Task 1B does not create or modify ownership of `feathermark-types` or `feathermark-protocol`.
- [ ] Red command: `cargo test -p feathermark-core`; expected failures for UTF-8 split edits, composition grouping, stale viewport, post-edit cap, and snapshot sharing.
- [ ] Green command: `cargo test -p feathermark-core && cargo bench -p feathermark-core --bench edit`; no full-buffer copy may occur on ordinary edits.

### 1C — First-class platform seams

- [ ] Linux spike: GTK `Application` main thread calls `gtk::init`, owns GtkSourceView + `Paned` + shown `gtk::Fixed`, imports `WebViewBuilderExtUnix`, configures `WebViewBuilder::new()` with protocol/pending URL/IPC/navigation/bounds, and consumes it with `WebViewBuilderExtUnix::build_gtk(builder, &container)`. No obsolete GTK constructor spelling exists. Prove both `env -u WAYLAND_DISPLAY GDK_BACKEND=x11` and `env -u DISPLAY GDK_BACKEND=wayland`; the latter must report `XDG_SESSION_TYPE=wayland`, a live `WAYLAND_DISPLAY`, no `DISPLAY`, and Wry bounds/focus/IME/paint success.
- [ ] macOS spikes: iced and egui/eframe each use the same contracts and `feathermark-spike-support` scheduler, revisioned custom-scheme full-page transport, typed security boundary, source/preview mapper, GUI-control pipe, and deterministic WebView-first teardown. These are spike evidence, not production ownership; promotion follows the mapping above.
- [ ] Implement first real typed renderer/source-block constructor in `spikes/feathermark-spike-support/src/typed_render.rs`. Recreate render/block harnesses against real APIs, claim reserved corpora, and run unit/property plus pinned 10,000-run corpora before Task 1E. No surrogate/ignored/build-only harness.
- [ ] Route every candidate, GUI, metric, release, package, installed-smoke, lifecycle, and startup application launch through `xtask/src/app_launch.rs` consuming fresh `VerifiedRunner`. Non-application tools use separate closed API. Compile-fail, Clippy disallowed-method, and per-launcher tests enforce.
- [ ] All spikes perform valid/invalid UTF-8 edits, one complete Japanese IME composition, undo/redo, 100 top-visible-byte probes, the `no_scroll` first branch for symmetric short documents and both asymmetric `(source_max_top=0, preview_max_y>0)` / `(source_max_top>0, preview_max_y=0)` cases in both directions, 1 MiB/5 MiB typing+preview traces, rapid resize/focus transfer, 1,000-edit backpressure, and 50 lifecycle cycles.
- [ ] Declare exact candidate identities: package/bin/feature triples are `linux-gtk-wry-spike`/`linux-gtk-wry-spike`/`linux-gtk`, `macos-iced-wry-spike`/`macos-iced-wry-spike`/`macos-iced`, and `macos-egui-wry-spike`/`macos-egui-wry-spike`/`macos-egui`; each package has `default = []` and optional `test-control`.
- [ ] Build six non-overwriting candidate binaries with these exact commands on the five runner rows declared in `xtask/candidate-matrix-v1.toml`:

  ```bash
  CARGO_TARGET_DIR=target/instrumented/linux-gtk cargo build --release --locked -p linux-gtk-wry-spike --bin linux-gtk-wry-spike --no-default-features --features linux-gtk,test-control
  CARGO_TARGET_DIR=target/release-like/linux-gtk cargo build --release --locked -p linux-gtk-wry-spike --bin linux-gtk-wry-spike --no-default-features --features linux-gtk
  CARGO_TARGET_DIR=target/instrumented/macos-iced cargo build --release --locked -p macos-iced-wry-spike --bin macos-iced-wry-spike --no-default-features --features macos-iced,test-control
  CARGO_TARGET_DIR=target/release-like/macos-iced cargo build --release --locked -p macos-iced-wry-spike --bin macos-iced-wry-spike --no-default-features --features macos-iced
  CARGO_TARGET_DIR=target/instrumented/macos-egui cargo build --release --locked -p macos-egui-wry-spike --bin macos-egui-wry-spike --no-default-features --features macos-egui,test-control
  CARGO_TARGET_DIR=target/release-like/macos-egui cargo build --release --locked -p macos-egui-wry-spike --bin macos-egui-wry-spike --no-default-features --features macos-egui
  ```

- [ ] Create `xtask/candidate-matrix-v1.toml` with exactly three identities: `linux-gtk` maps its instrumented/release-like paths above to runners `fm-ubuntu-x11-v1`, `fm-ubuntu-wayland-v1`, and `fm-fedora-wayland-v1`; `macos-iced` maps its paths to `fm-macos-arm64-v1` and `fm-macos-x86_64-v1`; `macos-egui` maps its paths to those same two macOS runners. Each row names its package, bin, sole shell feature, instrumented path, release-like input path, release-candidate path, release-manifest path, and output directory. The parser denies unknown fields, duplicate candidate/runner pairs, missing five-runner coverage, and any path outside the candidate's isolated target directory.
- [ ] On every locked runner execute exactly `target/release/xtask candidate gate-matrix --matrix xtask/candidate-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --instrumented-scenarios ime-success,ime-stale,paced-latency,coalescing-stress,scroll-empty,scroll-short,scroll-long,security,lifecycle --release-scenarios release-startup,release-idle-rss,release-size,release-teardown`. The command verifies the current runner, selects exactly one applicable row per candidate, invokes the closed `prepare-linux` or `prepare-macos` strip/sign/hash pipeline for its release-like input, creates the hash-derived pristine snapshot, runs every named instrumented and release scenario, and writes one JSON file per scenario below the row's output directory. It then invokes `metrics assert` against that row; a compile-only stub, instrumented release sample, unsigned macOS candidate, unstripped binary, absent scenario, unexpected runner, or hash mismatch fails.
- [ ] Linux GTK/Wry is disqualified by any native-Wayland failure. A macOS candidate is disqualified by any correctness/security/lifecycle failure or absolute budget failure. If both macOS candidates pass, choose iced only when startup and RSS are each <=115% of egui and latency gates pass; otherwise choose the passing candidate with lower preview p95, then RSS, then startup.

### 1D — Measurable Ferrite-fork comparator

- [ ] Pin Ferrite's lightweight `v0.3.0` tag to the exact upstream commit `ac4f36695e5b60a0a41fe64a458e0c2279fca13c` in the comparator README and `upstream-lock.json`; capture it read-only with `git ls-remote https://github.com/OlaProeis/Ferrite.git refs/tags/v0.3.0`, require the returned 40-hex object to equal that value, then verify a detached checkout with `git -C target/comparator/ferrite-upstream rev-parse HEAD` and record the MIT license SHA-256. Do not merge or vendor Ferrite into the product workspace.
- [ ] After all pre-clock Task-1C `xtask` launcher/candidate/GUI/metric/release/package/installed-smoke changes are final, rebuild release `xtask` and regenerate/verify/commit `spikes/ferrite-wry-slice/scaffold-lock.json`. Neither lane starts from the earlier Task-1A scaffold. Any later fixture/contract/`xtask` change invalidates and restarts both clocks.
- [ ] Create both lane repositories from the exact `commit_sha`/`tree_sha` in `spikes/ferrite-wry-slice/scaffold-lock.json` and verify them with `comparator scaffold verify` before clock start. The greenfield lane adds its candidate only under `candidate/greenfield/`; the Ferrite lane checks out only upstream `ac4f36695e5b60a0a41fe64a458e0c2279fca13c` under `candidate/ferrite/`. Both receive identical read-only dependency caches and run on `fm-macos-arm64-v1` plus `fm-fedora-wayland-v1`; no other repository, branch, patch, source directory, generated artifact, or post-scaffold Task-1 implementation is mounted or readable in either lane.
- [ ] Start the only two clocks with `target/release/xtask comparator clock start --lane greenfield --limit-seconds 57600 --log target/comparator/logs/greenfield.ndjson` and `target/release/xtask comparator clock start --lane ferrite --limit-seconds 57600 --log target/comparator/logs/ferrite.ndjson`. For each exact lane/log pair, the only later verbs are `pause --reason runner_outage`, `pause --reason registry_outage`, `resume`, and `stop`. Active time is host monotonic elapsed time. Setup, dependency resolution, build repair, reading upstream source, investigation, coding, tests, and runner use count. A pause requires start/end wall timestamps in the append-only log and forbids shell commands, editor writes, agent work, research, or runner access until resume; the verifier compares filesystem mtimes and command audit rows and invalidates a lane with work during a pause. Each lane stops automatically at 57,600 active seconds and becomes read-only.
- [ ] Every command, changed path, clock transition, runner allocation, and accepted external input is appended to its lane NDJSON log. Run `target/release/xtask comparator verify-isolation --lane-root target/comparator/greenfield --scaffold-lock spikes/ferrite-wry-slice/scaffold-lock.json --upstream-lock spikes/ferrite-wry-slice/upstream-lock.json --audit-log target/comparator/logs/greenfield.ndjson` and `target/release/xtask comparator verify-isolation --lane-root target/comparator/ferrite --scaffold-lock spikes/ferrite-wry-slice/scaffold-lock.json --upstream-lock spikes/ferrite-wry-slice/upstream-lock.json --audit-log target/comparator/logs/ferrite.ndjson`. The verifier fails a cherry-pick, copied Task-1C code/finding, unlogged patch, network source other than locked crates/Ferrite, file outside the lane root, clock overrun, unequal pause policy, or work by anyone other than the one declared lane engineer. No finding or code from the parallel lane may cross before both trees are frozen.
- [ ] Give both lanes the identical slice and acceptance script: open the same 1/5 MiB fixtures; Rope-backed edit/IME/undo; literal Wry full-page transport; paced source/preview paint; source->preview and preview->source sync; native `SafeLinkTarget` validation and deny-by-default protocol policy; focus/resize; and deterministic close on `fm-macos-arm64-v1` plus `fm-fedora-wayland-v1`. Both run the closed `xtask/scenarios-v1.toml` instrumented correctness/security/lifecycle set. The Ferrite lane must also delete/disable every IDE/executable/plugin/workspace surface needed to make that slice shippable; that deletion work remains inside its 16 hours.
- [ ] Record with `tokei`, `git diff --numstat`, and `cargo tree --edges normal`: total retained non-test Rust/JS/CSS LOC; added+modified LOC; deleted LOC; direct/transitive dependencies; reused versus replaced modules; forbidden-surface modules remaining; platform rows passed; and newly owned preview/security modules. Record hours and blocked items even if time expires.
- [ ] Before comparison, run exactly `target/release/xtask comparator clock verify --lane greenfield --log target/comparator/logs/greenfield.ndjson`, `target/release/xtask comparator verify-isolation --lane-root target/comparator/greenfield --scaffold-lock spikes/ferrite-wry-slice/scaffold-lock.json --upstream-lock spikes/ferrite-wry-slice/upstream-lock.json --audit-log target/comparator/logs/greenfield.ndjson`, `target/release/xtask comparator clock verify --lane ferrite --log target/comparator/logs/ferrite.ndjson`, and `target/release/xtask comparator verify-isolation --lane-root target/comparator/ferrite --scaffold-lock spikes/ferrite-wry-slice/scaffold-lock.json --upstream-lock spikes/ferrite-wry-slice/upstream-lock.json --audit-log target/comparator/logs/ferrite.ndjson`; record each frozen full commit/tree SHA and log SHA-256, and prove both started from the locked shared-scaffold tree. Compare only those two frozen 16-hour trees and the identical acceptance output; later Task-1 work cannot improve either score. Greenfield remains selected only if it passes all hard gates and its maintained non-test LOC is <= the fork's retained non-test LOC, its direct dependencies are <= the fork's, and the fork does not demonstrate >=25% less added+modified LOC while leaving zero forbidden surfaces. Any contrary, unequal-scope, unequal-time, isolation failure, or incomplete result does not select Ferrite; it stops for Architect -> Critic review of the BUILD evidence.

### 1E — Evidence and stop boundary

- [ ] ADR 0001 attaches raw JSON, environment metadata, allocation counts, lifecycle logs, protocol tests, macOS choice, native-Wayland proof, and Ferrite comparison.
- [ ] Stop immediately on any hard failure, missing runner, missing raw samples, native-Wayland fallback to XWayland, unsupported GTK/Wry lifecycle, both macOS candidates failing, or contradictory Ferrite evidence. Stop also on unprovisioned/invalid trust, probe install/hash/key drift, either macOS row failing the signed immutable-copy/`posix_spawn` acceptance, enrollment snapshot/provider/image mismatch, incomplete ten-exchange lock, protocol/freshness/boot-session failure, missing or mismatched durable committed record, nondurable/quarantined output, pre-spawn/launch bypass, missing `preview_event` success-branch receipts or real renderer/block fuzz receipts, or stale pre-Task1C scaffold. Do not create `feathermark-app`.
- [ ] Only a new approving Architect review followed by an approving Critic review of ADR 0001 may set `task_1_gate.complete=true`. Before that commit, run `target/release/xtask spike retain-macos-winner --adr docs/decisions/0001-shell-feasibility.md --iced-dir spikes/macos-iced-wry --egui-dir spikes/macos-egui-wry`; it parses exactly one winner, deletes the other directory plus its dependency/feature entries, and fails unless exactly one directory remains. Record the deletion in ADR 0001, commit `spike: approve FeatherMark production shell seam`, and begin Task 2.

## Task 2: Complete Document Editing and History (M)

- [ ] Extend Task 1B red tests to cover random multi-edit transactions, revision overflow handling, selection-breaking group rules, save group closure, redo invalidation, 20 MiB boundary edits, and 5 MiB String-oracle sequences.
- [ ] Implement the remaining core behavior without adding file paths or native-widget dependencies to `Document`.
- [ ] Run `cargo test -p feathermark-core` and `cargo bench -p feathermark-core --bench edit`; require edit-preparation p95 <8 ms at 5 MiB and zero ordinary-edit full copies.
- [ ] Commit `feat: complete rope document and bounded history`.

## Task 3: Implement Typed Secure Rendering and `SourceBlock`s (L)

- [ ] Copy the Task-1 typed-render/source-block black-box tests unchanged into `feathermark-core`; prove them red against the absent production modules, then promote only the reviewed implementation and delete those spike-local modules after green. Import `feathermark_types::SafeLinkTarget`; Task 3 must not define, wrap, or re-export a second URL-policy type.
- [ ] Write golden/property tests for every supported Markdown extension, nested/container precedence, Unicode offsets, giant-block continuation, malformed ranges, escaped raw HTML, exact element/attribute allowlist, typed link normalization, image-to-alt conversion, generated HTML inspection, and 80/96 MiB output caps.
- [ ] Implement typed `SafeNode` generation from pulldown-cmark events and the exact `SourceBlock` algorithm. The complete page references only the fixed same-origin CSS/bridge assets and carries revision metadata.
- [ ] Red/green: `cargo test -p feathermark-core`.
- [ ] Fuzz with the standalone package: `cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz render_markdown -- -max_total_time=60`, `cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz source_blocks -- -max_total_time=60`, and `cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz preview_event -- -max_total_time=60`. Seeds live in `fuzz/corpus/{target}`; crashes in `fuzz/artifacts/{target}` fail CI. Oracle reparses generated output and rejects any non-allowlisted tag/attribute/URL or invalid block invariant.
- [ ] Bench: `cargo bench -p feathermark-core --bench render`.
- [ ] Commit `feat: render typed revisioned secure HTML`.

## Task 4: Implement Revisioned Two-Way Scroll Synchronization (M)

- [ ] Copy the Task-1 scroll black-box tests unchanged into `feathermark-core`; prove them red, promote the reviewed mapper into `feathermark-core::scroll`, then remove the spike-local mapper after green.
- [ ] Write unit/property tests for `A(b)`, empty/duplicate/gap/continuation blocks, both directions, stale revision rejection, deterministic samples, interaction ownership, 150 ms/two-frame lease, 500 ms cap, and the ping-pong definition. The first assertion in each mapping/oracle/grading/endpoint test is `no_scroll = source_max_top == 0 || preview_max_y == 0`; cover `(0,0)`, `(0,positive)`, and `(positive,0)` and require command/settled offsets zero before any bottom/EOF or ordinal assertion.
- [ ] Implement binary-search mapping, preview top-block reporting, editor viewport mapping, fresh user ids, and echo suppression.
- [ ] Run `cargo test -p feathermark-core` and `cargo bench -p feathermark-core --bench scroll`; lookup p95 <0.1 ms for 100,000 blocks.
- [ ] Run the 100-sample bidirectional GUI scenario and assert the exact formula with xtask.
- [ ] Commit `feat: add revisioned offset scroll synchronization`.

## Task 5: Implement the Sole File Service and Conflict Policy (M)

```rust
pub struct DiskVersion { pub digest: blake3::Hash, pub modified: std::time::SystemTime, pub len: u64 }
pub struct LoadedDocument { pub document: Document, pub disk: DiskVersion }
pub enum ExternalResolution { ReloadDisk, KeepBuffer, SaveBufferAs(std::path::PathBuf) }
pub trait FileService {
    fn load(&self, path: &std::path::Path, max: usize) -> Result<LoadedDocument, FileError>;
    fn save_atomic(&self, path: &std::path::Path, snapshot: &DocumentSnapshot) -> Result<DiskVersion, FileError>;
}
```

- [ ] Define `DiskVersion`, `ExternalResolution`, errors, and `FileService` before app messages use them. `AppState` owns the optional path and saved version; no second loader exists in Document or platform adapters.
- [ ] Test UTF-8/BOM, invalid UTF-8, 20 MiB limit, same-directory tempfile, flush+file `sync_all`, rename, parent-directory sync where supported, injected pre-rename failure, notify debounce, clean reload, and dirty three-choice conflict.
- [ ] Run `cargo test -p feathermark-core`; commit `feat: add sole atomic file service`.

## Task 6: Create the App Crate and Exact Bounded Preview Host (L)

- [ ] Add package `feathermark-app`, bin `feathermark`, `default = []`, features `linux-gtk`, `macos-shell`, and `test-control` to the workspace. Create `src/preview_host.rs`, `src/platform/linux_gtk.rs`, and one `src/platform/macos.rs` implemented from the ADR-0001 winner. ADR 0001 maps `macos-shell` to exactly the winning iced or egui dependency; no losing dependency, feature name, or production source file is created.
- [ ] Copy the Task-1 scheduler/transport black-box tests unchanged into `feathermark-app`; prove them red, promote the reviewed implementations into `render_scheduler`/`preview_host`, and delete the final spike-support package after green. `cargo tree -p feathermark-app` and `rg 'feathermark-spike-support|spikes/' crates/` must show no production dependency/reference.
- [ ] Write scheduler tests for debounce, one running/one pending, replacement, stale result disposal, size errors, and 1,000-edit pressure.
- [ ] Write host tests for exact methods/hosts/paths/nonces/revisions, headers/MIME/CSP, 404s, IPC framing/limits/schema, native `LinkActivated` reparsing to `SafeLinkTarget`, stale paint/scroll, `ScrollTo` size, and zero document use of `evaluate_script`.
- [ ] Implement `RenderScheduler`, the complete-page custom protocol, navigation allowlist, fixed assets, two-frame paint acknowledgement, and typed errors.
- [ ] On Linux run `cargo test --locked -p feathermark-app --no-default-features --features linux-gtk,test-control` and `target/release/xtask gui matrix --package feathermark-app --app-bin feathermark --app-feature linux-gtk --scenario protocol --out target/metrics/protocol/linux`; on macOS run `cargo test --locked -p feathermark-app --no-default-features --features macos-shell,test-control` and `target/release/xtask gui matrix --package feathermark-app --app-bin feathermark --app-feature macos-shell --scenario protocol --out target/metrics/protocol/macos`. The matrix covers WKWebView, Ubuntu X11, Ubuntu native Wayland, and Fedora 43 native Wayland.
- [ ] Commit `feat: add bounded revisioned preview host`.

## Task 7: Assemble the Approved Native Product Shells (L)

- [ ] Complete `feathermark-app` only with the Task-1-approved Linux GTK adapter and the single `platform/macos.rs`. The losing macOS spike directory is already deleted at the Task-1 gate; `rg 'macos_(iced|egui)|macos-(iced|egui)' crates/feathermark-app` may match only the winner named by ADR 0001. Compile all remaining spike/test-control code out of packaged releases.
- [ ] Reducer tests cover new/open/edit/save/save-as/undo/redo, IME, render coalescing, stale acknowledgements, both scroll directions, generated-HTML read-only mode, and external conflict resolution.
- [ ] Product GUI tests cover 50/50 resizable panes, 1/5 MiB, generated source exactness, focus/resize/hide/show, suspend/resume, open-edit-preview-sync-inspect-save-reopen, and 50 clean closes.
- [ ] Keep WebContext/WebView alive in explicit platform state and destroy WebView first. Devtools, drag/drop, clipboard bridge, downloads, and document paths are disabled/absent.
- [ ] Build distinct product binaries: on Linux, `CARGO_TARGET_DIR=target/instrumented/product-linux cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features linux-gtk,test-control` and `CARGO_TARGET_DIR=target/release-like/product-linux cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features linux-gtk`; on macOS, `CARGO_TARGET_DIR=target/instrumented/product-macos cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features macos-shell,test-control` and `CARGO_TARGET_DIR=target/release-like/product-macos cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features macos-shell`. Run `target/release/xtask gui product-matrix --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --linux-bin target/instrumented/product-linux/release/feathermark --macos-bin target/instrumented/product-macos/release/feathermark --scenarios ime-success,ime-stale,paced-latency,coalescing-stress,scroll-empty,scroll-short,scroll-long,security,lifecycle,product-functional --out target/metrics/product`; it selects only the verified runner's applicable binary and must produce every named instrumented scenario. Create the exact five-row `xtask/release-matrix-v1.toml` declared above; commit `feat: assemble approved native FeatherMark shells`.

## Task 8: Enforce Metrics, Security, Packaging, and Release Gates (L)

- [ ] Run the two exact `release prepare-linux`/`release prepare-macos` commands in the closed release-candidate section, run `metrics release-row` plus `evidence pack-release` on all five locked runners, download the five uniquely named archives, and run the one exact `metrics assert-global` command. The fan-in must contain four release JSON files per runner; the global assertion mechanically enforces every release acceptance threshold, executable hash, runner lock, pristine snapshot, exact scenario, and required row while rejecting extras and forbidding package fields in `release-size.json`.
- [ ] Security runner starts loopback HTTP and HTTPS sentinels plus DNS/file access observers, opens the hostile corpus, and requires zero requests/access while all protocol/navigation tests pass.
- [ ] Build packages only after `metrics assert-global` passes. Linux runs `target/release/xtask package build-linux --candidate target/release-candidate/linux/feathermark --release-manifest target/release-candidate/linux/release-manifest.json --out target/packages/linux`; macOS runs `target/release/xtask package build-macos --bundle target/release-candidate/macos/FeatherMark.app --release-manifest target/release-candidate/macos/release-manifest.json --codesign-identity "$FEATHERMARK_CODESIGN_IDENTITY" --timestamp --out target/packages/macos`. Each command verifies the measured candidate hash first, makes no change to the candidate executable, signs final macOS outer artifacts before hashing them, and writes `package-manifest.json` with every final artifact SHA-256 and the release-manifest/candidate hash. On all five clean runtime rows, run the exact `package size-row` command before `package smoke-row`, pack one unique package archive per runner, fan in the five exact archives, and run the one exact `package assert-global` command defined below. Smoke verifies the installed executable SHA-256 equals the release manifest, runs create/open/fixed ASCII edit/preview/both sync directions/save/reopen/close/uninstall, and records no IME claim.
- [ ] Write ADR 0002 with executable/package hashes, raw metric artifact paths, installed dependency evidence, and no exceptions. Any hard-gate exception returns to Architect -> Critic; it cannot be waived in release notes.
- [ ] Run the verification block and commit `release: enforce FeatherMark platform and budget gates` only after independent security and verifier sign-off.

### Closed installed-package size, smoke, and fan-in pipeline

After the final `.dmg`, `.deb`, and `.rpm` are signed where applicable and hashed into their package manifests, each locked runtime row runs these commands in order. `FORGEJO_RUNNER_NAME` must be one of the five exact runner-lock ids; `size-row` selects the row's required final package kind (`.dmg` on macOS, `.deb` on Ubuntu, `.rpm` on Fedora), verifies its SHA-256 against the selected package manifest and its embedded executable SHA-256 against the release manifest, enforces <=20 MiB, and writes `package-size.json` before any install or launch:

```bash
target/release/xtask package size-row --runner "$FORGEJO_RUNNER_NAME" --matrix xtask/release-matrix-v1.toml --runner-lock xtask/runner-lock-v1.json --linux-package-manifest target/packages/linux/package-manifest.json --macos-package-manifest target/packages/macos/package-manifest.json --out "target/metrics/package/$FORGEJO_RUNNER_NAME/package-size.json"
target/release/xtask package smoke-row --runner "$FORGEJO_RUNNER_NAME" --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --linux-package-manifest target/packages/linux/package-manifest.json --macos-package-manifest target/packages/macos/package-manifest.json --size-evidence "target/metrics/package/$FORGEJO_RUNNER_NAME/package-size.json" --scenario package-smoke --out "target/metrics/package/$FORGEJO_RUNNER_NAME/package-smoke.json"
target/release/xtask evidence pack-package --runner "$FORGEJO_RUNNER_NAME" --input "target/metrics/package/$FORGEJO_RUNNER_NAME" --out "target/evidence/package/package-$FORGEJO_RUNNER_NAME.tar.zst"
```

`package-size.json` contains schema version, runner id, runner-lock hash, release-manifest hash, candidate executable SHA-256, package-manifest SHA-256, exact package filename/kind, package bytes, package SHA-256, 20 MiB limit, and `passed=true`. `smoke-row` refuses to install unless that exact evidence exists and passes; it recomputes the manifest and package hashes, requires them to equal the size record, records the same package SHA-256 plus the installed executable SHA-256 in `package-smoke.json`, and records `smoke_started_after_size_check=true`. Thus package size is hash-bound and enforced before smoke, not inferred afterward.

The five jobs upload exactly `feathermark-package-fm-macos-arm64-v1`, `feathermark-package-fm-macos-x86_64-v1`, `feathermark-package-fm-ubuntu-x11-v1`, `feathermark-package-fm-ubuntu-wayland-v1`, and `feathermark-package-fm-fedora-wayland-v1`, each containing only its like-named `package-<runner>.tar.zst`. The package fan-in job downloads them into `target/evidence/package-fan-in/` and runs exactly one global assertion with five explicit, non-glob archives:

```bash
target/release/xtask package assert-global --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --archive target/evidence/package-fan-in/package-fm-macos-arm64-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-macos-x86_64-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-ubuntu-x11-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-ubuntu-wayland-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-fedora-wayland-v1.tar.zst
```

`package assert-global` rejects missing, extra, duplicate, renamed, wrong-runner, unhashed, or hash-mismatched archives; requires exactly one `package-size.json` and one `package-smoke.json` for every runner; proves the two macOS `.dmg` hashes, both Ubuntu `.deb` rows, and the Fedora `.rpm` hash are the same bytes measured before their smoke; enforces every required package <=20 MiB; checks all five installed executable hashes against the matching release manifests; and requires all five non-IME installed-smoke workflows to pass. No second global package assertion exists.

## Exact Platform and Package Matrix

Each clean runtime receives only the built package, the target-native `target/release/xtask` accessibility driver, and fixed smoke fixtures. It does not install Rust, headers, compilers, or developer packages.

| Row | Build image/target and exact prerequisites | Artifact and runtime | Clean installed smoke |
|---|---|---|---|
| macOS arm64 | macOS 15.5 build host, Xcode 16.4 selected with `xcode-select`; `rustup target add aarch64-apple-darwin`; `MACOSX_DEPLOYMENT_TARGET=13.0` | `FeatherMark-aarch64.app.zip` and `.dmg`; system WKWebView only | Fresh macOS 13.7 arm64 runner: `hdiutil attach target/packages/macos/FeatherMark-aarch64.dmg -nobrowse`; `ditto /Volumes/FeatherMark/FeatherMark.app /Applications/FeatherMark.app`; `codesign --verify --deep --strict /Applications/FeatherMark.app`; `target/release/xtask gui installed --bin /Applications/FeatherMark.app/Contents/MacOS/feathermark --out target/metrics/package-smoke`; `hdiutil detach /Volumes/FeatherMark`; `rm -rf /Applications/FeatherMark.app` |
| macOS x86_64 | Intel macOS 15.5 build host, Xcode 16.4; `rustup target add x86_64-apple-darwin`; `MACOSX_DEPLOYMENT_TARGET=13.0` | `FeatherMark-x86_64.app.zip` and `.dmg`; system WKWebView only | Fresh Intel macOS 13.7 runner: `test "$(uname -m)" = x86_64`; `hdiutil attach target/packages/macos/FeatherMark-x86_64.dmg -nobrowse`; `ditto /Volumes/FeatherMark/FeatherMark.app /Applications/FeatherMark.app`; `codesign --verify --deep --strict /Applications/FeatherMark.app`; `target/release/xtask gui installed --bin /Applications/FeatherMark.app/Contents/MacOS/feathermark --out target/metrics/package-smoke`; `hdiutil detach /Volumes/FeatherMark`; `rm -rf /Applications/FeatherMark.app` |
| Ubuntu 24.04 x86_64 X11 | `x86_64-unknown-linux-gnu`; `apt-get install build-essential pkg-config libgtk-3-dev libgtksourceview-4-dev libwebkit2gtk-4.1-dev patchelf dpkg-dev zstd` | `.deb` and `.tar.zst`; runtime dependencies declare `libgtk-3-0t64`, `libgtksourceview-4-0`, `libwebkit2gtk-4.1-0` | Fresh VM: `sudo apt install ./target/packages/linux/feathermark_amd64.deb`; `env -u WAYLAND_DISPLAY GDK_BACKEND=x11 target/release/xtask gui installed --bin /usr/bin/feathermark --out target/metrics/package-smoke`; `sudo apt purge feathermark` |
| Ubuntu 24.04 x86_64 native Wayland | `x86_64-unknown-linux-gnu`; `apt-get install build-essential pkg-config libgtk-3-dev libgtksourceview-4-dev libwebkit2gtk-4.1-dev patchelf dpkg-dev zstd` | `feathermark_amd64.deb` and `feathermark_amd64.tar.zst`; no XWayland-only variant; runtime dependencies declare `libgtk-3-0t64`, `libgtksourceview-4-0`, `libwebkit2gtk-4.1-0` | Fresh GNOME Wayland VM: `sudo apt install ./target/packages/linux/feathermark_amd64.deb`; `test "$XDG_SESSION_TYPE" = wayland`; `test -n "$WAYLAND_DISPLAY"`; `env -u DISPLAY GDK_BACKEND=wayland target/release/xtask gui installed --bin /usr/bin/feathermark --out target/metrics/package-smoke`; `sudo apt purge feathermark` |
| Fedora 43 x86_64 native Wayland | `x86_64-unknown-linux-gnu`; `dnf install gcc gcc-c++ make pkgconf-pkg-config gtk3-devel gtksourceview4-devel webkit2gtk4.1-devel patchelf rpm-build zstd` | `.rpm` and `.tar.zst`; runtime requires the verified Fedora 43 package names `gtk3`, `gtksourceview4`, `webkit2gtk4.1` | Fresh Fedora 43 GNOME Wayland VM: `sudo dnf install ./target/packages/linux/feathermark.x86_64.rpm`; `test "$XDG_SESSION_TYPE" = wayland`; `test -n "$WAYLAND_DISPLAY"`; `env -u DISPLAY GDK_BACKEND=wayland target/release/xtask gui installed --bin /usr/bin/feathermark --out target/metrics/package-smoke`; `sudo dnf remove feathermark` |

The package jobs record `pkg-config --modversion gtk+-3.0 gtksourceview-4 webkit2gtk-4.1`, dynamic-library resolution (`otool -L` or `ldd`), and installed file manifests. Linux packages depend on system WebKitGTK; they do not bundle it.

## Consensus and Durable Plan Publication

RALPLAN stopped after the maximum five sequential Architect -> Critic rounds. Architect r5 returned `SOUND`; Critic r5 returned `ITERATE`. The three Critic-r5 terminal defects were mechanically corrected after review, but the corrected artifact was not re-reviewed. Therefore the durable publication is a best-available planning record, not consensus and not execution authority.

The planning owner publishes byte-identical copies at `.omx/drafts/feathermark-build-plan.md`, `.omx/plans/feathermark-build-plan.md`, and `docs/plan/build-plan.md`. The same handoff must also persist:

- `docs/plan/ralplan-dr.md`, containing the RALPLAN decision record, ADR pointer, all five sequential review rounds, final statuses, and the post-review mechanical-cleanup disclosure;
- `.omx/state/ralplan-consensus-handoff.json`, containing all planning artifacts, all five Architect and Critic review paths in round order, final statuses, the final artifact SHA-256, and `ralplan_consensus_gate.complete=false`, reason `max_iterations_without_critic_approval`, and `execution_authorized=false`;
- the identical SHA-256 for all three plan copies.

Verify the durable record without modifying it:

```bash
cmp .omx/drafts/feathermark-build-plan.md .omx/plans/feathermark-build-plan.md
cmp .omx/drafts/feathermark-build-plan.md docs/plan/build-plan.md
shasum -a 256 .omx/drafts/feathermark-build-plan.md .omx/plans/feathermark-build-plan.md docs/plan/build-plan.md
```

Expected: both `cmp` commands exit 0 and all three SHA-256 values are identical to `final_artifact_sha256` in the handoff JSON. ADR 0001 and any future execution command reference `.omx/plans/feathermark-build-plan.md`; `docs/plan/build-plan.md` is the human-facing mirror. Any later plan edit invalidates the recorded hash and still requires a new sequential Architect -> Critic approval before execution.

## Verification Block

```bash
cargo deny --version
cargo audit --version
cargo fuzz --version
tokei --version
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo deny check
cargo audit
cargo bench -p feathermark-core --bench edit
cargo bench -p feathermark-core --bench render
cargo bench -p feathermark-core --bench scroll
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz render_markdown -- -max_total_time=60
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz source_blocks -- -max_total_time=60
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz preview_event -- -max_total_time=60
cargo build --release --locked -p xtask --bin xtask
CARGO_TARGET_DIR=target/instrumented/product-linux cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features linux-gtk,test-control
CARGO_TARGET_DIR=target/release-like/product-linux cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features linux-gtk
CARGO_TARGET_DIR=target/instrumented/product-macos cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features macos-shell,test-control
CARGO_TARGET_DIR=target/release-like/product-macos cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features macos-shell
target/release/xtask gui product-matrix --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --linux-bin target/instrumented/product-linux/release/feathermark --macos-bin target/instrumented/product-macos/release/feathermark --scenarios ime-success,ime-stale,paced-latency,coalescing-stress,scroll-empty,scroll-short,scroll-long,security,lifecycle,product-functional --out target/metrics/product
target/release/xtask release prepare-linux --input target/release-like/product-linux/release/feathermark --strip-tool /usr/bin/strip --candidate target/release-candidate/linux/feathermark --manifest target/release-candidate/linux/release-manifest.json
test -n "$FEATHERMARK_CODESIGN_IDENTITY"
target/release/xtask release prepare-macos --input target/release-like/product-macos/release/feathermark --strip-tool /usr/bin/strip --bundle target/release-candidate/macos/FeatherMark.app --codesign-identity "$FEATHERMARK_CODESIGN_IDENTITY" --codesign-options runtime --timestamp --manifest target/release-candidate/macos/release-manifest.json
codesign --verify --deep --strict --verbose=2 target/release-candidate/macos/FeatherMark.app
target/release/xtask metrics release-row --runner "$FORGEJO_RUNNER_NAME" --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --scenarios release-startup,release-idle-rss,release-size,release-teardown
target/release/xtask evidence pack-release --runner "$FORGEJO_RUNNER_NAME" --input "target/metrics/gate/$FORGEJO_RUNNER_NAME" --out "target/evidence/release/release-$FORGEJO_RUNNER_NAME.tar.zst"
target/release/xtask metrics assert-global --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --archive target/evidence/release-fan-in/release-fm-macos-arm64-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-macos-x86_64-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-ubuntu-x11-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-ubuntu-wayland-v1.tar.zst --archive target/evidence/release-fan-in/release-fm-fedora-wayland-v1.tar.zst
target/release/xtask package build-linux --candidate target/release-candidate/linux/feathermark --release-manifest target/release-candidate/linux/release-manifest.json --out target/packages/linux
target/release/xtask package build-macos --bundle target/release-candidate/macos/FeatherMark.app --release-manifest target/release-candidate/macos/release-manifest.json --codesign-identity "$FEATHERMARK_CODESIGN_IDENTITY" --timestamp --out target/packages/macos
target/release/xtask package size-row --runner "$FORGEJO_RUNNER_NAME" --matrix xtask/release-matrix-v1.toml --runner-lock xtask/runner-lock-v1.json --linux-package-manifest target/packages/linux/package-manifest.json --macos-package-manifest target/packages/macos/package-manifest.json --out "target/metrics/package/$FORGEJO_RUNNER_NAME/package-size.json"
target/release/xtask package smoke-row --runner "$FORGEJO_RUNNER_NAME" --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --linux-package-manifest target/packages/linux/package-manifest.json --macos-package-manifest target/packages/macos/package-manifest.json --size-evidence "target/metrics/package/$FORGEJO_RUNNER_NAME/package-size.json" --scenario package-smoke --out "target/metrics/package/$FORGEJO_RUNNER_NAME/package-smoke.json"
target/release/xtask evidence pack-package --runner "$FORGEJO_RUNNER_NAME" --input "target/metrics/package/$FORGEJO_RUNNER_NAME" --out "target/evidence/package/package-$FORGEJO_RUNNER_NAME.tar.zst"
target/release/xtask package assert-global --matrix xtask/release-matrix-v1.toml --registry xtask/scenarios-v1.toml --runner-lock xtask/runner-lock-v1.json --archive target/evidence/package-fan-in/package-fm-macos-arm64-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-macos-x86_64-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-ubuntu-x11-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-ubuntu-wayland-v1.tar.zst --archive target/evidence/package-fan-in/package-fm-fedora-wayland-v1.tar.zst
```

Expected: tool versions are exactly cargo-deny 0.20.2, cargo-audit 0.22.2, cargo-fuzz 0.13.2, and tokei 12.1.2. Each locked runner executes only its applicable feature-qualified app and matrix row; every command exits 0 and leaves the exact unique raw/archive artifacts. The one release global assertion reports five complete rows and executable-only `release-size`; the one package global assertion reports five hash-bound `package-size` -> installed-smoke chains and enforces every `.dmg`, `.deb`, and `.rpm` <=20 MiB before smoke. Security reports `http=0 https=0 dns=0 file=0 navigation=0 popup=0 download=0`; package verification proves final candidate/package hashes, install, non-IME smoke, and uninstall.

## Risks, Mitigations, and Stop Rules

| Risk | Mitigation / terminal rule |
|---|---|
| GTK/Wry native Wayland cannot meet lifecycle/focus/resize gates | XWayland cannot substitute. Stop after Task 1 and re-review a different native-Wayland shell while preserving the shared core. |
| Both macOS adapters fail production seam | Stop; no application shell is approved. Do not select the least-bad candidate. |
| Native widget mirror creates unacceptable copies | Allocation counter and 1/5 MiB gates force incremental adapter/custom viewport work; failure stops shell approval. |
| Full-page navigation misses preview latency | The one-transport spike measures it. Changing to DOM patches/evaluate-script requires a revised protocol ADR and full security re-review. |
| WebKit defeats lightweight RSS | Whole recursive process tree gate blocks Task 1/release; package size is never substituted. |
| Source mappings drift | Checked event stack, UTF-8 invariants, continuation anchors, deterministic formulas, property/fuzz/GUI tests. |
| Render pressure retains stale documents | One running/one pending scheduler, caps, stale disposal counters, 1,000-edit gate. |
| Preview authority expands | Typed nodes/URLs, exact protocol paths, no href/img, fixed assets, CSP, sentinels, independent security review. |
| Fork would actually own less code/risk | Bounded equivalent slice records comparable LOC/dependency/platform/security data; contrary evidence stops for review. |
| External change destroys work | Sole FileService and explicit Reload/Keep/Save As; no dirty auto-overwrite. |
| Distro/package drift | Exact Ubuntu/Fedora build/runtime rows and clean installed tests; baseline changes require ADR update and re-review. |

## ADR: BUILD a Shared Core; Require GTK/Wry for Linux and Spike-Approve macOS

**Decision:** Build FeatherMark's product-specific shared Rust core. Require a GTK3/GtkSourceView/Wry production seam on Linux so both X11 and native Wayland are real targets. Select iced or egui/eframe for macOS only from Task 1 evidence. Use Wry solely for complete revisioned HTML pages over one deny-by-default custom scheme.

**Drivers:** literal browser HTML/CSS; native macOS and native Linux Wayland; correct IME/UTF-8/history/viewport behavior; small measurable process footprint; typed preview security; narrow non-IDE scope.

**Alternatives considered:** adopt Ferrite unchanged; bounded Ferrite fork; port Marco; native-widget preview; single iced/egui+winit shell on Linux; Tauri/JS; XWayland as Linux support; dual GTK/AppKit hand-written shells; GTK Linux plus iced/egui macOS.

**Why chosen:** Existing editors miss the literal-browser or platform/security boundary. Wry's documented GTK-container path makes GTK the honest native-Wayland owner; pretending a direct winit child is cross-session would violate a hard requirement. The shared core keeps platform divergence at `EditorAdapter`/preview-host edges. Task 1 measures the full production seam and a Ferrite fork before the plan commits beyond the core.

**Consequences:** FeatherMark owns two platform adapters and system-webview lifecycle. Linux ships explicit GTK3/GtkSourceView/WebKitGTK dependencies. Native widgets may retain one incremental mirror. Complete-page navigation may impose a measurable latency floor. In return, the application has literal HTML without bundled Chromium, real native Wayland, a small shared core, and one auditable content transport.

**Follow-ups:** Task 1 ADR 0001 records shell evidence and can only approve through Architect -> Critic. ADR 0002 records release budgets. Raw HTML subsets, image loading, external link opening, custom CSS, Windows, more distros, and multiple documents each require a separate security/product ADR.

## Available Agent Types and Follow-Up Staffing

| Agent type | Lane | Reasoning level / independence |
|---|---|---|
| `rust-expert` / `executor` | Document, snapshot, history, typed renderer | High for byte/char, ownership, and bounded memory |
| `frontend-engineer` / `executor` | Linux GTK adapter or macOS adapter | High; one worker per platform after Task 1 only |
| `security-reviewer` | Protocol/CSP/typed HTML/sentinels | High; must not be the implementer |
| `test-engineer` | GUI-control, property, fuzz, fixtures | High for adversarial oracles |
| `performance-engineer` | Raw clocks, process trees, budgets | High for methodology; medium for plumbing |
| `architect` | Task 1 ADR and any fallback | High; mandatory before Critic |
| `critic` / `quality-reviewer` | Plan, Task 1, release evidence | High and independent |
| `verifier` | Clean command/package receipts | Medium; evidence only |

All delegated/background model selection routes through Pushing Dispatch. The leader owns the durable goal and stop boundaries.

## Team + Ultragoal Execution Handoff

**Current handoff:** none. Consensus is incomplete and execution is not authorized; do not invoke Ultragoal, Team, or any implementation worker from this artifact.

Only after a future Architect review and then Critic review both approve the same artifact SHA-256 may the default durable path become:

```text
$ultragoal ".omx/plans/feathermark-build-plan.md — execute Task 1 only; checkpoint raw seam/Ferrite evidence and stop for Architect then Critic approval of ADR 0001"
```

Task 1 is mostly sequential: one high-reasoning Rust/core worker, one Linux GTK worker, one macOS comparison worker, and independent performance/security evidence lanes. After `task_1_gate.complete=true`, parallel work is appropriate:

```text
omx team 4:executor "Implement approved Tasks 2-8 from .omx/plans/feathermark-build-plan.md. Split core/render, file/reducer, platform shell, and test/performance lanes. Return checkpoint-ready tests, raw metrics, hashes, and changed-file lists to the Ultragoal leader."
```

### Team Verification Path

1. Workers prove only their named task and never cross a stop gate.
2. Security reviewer reruns typed-output, protocol, CSP, and sentinel suites independently.
3. Performance engineer uploads raw JSON from all named runner classes; verifier recomputes percentiles and recursive RSS.
4. Verifier checks clean installs and the full command block from a clean checkout.
5. Ultragoal checkpoints planning consensus, Task 1 evidence approval, core complete, preview secure, package matrix complete, and release budgets before completion.

## Goal-Mode Follow-Up Suggestions

- `$ultragoal` becomes the default durable implementation path only after plan consensus because Task 1 and release are explicit sequential approval gates; it is not authorized by this best-available artifact.
- `$team` operates inside the Ultragoal-led effort only where lanes are independent; Task 1 Architect and Critic reviews remain sequential.
- `$performance-goal` is appropriate only for a bounded remediation plan after correctness/security pass but a metric fails.
- `$autoresearch-goal` is not the implementation path; use it only for a genuinely new external research question created by a stopped spike.
- `$ralph` remains an explicit user-selected single-owner fallback and cannot replace the durable ledger or independent reviews.

## Round-4 Review Resolution and Planner Self-Review

| Critic-r4 item | Revision-5 resolution |
|---|---|
| 1 | Assigned `feathermark-types` and `feathermark-protocol` exclusively to Task 1A everywhere; Task 1B owns only `feathermark-core` document/editor implementation |
| 2 | Made pre-package `release-size.json` stripped-executable-only and added hash-bound post-package `.dmg`/`.deb`/`.rpm` size evidence with <=20 MiB enforcement before smoke |
| 3 | Added five unique release archives plus one exact fan-in/global assertion, and repeated the same five-archive chain for package-size plus installed smoke |
| 4 | Pinned/installed/checked tokei 12.1.2, built locked release xtask before first use, removed runner placeholders, and supplied one closed five-runner capture/verify command |
| 5 | Defined `no_scroll = source_max_top == 0 || preview_max_y == 0` as the first branch in both mappings/oracles/grading/endpoints and covered symmetric plus both asymmetric short-document cases |

- Coverage: the five and only five Architect/Critic round-4 corrections map above; no product capability, platform, security authority, comparator rule, or prior hard gate changed.
- Ownership consistency: Task 1A creates types/protocol; Task 1B creates only core document/editor implementation; the dependency graph remains acyclic.
- Evidence consistency: release size is executable-only; package size is final-package/hash-bound and precedes smoke; both pipelines have five unique runner artifacts, exact fan-in, and one global assertion.
- Command consistency: tokei is pinned and checked, release xtask is built before invocation, no runner placeholder remains, and verification uses the closed five-runner identities.
- Geometry consistency: `no_scroll` precedes bottom/EOF and ordinal logic in both directions, including `(0,0)`, `(0,positive)`, and `(positive,0)` tests.
- Deferred-marker/fence scan: no unfinished marker, open runner label, obsolete matrix assertion, unclosed fence, ownership contradiction, package-bearing `release-size`, or pre-size package smoke remains.
- Changelog r5: applied exactly the five Architect/Critic round-4 corrections, preserved all other revision-4 decisions, and re-ran planner self-review; the plan remains planning-only and awaits a new Architect then Critic decision.

## Terminal Mechanical Cleanup After Round 5

- Critic r5 remained `ITERATE` after the five-iteration cap. No approval or consensus is claimed.
- Replaced the remaining bare comparator instruction with four exact `target/release/xtask` clock/isolation commands, each bound to its required lane and log/audit-log.
- Aligned all five installed-smoke package paths with the authoritative `target/packages/macos` and `target/packages/linux` build outputs.
- Added the durable RALPLAN decision record and machine-readable consensus handoff requirements, including the incomplete gate and explicit prohibition on execution.
- Self-review after cleanup: plan copies must be byte-identical; the final hash must match the handoff; the r5 Architect `SOUND` and Critic `ITERATE` statuses remain unchanged; and these mechanical corrections were not re-reviewed.
- Changelog terminal cleanup: corrected only the three Critic-r5 defects, published the best available plan at the iteration cap, and retained `ralplan_consensus_gate.complete=false` with `execution_authorized=false`.
