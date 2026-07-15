use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use feathermark_core::{
    AdapterCommitId, AutosaveEntryV1, AutosaveStore, ChangeSet, Counts, DiskVersion, Document,
    EditorEvent, ExternalChange, ExternalResolution, FileError, FileService, FindDirection,
    FindQuery, FormatCommand, LocalFileService, MAX_DOCUMENT_BYTES, RecoveredDocument,
    ScrollAnchorView, ScrollClock, ScrollGeometry, ScrollMap, ScrollOutcome, ScrollPosition,
    ScrollSynchronizer, ScrollTarget, Selection, SessionStateV1, SessionWindowV1,
    apply_editor_commit,
};
use feathermark_protocol::{PreviewHostCommand, RenderUrl};
use feathermark_types::{InteractionId, Revision};
use thiserror::Error;

use super::PlatformAdapter;
use crate::actions::{
    ExportOutput, FindSession, FormatApplied, InsertApplied, ReplaceApplied, SessionRestore,
};
use crate::app::{
    AppEffect, AppMessage, AppState, CloseDecision, CloseOutcome, NoticeSeverity, UserNotice,
};
use crate::preview_host::{HostError, PreviewHost};
use crate::render_scheduler::{
    CompletedRender, Completion, RenderPermit, RenderRequest, RenderScheduler,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppKitMainThread;

impl AppKitMainThread {
    pub fn claim() -> Result<Self, MacError> {
        unsafe extern "C" {
            fn pthread_main_np() -> std::ffi::c_int;
        }

        // SAFETY: pthread_main_np has no arguments, returns an integer, and has
        // no side effects. It is the supported process-main-thread predicate.
        if unsafe { pthread_main_np() } == 1 {
            Ok(Self)
        } else {
            Err(MacError::WrongThread)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneBounds {
    pub source_width: u32,
    pub preview_x: u32,
    pub preview_width: u32,
    pub height: u32,
}

pub const fn split_panes(width: u32, height: u32) -> PaneBounds {
    let source_width = width / 2;
    PaneBounds {
        source_width,
        preview_x: source_width,
        preview_width: width - source_width,
        height,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewIpcStats {
    pub accepted: u64,
    pub dropped_scroll: u64,
    pub required_lost: u64,
    pub disconnected: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewIpcOutcome {
    Accepted,
    DroppedCoalescibleScroll,
    RequiredFrameLost,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewIpcFatal {
    RequiredFrameLost,
    Disconnected,
}

#[derive(Default)]
struct PreviewIpcHealth {
    stats: PreviewIpcStats,
    fatal: Option<PreviewIpcFatal>,
}

#[derive(Clone)]
pub struct PreviewIpcIngress {
    sender: SyncSender<String>,
    health: Arc<Mutex<PreviewIpcHealth>>,
}

impl PreviewIpcIngress {
    pub fn try_push(&self, frame: String) -> PreviewIpcOutcome {
        let coalescible_scroll = serde_json::from_str::<serde_json::Value>(&frame)
            .ok()
            .and_then(|value| {
                value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("scroll");
        let outcome = match self.sender.try_send(frame) {
            Ok(()) => PreviewIpcOutcome::Accepted,
            Err(TrySendError::Full(_)) if coalescible_scroll => {
                PreviewIpcOutcome::DroppedCoalescibleScroll
            }
            Err(TrySendError::Full(_)) => PreviewIpcOutcome::RequiredFrameLost,
            Err(TrySendError::Disconnected(_)) => PreviewIpcOutcome::Disconnected,
        };
        if let Ok(mut health) = self.health.lock() {
            match outcome {
                PreviewIpcOutcome::Accepted => {
                    health.stats.accepted = health.stats.accepted.saturating_add(1)
                }
                PreviewIpcOutcome::DroppedCoalescibleScroll => {
                    health.stats.dropped_scroll = health.stats.dropped_scroll.saturating_add(1)
                }
                PreviewIpcOutcome::RequiredFrameLost => {
                    health.stats.required_lost = health.stats.required_lost.saturating_add(1);
                    health.fatal = Some(PreviewIpcFatal::RequiredFrameLost);
                }
                PreviewIpcOutcome::Disconnected => {
                    health.stats.disconnected = health.stats.disconnected.saturating_add(1);
                    health.fatal = Some(PreviewIpcFatal::Disconnected);
                }
            }
        }
        outcome
    }

    pub fn stats(&self) -> PreviewIpcStats {
        self.health
            .lock()
            .map(|health| health.stats)
            .unwrap_or(PreviewIpcStats {
                required_lost: 1,
                ..PreviewIpcStats::default()
            })
    }

    pub fn take_fatal(&self) -> Option<PreviewIpcFatal> {
        self.health.lock().ok()?.fatal.take()
    }
}

pub fn preview_ipc_channel(capacity: usize) -> (PreviewIpcIngress, Receiver<String>) {
    let (sender, receiver) = sync_channel(capacity);
    (
        PreviewIpcIngress {
            sender,
            health: Arc::new(Mutex::new(PreviewIpcHealth::default())),
        },
        receiver,
    )
}

pub struct MacShell<Window, WebView, Context = ()> {
    webview: Option<WebView>,
    context: Option<Context>,
    window: Option<Window>,
}

impl<Window, WebView> MacShell<Window, WebView, ()> {
    pub fn new(window: Window) -> Self {
        Self {
            webview: None,
            context: None,
            window: Some(window),
        }
    }
}

impl<Window, WebView, Context> MacShell<Window, WebView, Context> {
    pub fn new_with_context(window: Window, context: Context) -> Self {
        Self {
            webview: None,
            context: Some(context),
            window: Some(window),
        }
    }

    pub fn attach_webview(&mut self, webview: WebView) -> Result<(), MacError> {
        if self.webview.is_some() {
            return Err(MacError::WebViewAlreadyAttached);
        }
        self.webview = Some(webview);
        Ok(())
    }
}

impl<Window, WebView, Context> Drop for MacShell<Window, WebView, Context> {
    fn drop(&mut self) {
        drop(self.webview.take());
        drop(self.context.take());
        drop(self.window.take());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderReceipt {
    pub revision: u64,
    pub page_bytes: usize,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorVisualReceipt {
    pub presented_frames: u64,
    pub ink_pixels: usize,
    pub ime_commits: u64,
}

#[derive(Clone, Copy, Debug)]
struct MacScrollAnchor {
    revision: Revision,
    start: usize,
    end: usize,
    ordinal: u32,
}

impl ScrollAnchorView for MacScrollAnchor {
    fn revision(&self) -> Revision {
        self.revision
    }
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn preview_top(&self) -> f64 {
        self.start as f64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacScrollDispatch {
    Preview {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    },
    Source {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    },
    Suppressed,
}

pub struct MacScrollController {
    map: ScrollMap,
    synchronizer: ScrollSynchronizer,
}

impl MacScrollController {
    pub fn new(
        revision: Revision,
        document_len: usize,
        anchors: impl IntoIterator<Item = (usize, usize, u32)>,
        first_interaction_id: InteractionId,
    ) -> Result<Self, String> {
        let anchors = anchors
            .into_iter()
            .map(|(start, end, ordinal)| MacScrollAnchor {
                revision,
                start,
                end,
                ordinal,
            })
            .collect::<Vec<_>>();
        let map = ScrollMap::new(
            ScrollGeometry {
                revision,
                document_len,
                source_max_top: document_len,
                preview_max_y: document_len as f64,
            },
            anchors,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            map,
            synchronizer: ScrollSynchronizer::new(revision, first_interaction_id),
        })
    }

    pub fn next_interaction_id(&self) -> InteractionId {
        self.synchronizer.next_interaction_id()
    }

    pub fn source_user(
        &mut self,
        top_visible_byte: usize,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, String> {
        let outcome = self
            .synchronizer
            .handle_user(
                &self.map,
                feathermark_core::Pane::Source,
                ScrollPosition::SourceByte(top_visible_byte),
                clock,
            )
            .map_err(|error| error.to_string())?;
        Ok(match outcome {
            ScrollOutcome::Command(command) => MacScrollDispatch::Preview {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::PreviewY(y) => y as usize,
                    ScrollTarget::SourceByte(byte) => byte,
                },
                interaction_id: command.interaction_id,
            },
            ScrollOutcome::Suppressed(_) => MacScrollDispatch::Suppressed,
        })
    }

    pub fn preview(
        &mut self,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, String> {
        let revision = self.map.revision();
        let outcome = if user {
            self.synchronizer
                .handle_user(
                    &self.map,
                    feathermark_core::Pane::Preview,
                    ScrollPosition::PreviewY(source_start as f64),
                    clock,
                )
                .map_err(|error| error.to_string())?
        } else {
            self.synchronizer
                .handle_programmatic(
                    revision,
                    feathermark_core::Pane::Preview,
                    interaction_id,
                    clock,
                )
                .map_err(|error| error.to_string())?
        };
        Ok(match outcome {
            ScrollOutcome::Command(command) => MacScrollDispatch::Source {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::SourceByte(byte) => byte,
                    ScrollTarget::PreviewY(y) => y as usize,
                },
                interaction_id: command.interaction_id,
            },
            ScrollOutcome::Suppressed(_) => MacScrollDispatch::Suppressed,
        })
    }
}

impl EditorVisualReceipt {
    pub fn new(
        presented_frames: u64,
        ink_pixels: usize,
        ime_commits: u64,
    ) -> Result<Self, MacError> {
        if presented_frames == 0 {
            return Err(MacError::EditorNeverPresented);
        }
        if ink_pixels == 0 {
            return Err(MacError::EditorHasNoVisibleInk);
        }
        if ime_commits == 0 {
            return Err(MacError::EditorInputPathUntested);
        }
        Ok(Self {
            presented_frames,
            ink_pixels,
            ime_commits,
        })
    }
}

/// Outcome of a disk-version poll against the bound document path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacExternalOutcome {
    Unchanged,
    Reloaded { revision: Revision },
    Conflict,
}

/// What the adapter should do after [`ProductSession::request_save`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacSaveAction {
    /// Path-bound dirty document — atomic save is already completed or failed
    /// with a recoverable notice; inspect [`ProductSession::app_state`].
    Completed,
    /// Untitled (or path-less) dirty document: show Save As, then call
    /// [`ProductSession::request_save_as`].
    NeedSaveAs,
    /// Clean buffer or no-op from the reducer.
    Noop,
}

/// Result of applying a [`SessionRestore`] in last-file → selection → viewport order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MacRestoreReport {
    pub opened_last_file: bool,
    pub selection_applied: bool,
    pub viewport_applied: bool,
    pub notices: Vec<UserNotice>,
}

/// Policy for multi-URL / non-file open delivery (INT-001).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacOpenRequest {
    /// Single file URL/path accepted for open.
    File(PathBuf),
    /// Non-file scheme or unreadable URL rejected with an explicit reason.
    Malformed { reason: String },
    /// More than one URL in one open delivery — unsupported for this single-doc shell.
    UnsupportedMulti { count: usize },
}

/// Bounded background render worker: one running job + latest-pending in the scheduler.
struct MacRenderWorker {
    request: Option<SyncSender<RenderPermit>>,
    completion: Receiver<CompletedRender>,
    thread: Option<JoinHandle<()>>,
}

impl MacRenderWorker {
    fn new() -> Self {
        let (request_tx, request_rx) = sync_channel::<RenderPermit>(1);
        let (completion_tx, completion_rx) = sync_channel::<CompletedRender>(1);
        let thread = std::thread::Builder::new()
            .name("feathermark-macos-render".to_owned())
            .spawn(move || {
                while let Ok(permit) = request_rx.recv() {
                    if completion_tx.send(permit.execute()).is_err() {
                        break;
                    }
                }
            })
            .expect("macos render worker thread must start");
        Self {
            request: Some(request_tx),
            completion: completion_rx,
            thread: Some(thread),
        }
    }

    fn submit(&self, permit: RenderPermit) -> Result<(), MacError> {
        self.request
            .as_ref()
            .ok_or_else(|| MacError::Native("render worker is closed".into()))?
            .try_send(permit)
            .map_err(|error| {
                MacError::Native(format!("bounded render queue rejected work: {error}"))
            })
    }

    fn try_recv(&self) -> Result<Option<CompletedRender>, MacError> {
        match self.completion.try_recv() {
            Ok(completed) => Ok(Some(completed)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err(MacError::Native("render worker disconnected".into()))
            }
        }
    }
}

impl Drop for MacRenderWorker {
    fn drop(&mut self) {
        drop(self.request.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct ProductSession {
    /// Sole AppState/Document authority (Wave 2-A DocumentSessionCore).
    core: crate::session_core::DocumentSessionCore,
    scheduler: RenderScheduler,
    render_worker: MacRenderWorker,
    preview_host: Arc<Mutex<PreviewHost>>,
    scroll: Option<MacScrollController>,
    next_scroll_interaction_id: InteractionId,
    recovered_path: Option<PathBuf>,
    /// Monotonic clock for scheduler debounce (ms since an arbitrary origin).
    render_clock_ms: u64,
}

impl ProductSession {
    pub fn new_in_memory(source: &str) -> Result<Self, MacError> {
        let document = Document::new(source).map_err(|error| MacError::Core(error.to_string()))?;
        Self::from_document(document, None)
    }

    pub fn open(path: &Path) -> Result<Self, MacError> {
        let loaded = LocalFileService::new().load(path, MAX_DOCUMENT_BYTES)?;
        Self::from_document(loaded.document, Some((path.to_path_buf(), loaded.disk)))
    }

    fn from_document(
        document: Document,
        file: Option<(PathBuf, DiskVersion)>,
    ) -> Result<Self, MacError> {
        let mut app = AppState::new();
        match file {
            Some((path, disk)) => {
                app.reduce(AppMessage::DocumentOpened {
                    revision: document.revision(),
                    path,
                    disk,
                });
            }
            None => {
                app.reduce(AppMessage::NewDocument);
            }
        }
        let mut session = Self {
            core: crate::session_core::DocumentSessionCore::from_parts(app, document),
            scheduler: RenderScheduler::new(),
            render_worker: MacRenderWorker::new(),
            preview_host: Arc::new(Mutex::new(PreviewHost::new())),
            scroll: None,
            next_scroll_interaction_id: 1,
            recovered_path: None,
            render_clock_ms: 0,
        };
        session.queue_current();
        Ok(session)
    }

    pub fn source(&self) -> String {
        self.core.document().snapshot().to_string()
    }

    pub fn snapshot(&self) -> feathermark_core::DocumentSnapshot {
        self.core.document().snapshot()
    }

    pub fn app_state(&self) -> &AppState {
        self.core.app()
    }

    pub fn core(&self) -> &crate::session_core::DocumentSessionCore {
        &self.core
    }

    pub fn core_mut(&mut self) -> &mut crate::session_core::DocumentSessionCore {
        &mut self.core
    }

    pub fn decide_close(&mut self, decision: CloseDecision) -> Result<CloseOutcome, MacError> {
        if !self.core.app().dirty() {
            return Ok(CloseOutcome::Close);
        }
        match decision {
            CloseDecision::Cancel => Ok(CloseOutcome::KeepOpen),
            CloseDecision::Discard => Ok(CloseOutcome::Close),
            CloseDecision::Save { untitled_path } => {
                if self.core.app().path().is_some() {
                    self.save()?;
                } else {
                    let path = untitled_path.ok_or(MacError::MissingSavePath)?;
                    self.save_as(&path)?;
                }
                // A save whose durability is unknown leaves dirty set, so the
                // window must stay open until the user can re-save.
                if self.core.app().dirty() {
                    Ok(CloseOutcome::KeepOpen)
                } else {
                    Ok(CloseOutcome::Close)
                }
            }
        }
    }

    pub fn preview_host(&self) -> Arc<Mutex<PreviewHost>> {
        Arc::clone(&self.preview_host)
    }

    pub(crate) fn accept_preview_event(
        &mut self,
        event: feathermark_protocol::PreviewEventV1,
    ) -> Vec<crate::app::AppEffect> {
        self.core.reduce(AppMessage::PreviewEvent(event))
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.core.app().path()
    }

    pub fn apply_editor_event(
        &mut self,
        event: EditorEvent,
    ) -> Result<Option<(AdapterCommitId, ChangeSet)>, MacError> {
        let EditorEvent::CommitRequested {
            adapter_commit_id,
            commit,
        } = event
        else {
            return Ok(None);
        };
        let change = apply_editor_commit(self.core.document_mut(), adapter_commit_id, commit)
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.core.reduce(AppMessage::DocumentEdited {
            revision: self.core.document().revision(),
        });
        self.queue_current();
        Ok(Some((adapter_commit_id, change)))
    }

    pub(crate) fn undo(&mut self) -> Option<ChangeSet> {
        let change = self.core.document_mut().undo()?;
        self.core.reduce(AppMessage::DocumentEdited {
            revision: self.core.document().revision(),
        });
        self.queue_current();
        Some(change)
    }

    pub(crate) fn redo(&mut self) -> Option<ChangeSet> {
        let change = self.core.document_mut().redo()?;
        self.core.reduce(AppMessage::DocumentEdited {
            revision: self.core.document().revision(),
        });
        self.queue_current();
        Some(change)
    }

    pub fn render_now(&mut self) -> Result<RenderReceipt, MacError> {
        let permit = self
            .scheduler
            .start_ready(u64::MAX)
            .ok_or(MacError::RenderNotReady)?;
        let completion = self
            .scheduler
            .finish(permit.execute(), self.core.document().revision());
        let page = match completion {
            Completion::Accepted(page) => page,
            Completion::PreviewTooLarge { revision } => {
                self.core.reduce(AppMessage::RenderFailed {
                    revision,
                    error: feathermark_core::RenderError::PreviewTooLarge,
                });
                return Err(MacError::PreviewTooLarge);
            }
            Completion::Failed { error, .. } => return Err(MacError::Core(error.to_string())),
            Completion::DiscardedStale { .. } | Completion::UnknownJob { .. } => {
                return Err(MacError::StaleRender);
            }
        };

        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| MacError::Entropy(error.to_string()))?;
        let render_url = RenderUrl::new(page.revision, nonce);
        let page_bytes = page.page.len();
        let scroll = MacScrollController::new(
            page.revision,
            self.core.document().len_bytes(),
            page.blocks
                .iter()
                .map(|block| (block.start, block.end, block.ordinal)),
            self.next_scroll_interaction_id,
        )
        .map_err(MacError::Core)?;
        self.next_scroll_interaction_id = scroll.next_interaction_id();
        self.scroll = Some(scroll);
        let command = self
            .preview_host
            .lock()
            .map_err(|_| MacError::PreviewLock)?
            .stage_document(render_url, Arc::from(page.page.into_bytes()))?;
        let PreviewHostCommand::Navigate { revision, url, .. } = command else {
            return Err(MacError::InvalidHostCommand);
        };
        self.core.reduce(AppMessage::RenderAccepted {
            revision,
            page_bytes,
        });
        Ok(RenderReceipt {
            revision,
            page_bytes,
            url: format!("feathermark://preview{}", url.document_path()),
        })
    }

    pub(crate) fn source_scroll(
        &mut self,
        top_visible_byte: usize,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, MacError> {
        let Some(scroll) = self.scroll.as_mut() else {
            return Ok(MacScrollDispatch::Suppressed);
        };
        let dispatch = scroll
            .source_user(top_visible_byte, clock)
            .map_err(MacError::Core)?;
        self.next_scroll_interaction_id = scroll.next_interaction_id();
        Ok(dispatch)
    }

    pub(crate) fn preview_scroll(
        &mut self,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
        clock: ScrollClock,
    ) -> Result<MacScrollDispatch, MacError> {
        let Some(scroll) = self.scroll.as_mut() else {
            return Ok(MacScrollDispatch::Suppressed);
        };
        let dispatch = scroll
            .preview(source_start, interaction_id, user, clock)
            .map_err(MacError::Core)?;
        self.next_scroll_interaction_id = scroll.next_interaction_id();
        Ok(dispatch)
    }

    pub fn save(&mut self) -> Result<(), MacError> {
        match self.request_save()? {
            MacSaveAction::Completed | MacSaveAction::Noop => Ok(()),
            MacSaveAction::NeedSaveAs => Err(MacError::MissingSavePath),
        }
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), MacError> {
        self.request_save_as(path.to_path_buf())
    }

    /// Wave 2-B: `SaveRequested` → `PerformSave` / `RequestCloseDecision` (NeedSaveAs).
    ///
    /// Recoverable save failures reduce `SaveFailed` and return `Err` without
    /// clearing dirty state (MAC-001).
    pub fn request_save(&mut self) -> Result<MacSaveAction, MacError> {
        let effects = self.core.reduce(AppMessage::SaveRequested);
        let mut action = MacSaveAction::Noop;
        for effect in effects {
            match effect {
                AppEffect::PerformSave { path } => {
                    self.perform_save_effect(&path)?;
                    action = MacSaveAction::Completed;
                }
                AppEffect::RequestCloseDecision => {
                    action = MacSaveAction::NeedSaveAs;
                }
                AppEffect::PresentNotice { .. } => {}
                other => {
                    let _ = other;
                }
            }
        }
        Ok(action)
    }

    /// Wave 2-B: `SaveAsRequested` → `PerformSaveAs` (MAC-002 untitled path).
    pub fn request_save_as(&mut self, path: PathBuf) -> Result<(), MacError> {
        let effects = self
            .core
            .app_mut()
            .reduce(AppMessage::SaveAsRequested { path: path.clone() });
        // Clean document: reducer yields no PerformSaveAs; still allow explicit
        // rebinding via direct save for tests that call save_as on a clean buffer.
        let mut performed = false;
        for effect in effects {
            if let AppEffect::PerformSaveAs { path } = effect {
                self.perform_save_effect(&path)?;
                performed = true;
            }
        }
        if !performed && self.core.app().dirty() {
            // Path was requested but reducer declined (unexpected); fall through
            // to atomic save so the adapter still persists exact bytes.
            self.perform_save_effect(&path)?;
        } else if !performed && !self.core.app().dirty() {
            self.perform_save_effect(&path)?;
        }
        Ok(())
    }

    fn perform_save_effect(&mut self, path: &Path) -> Result<(), MacError> {
        let outcome = LocalFileService::new().save_atomic(path, &self.core.document().snapshot());
        let revision = self.core.document().revision();
        let path_buf = path.to_path_buf();
        match outcome {
            feathermark_core::SaveOutcome::Committed { disk } => {
                self.core.reduce(AppMessage::SaveCompleted {
                    revision,
                    path: path_buf,
                    disk,
                });
                Ok(())
            }
            feathermark_core::SaveOutcome::CommittedDurabilityUnknown { disk, .. } => {
                self.core.reduce(AppMessage::SaveDurabilityUnknown {
                    revision,
                    path: path_buf,
                    disk,
                });
                // Durability unknown keeps dirty set; surface as recoverable error.
                Err(MacError::SaveDurabilityUnknown)
            }
            feathermark_core::SaveOutcome::NotCommitted { reason } => {
                self.core.reduce(AppMessage::SaveFailed { revision });
                Err(MacError::Core(reason.to_string()))
            }
        }
    }

    pub fn reload(&mut self) -> Result<(), MacError> {
        let path = self
            .core
            .app()
            .path()
            .map(Path::to_path_buf)
            .ok_or(MacError::MissingSavePath)?;
        self.open_document_path(path)
    }

    /// Wave 2-B: `OpenDocument` → `PerformOpen` → `OpenRequestCompleted`.
    pub fn open_document_path(&mut self, path: PathBuf) -> Result<(), MacError> {
        let _generation = self.core.begin_open();
        let effects = self.core.reduce(AppMessage::OpenDocument { path });
        self.apply_open_effects(effects)
    }

    /// Classifies OS open delivery (cold launch, second-launch URL, drag/drop).
    pub fn classify_open_urls(urls: &[String]) -> MacOpenRequest {
        if urls.is_empty() {
            return MacOpenRequest::Malformed {
                reason: "empty open delivery".into(),
            };
        }
        if urls.len() > 1 {
            return MacOpenRequest::UnsupportedMulti { count: urls.len() };
        }
        let raw = &urls[0];
        let path = if let Some(rest) = raw.strip_prefix("file://") {
            // Percent-decode minimal form: path portion only, no host.
            let decoded = percent_decode_path(rest);
            PathBuf::from(decoded)
        } else if raw.contains("://") {
            return MacOpenRequest::Malformed {
                reason: format!("unsupported URL scheme: {raw}"),
            };
        } else {
            PathBuf::from(raw)
        };
        if path.as_os_str().is_empty() {
            return MacOpenRequest::Malformed {
                reason: "empty file path".into(),
            };
        }
        MacOpenRequest::File(path)
    }

    /// Opens a classified request through the shared open command; rejects
    /// multi/malformed with a durable notice and no document mutation.
    pub fn handle_open_request(&mut self, request: MacOpenRequest) -> Result<(), MacError> {
        match request {
            MacOpenRequest::File(path) => self.open_document_path(path),
            MacOpenRequest::Malformed { reason } => {
                let effects = self.core.reduce(AppMessage::OpenRequestCompleted {
                    result: Err(reason),
                });
                self.collect_notices_from_effects(&effects);
                Ok(())
            }
            MacOpenRequest::UnsupportedMulti { count } => {
                let effects = self.core.reduce(AppMessage::OpenRequestCompleted {
                    result: Err(format!(
                        "opening {count} files at once is not supported; open one Markdown file"
                    )),
                });
                self.collect_notices_from_effects(&effects);
                Ok(())
            }
        }
    }

    fn apply_open_effects(&mut self, effects: Vec<AppEffect>) -> Result<(), MacError> {
        for effect in effects {
            match effect {
                AppEffect::PerformOpen { path } => {
                    let result = LocalFileService::new()
                        .load(&path, MAX_DOCUMENT_BYTES)
                        .map(|loaded| {
                            (
                                loaded.document.revision(),
                                path.clone(),
                                loaded.disk,
                                loaded.document,
                            )
                        })
                        .map_err(|error| error.to_string());
                    match result {
                        Ok((revision, path, disk, document)) => {
                            self.core.set_document(document);
                            self.scheduler = RenderScheduler::new();
                            self.scroll = None;
                            let complete = self.core.reduce(AppMessage::OpenRequestCompleted {
                                result: Ok((revision, path, disk)),
                            });
                            for effect in complete {
                                if let AppEffect::ScheduleRender { .. } = effect {
                                    self.queue_current();
                                }
                            }
                        }
                        Err(error) => {
                            let complete = self
                                .core
                                .app_mut()
                                .reduce(AppMessage::OpenRequestCompleted { result: Err(error) });
                            self.collect_notices_from_effects(&complete);
                        }
                    }
                }
                AppEffect::PresentNotice { .. } => {}
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_notices_from_effects(&self, _effects: &[AppEffect]) {
        // Notices are already stored on AppState via reduce; nothing to copy.
    }

    /// Wave 2-B close path: `CloseRequested` → save / quit effects.
    pub fn request_close(&mut self, decision: CloseDecision) -> Result<CloseOutcome, MacError> {
        if !self.core.app().dirty() && matches!(decision, CloseDecision::Cancel) {
            return Ok(CloseOutcome::KeepOpen);
        }
        if !self.core.app().dirty() {
            return Ok(CloseOutcome::Close);
        }
        let effects = self.core.reduce(AppMessage::CloseRequested {
            decision: decision.clone(),
        });
        let effects_empty = effects.is_empty();
        let mut outcome = CloseOutcome::KeepOpen;
        for effect in effects {
            match effect {
                AppEffect::PerformSave { path } => match self.perform_save_effect(&path) {
                    Ok(()) if !self.core.app().dirty() => outcome = CloseOutcome::Close,
                    Ok(()) => outcome = CloseOutcome::KeepOpen,
                    Err(MacError::SaveDurabilityUnknown) => outcome = CloseOutcome::KeepOpen,
                    Err(error) => return Err(error),
                },
                AppEffect::QuitApplication => outcome = CloseOutcome::Close,
                AppEffect::RequestCloseDecision => {
                    // Untitled close-save without a path: surface NeedSaveAs via MissingSavePath.
                    return Err(MacError::MissingSavePath);
                }
                AppEffect::PresentNotice { .. } => {}
                _ => {}
            }
        }
        // Discard/cancel that produced no effects: fall back to legacy semantics.
        if effects_empty {
            return self.decide_close(decision);
        }
        Ok(outcome)
    }

    /// Disk-version poll + external-conflict detection before overwrite (MAC-005).
    pub fn inspect_external_change(&mut self) -> Result<MacExternalOutcome, MacError> {
        let (Some(path), Some(saved)) = (
            self.core.app().path().map(Path::to_path_buf),
            self.core.app().saved_disk().cloned(),
        ) else {
            return Ok(MacExternalOutcome::Unchanged);
        };
        match LocalFileService::new().inspect_external_change(
            &path,
            &saved,
            self.core.app().dirty(),
            MAX_DOCUMENT_BYTES,
        )? {
            ExternalChange::Unchanged => Ok(MacExternalOutcome::Unchanged),
            ExternalChange::Reloaded(loaded) => {
                self.core.set_document(loaded.document);
                self.scheduler = RenderScheduler::new();
                self.scroll = None;
                let revision = self.core.document().revision();
                for effect in self.core.reduce(AppMessage::DocumentOpened {
                    revision,
                    path,
                    disk: loaded.disk,
                }) {
                    if let AppEffect::ScheduleRender { .. } = effect {
                        self.queue_current();
                    }
                }
                Ok(MacExternalOutcome::Reloaded { revision })
            }
            ExternalChange::Conflict { disk } => {
                let effects = self
                    .core
                    .app_mut()
                    .reduce(AppMessage::ExternalConflictDetected { disk });
                Ok(
                    if effects
                        .iter()
                        .any(|effect| matches!(effect, AppEffect::PresentExternalConflict { .. }))
                    {
                        MacExternalOutcome::Conflict
                    } else {
                        MacExternalOutcome::Unchanged
                    },
                )
            }
        }
    }

    pub fn has_external_conflict(&self) -> bool {
        self.core.app().external_conflict().is_some()
    }

    pub fn resolve_external_conflict(
        &mut self,
        resolution: ExternalResolution,
    ) -> Result<(), MacError> {
        for effect in self
            .core
            .app_mut()
            .reduce(AppMessage::ResolveExternalConflict(resolution))
        {
            match effect {
                AppEffect::ReloadExternal { path } => self.open_document_path(path)?,
                AppEffect::SaveExternalAs { path } => self.request_save_as(path)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// Conflict check immediately before a save attempt (MAC-005).
    pub fn ensure_no_external_conflict_before_save(&mut self) -> Result<(), MacError> {
        match self.inspect_external_change()? {
            MacExternalOutcome::Unchanged | MacExternalOutcome::Reloaded { .. } => Ok(()),
            MacExternalOutcome::Conflict => Err(MacError::ExternalConflict),
        }
    }

    /// Autosave via `AutosaveTick` → `PerformAutosave` → `AutosaveCompleted`.
    pub fn request_autosave(&mut self, captured_at_unix_ms: u64) -> Result<(), MacError> {
        let effects = self.core.reduce(AppMessage::AutosaveTick);
        for effect in effects {
            if let AppEffect::PerformAutosave = effect {
                let result = match self.core.app().autosave_store().cloned() {
                    Some(store) => {
                        let snapshot = self.core.document().snapshot();
                        let document_path = self
                            .core
                            .app()
                            .path()
                            .map(|path| path.to_string_lossy().into_owned());
                        store
                            .record(&snapshot, document_path.as_deref(), captured_at_unix_ms)
                            .map_err(|error| error.to_string())
                    }
                    None => Err("autosave store unbound".to_owned()),
                };
                let _ = self.core.reduce(AppMessage::AutosaveCompleted { result });
            }
        }
        Ok(())
    }

    /// Recovery dismiss via shared `RecoveryDismissed` message.
    pub fn request_recovery_dismiss(&mut self) {
        let _ = self.core.reduce(AppMessage::RecoveryDismissed);
    }

    // --- Wave 2M: native input → shared Wave-2S action surface --------------

    /// Applies a [`FormatCommand`] at `selection` through the shared
    /// [`AppState::apply_format_command`](crate::app::AppState::apply_format_command),
    /// then re-queues a render. The caller re-synchronizes its editor mirror to
    /// [`FormatApplied::selection_after`] via `IcedEditorAdapter::resync_to`.
    pub fn apply_format(
        &mut self,
        selection: Selection,
        command: FormatCommand,
    ) -> Result<FormatApplied, MacError> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.apply_format_command(document, selection, command)
                .map_err(|error| MacError::Core(error.to_string()))?
        };
        self.queue_current();
        Ok(applied)
    }

    /// Routes an Enter keystroke to [`AppState::smart_enter`](crate::app::AppState::smart_enter).
    pub fn smart_enter(&mut self, selection: Selection) -> Result<FormatApplied, MacError> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.smart_enter(document, selection)
                .map_err(|error| MacError::Core(error.to_string()))?
        };
        self.queue_current();
        Ok(applied)
    }

    /// Opens (or replaces) the find session bound to this document.
    pub fn start_find(&mut self, query: FindQuery, direction: FindDirection, wrap: bool) {
        self.core.app_mut().start_find(query, direction, wrap);
    }

    /// Closes the active find session.
    pub fn end_find(&mut self) {
        self.core.app_mut().end_find();
    }

    /// Borrows the active find session, if any.
    pub fn find_session(&self) -> Option<&FindSession> {
        self.core.app().find_session()
    }

    /// Locates the next match from `from_byte`, recording it on the session.
    pub fn find_next(&mut self, from_byte: usize) -> Result<Option<Range<usize>>, MacError> {
        let (app, document) = self.core.app_mut_and_document();
        app.find_next(document, from_byte)
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Locates the previous match from `from_byte`, recording it on the session.
    pub fn find_prev(&mut self, from_byte: usize) -> Result<Option<Range<usize>>, MacError> {
        let (app, document) = self.core.app_mut_and_document();
        app.find_prev(document, from_byte)
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Replaces the session's current match, re-queuing a render when it edited.
    pub fn replace_current(&mut self, replacement: String) -> Result<ReplaceApplied, MacError> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.replace_current(document, replacement)
                .map_err(|error| MacError::Core(error.to_string()))?
        };
        if applied.replaced > 0 {
            self.queue_current();
        }
        Ok(applied)
    }

    /// Replaces every match of the session's query, re-queuing a render when it
    /// edited.
    pub fn replace_all(&mut self, replacement: String) -> Result<ReplaceApplied, MacError> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.replace_all(document, replacement)
                .map_err(|error| MacError::Core(error.to_string()))?
        };
        if applied.replaced > 0 {
            self.queue_current();
        }
        Ok(applied)
    }

    /// Inserts `text` over `selection` through the shared
    /// [`AppState::insert_text`](crate::app::AppState::insert_text) primitive,
    /// re-queuing a render. Smart paste (clipboard-HTML → markdown, or the
    /// plain-text fallback) routes through here so both shells share one insert
    /// path; the caller follows [`InsertApplied::changes`] incrementally via the
    /// adapter, preserving the viewport.
    pub fn insert_text(
        &mut self,
        selection: Selection,
        text: &str,
    ) -> Result<InsertApplied, MacError> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.insert_text(document, selection, text)
                .map_err(|error| MacError::Core(error.to_string()))?
        };
        self.queue_current();
        Ok(applied)
    }

    /// Produces the validated, self-contained export page plus a suggested file
    /// name (save-as-HTML and copy-as-HTML share this).
    pub fn export_html(&self) -> Result<ExportOutput, MacError> {
        self.core
            .app()
            .export_html(self.core.document(), None)
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Writes the self-contained export page to `path`.
    pub fn save_html_as(&self, path: &Path) -> Result<(), MacError> {
        let output = self.export_html()?;
        std::fs::write(path, output.html).map_err(|error| MacError::Native(error.to_string()))
    }

    /// Live word/character/reading-time counts for the status bar.
    pub fn counts(&self) -> Counts {
        self.core.app().counts(self.core.document())
    }

    /// Binds the autosave/session store rooted at `dir` (which must exist).
    pub fn bind_autosave(&mut self, dir: PathBuf) -> Result<(), MacError> {
        self.core
            .app_mut()
            .bind_autosave(AutosaveStore::new(dir))
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Writes one autosave entry for the current snapshot. `Ok(None)` when no
    /// store is bound.
    pub fn autosave_tick(
        &mut self,
        captured_at_unix_ms: u64,
    ) -> Result<Option<AutosaveEntryV1>, MacError> {
        let (app, document) = self.core.app_mut_and_document();
        app.autosave_tick(document, captured_at_unix_ms)
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Recovery-on-startup: the highest verifiable autosaved document, if any.
    pub fn recover(&self) -> Result<Option<RecoveredDocument>, MacError> {
        self.core
            .app()
            .recover()
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// The original path of the last adopted recovered document, if it had one
    /// (a save-panel default hint after recovery).
    pub fn recovered_path(&self) -> Option<&Path> {
        self.recovered_path.as_deref()
    }

    /// Adopts a crash-recovered document as the working buffer via
    /// [`AppMessage::RecoveryAdopted`]. The recovered document — newer than any
    /// on-disk copy — becomes the current *dirty* buffer; a save targets the
    /// recovered path. Any bound autosave store is preserved.
    pub fn adopt_recovered(&mut self, recovered: RecoveredDocument) -> Result<(), MacError> {
        let hint = recovered.entry.document_path.clone().map(PathBuf::from);
        let text = recovered.document.snapshot().to_string();
        self.scheduler = RenderScheduler::new();
        self.scroll = None;
        self.recovered_path = hint;
        for effect in self.core.reduce(AppMessage::RecoveryAdopted {
            document: recovered,
        }) {
            if let AppEffect::ScheduleRender { .. } = effect {
                self.queue_current();
            }
        }
        self.core
            .set_document(Document::new(&text).map_err(|error| MacError::Core(error.to_string()))?);
        Ok(())
    }

    /// Ordered session restore: last file → selection → viewport (MAC-006).
    /// Degraded steps surface nonfatal [`UserNotice`]s and continue.
    pub fn apply_session_restore(
        &mut self,
        restore: &SessionRestore,
    ) -> Result<MacRestoreReport, MacError> {
        let mut report = MacRestoreReport::default();
        let effects = self.core.reduce(AppMessage::SessionRestored {
            state: SessionStateV1 {
                schema: feathermark_core::SESSION_SCHEMA_V1.to_owned(),
                v: 1,
                saved_at_unix_ms: 0,
                last_file: restore
                    .last_file
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                selection: restore.selection.map(|selection| {
                    feathermark_core::SessionSelectionV1 {
                        anchor: selection.anchor as u64,
                        head: selection.head as u64,
                    }
                }),
                top_visible_byte: restore.top_visible_byte.map(|b| b as u64),
                window: restore.window,
                recent_files: restore
                    .last_file
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .into_iter()
                    .collect(),
            },
        });
        for effect in effects {
            if let AppEffect::PerformOpen { path } = effect {
                match LocalFileService::new().load(&path, MAX_DOCUMENT_BYTES) {
                    Ok(loaded) => {
                        self.core.set_document(loaded.document);
                        self.scheduler = RenderScheduler::new();
                        self.scroll = None;
                        let revision = self.core.document().revision();
                        let complete = self.core.reduce(AppMessage::OpenRequestCompleted {
                            result: Ok((revision, path, loaded.disk)),
                        });
                        for effect in complete {
                            if let AppEffect::ScheduleRender { .. } = effect {
                                self.queue_current();
                            }
                        }
                        report.opened_last_file = true;
                    }
                    Err(error) => {
                        let complete = self.core.reduce(AppMessage::OpenRequestCompleted {
                            result: Err(format!(
                                "session restore could not open last file: {error}"
                            )),
                        });
                        for effect in complete {
                            if let AppEffect::PresentNotice { notice } = effect {
                                report.notices.push(notice);
                            }
                        }
                    }
                }
            }
        }

        // Selection / viewport are advisory and validated against the loaded doc.
        if let Some(selection) = restore.selection {
            let len = self.core.document().len_bytes();
            if selection.anchor <= len && selection.head <= len {
                report.selection_applied = true;
            } else {
                report.notices.push(self.push_degraded_notice(
                    "Session selection could not be restored",
                    format!("selection {selection:?} outside document length {len}"),
                ));
            }
        }
        if let Some(top) = restore.top_visible_byte {
            let len = self.core.document().len_bytes();
            if top <= len {
                report.viewport_applied = true;
            } else {
                report.notices.push(self.push_degraded_notice(
                    "Session viewport could not be restored",
                    format!("top_visible_byte {top} outside document length {len}"),
                ));
            }
        }
        Ok(report)
    }

    fn push_degraded_notice(&mut self, message: &str, source: String) -> UserNotice {
        let effects = self.core.reduce(AppMessage::SurfaceNotice {
            severity: NoticeSeverity::Warning,
            message: format!("{message}: {source}"),
            source_error: source.clone(),
        });
        for effect in effects {
            if let AppEffect::PresentNotice { notice } = effect {
                return notice;
            }
        }
        UserNotice {
            id: 0,
            severity: NoticeSeverity::Warning,
            message: message.to_owned(),
            source_error: source,
            action_label: None,
            shown: false,
            dismissed: false,
        }
    }

    /// Mirror failure: one full resync, then durable notice (W2-A contract).
    pub fn report_mirror_failure(&mut self, error: impl Into<String>) -> Vec<AppEffect> {
        self.core.reduce(AppMessage::MirrorFailed {
            error: error.into(),
        })
    }

    pub fn complete_mirror_resync(&mut self, result: Result<(), String>) -> Vec<AppEffect> {
        self.core
            .app_mut()
            .reduce(AppMessage::MirrorResyncCompleted { result })
    }

    /// Bounded background render: start one ready job if the worker is free.
    pub fn pump_render_start(&mut self, now_ms: u64) -> Result<bool, MacError> {
        self.render_clock_ms = now_ms;
        if let Some(permit) = self.scheduler.start_ready(now_ms) {
            self.render_worker.submit(permit)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Poll the background worker; discard stale completions (MAC-007).
    pub fn pump_render_completions(&mut self) -> Result<Option<RenderReceipt>, MacError> {
        let Some(completed) = self.render_worker.try_recv()? else {
            return Ok(None);
        };
        let current = self.core.document().revision();
        match self.scheduler.finish(completed, current) {
            Completion::Accepted(page) => {
                let mut nonce = [0_u8; 16];
                getrandom::fill(&mut nonce)
                    .map_err(|error| MacError::Entropy(error.to_string()))?;
                let render_url = RenderUrl::new(page.revision, nonce);
                let page_bytes = page.page.len();
                let scroll = MacScrollController::new(
                    page.revision,
                    self.core.document().len_bytes(),
                    page.blocks
                        .iter()
                        .map(|block| (block.start, block.end, block.ordinal)),
                    self.next_scroll_interaction_id,
                )
                .map_err(MacError::Core)?;
                self.next_scroll_interaction_id = scroll.next_interaction_id();
                self.scroll = Some(scroll);
                let command = self
                    .preview_host
                    .lock()
                    .map_err(|_| MacError::PreviewLock)?
                    .stage_document(render_url, Arc::from(page.page.into_bytes()))?;
                let PreviewHostCommand::Navigate { revision, url, .. } = command else {
                    return Err(MacError::InvalidHostCommand);
                };
                self.core.reduce(AppMessage::RenderAccepted {
                    revision,
                    page_bytes,
                });
                Ok(Some(RenderReceipt {
                    revision,
                    page_bytes,
                    url: format!("feathermark://preview{}", url.document_path()),
                }))
            }
            Completion::PreviewTooLarge { revision } => {
                self.core.reduce(AppMessage::RenderFailed {
                    revision,
                    error: feathermark_core::RenderError::PreviewTooLarge,
                });
                Err(MacError::PreviewTooLarge)
            }
            Completion::Failed { error, .. } => Err(MacError::Core(error.to_string())),
            Completion::DiscardedStale { .. } | Completion::UnknownJob { .. } => Ok(None),
        }
    }

    pub fn scheduler_stats(&self) -> crate::render_scheduler::SchedulerStats {
        self.scheduler.stats()
    }

    /// Real pasteboard write with success/failure (MAC-008).
    pub fn write_clipboard_html(html: &str) -> Result<(), MacError> {
        clipboard::write_html(html)
    }

    /// Real pasteboard read for smart paste; fails when unavailable (MAC-008).
    pub fn read_clipboard_paste_text() -> Result<String, MacError> {
        clipboard::read_paste_text()
    }

    /// Dismiss a notice by id through the shared reducer.
    pub fn dismiss_notice(&mut self, id: usize) {
        let _ = self.core.reduce(AppMessage::NoticeDismissed { id });
    }

    /// Captures session-restore state from the current path plus the platform's
    /// selection, viewport, and window frame.
    pub fn capture_session_state(
        &self,
        saved_at_unix_ms: u64,
        selection: Option<Selection>,
        top_visible_byte: Option<usize>,
        window: Option<SessionWindowV1>,
    ) -> SessionStateV1 {
        self.core
            .app()
            .capture_session_state(saved_at_unix_ms, selection, top_visible_byte, window)
    }

    /// Persists session-restore `state`.
    pub fn save_session_state(&self, state: &SessionStateV1) -> Result<(), MacError> {
        self.core
            .app()
            .save_session_state(state)
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Loads persisted session state, if any.
    pub fn load_session_state(&self) -> Result<Option<SessionStateV1>, MacError> {
        self.core
            .app()
            .load_session_state()
            .map_err(|error| MacError::Core(error.to_string()))
    }

    /// Lowers persisted state into the platform-actionable restore parts.
    pub fn restore_session(&self, state: &SessionStateV1) -> SessionRestore {
        self.core.app().restore_session(state)
    }

    fn queue_current(&mut self) {
        self.scheduler.submit(
            RenderRequest::new(
                self.core.document().revision(),
                Arc::from(self.core.document().snapshot().to_string()),
            ),
            0,
        );
    }
}

#[derive(Debug, Error)]
pub enum MacError {
    #[error("the macOS shell must start on the AppKit main thread")]
    WrongThread,
    #[error("a WKWebView is already attached")]
    WebViewAlreadyAttached,
    #[error("render was not ready")]
    RenderNotReady,
    #[error("render completion was stale")]
    StaleRender,
    #[error("generated preview is too large")]
    PreviewTooLarge,
    #[error("save requires a path")]
    MissingSavePath,
    #[error("save committed but durability is unknown")]
    SaveDurabilityUnknown,
    #[error("external file conflict must be resolved before saving")]
    ExternalConflict,
    #[error("preview host lock is poisoned")]
    PreviewLock,
    #[error("preview host returned a non-navigation command")]
    InvalidHostCommand,
    #[error("the native editor never presented a frame")]
    EditorNeverPresented,
    #[error("the native editor frame contained only background pixels")]
    EditorHasNoVisibleInk,
    #[error("the native editor IME path was not exercised")]
    EditorInputPathUntested,
    #[error("entropy source failed: {0}")]
    Entropy(String),
    #[error("core operation failed: {0}")]
    Core(String),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Host(#[from] HostError),
    #[error("native shell failed: {0}")]
    Native(String),
}

pub struct MacosAdapter;

impl PlatformAdapter for MacosAdapter {
    fn run() -> Result<(), String> {
        run_native(None, false).map_err(|error| error.to_string())
    }
}

pub fn run_cli(path: Option<PathBuf>, smoke: bool) -> Result<(), MacError> {
    run_native(path, smoke)
}

mod accessibility;
mod clipboard;
mod editor;
mod native;
mod open_events;

pub(crate) use accessibility::{AxUiState, MacAccessibilityState};
pub use editor::{IcedAdapterStats, IcedEditorAdapter};
use native::run_native;
pub use open_events::{
    MacMenuCommand, MacUserEvent, bind_open_proxy, forward_open_urls,
    install_file_menu_with_actions,
};

fn percent_decode_path(segment: &str) -> String {
    url::Url::parse(&format!("file://{segment}"))
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| segment.to_owned())
}
