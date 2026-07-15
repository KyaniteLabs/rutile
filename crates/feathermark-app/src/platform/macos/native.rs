use std::borrow::Cow;
use std::collections::VecDeque;
use std::future::Future;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use feathermark_core::{
    ChangeSet, CompositionCancelReason, Counts, EditorAdapter, EditorEvent, FindDirection,
    FindQuery, FormatCommand, MatchMode, RecoveredDocument, ScrollClock, Selection,
    SessionWindowV1, html_to_markdown,
};
use feathermark_protocol::{PreviewEventV1, ProtocolError};
use iced_widget::text_editor;
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::program::Program;
use iced_winit::program::core;
use iced_winit::program::graphics;
use iced_winit::program::runtime::Task;
use iced_winit::program::runtime::UserInterface;
use iced_winit::program::runtime::user_interface;
use iced_winit::winit;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::NSString;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, ModifiersState};
use winit::window::{Window, WindowId};
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder};

use super::{
    AppKitMainThread, EditorVisualReceipt, IcedEditorAdapter, MacError, MacExternalOutcome,
    MacMenuCommand, MacSaveAction, MacScrollDispatch, MacUserEvent, PreviewIpcFatal,
    PreviewIpcIngress, ProductSession, bind_open_proxy, forward_open_urls,
    install_file_menu_with_actions, preview_ipc_channel, split_panes,
};
use crate::actions::SessionRestore;
use crate::app::{AppEffect, CloseDecision, CloseOutcome};
use crate::brand::{PRODUCT_NAME, SOURCE_EDITOR_LABEL, STARTER_DOCUMENT, status_title};
use crate::preview_host::{
    HostError, NavigationKind, PreviewControlSink, PreviewHost, SchemeRequest, SchemeResponse,
    ScrollDelivery,
};

const WINDOW_WIDTH: u32 = 1_000;
const WINDOW_HEIGHT: u32 = 720;
const SMOKE_TIMEOUT: Duration = Duration::from_secs(60);
const SMOKE_LIFECYCLE_CYCLES: u64 = 50;

/// Autosave cadence for the crash-recovery journal (SPEC §9). A dirty buffer is
/// journaled at most this often from `about_to_wait`.
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Debounced disk-version polling for external-change detection (MAC-005).
const DISK_POLL_INTERVAL: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// DESIGN-SYSTEM.md "specimen case" app-chrome tokens (committed 2026-07-10).
// Rutile-gold accent used at needle weight only; warm translucent selection;
// oatmeal-paper ground in light mode; full-contrast ink for content.
// The gold *caret* is deferred to Wave 3: iced 0.14 `text_editor::Style` has no
// independent caret color (the caret follows `value`), so a gold needle caret
// without staining the body text is not reachable in this shell yet.
// ---------------------------------------------------------------------------

/// Oatmeal paper ground, `#E9E2D6` (DESIGN-SYSTEM light-mode paper family).
const GROUND_LIGHT: (u8, u8, u8) = (233, 226, 214);
/// Full-contrast document ink, `#1C1C1A`.
const INK: core::Color = core::Color::from_rgb8(28, 28, 26);
/// Rutile-gold accent anchor, `#C9921E`.
const ACCENT_GOLD: core::Color = core::Color::from_rgb8(201, 146, 30);
/// Warm translucent gold selection tint (never OS blue).
const SELECTION_GOLD: core::Color = core::Color::from_rgba8(201, 146, 30, 0.28);
/// Receding muted ink for chrome labels (toolbar, status, syntax staining).
const MUTED_INK: core::Color = core::Color::from_rgba8(28, 28, 26, 0.60);
/// Toolbar/status chrome font size.
const CHROME_FONT_SIZE: f32 = 12.0;

pub(super) fn run_native(path: Option<PathBuf>, smoke: bool) -> Result<(), MacError> {
    AppKitMainThread::claim()?;
    if smoke {
        eprintln!("SMOKE_TRACE stage=0 event=launch");
    }
    let session = match path {
        Some(path) => ProductSession::open(&path)?,
        None => ProductSession::new_in_memory(STARTER_DOCUMENT)?,
    };
    let event_loop = EventLoop::<MacUserEvent>::with_user_event()
        .build()
        .map_err(|error| MacError::Native(error.to_string()))?;
    bind_open_proxy(event_loop.create_proxy());
    let display_handle = event_loop.owned_display_handle();
    let mut runner = ProductRunner::new(session, smoke, display_handle)?;
    event_loop
        .run_app(&mut runner)
        .map_err(|error| MacError::Native(error.to_string()))?;
    runner.finish()
}

struct ProductRunner {
    webview: Option<WebView>,
    web_context: Option<WebContext>,
    window: Option<Arc<Window>>,
    session: ProductSession,
    preview_host: Arc<Mutex<PreviewHost>>,
    ipc_receiver: Receiver<String>,
    ipc_ingress: PreviewIpcIngress,
    display_handle: OwnedDisplayHandle,
    source_pane: IcedSourcePane,
    initial_url: String,
    modifiers: ModifiersState,
    smoke: bool,
    smoke_stage: u8,
    smoke_path: Option<PathBuf>,
    deadline: Instant,
    failure: Option<String>,
    webview_first: bool,
    painted_revision: Option<u64>,
    smoke_resize_requested: bool,
    resize_proven: bool,
    preview_scroll_events: u64,
    editor_events: Arc<Mutex<VecDeque<EditorEvent>>>,
    pending_close: bool,
    started: Instant,
    preview_frame: u64,
    visibility_cycles: u64,
    compositor_resume_cycles: u64,
    // Wave 2M shell-integration state.
    find_bar: Option<FindBarView>,
    format_commands: Arc<Mutex<VecDeque<FormatCommand>>>,
    last_autosave: Instant,
    pending_recovery: Option<RecoveredDocument>,
    pending_restore: Option<SessionRestore>,
    last_disk_poll: Instant,
    menu_installed: bool,
}

impl ProductRunner {
    fn new(
        mut session: ProductSession,
        smoke: bool,
        display_handle: OwnedDisplayHandle,
    ) -> Result<Self, MacError> {
        let snapshot = session.snapshot();
        let preview_host = session.preview_host();
        let initial_url = session.render_now()?.url;
        // A bounded burst budget holds bridge-ready, paint, and scroll frames
        // from one navigation without dropping typed evidence under an
        // optimized event loop. Frames remain individually capped and parsed.
        let (ipc_ingress, ipc_receiver) = preview_ipc_channel(8);
        let editor_events = Arc::new(Mutex::new(VecDeque::new()));
        let mut source_pane = IcedSourcePane::new(&snapshot)?;
        source_pane.set_event_queue(Arc::clone(&editor_events));
        let format_commands = Arc::new(Mutex::new(VecDeque::new()));
        source_pane.set_format_sink(Arc::clone(&format_commands));
        source_pane.update_counts(session.counts());

        // Crash-recovery + session-restore setup (never in smoke, which owns a
        // deterministic proof flow and must not be perturbed by prior journals).
        let mut pending_recovery = None;
        let mut pending_restore = None;
        if !smoke
            && let Some(dir) = autosave_dir()
            && std::fs::create_dir_all(&dir).is_ok()
            && session.bind_autosave(dir).is_ok()
        {
            // Offer recovery only when the journal holds content that differs
            // from what actually loaded (otherwise every clean restart would
            // prompt). Journal rotation on clean exit is a Wave 3 refinement.
            pending_recovery =
                session.recover().ok().flatten().filter(|recovered| {
                    recovered.document.snapshot().to_string() != session.source()
                });
            if let Ok(Some(state)) = session.load_session_state() {
                pending_restore = Some(session.restore_session(&state));
            }
        }

        Ok(Self {
            webview: None,
            web_context: Some(WebContext::new(None)),
            window: None,
            session,
            preview_host,
            ipc_receiver,
            ipc_ingress,
            display_handle,
            source_pane,
            initial_url,
            modifiers: ModifiersState::empty(),
            smoke,
            smoke_stage: 0,
            smoke_path: smoke.then(|| {
                std::env::temp_dir().join(format!(
                    "feathermark-native-smoke-{}.md",
                    std::process::id()
                ))
            }),
            deadline: Instant::now() + SMOKE_TIMEOUT,
            failure: None,
            webview_first: false,
            painted_revision: None,
            smoke_resize_requested: false,
            resize_proven: false,
            preview_scroll_events: 0,
            editor_events,
            pending_close: false,
            started: Instant::now(),
            preview_frame: 0,
            visibility_cycles: 0,
            compositor_resume_cycles: 0,
            find_bar: None,
            format_commands,
            last_autosave: Instant::now(),
            pending_recovery,
            pending_restore,
            last_disk_poll: Instant::now(),
            menu_installed: false,
        })
    }

    fn render_clock_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    fn schedule_render(&mut self) -> Result<(), MacError> {
        self.session.pump_render_start(self.render_clock_ms())?;
        Ok(())
    }

    fn drain_render_completions(&mut self) -> Result<(), MacError> {
        while let Some(receipt) = self.session.pump_render_completions()? {
            if let Some(webview) = &self.webview {
                webview
                    .load_url(&receipt.url)
                    .map_err(|error| MacError::Native(error.to_string()))?;
            }
        }
        Ok(())
    }

    fn refresh_after_document_swap(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), MacError> {
        self.source_pane
            .replace(&self.session.snapshot())
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.source_pane.update_counts(self.session.counts());
        self.schedule_render()?;
        self.drain_render_completions()?;
        let _ = event_loop;
        Ok(())
    }

    fn handle_open_delivery(&mut self, event_loop: &ActiveEventLoop, urls: Vec<String>) {
        let request = ProductSession::classify_open_urls(&urls);
        if let Err(error) = self.session.handle_open_request(request) {
            self.fail(event_loop, error.to_string());
            return;
        }
        if let Err(error) = self.refresh_after_document_swap(event_loop) {
            self.fail(event_loop, error.to_string());
        }
    }

    fn handle_menu_command(&mut self, event_loop: &ActiveEventLoop, command: MacMenuCommand) {
        // Recovery / dirty-close decisions take priority over menu chrome.
        if self.pending_recovery.is_some() || self.pending_close {
            self.surface_error("Finish the pending decision before using the File menu");
            return;
        }
        match command {
            MacMenuCommand::Open => self.run_open_panel(event_loop),
            MacMenuCommand::Save => self.run_save(event_loop),
            MacMenuCommand::SaveAs => self.run_save_as_md(event_loop),
            MacMenuCommand::Close => {
                if self.session.app_state().dirty() {
                    self.pending_close = true;
                    self.sync_window_title();
                } else {
                    self.save_session_on_exit();
                    event_loop.exit();
                }
            }
        }
    }

    fn run_open_panel(&mut self, event_loop: &ActiveEventLoop) {
        match choose_open_path() {
            Ok(Some(path)) => {
                self.handle_open_delivery(event_loop, vec![path.display().to_string()])
            }
            Ok(None) => {}
            Err(error) => self.surface_error(error.to_string()),
        }
    }

    fn run_save(&mut self, event_loop: &ActiveEventLoop) {
        if self.session.has_external_conflict() {
            self.surface_error("Resolve the external file conflict before saving");
            return;
        }
        if let Err(MacError::ExternalConflict) =
            self.session.ensure_no_external_conflict_before_save()
        {
            self.surface_error("The file changed on disk; resolve the conflict before saving");
            return;
        }
        match self.session.request_save() {
            Ok(MacSaveAction::Completed) | Ok(MacSaveAction::Noop) => {
                self.sync_window_title();
            }
            Ok(MacSaveAction::NeedSaveAs) => self.run_save_as_md(event_loop),
            Err(error) => self.surface_error(format!("Save failed: {error}")),
        }
    }

    fn run_save_as_md(&mut self, _event_loop: &ActiveEventLoop) {
        let default = self
            .session
            .path()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.md".to_owned());
        match choose_save_path(&default) {
            Ok(Some(path)) => match self.session.request_save_as(path) {
                Ok(()) => self.sync_window_title(),
                Err(error) => self.surface_error(format!("Save As failed: {error}")),
            },
            Ok(None) => {}
            Err(error) => self.surface_error(error.to_string()),
        }
    }

    fn sync_window_title(&mut self) {
        if let Some(window) = &self.window {
            if let Some(notice) = self
                .session
                .app_state()
                .notices()
                .iter()
                .find(|notice| !notice.dismissed)
            {
                window.set_title(&status_title(&notice.message));
            } else if self.pending_close {
                window.set_title(&status_title(
                    "Unsaved changes: ⌘S Save · ⌘D Don’t Save · Esc Cancel",
                ));
            } else if self.pending_recovery.is_some() {
                window.set_title(&status_title(
                    "Recovered unsaved changes: ⌘Y Restore · Esc Dismiss",
                ));
            } else if self.session.has_external_conflict() {
                window.set_title(&status_title(
                    "External change detected: reload or save elsewhere",
                ));
            } else {
                window.set_title(PRODUCT_NAME);
            }
            window.request_redraw();
        }
    }

    fn maybe_poll_external_change(&mut self) {
        if self.smoke || self.last_disk_poll.elapsed() < DISK_POLL_INTERVAL {
            return;
        }
        self.last_disk_poll = Instant::now();
        match self.session.inspect_external_change() {
            Ok(MacExternalOutcome::Unchanged) => {}
            Ok(MacExternalOutcome::Reloaded { .. }) => {
                let _ = self.source_pane.replace(&self.session.snapshot());
                let _ = self.schedule_render();
                self.sync_window_title();
            }
            Ok(MacExternalOutcome::Conflict) => self.sync_window_title(),
            Err(error) => self.surface_error(format!("Disk check failed: {error}")),
        }
    }

    fn install_file_menu(&mut self) {
        if self.menu_installed {
            return;
        }
        if install_file_menu().is_ok() {
            self.menu_installed = true;
        }
    }

    fn process_editor_events(&mut self, event_loop: &ActiveEventLoop) -> Result<bool, MacError> {
        let events = {
            let mut queue = self
                .editor_events
                .lock()
                .map_err(|_| MacError::Native("editor event queue lock poisoned".into()))?;
            queue.drain(..).collect::<Vec<_>>()
        };
        // MAC-004: discard buffer mutations while a recovery decision is pending.
        // Viewport-only scroll is still allowed so the user can read the buffer.
        if self.pending_recovery.is_some() {
            for event in events {
                if let EditorEvent::ViewportChanged {
                    top_visible_byte,
                    user: true,
                    ..
                } = event
                    && let MacScrollDispatch::Preview {
                        revision,
                        source_start,
                        interaction_id,
                    } = self
                        .session
                        .source_scroll(top_visible_byte, self.scroll_clock())?
                {
                    self.deliver_preview_scroll(revision, source_start, interaction_id)?;
                }
            }
            let _ = event_loop;
            return Ok(false);
        }
        let mut edited = false;
        for event in events {
            match event {
                EditorEvent::ViewportChanged {
                    top_visible_byte,
                    user: true,
                    ..
                } => {
                    if let MacScrollDispatch::Preview {
                        revision,
                        source_start,
                        interaction_id,
                    } = self
                        .session
                        .source_scroll(top_visible_byte, self.scroll_clock())?
                    {
                        self.deliver_preview_scroll(revision, source_start, interaction_id)?;
                    }
                }
                event => {
                    if let Some((adapter_commit_id, change)) =
                        self.session.apply_editor_event(event)?
                    {
                        self.source_pane
                            .editor_mut()
                            .acknowledge_local_commit(adapter_commit_id, &change)
                            .map_err(|error| MacError::Core(error.to_string()))?;
                        edited = true;
                    }
                }
            }
        }
        if edited {
            self.schedule_render()?;
        }
        let _ = event_loop;
        Ok(edited)
    }

    fn scroll_clock(&self) -> ScrollClock {
        ScrollClock {
            monotonic_ms: self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            preview_frame: self.preview_frame,
        }
    }

    fn deliver_preview_scroll(
        &mut self,
        revision: u64,
        source_start: usize,
        interaction_id: u64,
    ) -> Result<(), MacError> {
        let webview = self
            .webview
            .as_ref()
            .ok_or_else(|| MacError::Native("preview WebView is not ready".into()))?;
        self.preview_host
            .lock()
            .map_err(|_| MacError::PreviewLock)?
            .deliver_scroll_to(
                &mut WebViewScrollSink(webview),
                revision,
                source_start,
                interaction_id,
            )?;
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        self.failure = Some(message.into());
        event_loop.exit();
    }

    fn surface_error(&self, message: impl AsRef<str>) {
        if let Some(window) = &self.window {
            window.set_title(&status_title(message.as_ref()));
            window.request_redraw();
        }
    }

    fn finish(&mut self) -> Result<(), MacError> {
        let editor_receipt = self.source_pane.visual_receipt();
        drop(self.webview.take());
        drop(self.web_context.take());
        self.source_pane.teardown();
        self.webview_first = self.webview.is_none() && self.window.is_some();
        drop(self.window.take());
        if let Some(path) = self.smoke_path.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(failure) = self.failure.take() {
            return Err(MacError::Native(failure));
        }
        if self.smoke {
            let editor_receipt = editor_receipt?;
            if self.smoke_stage != 2
                || !self.webview_first
                || !self.resize_proven
                || self.source_pane.focus_transfers == 0
                || self.preview_scroll_events == 0
                || self.visibility_cycles < SMOKE_LIFECYCLE_CYCLES
                || self.compositor_resume_cycles < SMOKE_LIFECYCLE_CYCLES
            {
                return Err(MacError::Native(format!(
                    "native smoke proof incomplete: stage={} webview_first={} resize_50_50={} focus_transfers={} preview_scroll_events={} visibility_cycles={} compositor_resume_cycles={} target={}",
                    self.smoke_stage,
                    self.webview_first,
                    self.resize_proven,
                    self.source_pane.focus_transfers,
                    self.preview_scroll_events,
                    self.visibility_cycles,
                    self.compositor_resume_cycles,
                    SMOKE_LIFECYCLE_CYCLES,
                )));
            }
            println!(
                "feathermark-native-smoke-ok editor_frames={} editor_ink_pixels={} ime_commits={} resize_50_50=true focus_transfers={} preview_scroll_events={} hide_show_cycles={} suspend_resume_cycles={} lifecycle_cycles={}",
                editor_receipt.presented_frames,
                editor_receipt.ink_pixels,
                editor_receipt.ime_commits,
                self.source_pane.focus_transfers,
                self.preview_scroll_events,
                self.visibility_cycles,
                self.compositor_resume_cycles,
                SMOKE_LIFECYCLE_CYCLES,
            );
        }
        Ok(())
    }

    fn render_and_navigate(&mut self) -> Result<(), MacError> {
        self.schedule_render()?;
        self.drain_render_completions()
    }

    // --- Wave 2M: native input routed to the shared action surface ----------

    /// Re-synchronizes the editor mirror, status counts, and preview after a
    /// shared (AppState-driven) document mutation — format, smart Enter, or a
    /// find/replace — installing `selection_after`.
    ///
    /// Follows the mutation incrementally by replaying its `changes` through the
    /// adapter's `apply_external_change` path (the same path external edits and
    /// undo/redo use), which preserves the viewport. A full `resync_to` is kept
    /// only as a correctness fallback for a `ChangeSet` that cannot be applied
    /// incrementally.
    fn after_shared_edit(
        &mut self,
        event_loop: &ActiveEventLoop,
        changes: &[ChangeSet],
        selection: Selection,
    ) {
        if let Err(error) = self.apply_shared_changes(changes, selection) {
            self.fail(event_loop, error.to_string());
            return;
        }
        self.source_pane.update_counts(self.session.counts());
        if let Err(error) = self.render_and_navigate() {
            self.fail(event_loop, error.to_string());
            return;
        }
        self.source_pane.request_redraw();
    }

    /// Replays the shared mutation's `changes` incrementally, then installs
    /// `selection`. Returns `Ok` when the incremental path (or its full-resync
    /// fallback) succeeds. The fallback recomputes the minimal diff between the
    /// current mirror and the authoritative snapshot, so it recovers correctly
    /// even from a partially-applied change sequence.
    fn apply_shared_changes(
        &mut self,
        changes: &[ChangeSet],
        selection: Selection,
    ) -> Result<(), MacError> {
        let applied_incrementally = !changes.is_empty() && {
            let editor = self.source_pane.editor_mut();
            let mut ok = true;
            for change in changes {
                if editor.apply_external_change(change).is_err() {
                    ok = false;
                    break;
                }
            }
            if let (true, Some(last)) = (ok, changes.last()) {
                ok = editor.set_selection(last.after, selection).is_ok();
            }
            ok
        };
        if applied_incrementally {
            return Ok(());
        }
        let snapshot = self.session.snapshot();
        self.source_pane
            .editor_mut()
            .resync_to(&snapshot, selection)
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Routes a [`FormatCommand`] (key binding or toolbar button) through
    /// [`ProductSession::apply_format`]. A clean engine rejection (e.g. an empty
    /// plan) becomes a status message, never a fatal error.
    fn run_format_command(&mut self, event_loop: &ActiveEventLoop, command: FormatCommand) {
        let selection = match self.source_pane.editor().current_selection() {
            Ok(selection) => selection,
            Err(error) => {
                self.surface_error(error.to_string());
                return;
            }
        };
        match self.session.apply_format(selection, command) {
            Ok(applied) => {
                self.after_shared_edit(event_loop, &applied.changes, applied.selection_after)
            }
            Err(error) => self.surface_error(error.to_string()),
        }
    }

    /// Routes Enter to [`ProductSession::smart_enter`].
    fn run_smart_enter(&mut self, event_loop: &ActiveEventLoop) {
        let selection = match self.source_pane.editor().current_selection() {
            Ok(selection) => selection,
            Err(error) => {
                self.surface_error(error.to_string());
                return;
            }
        };
        match self.session.smart_enter(selection) {
            Ok(applied) => {
                self.after_shared_edit(event_loop, &applied.changes, applied.selection_after)
            }
            Err(error) => self.surface_error(error.to_string()),
        }
    }

    /// Drains toolbar-emitted format commands (pushed from the Iced program).
    fn drain_format_commands(&mut self, event_loop: &ActiveEventLoop) {
        loop {
            let command = self
                .format_commands
                .lock()
                .ok()
                .and_then(|mut queue| queue.pop_front());
            match command {
                Some(command) => self.run_format_command(event_loop, command),
                None => break,
            }
        }
    }

    /// Smart paste: convert clipboard HTML (`public.html`) via the core
    /// `html_to_markdown` before insert, falling back to plain text when there
    /// is no HTML flavor or the conversion is rejected.
    fn run_smart_paste(&mut self, event_loop: &ActiveEventLoop) {
        let text = ProductSession::read_clipboard_paste_text()
            .ok()
            .and_then(|raw| {
                if raw.trim_start().starts_with('<') {
                    html_to_markdown(&raw).ok().or(Some(raw))
                } else {
                    Some(raw)
                }
            });
        let Some(text) = text else {
            self.surface_error("Paste failed: clipboard is empty or unavailable");
            return;
        };
        // Route the paste through the shared AppState insert primitive and follow
        // it incrementally (same path as format/replace), unifying the reducer
        // surface with Linux and preserving the viewport.
        let selection = match self.source_pane.editor().current_selection() {
            Ok(selection) => selection,
            Err(error) => {
                self.surface_error(error.to_string());
                return;
            }
        };
        match self.session.insert_text(selection, &text) {
            Ok(applied) => {
                self.after_shared_edit(event_loop, &applied.changes, applied.selection_after)
            }
            Err(error) => self.surface_error(error.to_string()),
        }
    }

    /// Save-as-HTML through a native `NSSavePanel`, using the validated
    /// self-contained export page.
    fn run_save_html(&mut self) {
        let default = self
            .session
            .path()
            .and_then(Path::file_stem)
            .map(|stem| format!("{}.html", stem.to_string_lossy()))
            .unwrap_or_else(|| "Untitled.html".to_owned());
        match choose_save_path(&default) {
            Ok(Some(path)) => match self.session.save_html_as(&path) {
                Ok(()) => self.surface_error(format!("Exported HTML to {}", path.display())),
                Err(error) => self.surface_error(format!("Export failed: {error}")),
            },
            Ok(None) => {}
            Err(error) => self.surface_error(error.to_string()),
        }
    }

    /// Copy-as-HTML onto the general pasteboard (HTML + plain flavors).
    fn run_copy_html(&mut self) {
        match self.session.export_html() {
            Ok(output) => match ProductSession::write_clipboard_html(&output.html) {
                Ok(()) => self.surface_error("Copied HTML to clipboard"),
                Err(error) => self.surface_error(format!("Copy failed: {error}")),
            },
            Err(error) => self.surface_error(format!("Copy HTML failed: {error}")),
        }
    }

    fn toggle_toolbar(&mut self) {
        let visible = self.source_pane.toggle_toolbar();
        self.surface_error(if visible {
            "Formatting toolbar shown"
        } else {
            "Formatting toolbar hidden"
        });
    }

    // --- find / replace bar -------------------------------------------------

    fn open_find(&mut self, replace_enabled: bool) {
        let seed = self
            .source_pane
            .editor()
            .current_selection()
            .ok()
            .filter(|selection| !selection.is_collapsed())
            .and_then(|selection| {
                let (start, end) = (
                    selection.anchor.min(selection.head),
                    selection.anchor.max(selection.head),
                );
                self.session.source().get(start..end).map(str::to_owned)
            });
        let mut bar = self.find_bar.take().unwrap_or_else(|| FindBarView {
            query: String::new(),
            replacement: String::new(),
            replace_enabled,
            focus: FindField::Query,
            status: String::new(),
        });
        if let Some(seed) = seed {
            bar.query = seed;
        }
        bar.replace_enabled = bar.replace_enabled || replace_enabled;
        bar.focus = FindField::Query;
        self.find_bar = Some(bar);
        self.refresh_search();
    }

    fn close_find(&mut self) {
        self.find_bar = None;
        self.session.end_find();
        self.source_pane.set_find_bar(None);
    }

    fn set_find_status(&mut self, status: impl Into<String>) {
        if let Some(bar) = self.find_bar.as_mut() {
            bar.status = status.into();
        }
    }

    fn push_find_display(&mut self) {
        self.source_pane.set_find_bar(self.find_bar.clone());
    }

    fn toggle_find_focus(&mut self) {
        if let Some(bar) = self.find_bar.as_mut()
            && bar.replace_enabled
        {
            bar.focus = match bar.focus {
                FindField::Query => FindField::Replace,
                FindField::Replace => FindField::Query,
            };
        }
        self.push_find_display();
    }

    fn find_input(&mut self, character: char) {
        let mut requery = false;
        if let Some(bar) = self.find_bar.as_mut() {
            match bar.focus {
                FindField::Query => {
                    bar.query.push(character);
                    requery = true;
                }
                FindField::Replace => bar.replacement.push(character),
            }
        }
        if requery {
            self.refresh_search();
        } else {
            self.push_find_display();
        }
    }

    fn find_backspace(&mut self) {
        let mut requery = false;
        if let Some(bar) = self.find_bar.as_mut() {
            match bar.focus {
                FindField::Query => {
                    bar.query.pop();
                    requery = true;
                }
                FindField::Replace => {
                    bar.replacement.pop();
                }
            }
        }
        if requery {
            self.refresh_search();
        } else {
            self.push_find_display();
        }
    }

    /// (Re)starts the find session from the current query and locates the first
    /// match from the caret.
    fn refresh_search(&mut self) {
        let query = self.find_bar.as_ref().map(|bar| bar.query.clone());
        let Some(query) = query else {
            return;
        };
        if query.is_empty() {
            self.session.end_find();
            self.set_find_status("Type to search");
            self.push_find_display();
            return;
        }
        let Ok(find_query) = FindQuery::new(query, MatchMode::Plain, false) else {
            self.set_find_status("Invalid search pattern");
            self.push_find_display();
            return;
        };
        self.session
            .start_find(find_query, FindDirection::Forward, true);
        let from = self.source_pane.editor().caret_byte().unwrap_or(0);
        self.locate_and_report(FindDirection::Forward, from);
    }

    /// Advances to the next/previous match relative to the current one.
    fn find_move(&mut self, direction: FindDirection) {
        if self.session.find_session().is_none() {
            self.refresh_search();
            return;
        }
        let current = self
            .session
            .find_session()
            .and_then(|session| session.current.clone());
        let from = match (&current, direction) {
            (Some(range), FindDirection::Forward) => range.start.saturating_add(1),
            (Some(range), FindDirection::Backward) => range.start.saturating_sub(1),
            (None, _) => self.source_pane.editor().caret_byte().unwrap_or(0),
        };
        self.locate_and_report(direction, from);
    }

    fn locate_and_report(&mut self, direction: FindDirection, from: usize) {
        let result = match direction {
            FindDirection::Forward => self.session.find_next(from),
            FindDirection::Backward => self.session.find_prev(from),
        };
        match result {
            Ok(Some(range)) => {
                self.highlight_match(&range);
                self.set_find_status(format!("Match at byte {}", range.start));
            }
            Ok(None) => self.set_find_status("No matches"),
            Err(error) => self.set_find_status(error.to_string()),
        }
        self.push_find_display();
    }

    fn highlight_match(&mut self, range: &Range<usize>) {
        let revision = self.session.snapshot().revision;
        let _ = self.source_pane.editor_mut().set_selection(
            revision,
            Selection {
                anchor: range.start,
                head: range.end,
            },
        );
        self.source_pane.request_redraw();
    }

    fn run_replace_current(&mut self, event_loop: &ActiveEventLoop) {
        let replacement = self.find_bar.as_ref().map(|bar| bar.replacement.clone());
        let Some(replacement) = replacement else {
            return;
        };
        match self.session.replace_current(replacement) {
            Ok(applied) => {
                if let Some(selection) = applied.selection_after {
                    self.after_shared_edit(event_loop, &applied.changes, selection);
                }
                self.find_move(FindDirection::Forward);
            }
            Err(error) => {
                self.set_find_status(error.to_string());
                self.push_find_display();
            }
        }
    }

    fn run_replace_all(&mut self, event_loop: &ActiveEventLoop) {
        let replacement = self.find_bar.as_ref().map(|bar| bar.replacement.clone());
        let Some(replacement) = replacement else {
            return;
        };
        match self.session.replace_all(replacement) {
            Ok(applied) => {
                if let Some(selection) = applied.selection_after {
                    self.after_shared_edit(event_loop, &applied.changes, selection);
                }
                self.set_find_status(format!("Replaced {} match(es)", applied.replaced));
            }
            Err(error) => self.set_find_status(error.to_string()),
        }
        self.push_find_display();
    }

    /// Handles a keystroke while the find bar is open. Returns `true` when the
    /// keystroke was consumed by the bar (so the editor must not also see it).
    fn handle_find_key(
        &mut self,
        event_loop: &ActiveEventLoop,
        key_event: &winit::event::KeyEvent,
        command: bool,
        shift: bool,
    ) -> bool {
        use winit::keyboard::NamedKey;
        if self.find_bar.is_none() {
            return false;
        }
        if command {
            return match &key_event.logical_key {
                Key::Named(NamedKey::Enter) => {
                    self.run_replace_all(event_loop);
                    true
                }
                Key::Character(character) if character.eq_ignore_ascii_case("g") => {
                    self.find_move(if shift {
                        FindDirection::Backward
                    } else {
                        FindDirection::Forward
                    });
                    true
                }
                // Let other command bindings (Cmd+F/Cmd+Shift+F/Cmd+S/…) run.
                _ => false,
            };
        }
        match &key_event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.close_find();
                true
            }
            Key::Named(NamedKey::Enter) => {
                let focus = self.find_bar.as_ref().map(|bar| bar.focus);
                if shift {
                    self.find_move(FindDirection::Backward);
                } else if focus == Some(FindField::Replace) {
                    self.run_replace_current(event_loop);
                } else {
                    self.find_move(FindDirection::Forward);
                }
                true
            }
            Key::Named(NamedKey::Tab) => {
                self.toggle_find_focus();
                true
            }
            Key::Named(NamedKey::Backspace) => {
                self.find_backspace();
                true
            }
            Key::Named(NamedKey::Space) => {
                self.find_input(' ');
                true
            }
            Key::Character(text) => {
                for character in text.chars() {
                    self.find_input(character);
                }
                true
            }
            _ => false,
        }
    }

    // --- autosave / recovery / session restore ------------------------------

    fn maybe_autosave(&mut self) {
        if self.smoke {
            return;
        }
        if self.session.app_state().dirty() && self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
            if let Err(error) = self.session.autosave_tick(unix_ms_now()) {
                self.surface_error(format!("Autosave failed: {error}"));
            }
            self.last_autosave = Instant::now();
        }
    }

    fn adopt_recovery(&mut self, event_loop: &ActiveEventLoop) {
        let Some(recovered) = self.pending_recovery.take() else {
            return;
        };
        if let Err(error) = self.session.adopt_recovered(recovered) {
            self.fail(event_loop, error.to_string());
            return;
        }
        if let Err(error) = self.source_pane.replace(&self.session.snapshot()) {
            self.fail(event_loop, error.to_string());
            return;
        }
        self.source_pane.update_counts(self.session.counts());
        if let Err(error) = self.render_and_navigate() {
            self.fail(event_loop, error.to_string());
            return;
        }
        if let Some(window) = &self.window {
            window.set_title(&status_title("Recovered unsaved changes"));
        }
    }

    fn dismiss_recovery(&mut self) {
        self.pending_recovery = None;
        if let Some(window) = &self.window {
            window.set_title(PRODUCT_NAME);
        }
    }

    fn apply_restore(&mut self, event_loop: &ActiveEventLoop) {
        let Some(restore) = self.pending_restore.take() else {
            return;
        };
        if let (Some(window), Some(frame)) = (&self.window, restore.window) {
            let _ =
                window.request_inner_size(winit::dpi::LogicalSize::new(frame.width, frame.height));
            window.set_outer_position(winit::dpi::LogicalPosition::new(frame.x, frame.y));
        }
        let selection = restore.selection;
        let viewport = restore.top_visible_byte;
        match self.session.apply_session_restore(&restore) {
            Ok(report) => {
                if report.opened_last_file {
                    if let Err(error) = self.refresh_after_document_swap(event_loop) {
                        self.fail(event_loop, error.to_string());
                        return;
                    }
                }
                if report.selection_applied {
                    if let Some(selection) = selection {
                        let revision = self.session.snapshot().revision;
                        let _ = self
                            .source_pane
                            .editor_mut()
                            .set_selection(revision, selection);
                    }
                }
                if report.viewport_applied {
                    if let Some(top) = viewport {
                        let revision = self.session.snapshot().revision;
                        let _ = self
                            .source_pane
                            .editor_mut()
                            .scroll_to_byte(revision, top, revision);
                    }
                }
                for notice in report.notices {
                    self.surface_error(notice.message);
                }
                self.sync_window_title();
            }
            Err(error) => self.surface_error(format!("Session restore failed: {error}")),
        }
    }

    fn save_session_on_exit(&mut self) {
        if self.smoke {
            return;
        }
        let revision = self.session.snapshot().revision;
        let selection = self.source_pane.editor().current_selection().ok();
        let top_visible_byte = self.source_pane.editor().top_visible_byte(revision).ok();
        let window = self.window.as_ref().map(|window| {
            let position = window.outer_position().ok();
            let size = window.inner_size();
            SessionWindowV1 {
                x: position.map(|p| p.x).unwrap_or(0),
                y: position.map(|p| p.y).unwrap_or(0),
                width: size.width,
                height: size.height,
            }
        });
        let state =
            self.session
                .capture_session_state(unix_ms_now(), selection, top_visible_byte, window);
        let _ = self.session.save_session_state(&state);
    }

    fn process_preview_event(&mut self, event_loop: &ActiveEventLoop, event: PreviewEventV1) {
        if matches!(&event, PreviewEventV1::Scroll { .. }) {
            self.preview_scroll_events = self.preview_scroll_events.saturating_add(1);
        }
        let painted = matches!(&event, PreviewEventV1::Painted { .. });
        if let PreviewEventV1::Painted { frame_seq, .. } = &event {
            self.preview_frame = *frame_seq;
        }
        let revision = match &event {
            PreviewEventV1::BridgeReady { revision }
            | PreviewEventV1::Painted { revision, .. }
            | PreviewEventV1::Scroll { revision, .. }
            | PreviewEventV1::LinkActivated { revision, .. } => *revision,
        };
        if matches!(&event, PreviewEventV1::BridgeReady { .. }) {
            let delivery = self
                .source_pane
                .editor_mut()
                .top_visible_byte(revision)
                .map_err(|error| HostError::Platform(error.to_string()))
                .and_then(|source_start| {
                    self.deliver_preview_scroll(revision, source_start, revision)
                        .map_err(|error| HostError::Platform(error.to_string()))
                });
            if let Err(error) = delivery {
                self.fail(event_loop, error.to_string());
                return;
            }
        }
        if let PreviewEventV1::Scroll {
            source_start,
            interaction_id,
            user,
            ..
        } = &event
        {
            match self.session.preview_scroll(
                *source_start,
                *interaction_id,
                *user,
                self.scroll_clock(),
            ) {
                Ok(MacScrollDispatch::Source {
                    revision,
                    source_start,
                    interaction_id,
                }) => {
                    if let Err(error) = self.source_pane.editor_mut().scroll_to_byte(
                        revision,
                        source_start,
                        interaction_id,
                    ) {
                        self.fail(event_loop, error.to_string());
                        return;
                    }
                    self.source_pane.request_redraw();
                }
                Ok(MacScrollDispatch::Suppressed) => {}
                Ok(MacScrollDispatch::Preview { .. }) => {
                    self.fail(event_loop, "preview scroll mapped to the wrong pane");
                    return;
                }
                Err(error) => {
                    self.fail(event_loop, error.to_string());
                    return;
                }
            }
        }
        for effect in self.session.accept_preview_event(event) {
            if let AppEffect::ScrollSource { .. } = effect {
                self.source_pane.request_redraw();
            }
        }

        if painted {
            self.painted_revision = Some(revision);
        }
        self.advance_smoke(event_loop);
    }

    fn advance_smoke(&mut self, event_loop: &ActiveEventLoop) {
        if !self.smoke {
            return;
        }
        if self.smoke_stage == 0
            && smoke_stage_zero_ready_to_edit(
                self.painted_revision,
                self.source_pane.presented_frames,
                self.preview_scroll_events,
            )
        {
            while self.visibility_cycles < SMOKE_LIFECYCLE_CYCLES {
                if let Some(webview) = &self.webview {
                    if let Err(error) = webview
                        .set_visible(false)
                        .and_then(|_| webview.set_visible(true))
                    {
                        self.fail(event_loop, error.to_string());
                        return;
                    }
                }
                if let Some(window) = &self.window {
                    window.set_visible(false);
                    window.set_visible(true);
                }
                self.visibility_cycles = self.visibility_cycles.saturating_add(1);
            }
            while self.compositor_resume_cycles < SMOKE_LIFECYCLE_CYCLES {
                let Some(window) = self.window.clone() else {
                    self.fail(event_loop, "native smoke window is missing");
                    return;
                };
                self.source_pane.teardown();
                if let Err(error) = self
                    .source_pane
                    .attach_window(window, self.display_handle.clone())
                {
                    self.fail(event_loop, error.to_string());
                    return;
                }
                self.compositor_resume_cycles = self.compositor_resume_cycles.saturating_add(1);
            }
            self.source_pane.request_redraw();
            if !self.smoke_resize_requested {
                self.smoke_resize_requested = true;
                if let Some(window) = &self.window {
                    eprintln!(
                        "SMOKE_TRACE stage={} event=resize-request logical=1200x760",
                        self.smoke_stage
                    );
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(1_200, 760));
                }
                return;
            }
            if !self.resize_proven {
                return;
            }
            if let Err(error) = self
                .source_pane
                .editor_mut()
                .perform(text_editor::Action::SelectAll)
                .and_then(|_| {
                    self.source_pane
                        .editor_mut()
                        .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                            Arc::new("# Native smoke 🪶\n\n".to_owned()),
                        )))
                })
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            if let Err(error) = self.process_editor_events(event_loop) {
                self.fail(event_loop, error.to_string());
                return;
            }
            match self.source_pane.commit_ime_for_smoke("Edited.\n") {
                Ok(true) if self.source_pane.text() == "# Native smoke 🪶\n\nEdited.\n" => {}
                Ok(changed) => {
                    self.fail(
                        event_loop,
                        format!(
                            "Iced IME commit receipt mismatch: changed={changed} source={:?}",
                            self.source_pane.text()
                        ),
                    );
                    return;
                }
                Err(error) => {
                    self.fail(event_loop, error.to_string());
                    return;
                }
            }
            if let Err(error) = self.process_editor_events(event_loop) {
                self.fail(event_loop, error.to_string());
                return;
            }
            let Some(undo) = self.session.undo() else {
                self.fail(event_loop, "native smoke undo had no effect");
                return;
            };
            if let Err(error) = self
                .source_pane
                .editor_mut()
                .apply_external_change(&undo)
                .map_err(|error| MacError::Core(error.to_string()))
                .and_then(|_| self.render_and_navigate())
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            let Some(redo) = self.session.redo() else {
                self.fail(event_loop, "native smoke redo had no effect");
                return;
            };
            if let Err(error) = self
                .source_pane
                .editor_mut()
                .apply_external_change(&redo)
                .map_err(|error| MacError::Core(error.to_string()))
                .and_then(|_| self.render_and_navigate())
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            self.smoke_stage = 1;
            eprintln!("SMOKE_TRACE stage=1 event=edited");
        } else if self.smoke_stage == 1
            && self.painted_revision == Some(self.session.snapshot().revision)
            && self.source_pane.presented_text == "# Native smoke 🪶\n\nEdited.\n"
        {
            if let Err(error) = self.source_pane.visual_receipt() {
                self.fail(event_loop, error.to_string());
                return;
            }
            let Some(path) = self.smoke_path.clone() else {
                self.fail(event_loop, "native smoke path is missing");
                return;
            };
            if let Err(error) = self.session.save_as(&path) {
                self.fail(event_loop, error.to_string());
                return;
            }
            match ProductSession::open(&path) {
                Ok(reopened) if reopened.source() == "# Native smoke 🪶\n\nEdited.\n" => {
                    self.session = reopened;
                    self.smoke_stage = 2;
                    eprintln!("SMOKE_TRACE stage=2 event=save-reopen");
                    event_loop.exit();
                }
                Ok(_) => self.fail(event_loop, "reopened source did not match saved UTF-8"),
                Err(error) => self.fail(event_loop, error.to_string()),
            }
        }
    }
}

/// Stage-zero readiness for the supervised macOS smoke: the flow must not
/// perform its first edit until the preview has painted revision 0, the source
/// editor has presented at least one frame, and the preview has echoed back a
/// scroll receipt (`preview_scroll_events > 0`). Pure over plain integers so
/// the invariant can be unit-tested without a live compositor or window.
fn smoke_stage_zero_ready_to_edit(
    painted_revision: Option<u64>,
    presented_frames: u64,
    preview_scroll_events: u64,
) -> bool {
    painted_revision == Some(0) && presented_frames > 0 && preview_scroll_events > 0
}

impl ApplicationHandler<MacUserEvent> for ProductRunner {
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: MacUserEvent) {
        match event {
            MacUserEvent::OpenUrls(urls) => self.handle_open_delivery(event_loop, urls),
            MacUserEvent::MenuCommand(command) => self.handle_menu_command(event_loop, command),
        }
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if self.smoke {
            eprintln!("SMOKE_TRACE stage=0 event=resumed");
        }
        let attributes = Window::default_attributes()
            .with_title(PRODUCT_NAME)
            .with_inner_size(winit::dpi::LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        if let Err(error) = self
            .source_pane
            .attach_window(Arc::clone(&window), self.display_handle.clone())
        {
            self.fail(event_loop, error.to_string());
            return;
        }
        let physical = window.inner_size();
        let logical = physical.to_logical::<u32>(window.scale_factor());
        let panes = split_panes(logical.width, logical.height);

        let protocol_host = Arc::clone(&self.preview_host);
        let navigation_host = Arc::clone(&self.preview_host);
        let download_host = Arc::clone(&self.preview_host);
        let new_window_host = Arc::clone(&self.preview_host);
        let ipc_ingress = self.ipc_ingress.clone();
        let Some(web_context) = self.web_context.as_mut() else {
            self.fail(event_loop, "preview WebContext is missing");
            return;
        };
        let builder = WebViewBuilder::new_with_web_context(web_context)
            .with_bounds(preview_bounds(panes))
            .with_custom_protocol("feathermark".to_owned(), move |_id, request| {
                let scheme_request =
                    SchemeRequest::new(request.method().as_str(), request.uri().to_string());
                let response = protocol_host
                    .lock()
                    .map(|host| host.serve(&scheme_request))
                    .unwrap_or_else(|_| SchemeResponse {
                        status: 500,
                        headers: vec![],
                        body: Arc::from([]),
                    });
                to_wry_response(response)
            })
            .with_navigation_handler(move |url| {
                navigation_host
                    .lock()
                    .map(|mut host| host.allow_navigation(&url, NavigationKind::AppInitiated))
                    .unwrap_or(false)
            })
            .with_download_started_handler(move |url, _| {
                download_host
                    .lock()
                    .map(|host| host.allow_download(&url))
                    .unwrap_or(false)
            })
            .with_new_window_req_handler(move |url, _| {
                if new_window_host
                    .lock()
                    .map(|host| host.allow_new_window(&url))
                    .unwrap_or(false)
                {
                    NewWindowResponse::Allow
                } else {
                    NewWindowResponse::Deny
                }
            })
            .with_ipc_handler(move |request| {
                ipc_ingress.try_push(request.body().clone());
            })
            .with_url(&self.initial_url);

        match builder.build_as_child(window.as_ref()) {
            Ok(webview) => {
                self.webview = Some(webview);
                self.window = Some(window);
                self.install_file_menu();
                self.apply_restore(event_loop);
                if self.pending_recovery.is_some()
                    && let Some(window) = &self.window
                {
                    window.set_title(&status_title(
                        "Recovered unsaved changes: ⌘Y Restore · Esc Dismiss",
                    ));
                }
                if self.smoke {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
                } else {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + AUTOSAVE_INTERVAL,
                    ));
                }
            }
            Err(error) => self.fail(event_loop, error.to_string()),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if matches!(&event, WindowEvent::CloseRequested) {
            if self.session.app_state().dirty() {
                self.pending_close = true;
                self.sync_window_title();
            } else {
                self.save_session_on_exit();
                event_loop.exit();
            }
            return;
        }

        if let WindowEvent::DroppedFile(path) = &event {
            let delivery = vec![path.display().to_string()];
            if forward_open_urls(delivery.clone()).is_err() {
                self.handle_open_delivery(event_loop, delivery);
            }
            return;
        }

        if let WindowEvent::Resized(size) = &event
            && size.width > 0
            && size.height > 0
            && let Some(window) = &self.window
        {
            let scale_factor = window.scale_factor();
            let logical = size.to_logical::<u32>(scale_factor);
            let panes = split_panes(logical.width, logical.height);
            self.source_pane.resize(
                panes.source_width,
                size.width,
                size.height,
                scale_factor as f32,
            );
            if let Some(webview) = &self.webview
                && let Err(error) = webview.set_bounds(preview_bounds(panes))
            {
                self.fail(event_loop, error.to_string());
                return;
            }
            if self.smoke
                && self.smoke_resize_requested
                && logical.width == 1_200
                && panes.source_width == 600
                && panes.preview_x == 600
                && panes.preview_width == 600
                && (self.source_pane.state.pane_width - 600.0).abs() < f32::EPSILON
            {
                self.resize_proven = true;
                eprintln!(
                    "SMOKE_TRACE stage={} event=resize logical={}x{} source_width={} preview_width={}",
                    self.smoke_stage,
                    logical.width,
                    logical.height,
                    panes.source_width,
                    panes.preview_width,
                );
            }
        }

        if matches!(&event, WindowEvent::Focused(true))
            && let Err(error) = self.source_pane.focus_editor()
        {
            self.fail(event_loop, error.to_string());
            return;
        }

        if let WindowEvent::ModifiersChanged(modifiers) = &event {
            self.modifiers = modifiers.state();
        }

        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
        {
            let command = self.modifiers.super_key();
            if self.pending_close {
                if matches!(
                    key_event.logical_key,
                    Key::Named(winit::keyboard::NamedKey::Escape)
                ) {
                    let _ = self.session.request_close(CloseDecision::Cancel);
                    self.pending_close = false;
                    self.sync_window_title();
                    return;
                }
                if command
                    && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("d"))
                {
                    match self.session.request_close(CloseDecision::Discard) {
                        Ok(CloseOutcome::Close) => {
                            self.save_session_on_exit();
                            event_loop.exit();
                        }
                        Ok(CloseOutcome::KeepOpen) => {
                            self.pending_close = false;
                            self.sync_window_title();
                        }
                        Err(error) => self.surface_error(error.to_string()),
                    }
                    return;
                }
                if command
                    && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("s"))
                {
                    let untitled_path = if self.session.path().is_none() {
                        match choose_save_path("Untitled.md") {
                            Ok(Some(path)) => Some(path),
                            Ok(None) => {
                                let _ = self.session.request_close(CloseDecision::Cancel);
                                self.pending_close = false;
                                self.sync_window_title();
                                return;
                            }
                            Err(error) => {
                                self.surface_error(error.to_string());
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    match self
                        .session
                        .request_close(CloseDecision::Save { untitled_path })
                    {
                        Ok(CloseOutcome::Close) => {
                            self.save_session_on_exit();
                            event_loop.exit();
                        }
                        Ok(CloseOutcome::KeepOpen) => {
                            self.pending_close = false;
                            self.sync_window_title();
                        }
                        Err(error) => self.surface_error(format!("Save failed: {error}")),
                    }
                    return;
                }
            }
            let shift = self.modifiers.shift_key();

            // Crash-recovery prompt (non-smoke): ⌘Y restore · Esc dismiss.
            // While pending, all other key-driven edits and commands are blocked
            // (MAC-004): the only accepted keys are the decision keys.
            if self.pending_recovery.is_some() {
                if command
                    && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("y"))
                {
                    self.adopt_recovery(event_loop);
                    return;
                }
                if matches!(
                    key_event.logical_key,
                    Key::Named(winit::keyboard::NamedKey::Escape)
                ) {
                    self.dismiss_recovery();
                    return;
                }
                return;
            }

            // Find/replace bar captures non-command keys while open.
            if self.find_bar.is_some()
                && self.handle_find_key(event_loop, key_event, command, shift)
            {
                self.source_pane.request_redraw();
                return;
            }

            // Open document (⌘O).
            if command
                && !shift
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("o"))
            {
                self.run_open_panel(event_loop);
                return;
            }

            // Open find (⌘F) / open find+replace (⌘⇧F).
            if command
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("f"))
            {
                self.open_find(shift);
                return;
            }
            // Find next (⌘G) / find previous (⌘⇧G).
            if command
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("g"))
            {
                self.find_move(if shift {
                    FindDirection::Backward
                } else {
                    FindDirection::Forward
                });
                return;
            }

            // Smart paste (⌘V): clipboard HTML → Markdown, else plain text.
            if command
                && !shift
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("v"))
            {
                self.run_smart_paste(event_loop);
                return;
            }

            // Save-as-HTML (⌘⇧S) and Copy-as-HTML (⌘⇧C).
            if command
                && shift
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("s"))
            {
                self.run_save_html();
                return;
            }
            if command
                && shift
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("c"))
            {
                self.run_copy_html();
                return;
            }

            // Toggle the formatting toolbar (⌘⇧T; default off).
            if command
                && shift
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("t"))
            {
                self.toggle_toolbar();
                return;
            }

            // Format commands: ⌘B/I/K/E and ⌘⇧K/H/B/L/O/X.
            if let Some(format_command) = format_command_for(&key_event.logical_key, command, shift)
            {
                self.run_format_command(event_loop, format_command);
                return;
            }

            // Smart Enter routes plain Enter through the format engine (not while
            // an IME composition is in flight — that Enter confirms the preedit).
            if !command
                && matches!(
                    key_event.logical_key,
                    Key::Named(winit::keyboard::NamedKey::Enter)
                )
                && !self.source_pane.editor().is_composing()
            {
                self.run_smart_enter(event_loop);
                return;
            }

            if command
                && !shift
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("s"))
            {
                self.run_save(event_loop);
                return;
            }
            if command
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("r"))
            {
                if self.session.path().is_some() {
                    match self.session.reload() {
                        Ok(()) => {
                            if let Err(error) = self.refresh_after_document_swap(event_loop) {
                                self.fail(event_loop, error.to_string());
                            }
                        }
                        Err(error) => self.surface_error(error.to_string()),
                    }
                }
                return;
            }
            if command
                && matches!(key_event.logical_key, Key::Character(ref key) if key.eq_ignore_ascii_case("z"))
            {
                let redo = self.modifiers.shift_key();
                let change = if redo {
                    self.session.redo()
                } else {
                    self.session.undo()
                };
                if let Some(change) = change {
                    if let Err(error) = self
                        .source_pane
                        .editor_mut()
                        .apply_external_change(&change)
                        .map_err(|error| MacError::Core(error.to_string()))
                        .and_then(|_| self.render_and_navigate())
                    {
                        self.fail(event_loop, error.to_string());
                    }
                }
                return;
            }
        }

        if matches!(&event, WindowEvent::RedrawRequested) {
            if let Err(error) = self.drain_render_completions() {
                self.fail(event_loop, error.to_string());
                return;
            }
            if let Err(error) = self.source_pane.draw_and_present() {
                self.fail(event_loop, error.to_string());
                return;
            }
            self.advance_smoke(event_loop);
            return;
        }

        let Some(window) = &self.window else {
            return;
        };
        match self.source_pane.handle_winit_event(
            event,
            window.scale_factor() as f32,
            self.modifiers,
        ) {
            Ok(true) => {
                if let Err(error) = self.process_editor_events(event_loop) {
                    self.fail(event_loop, error.to_string());
                }
            }
            Ok(false) => {}
            Err(error) => self.fail(event_loop, error.to_string()),
        }
        // Toolbar button presses surface as queued format commands, not editor
        // edits, so drain them after every processed event.
        self.drain_format_commands(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(fatal) = self.ipc_ingress.take_fatal() {
            let message = match fatal {
                PreviewIpcFatal::RequiredFrameLost => {
                    "required preview IPC frame was lost to backpressure"
                }
                PreviewIpcFatal::Disconnected => "preview IPC ingress disconnected",
            };
            self.fail(event_loop, message);
            return;
        }
        loop {
            match self.ipc_receiver.try_recv() {
                Ok(frame) => {
                    let event = self
                        .preview_host
                        .lock()
                        .map_err(|_| HostError::Platform("preview host lock poisoned".into()))
                        .and_then(|host| host.handle_ipc(frame.as_bytes()));
                    match event {
                        Ok(event) => self.process_preview_event(event_loop, event),
                        Err(
                            HostError::NoLoadedDocument
                            | HostError::Protocol(ProtocolError::StaleRevision),
                        ) => continue,
                        Err(error) => {
                            self.fail(event_loop, error.to_string());
                            return;
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.fail(event_loop, "preview IPC channel disconnected");
                    return;
                }
            }
        }
        if let Err(error) = self.drain_render_completions() {
            self.fail(event_loop, error.to_string());
            return;
        }
        self.maybe_autosave();
        self.maybe_poll_external_change();
        self.sync_window_title();
        if !self.smoke {
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + AUTOSAVE_INTERVAL));
        }
        if self.smoke && Instant::now() >= self.deadline {
            self.fail(
                event_loop,
                format!(
                    "native product smoke timed out: stage={} painted={:?} frames={} resize_requested={} resize_proven={} visibility={} resume={}",
                    self.smoke_stage,
                    self.painted_revision,
                    self.source_pane.presented_frames,
                    self.smoke_resize_requested,
                    self.resize_proven,
                    self.visibility_cycles,
                    self.compositor_resume_cycles,
                ),
            );
        }
    }
}

struct WebViewScrollSink<'a>(&'a WebView);

impl PreviewControlSink for WebViewScrollSink<'_> {
    fn deliver_scroll_to(&mut self, delivery: ScrollDelivery) -> Result<(), HostError> {
        let frame = std::str::from_utf8(delivery.as_bytes())
            .map_err(|error| HostError::Platform(error.to_string()))?;
        let argument =
            serde_json::to_string(frame).map_err(|error| HostError::Platform(error.to_string()))?;
        self.0
            .evaluate_script(&format!("window.__feathermarkReceiveScrollTo({argument})"))
            .map_err(|error| HostError::Platform(error.to_string()))
    }
}

fn preview_bounds(panes: super::PaneBounds) -> Rect {
    Rect {
        position: wry::dpi::LogicalPosition::new(panes.preview_x, 0).into(),
        size: wry::dpi::LogicalSize::new(panes.preview_width, panes.height).into(),
    }
}

fn to_wry_response(response: SchemeResponse) -> Response<Cow<'static, [u8]>> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Cow::Owned(response.body.to_vec()))
        .expect("typed preview response headers are valid")
}

/// Which field of the find bar has keyboard focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FindField {
    Query,
    Replace,
}

/// The rendered state of the find/replace bar (a projection of the runner's
/// live [`crate::actions::FindSession`] plus its editable buffers).
#[derive(Clone, Debug)]
struct FindBarView {
    query: String,
    replacement: String,
    replace_enabled: bool,
    focus: FindField,
    status: String,
}

#[derive(Default)]
struct SourceProgram;

struct SourceState {
    editor: IcedEditorAdapter,
    error: Option<String>,
    window: Option<Arc<Window>>,
    pane_width: f32,
    toolbar_visible: bool,
    counts: Counts,
    reading_seconds: u64,
    find_bar: Option<FindBarView>,
    format_sink: Option<Arc<Mutex<VecDeque<FormatCommand>>>>,
}

#[derive(Clone, Debug)]
enum SourceMessage {
    Edit(text_editor::Action),
    Format(FormatCommand),
}

/// Text-only, borderless toolbar button per DESIGN-SYSTEM ("text-only,
/// borderless buttons, muted ink, needle-underline hover. No icons in chrome").
fn chrome_button_style(
    _theme: &core::theme::Theme,
    status: iced_widget::button::Status,
) -> iced_widget::button::Style {
    use iced_widget::button::Status;
    let text_color = match status {
        Status::Hovered | Status::Pressed => ACCENT_GOLD,
        _ => MUTED_INK,
    };
    iced_widget::button::Style {
        background: None,
        text_color,
        border: core::Border::default(),
        shadow: core::Shadow::default(),
        snap: false,
    }
}

fn editor_style(_theme: &core::theme::Theme, _status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: core::Background::Color(core::Color::from_rgb8(
            GROUND_LIGHT.0,
            GROUND_LIGHT.1,
            GROUND_LIGHT.2,
        )),
        border: core::Border::default(),
        placeholder: MUTED_INK,
        value: INK,
        selection: SELECTION_GOLD,
    }
}

/// Toolbar entries: (label, command). Text-only labels, no icons (LD-3 chrome).
const TOOLBAR_ITEMS: &[(&str, FormatCommand)] = &[
    ("Bold", FormatCommand::ToggleBold),
    ("Italic", FormatCommand::ToggleItalic),
    ("Code", FormatCommand::ToggleCodeSpan),
    ("Heading", FormatCommand::CycleHeading),
    ("Quote", FormatCommand::ToggleQuote),
    ("List", FormatCommand::ToggleBulletList),
    ("Ordered", FormatCommand::ToggleOrderedList),
    ("Checklist", FormatCommand::ToggleChecklist),
];

impl Program for SourceProgram {
    type State = SourceState;
    type Message = SourceMessage;
    type Theme = core::theme::Theme;
    type Renderer = iced_renderer::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        SOURCE_EDITOR_LABEL
    }

    fn settings(&self) -> core::Settings {
        core::Settings::default()
    }

    fn window(&self) -> Option<core::window::Settings> {
        Some(core::window::Settings::default())
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            SourceState {
                editor: IcedEditorAdapter::new(),
                error: None,
                window: None,
                pane_width: WINDOW_WIDTH as f32 / 2.0,
                toolbar_visible: false,
                counts: Counts::default(),
                reading_seconds: 0,
                find_bar: None,
                format_sink: None,
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            SourceMessage::Edit(action) => {
                if let Err(error) = state.editor.perform(action) {
                    state.error = Some(error.to_string());
                }
            }
            SourceMessage::Format(command) => {
                if let Some(sink) = &state.format_sink
                    && let Ok(mut queue) = sink.lock()
                {
                    queue.push_back(command);
                }
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: core::window::Id,
    ) -> core::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        let mut rows: Vec<core::Element<'a, Self::Message, Self::Theme, Self::Renderer>> =
            Vec::new();

        if state.toolbar_visible {
            let mut buttons: Vec<core::Element<'a, Self::Message, Self::Theme, Self::Renderer>> =
                Vec::new();
            for (label, command) in TOOLBAR_ITEMS {
                buttons.push(
                    iced_widget::button(iced_widget::text(*label).size(CHROME_FONT_SIZE))
                        .padding([2, 6])
                        .style(chrome_button_style)
                        .on_press(SourceMessage::Format(command.clone()))
                        .into(),
                );
            }
            rows.push(iced_widget::row(buttons).spacing(4).padding([4, 0]).into());
        }

        if let Some(find) = &state.find_bar {
            let query_marker = if find.focus == FindField::Query {
                "▸"
            } else {
                " "
            };
            let mut find_lines: Vec<core::Element<'a, Self::Message, Self::Theme, Self::Renderer>> =
                Vec::new();
            find_lines.push(
                iced_widget::text(format!("{query_marker} Find: {}", find.query))
                    .size(CHROME_FONT_SIZE)
                    .color(INK)
                    .into(),
            );
            if find.replace_enabled {
                let replace_marker = if find.focus == FindField::Replace {
                    "▸"
                } else {
                    " "
                };
                find_lines.push(
                    iced_widget::text(format!("{replace_marker} Replace: {}", find.replacement))
                        .size(CHROME_FONT_SIZE)
                        .color(INK)
                        .into(),
                );
            }
            find_lines.push(
                iced_widget::text(find.status.clone())
                    .size(CHROME_FONT_SIZE)
                    .color(MUTED_INK)
                    .into(),
            );
            rows.push(
                iced_widget::column(find_lines)
                    .spacing(2)
                    .padding([4, 0])
                    .into(),
            );
        }

        rows.push(
            iced_widget::container(
                text_editor(state.editor.content())
                    .id(core::widget::Id::new("feathermark-source-editor"))
                    .placeholder("Write Markdown…")
                    .style(editor_style)
                    .on_action(SourceMessage::Edit),
            )
            .height(core::Length::Fill)
            .into(),
        );

        let reading = state.reading_seconds;
        let status = format!(
            "{} words · {} chars · {}:{:02} read",
            state.counts.words,
            state.counts.chars,
            reading / 60,
            reading % 60,
        );
        rows.push(
            iced_widget::text(status)
                .size(CHROME_FONT_SIZE)
                .color(MUTED_INK)
                .into(),
        );

        iced_widget::container(iced_widget::column(rows).spacing(6))
            .width(core::Length::Fixed(state.pane_width))
            .height(core::Length::Fill)
            .padding(16)
            .into()
    }
}

type IcedCompositor = iced_renderer::Compositor;
type IcedSurface = <IcedCompositor as graphics::Compositor>::Surface;

struct IcedSourcePane {
    program: SourceProgram,
    state: SourceState,
    cache: user_interface::Cache,
    renderer: Option<iced_renderer::Renderer>,
    compositor: Option<IcedCompositor>,
    surface: Option<IcedSurface>,
    viewport: Option<graphics::Viewport>,
    clipboard: Clipboard,
    cursor: core::mouse::Cursor,
    presented_frames: u64,
    ink_pixels: usize,
    ime_commits: u64,
    presented_text: String,
    focus_transfers: u64,
}

impl IcedSourcePane {
    fn new(snapshot: &feathermark_core::DocumentSnapshot) -> Result<Self, MacError> {
        let program = SourceProgram;
        let (mut state, _) = program.boot();
        state
            .editor
            .install_open_snapshot(snapshot)
            .map_err(|error| MacError::Core(error.to_string()))?;
        Ok(Self {
            program,
            state,
            cache: user_interface::Cache::new(),
            renderer: None,
            compositor: None,
            surface: None,
            viewport: None,
            clipboard: Clipboard::unconnected(),
            cursor: core::mouse::Cursor::Unavailable,
            presented_frames: 0,
            ink_pixels: 0,
            ime_commits: 0,
            presented_text: String::new(),
            focus_transfers: 0,
        })
    }

    fn set_event_queue(&mut self, events: Arc<Mutex<VecDeque<EditorEvent>>>) {
        self.state.editor.set_event_sink(Box::new(move |event| {
            if let Ok(mut queue) = events.lock() {
                queue.push_back(event);
            }
        }));
    }

    fn editor_mut(&mut self) -> &mut IcedEditorAdapter {
        &mut self.state.editor
    }

    fn editor(&self) -> &IcedEditorAdapter {
        &self.state.editor
    }

    fn set_format_sink(&mut self, sink: Arc<Mutex<VecDeque<FormatCommand>>>) {
        self.state.format_sink = Some(sink);
    }

    fn toggle_toolbar(&mut self) -> bool {
        self.state.toolbar_visible = !self.state.toolbar_visible;
        self.request_redraw();
        self.state.toolbar_visible
    }

    fn update_counts(&mut self, counts: Counts) {
        self.state.counts = counts;
        self.state.reading_seconds = counts.reading_time_seconds();
    }

    fn set_find_bar(&mut self, find_bar: Option<FindBarView>) {
        self.state.find_bar = find_bar;
        self.request_redraw();
    }

    fn attach_window(
        &mut self,
        window: Arc<Window>,
        display_handle: OwnedDisplayHandle,
    ) -> Result<(), MacError> {
        use graphics::Compositor as _;

        let physical = window.inner_size();
        let scale_factor = window.scale_factor() as f32;
        let logical = physical.to_logical::<f32>(f64::from(scale_factor));
        self.state.pane_width = logical.width / 2.0;
        let mut compositor = block_on_ready(IcedCompositor::new(
            graphics::Settings::default(),
            display_handle,
            Arc::clone(&window),
            graphics::Shell::headless(),
        ))?
        .map_err(|error| MacError::Native(error.to_string()))?;
        let renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), physical.width, physical.height);

        self.viewport = Some(graphics::Viewport::with_physical_size(
            core::Size::new(physical.width, physical.height),
            scale_factor,
        ));
        self.clipboard = Clipboard::connect(Arc::clone(&window));
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.compositor = Some(compositor);
        self.state.window = Some(Arc::clone(&window));
        window.request_redraw();
        Ok(())
    }

    fn replace(&mut self, snapshot: &feathermark_core::DocumentSnapshot) -> Result<(), MacError> {
        self.state
            .editor
            .install_open_snapshot(snapshot)
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.state
            .editor
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd))
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.request_redraw();
        Ok(())
    }

    fn resize(
        &mut self,
        source_width: u32,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) {
        use graphics::Compositor as _;

        self.state.pane_width = source_width as f32;
        self.viewport = Some(graphics::Viewport::with_physical_size(
            core::Size::new(physical_width, physical_height),
            scale_factor,
        ));
        if physical_width > 0
            && physical_height > 0
            && let (Some(compositor), Some(surface)) = (&mut self.compositor, &mut self.surface)
        {
            compositor.configure_surface(surface, physical_width, physical_height);
        }
        self.request_redraw();
    }

    fn focus(&self) {
        if let Some(window) = &self.state.window {
            window.focus_window();
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.state.window {
            window.request_redraw();
        }
    }

    fn text(&self) -> String {
        self.state.editor.mirror().to_owned()
    }

    fn focus_editor(&mut self) -> Result<(), MacError> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| MacError::Native("editor renderer is not initialized".into()))?;
        let viewport = self
            .viewport
            .as_ref()
            .ok_or_else(|| MacError::Native("editor viewport is not initialized".into()))?;
        let view = self.program.view(&self.state, core::window::Id::unique());
        let mut interface = UserInterface::build(
            view,
            viewport.logical_size(),
            std::mem::take(&mut self.cache),
            renderer,
        );
        let mut operation = core::widget::operation::focusable::focus::<()>(core::widget::Id::new(
            "feathermark-source-editor",
        ));
        interface.operate(renderer, &mut operation);
        self.cache = interface.into_cache();
        self.focus();
        self.focus_transfers = self.focus_transfers.saturating_add(1);
        self.request_redraw();
        Ok(())
    }

    fn handle_winit_event(
        &mut self,
        event: WindowEvent,
        scale_factor: f32,
        modifiers: ModifiersState,
    ) -> Result<bool, MacError> {
        match &event {
            WindowEvent::CursorMoved { position, .. } => {
                let logical = position.to_logical::<f32>(f64::from(scale_factor));
                self.cursor =
                    core::mouse::Cursor::Available(core::Point::new(logical.x, logical.y));
            }
            WindowEvent::CursorLeft { .. } => self.cursor = core::mouse::Cursor::Unavailable,
            _ => {}
        }
        let Some(event) = conversion::window_event(event, scale_factor, modifiers) else {
            return Ok(false);
        };
        self.handle_iced_events(&[event])
    }

    fn commit_ime_for_smoke(&mut self, text: &str) -> Result<bool, MacError> {
        self.focus_editor()?;
        let changed = self.handle_iced_events(&[
            core::Event::InputMethod(core::input_method::Event::Opened),
            core::Event::InputMethod(core::input_method::Event::Commit(text.to_owned())),
        ])?;
        Ok(changed)
    }

    fn handle_iced_events(&mut self, events: &[core::Event]) -> Result<bool, MacError> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| MacError::Native("editor renderer is not initialized".into()))?;
        let viewport = self
            .viewport
            .as_ref()
            .ok_or_else(|| MacError::Native("editor viewport is not initialized".into()))?;
        let view = self.program.view(&self.state, core::window::Id::unique());
        let mut interface = UserInterface::build(
            view,
            viewport.logical_size(),
            std::mem::take(&mut self.cache),
            renderer,
        );
        let mut messages = Vec::new();
        let (ui_state, _) = interface.update(
            events,
            self.cursor,
            renderer,
            &mut self.clipboard,
            &mut messages,
        );
        self.cache = interface.into_cache();

        let mut changed = false;
        let input_commit = events.iter().find_map(|event| match event {
            core::Event::InputMethod(core::input_method::Event::Commit(text)) => Some(text.clone()),
            _ => None,
        });
        for event in events {
            match event {
                core::Event::InputMethod(core::input_method::Event::Opened) => {
                    if self.state.editor.start_composition().is_err() {
                        self.state
                            .editor
                            .cancel_composition(CompositionCancelReason::StaleRevision);
                        self.state
                            .editor
                            .start_composition()
                            .map_err(|error| MacError::Core(error.to_string()))?;
                    }
                }
                core::Event::InputMethod(core::input_method::Event::Preedit(text, _)) => {
                    if self.state.editor.update_composition(text).is_err() {
                        self.state
                            .editor
                            .start_composition()
                            .and_then(|_| self.state.editor.update_composition(text))
                            .map_err(|error| MacError::Core(error.to_string()))?;
                    }
                }
                core::Event::InputMethod(core::input_method::Event::Closed) => self
                    .state
                    .editor
                    .cancel_composition(CompositionCancelReason::FocusLost),
                _ => {}
            }
        }
        let mut skipped_ime_paste = false;
        for message in messages {
            match message {
                SourceMessage::Edit(action) => {
                    if input_commit.is_some() && action.is_edit() && !skipped_ime_paste {
                        skipped_ime_paste = true;
                        continue;
                    }
                    changed |= self
                        .state
                        .editor
                        .perform(action)
                        .map_err(|error| MacError::Core(error.to_string()))?;
                }
                SourceMessage::Format(command) => {
                    if let Some(sink) = &self.state.format_sink
                        && let Ok(mut queue) = sink.lock()
                    {
                        queue.push_back(command);
                    }
                }
            }
        }
        if let Some(text) = input_commit {
            changed |= self
                .state
                .editor
                .commit_composition(&text)
                .map_err(|error| MacError::Core(error.to_string()))?;
            self.ime_commits = self.ime_commits.saturating_add(1);
        }
        if let user_interface::State::Updated { input_method, .. } = ui_state
            && let Some(window) = &self.state.window
        {
            match input_method {
                core::InputMethod::Disabled => window.set_ime_allowed(false),
                core::InputMethod::Enabled { cursor, .. } => {
                    window.set_ime_allowed(true);
                    window.set_ime_cursor_area(
                        winit::dpi::LogicalPosition::new(cursor.x, cursor.y),
                        winit::dpi::LogicalSize::new(cursor.width, cursor.height),
                    );
                }
            }
        }
        self.request_redraw();
        Ok(changed)
    }

    fn draw_and_present(&mut self) -> Result<(), MacError> {
        use graphics::Compositor as _;

        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| MacError::Native("editor renderer is not initialized".into()))?;
        let compositor = self
            .compositor
            .as_mut()
            .ok_or_else(|| MacError::Native("editor compositor is not initialized".into()))?;
        let surface = self
            .surface
            .as_mut()
            .ok_or_else(|| MacError::Native("editor surface is not initialized".into()))?;
        let viewport = self
            .viewport
            .as_ref()
            .ok_or_else(|| MacError::Native("editor viewport is not initialized".into()))?;
        let view = self.program.view(&self.state, core::window::Id::unique());
        let mut interface = UserInterface::build(
            view,
            viewport.logical_size(),
            std::mem::take(&mut self.cache),
            renderer,
        );
        let theme = <core::theme::Theme as core::theme::Base>::default(core::theme::Mode::Light);
        let background = core::Color::from_rgb8(GROUND_LIGHT.0, GROUND_LIGHT.1, GROUND_LIGHT.2);
        interface.draw(
            renderer,
            &theme,
            &core::renderer::Style { text_color: INK },
            self.cursor,
        );
        self.cache = interface.into_cache();

        let pixels = compositor.screenshot(renderer, viewport, background);
        let background_rgba = [GROUND_LIGHT.0, GROUND_LIGHT.1, GROUND_LIGHT.2, 255];
        self.ink_pixels = pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel != background_rgba)
            .count();
        let window = self
            .state
            .window
            .as_ref()
            .ok_or_else(|| MacError::Native("editor window is not initialized".into()))?;
        compositor
            .present(renderer, surface, viewport, background, || {
                window.pre_present_notify();
            })
            .map_err(|error| MacError::Native(error.to_string()))?;
        self.presented_frames = self.presented_frames.saturating_add(1);
        self.presented_text = self.state.editor.mirror().to_owned();
        Ok(())
    }

    fn visual_receipt(&self) -> Result<EditorVisualReceipt, MacError> {
        EditorVisualReceipt::new(self.presented_frames, self.ink_pixels, self.ime_commits)
    }

    fn teardown(&mut self) {
        drop(self.surface.take());
        drop(self.compositor.take());
        drop(self.renderer.take());
        self.clipboard = Clipboard::unconnected();
        drop(self.state.window.take());
    }
}

fn block_on_ready<F: Future>(future: F) -> Result<F::Output, MacError> {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(output) => Ok(output),
        std::task::Poll::Pending => Err(MacError::Native(
            "the tiny-skia compositor unexpectedly required an asynchronous executor".into(),
        )),
    }
}

/// Maps a `⌘`-modified key to its [`FormatCommand`], per the DESIGN-SYSTEM
/// text-formatting bindings. Returns `None` for non-command or unmapped keys.
fn format_command_for(key: &Key, command: bool, shift: bool) -> Option<FormatCommand> {
    if !command {
        return None;
    }
    let Key::Character(text) = key else {
        return None;
    };
    let is = |candidate: &str| text.eq_ignore_ascii_case(candidate);
    if shift {
        if is("k") {
            Some(FormatCommand::ToggleCodeBlock)
        } else if is("h") {
            Some(FormatCommand::CycleHeading)
        } else if is("b") {
            Some(FormatCommand::ToggleQuote)
        } else if is("l") {
            Some(FormatCommand::ToggleBulletList)
        } else if is("o") {
            Some(FormatCommand::ToggleOrderedList)
        } else if is("x") {
            Some(FormatCommand::ToggleChecklist)
        } else {
            None
        }
    } else if is("b") {
        Some(FormatCommand::ToggleBold)
    } else if is("i") {
        Some(FormatCommand::ToggleItalic)
    } else if is("k") {
        Some(FormatCommand::InsertLink { url: None })
    } else if is("e") {
        Some(FormatCommand::ToggleCodeSpan)
    } else {
        None
    }
}

fn choose_open_path() -> Result<Option<PathBuf>, MacError> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| MacError::Native("open panel must run on the AppKit main thread".into()))?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setAllowsMultipleSelection(false);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    Ok(panel
        .URLs()
        .firstObject()
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string())))
}

fn install_file_menu() -> Result<(), MacError> {
    install_file_menu_with_actions().map_err(MacError::Native)
}

fn choose_save_path(default_name: &str) -> Result<Option<PathBuf>, MacError> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| MacError::Native("save panel must run on the AppKit main thread".into()))?;
    let panel = NSSavePanel::savePanel(mtm);
    panel.setNameFieldStringValue(&NSString::from_str(default_name));
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    Ok(panel
        .URL()
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string())))
}

/// Stable per-user autosave/session directory
/// (`~/Library/Application Support/Rutile`).
fn autosave_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Rutile"))
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::smoke_stage_zero_ready_to_edit;

    #[test]
    fn stage_zero_blocks_the_first_edit_until_a_preview_scroll_receipt_arrives() {
        // The preview has painted revision 0 and the editor has presented a
        // frame, but no scroll receipt has echoed back yet: editing must stay
        // blocked. This is the invariant the supervised smoke relies on.
        assert!(!smoke_stage_zero_ready_to_edit(Some(0), 1, 0));
    }

    #[test]
    fn stage_zero_is_ready_to_edit_once_the_preview_scroll_receipt_arrives() {
        // A single receipt is sufficient, and further receipts keep it ready.
        assert!(smoke_stage_zero_ready_to_edit(Some(0), 1, 1));
        assert!(smoke_stage_zero_ready_to_edit(Some(0), 4, 7));
    }

    #[test]
    fn stage_zero_blocks_without_a_painted_preview_revision_zero() {
        // No painted preview yet.
        assert!(!smoke_stage_zero_ready_to_edit(None, 1, 1));
        // Painted, but not revision 0.
        assert!(!smoke_stage_zero_ready_to_edit(Some(3), 1, 1));
    }

    #[test]
    fn stage_zero_blocks_until_the_source_editor_has_presented_a_frame() {
        assert!(!smoke_stage_zero_ready_to_edit(Some(0), 0, 1));
    }
}
