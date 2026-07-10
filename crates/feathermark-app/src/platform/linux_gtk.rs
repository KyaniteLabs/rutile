use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use feathermark_core::{
    AdapterCommitId, ChangeSet, CompositionCancelReason, CompositionTracker, Document, Edit,
    EditTransaction, EditorAdapter, EditorCommit, EditorError, EditorEvent, EditorEventSink,
    ExternalChange, ExternalResolution, FileService, LocalCommitRejection, LocalFileService,
    RenderError, ScrollAnchorView, ScrollClock, ScrollGeometry, ScrollMap, ScrollOutcome,
    ScrollPosition, ScrollSynchronizer, ScrollTarget, StaleRevision, TransactionKind,
    apply_editor_commit,
};
use feathermark_protocol::RenderUrl;
use feathermark_types::{InteractionId, Revision};
use gtk::prelude::*;
use sourceview4::prelude::*;
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder, WebViewBuilderExtUnix};

use super::PlatformAdapter;
use crate::app::{AppEffect, AppMessage, AppState, CloseDecision, CloseOutcome};
use crate::preview_host::{
    HostError, NavigationKind, PreviewControlSink, PreviewHost, SchemeRequest, ScrollDelivery,
};
use crate::render_scheduler::{
    CompletedRender, Completion, RenderPermit, RenderRequest, RenderScheduler,
};

const APP_ID: &str = "tech.kyanitelabs.feathermark";
const INITIAL_WIDTH: u32 = 1100;
const INITIAL_HEIGHT: u32 = 760;
const RENDER_POLL_MS: u64 = 10;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IncrementalAdapterStats {
    pub full_snapshot_installs: u64,
    pub incremental_native_edits: u64,
    pub acknowledgements: u64,
    pub source_paints: u64,
}

#[derive(Clone, Debug)]
struct LineByteIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineByteIndex {
    fn from_text(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
        );
        Self {
            starts,
            len: text.len(),
        }
    }

    fn byte_at(&self, line: i32, byte_on_line: i32) -> Result<usize, EditorError> {
        let line = usize::try_from(line)
            .map_err(|_| EditorError::Platform("negative GTK line".to_owned()))?;
        let byte_on_line = usize::try_from(byte_on_line)
            .map_err(|_| EditorError::Platform("negative GTK line byte".to_owned()))?;
        let start = *self
            .starts
            .get(line)
            .ok_or_else(|| EditorError::Platform("GTK line is outside mirror".to_owned()))?;
        start
            .checked_add(byte_on_line)
            .filter(|byte| *byte <= self.len)
            .ok_or_else(|| EditorError::Platform("GTK byte is outside mirror".to_owned()))
    }

    fn line_and_index(&self, byte: usize) -> Result<(i32, i32), EditorError> {
        if byte > self.len {
            return Err(EditorError::Platform(
                "requested byte is outside GTK mirror".to_owned(),
            ));
        }
        let line = self.starts.partition_point(|start| *start <= byte) - 1;
        let index = byte - self.starts[line];
        Ok((
            i32::try_from(line)
                .map_err(|_| EditorError::Platform("GTK line overflow".to_owned()))?,
            i32::try_from(index)
                .map_err(|_| EditorError::Platform("GTK line-byte overflow".to_owned()))?,
        ))
    }

    fn apply(&mut self, range: std::ops::Range<usize>, replacement: &str) {
        let removed = range.end - range.start;
        let delta = replacement.len() as isize - removed as isize;
        self.starts
            .retain(|start| *start <= range.start || *start > range.end);
        for start in &mut self.starts {
            if *start > range.end {
                *start = start.saturating_add_signed(delta);
            }
        }
        self.starts.extend(
            replacement
                .bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(range.start + offset + 1)),
        );
        self.starts.sort_unstable();
        self.starts.dedup();
        self.len = self.len.saturating_add_signed(delta);
    }
}

#[derive(Clone, Debug)]
struct PendingNativeEdit {
    range: std::ops::Range<usize>,
    replacement: String,
}

#[derive(Clone, Debug)]
struct GtkComposition {
    id: u64,
    base_revision: Revision,
    range: std::ops::Range<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgrammaticViewport {
    pub revision: Revision,
    pub top_visible_byte: usize,
    pub interaction_id: InteractionId,
}

struct GtkEditorInner {
    sink: Option<EditorEventSink>,
    index: LineByteIndex,
    revision: Revision,
    next_commit_id: AdapterCommitId,
    next_composition_id: u64,
    pending_native: Option<PendingNativeEdit>,
    pending_commit: Option<AdapterCommitId>,
    pending_paint: Option<Revision>,
    composition_tracker: CompositionTracker,
    composition: Option<GtkComposition>,
    stale_composition_guard: bool,
    view: Option<sourceview4::View>,
    top_visible_byte: usize,
    pending_programmatic_viewport: Option<ProgrammaticViewport>,
    observed_programmatic_viewport: Option<ProgrammaticViewport>,
    suppress_native: bool,
    stats: IncrementalAdapterStats,
}

impl GtkEditorInner {
    fn emit(&mut self, event: EditorEvent) {
        if let Some(sink) = self.sink.as_mut() {
            sink(event);
        }
    }

    fn next_commit(&mut self) -> AdapterCommitId {
        self.next_commit_id = self.next_commit_id.saturating_add(1);
        self.next_commit_id
    }

    fn finish_native_edit(&mut self) {
        let Some(edit) = self.pending_native.take() else {
            return;
        };
        self.index.apply(edit.range.clone(), &edit.replacement);
        self.stats.incremental_native_edits = self.stats.incremental_native_edits.saturating_add(1);
        let commit_id = self.next_commit();
        let event = if let Some(active) = self.composition.clone() {
            if edit.replacement.is_empty() {
                // Selection deletion is the first half of a GTK IME commit;
                // the following insertion is folded into one typed ImeCommit.
                return;
            }
            let Some(event) = self.composition_tracker.commit(
                active.id,
                active.base_revision,
                commit_id,
                edit.replacement,
            ) else {
                return;
            };
            self.composition = None;
            event
        } else {
            EditorEvent::CommitRequested {
                adapter_commit_id: commit_id,
                commit: EditorCommit::Edit {
                    transaction: EditTransaction {
                        base_revision: self.revision,
                        id: commit_id,
                        kind: TransactionKind::Typing,
                        edits: vec![Edit {
                            byte_range: edit.range,
                            replacement: edit.replacement,
                        }],
                    },
                    history: None,
                },
            }
        };
        self.pending_commit = Some(commit_id);
        self.emit(event);
    }
}

/// Incremental GTK source adapter. Native insert/delete signals carry exact
/// byte ranges through a line-byte index; ordinary typing never snapshots or
/// replaces the whole GtkSourceBuffer.
#[derive(Clone)]
pub struct GtkSourceEditorAdapter {
    buffer: sourceview4::Buffer,
    inner: Rc<RefCell<GtkEditorInner>>,
}

impl GtkSourceEditorAdapter {
    pub fn new(buffer: &sourceview4::Buffer) -> Self {
        let inner = Rc::new(RefCell::new(GtkEditorInner {
            sink: None,
            index: LineByteIndex::from_text(""),
            revision: 0,
            next_commit_id: 0,
            next_composition_id: 0,
            pending_native: None,
            pending_commit: None,
            pending_paint: None,
            composition_tracker: CompositionTracker::default(),
            composition: None,
            stale_composition_guard: false,
            view: None,
            top_visible_byte: 0,
            pending_programmatic_viewport: None,
            observed_programmatic_viewport: None,
            suppress_native: false,
            stats: IncrementalAdapterStats::default(),
        }));
        {
            let inner = Rc::clone(&inner);
            buffer.connect_insert_text(move |buffer, location, text| {
                let mut inner = inner.borrow_mut();
                if inner.suppress_native {
                    return;
                }
                if inner.stale_composition_guard {
                    buffer.stop_signal_emission_by_name("insert-text");
                    return;
                }
                let Ok(byte) = inner.index.byte_at(location.line(), location.line_index()) else {
                    return;
                };
                inner.pending_native = Some(PendingNativeEdit {
                    range: byte..byte,
                    replacement: text.to_owned(),
                });
            });
        }
        {
            let inner = Rc::clone(&inner);
            buffer.connect_delete_range(move |buffer, start, end| {
                let mut inner = inner.borrow_mut();
                if inner.suppress_native {
                    return;
                }
                if inner.stale_composition_guard {
                    buffer.stop_signal_emission_by_name("delete-range");
                    return;
                }
                let Ok(start) = inner.index.byte_at(start.line(), start.line_index()) else {
                    return;
                };
                let Ok(end) = inner.index.byte_at(end.line(), end.line_index()) else {
                    return;
                };
                inner.pending_native = Some(PendingNativeEdit {
                    range: start..end,
                    replacement: String::new(),
                });
            });
        }
        {
            let inner = Rc::clone(&inner);
            buffer.connect_changed(move |_buffer| {
                let mut inner = inner.borrow_mut();
                if !inner.suppress_native {
                    inner.finish_native_edit();
                }
            });
        }
        Self {
            buffer: buffer.clone(),
            inner,
        }
    }

    pub fn bind_view(&self, view: &sourceview4::View) {
        self.inner.borrow_mut().view = Some(view.clone());
        let inner_rc = Rc::clone(&self.inner);
        let buffer = self.buffer.clone();
        view.connect_preedit_changed(move |_view, preedit| {
            let mut inner = inner_rc.borrow_mut();
            if preedit.is_empty() {
                if inner.stale_composition_guard {
                    inner.stale_composition_guard = false;
                    return;
                }
                let expected = inner.composition.as_ref().map(|active| active.id);
                let deferred = Rc::clone(&inner_rc);
                drop(inner);
                gtk::glib::idle_add_local_once(move || {
                    let mut inner = deferred.borrow_mut();
                    if inner.composition.as_ref().map(|active| active.id) == expected
                        && let Some(cancelled) = inner
                            .composition_tracker
                            .cancel(CompositionCancelReason::User)
                    {
                        inner.composition = None;
                        inner.emit(cancelled);
                    }
                });
                return;
            }
            if inner.composition.is_none() {
                let range = buffer
                    .selection_bounds()
                    .and_then(|(start, end)| {
                        Some(
                            inner.index.byte_at(start.line(), start.line_index()).ok()?
                                ..inner.index.byte_at(end.line(), end.line_index()).ok()?,
                        )
                    })
                    .unwrap_or_else(|| {
                        let insert = buffer
                            .get_insert()
                            .map(|mark| buffer.iter_at_mark(&mark))
                            .unwrap_or_else(|| buffer.start_iter());
                        let byte = inner
                            .index
                            .byte_at(insert.line(), insert.line_index())
                            .unwrap_or(inner.index.len);
                        byte..byte
                    });
                inner.next_composition_id = inner.next_composition_id.saturating_add(1);
                let active = GtkComposition {
                    id: inner.next_composition_id,
                    base_revision: inner.revision,
                    range: range.clone(),
                };
                if let Some(started) = inner.composition_tracker.start(
                    active.id,
                    active.base_revision,
                    active.range.clone(),
                ) {
                    inner.composition = Some(active.clone());
                    inner.emit(started);
                }
            }
            if let Some(active) = inner.composition.clone()
                && let Some(updated) =
                    inner
                        .composition_tracker
                        .update(active.id, active.base_revision, preedit)
            {
                inner.emit(updated);
            }
        });
    }

    pub fn native_layout(&self, frame_seq: u64) {
        let mut inner = self.inner.borrow_mut();
        let Some(revision) = inner.pending_paint.take() else {
            return;
        };
        inner.stats.source_paints = inner.stats.source_paints.saturating_add(1);
        inner.emit(EditorEvent::SourcePainted {
            revision,
            frame_seq,
        });
    }

    pub fn stats(&self) -> IncrementalAdapterStats {
        self.inner.borrow().stats
    }

    pub fn observe_viewport(&self, user: bool) -> Result<(), EditorError> {
        let mut inner = self.inner.borrow_mut();
        let view = inner
            .view
            .clone()
            .ok_or_else(|| EditorError::Platform("GTK source view is not bound".to_owned()))?;
        let programmatic = inner.pending_programmatic_viewport.take();
        let visible = view.visible_rect();
        let byte = match view.iter_at_location(visible.x(), visible.y()) {
            Some(iterator) => inner
                .index
                .byte_at(iterator.line(), iterator.line_index())?,
            None => programmatic
                .map(|viewport| viewport.top_visible_byte)
                .ok_or_else(|| {
                    EditorError::Platform("GTK viewport has no text iterator".to_owned())
                })?,
        };
        inner.top_visible_byte = byte;
        let revision = inner.revision;
        if let Some(programmatic) = programmatic {
            inner.observed_programmatic_viewport = Some(ProgrammaticViewport {
                top_visible_byte: byte,
                ..programmatic
            });
        }
        inner.emit(EditorEvent::ViewportChanged {
            revision,
            top_visible_byte: byte,
            user: user && programmatic.is_none(),
        });
        Ok(())
    }

    pub fn take_programmatic_viewport(&self) -> Option<ProgrammaticViewport> {
        self.inner
            .borrow_mut()
            .observed_programmatic_viewport
            .take()
    }

    fn iter_at_byte(&self, byte: usize) -> Result<gtk::TextIter, EditorError> {
        let (line, index) = self.inner.borrow().index.line_and_index(byte)?;
        Ok(self.buffer.iter_at_line_index(line, index))
    }
}

impl EditorAdapter for GtkSourceEditorAdapter {
    fn set_event_sink(&mut self, sink: EditorEventSink) {
        self.inner.borrow_mut().sink = Some(sink);
    }

    fn install_open_snapshot(
        &mut self,
        snapshot: &feathermark_core::DocumentSnapshot,
    ) -> Result<(), EditorError> {
        let text = snapshot.to_string();
        self.inner.borrow_mut().suppress_native = true;
        self.buffer.set_text(&text);
        let mut inner = self.inner.borrow_mut();
        inner.suppress_native = false;
        inner.index = LineByteIndex::from_text(&text);
        inner.revision = snapshot.revision;
        inner.pending_native = None;
        inner.pending_commit = None;
        inner.pending_paint = None;
        inner.top_visible_byte = 0;
        inner.pending_programmatic_viewport = None;
        inner.observed_programmatic_viewport = None;
        inner.stats.full_snapshot_installs = inner.stats.full_snapshot_installs.saturating_add(1);
        Ok(())
    }

    fn acknowledge_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        change: &ChangeSet,
    ) -> Result<(), EditorError> {
        let mut inner = self.inner.borrow_mut();
        if inner.pending_commit != Some(adapter_commit_id) || change.before != inner.revision {
            return Err(EditorError::Platform(
                "unexpected GTK adapter acknowledgement".to_owned(),
            ));
        }
        inner.pending_commit = None;
        inner.revision = change.after;
        inner.pending_paint = Some(change.after);
        inner.stats.acknowledgements = inner.stats.acknowledgements.saturating_add(1);
        Ok(())
    }

    fn reject_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        _reason: LocalCommitRejection,
        authoritative: &feathermark_core::DocumentSnapshot,
    ) -> Result<(), EditorError> {
        if self.inner.borrow().pending_commit != Some(adapter_commit_id) {
            return Err(EditorError::Platform(
                "unexpected GTK adapter rejection".to_owned(),
            ));
        }
        self.install_open_snapshot(authoritative)
    }

    fn apply_external_change(&mut self, change: &ChangeSet) -> Result<(), EditorError> {
        {
            let mut inner = self.inner.borrow_mut();
            if let Some(cancelled) = inner
                .composition_tracker
                .invalidate_for_revision(change.after)
            {
                inner.composition = None;
                inner.stale_composition_guard = true;
                inner.emit(cancelled);
            }
            inner.suppress_native = true;
        }
        for edit in change.edits.iter().rev() {
            let mut start = self.iter_at_byte(edit.byte_range.start)?;
            let mut end = self.iter_at_byte(edit.byte_range.end)?;
            self.buffer.delete(&mut start, &mut end);
            self.buffer.insert(&mut start, &edit.replacement);
            self.inner
                .borrow_mut()
                .index
                .apply(edit.byte_range.clone(), &edit.replacement);
        }
        let mut inner = self.inner.borrow_mut();
        inner.suppress_native = false;
        inner.revision = change.after;
        inner.pending_paint = Some(change.after);
        Ok(())
    }

    fn top_visible_byte(&self, revision: Revision) -> Result<usize, StaleRevision> {
        let inner = self.inner.borrow();
        if revision != inner.revision {
            return Err(StaleRevision {
                expected: inner.revision,
                actual: revision,
            });
        }
        Ok(inner.top_visible_byte)
    }

    fn scroll_to_byte(
        &mut self,
        revision: Revision,
        byte: usize,
        id: InteractionId,
    ) -> Result<(), EditorError> {
        self.top_visible_byte(revision)?;
        let mut iterator = self.iter_at_byte(byte)?;
        self.buffer.place_cursor(&iterator);
        let view = {
            let mut inner = self.inner.borrow_mut();
            inner.pending_programmatic_viewport = Some(ProgrammaticViewport {
                revision,
                top_visible_byte: byte,
                interaction_id: id,
            });
            inner.view.clone()
        };
        if let Some(view) = view.as_ref() {
            view.scroll_to_iter(&mut iterator, 0.1, false, 0.0, 0.0);
        }
        self.inner.borrow_mut().top_visible_byte = byte;
        Ok(())
    }

    fn set_read_only_generated(
        &mut self,
        revision: Revision,
        html: Arc<str>,
    ) -> Result<(), EditorError> {
        self.top_visible_byte(revision)?;
        self.inner.borrow_mut().suppress_native = true;
        self.buffer.set_text(&html);
        let mut inner = self.inner.borrow_mut();
        inner.suppress_native = false;
        inner.index = LineByteIndex::from_text(&html);
        inner.top_visible_byte = 0;
        if let Some(view) = inner.view.as_ref() {
            view.set_editable(false);
        }
        Ok(())
    }
}

pub struct LinuxGtkAdapter;

#[derive(Clone, Copy, Debug)]
struct IdentityScrollAnchor {
    revision: Revision,
    start: usize,
    end: usize,
    ordinal: u32,
}

impl ScrollAnchorView for IdentityScrollAnchor {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinuxScrollDispatch {
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

/// Revision-bound lease controller shared by source and preview callbacks.
/// The preview protocol reports source offsets, so its measured coordinate is
/// represented by the same monotonic byte axis as the render anchors.
pub struct LinuxScrollController {
    map: ScrollMap,
    synchronizer: ScrollSynchronizer,
}

impl LinuxScrollController {
    pub fn next_interaction_id(&self) -> InteractionId {
        self.synchronizer.next_interaction_id()
    }

    pub fn new(
        revision: Revision,
        document_len: usize,
        anchors: impl IntoIterator<Item = (usize, usize, u32)>,
        first_interaction_id: InteractionId,
    ) -> Result<Self, String> {
        let anchors = anchors
            .into_iter()
            .map(|(start, end, ordinal)| IdentityScrollAnchor {
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

    pub fn source_user(
        &mut self,
        top_visible_byte: usize,
        clock: ScrollClock,
    ) -> Result<LinuxScrollDispatch, String> {
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
            ScrollOutcome::Command(command) => LinuxScrollDispatch::Preview {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::PreviewY(y) => y as usize,
                    ScrollTarget::SourceByte(byte) => byte,
                },
                interaction_id: command.interaction_id,
            },
            ScrollOutcome::Suppressed(_) => LinuxScrollDispatch::Suppressed,
        })
    }

    pub fn preview(
        &mut self,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
        clock: ScrollClock,
    ) -> Result<LinuxScrollDispatch, String> {
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
            ScrollOutcome::Command(command) => LinuxScrollDispatch::Source {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::SourceByte(byte) => byte,
                    ScrollTarget::PreviewY(y) => y as usize,
                },
                interaction_id: command.interaction_id,
            },
            ScrollOutcome::Suppressed(_) => LinuxScrollDispatch::Suppressed,
        })
    }

    pub fn source_programmatic(
        &mut self,
        revision: Revision,
        interaction_id: InteractionId,
        clock: ScrollClock,
    ) -> Result<LinuxScrollDispatch, String> {
        let outcome = self
            .synchronizer
            .handle_programmatic(
                revision,
                feathermark_core::Pane::Source,
                interaction_id,
                clock,
            )
            .map_err(|error| error.to_string())?;
        Ok(match outcome {
            ScrollOutcome::Suppressed(_) => LinuxScrollDispatch::Suppressed,
            ScrollOutcome::Command(command) => LinuxScrollDispatch::Preview {
                revision: command.revision,
                source_start: match command.target {
                    ScrollTarget::SourceByte(byte) => byte,
                    ScrollTarget::PreviewY(y) => y as usize,
                },
                interaction_id: command.interaction_id,
            },
        })
    }
}

impl PlatformAdapter for LinuxGtkAdapter {
    fn run() -> Result<(), String> {
        run_application()
    }
}

/// Toolkit-neutral product composition owned by the Linux adapter. Native
/// callbacks enter through these methods; document, render, file, and preview
/// authority remain in the shared Rust contracts.
pub struct LinuxProductSession {
    app: AppState,
    document: Document,
    scheduler: RenderScheduler,
    preview_host: PreviewHost,
    generated_source: Option<(Revision, Arc<str>)>,
    scroll: Option<LinuxScrollController>,
    file_service: LocalFileService,
    next_transaction_id: u64,
    next_scroll_interaction_id: InteractionId,
    stats: Cell<LinuxSessionStats>,
    closed: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LinuxSessionStats {
    pub ui_full_source_flattens: u64,
    pub rope_snapshot_render_submissions: u64,
    pub scroll_events: u64,
}

impl LinuxProductSession {
    pub fn new() -> Result<Self, String> {
        let document = Document::new("").map_err(|error| error.to_string())?;
        let mut app = AppState::new();
        let mut scheduler = RenderScheduler::new();
        for effect in app.reduce(AppMessage::NewDocument) {
            if let AppEffect::ScheduleRender { revision } = effect {
                scheduler.submit(RenderRequest::new(revision, Arc::from("")), 0);
            }
        }
        Ok(Self {
            app,
            document,
            scheduler,
            preview_host: PreviewHost::new(),
            generated_source: None,
            scroll: None,
            file_service: LocalFileService::new(),
            next_transaction_id: 0,
            next_scroll_interaction_id: 1,
            stats: Cell::new(LinuxSessionStats::default()),
            closed: false,
        })
    }

    pub fn revision(&self) -> Revision {
        self.document.revision()
    }

    pub fn dirty(&self) -> bool {
        self.app.dirty()
    }

    pub fn has_external_conflict(&self) -> bool {
        self.app.external_conflict().is_some()
    }

    pub fn preview_ready(&self) -> bool {
        matches!(self.app.preview(), crate::app::PreviewState::Ready { .. })
    }

    pub fn source(&self) -> String {
        let mut stats = self.stats.get();
        stats.ui_full_source_flattens = stats.ui_full_source_flattens.saturating_add(1);
        self.stats.set(stats);
        self.document.snapshot().to_string()
    }

    pub fn stats(&self) -> LinuxSessionStats {
        self.stats.get()
    }

    pub fn path(&self) -> Option<&Path> {
        self.app.path()
    }

    pub fn preview_host(&self) -> &PreviewHost {
        &self.preview_host
    }

    pub fn preview_host_mut(&mut self) -> &mut PreviewHost {
        &mut self.preview_host
    }

    pub fn generated_source(&self) -> Option<(Revision, Arc<str>)> {
        self.generated_source
            .as_ref()
            .map(|(revision, source)| (*revision, Arc::clone(source)))
    }

    pub fn replace_all(&mut self, replacement: &str, now_ms: u64) -> Result<(), String> {
        if self.closed {
            return Err("document session is closed".to_owned());
        }
        self.next_transaction_id = self
            .next_transaction_id
            .checked_add(1)
            .ok_or_else(|| "transaction id overflow".to_owned())?;
        let revision = self.document.revision();
        let change = self
            .document
            .apply(EditTransaction {
                base_revision: revision,
                id: self.next_transaction_id,
                kind: TransactionKind::Programmatic,
                edits: vec![Edit {
                    byte_range: 0..self.document.len_bytes(),
                    replacement: replacement.to_owned(),
                }],
            })
            .map_err(|error| error.to_string())?;
        for effect in self.app.reduce(AppMessage::DocumentEdited {
            revision: change.after,
        }) {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.submit_rope_render(revision, now_ms);
            }
        }
        Ok(())
    }

    pub fn undo(&mut self, now_ms: u64) -> Option<String> {
        self.undo_change(now_ms)?;
        Some(self.source())
    }

    pub fn redo(&mut self, now_ms: u64) -> Option<String> {
        self.redo_change(now_ms)?;
        Some(self.source())
    }

    pub fn undo_change(&mut self, now_ms: u64) -> Option<ChangeSet> {
        let change = self.document.undo()?;
        self.schedule_changed_revision(change.after, now_ms);
        Some(change)
    }

    pub fn redo_change(&mut self, now_ms: u64) -> Option<ChangeSet> {
        let change = self.document.redo()?;
        self.schedule_changed_revision(change.after, now_ms);
        Some(change)
    }

    pub fn new_document(&mut self, now_ms: u64) -> Result<(), String> {
        self.document = Document::new("").map_err(|error| error.to_string())?;
        self.closed = false;
        for effect in self.app.reduce(AppMessage::NewDocument) {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.scheduler
                    .submit(RenderRequest::new(revision, Arc::from("")), now_ms);
            }
        }
        Ok(())
    }

    pub fn open(&mut self, path: &Path, now_ms: u64) -> Result<(), String> {
        let loaded = self
            .file_service
            .load(path, feathermark_core::MAX_DOCUMENT_BYTES)
            .map_err(|error| error.to_string())?;
        self.document = loaded.document;
        self.closed = false;
        let revision = self.document.revision();
        for effect in self.app.reduce(AppMessage::DocumentOpened {
            revision,
            path: path.to_path_buf(),
            disk: loaded.disk,
        }) {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.submit_rope_render(revision, now_ms);
            }
        }
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self
            .app
            .path()
            .map(Path::to_path_buf)
            .ok_or_else(|| "save requires a document path".to_owned())?;
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        let revision = self.document.revision();
        let disk = self
            .file_service
            .save_atomic(path, &self.document.snapshot())
            .map_err(|error| error.to_string())?;
        self.app.reduce(AppMessage::SaveCompleted {
            revision,
            path: path.to_path_buf(),
            disk,
        });
        Ok(())
    }

    pub fn decide_close(&mut self, decision: CloseDecision) -> Result<CloseOutcome, String> {
        if !self.app.dirty() {
            return Ok(CloseOutcome::Close);
        }
        match decision {
            CloseDecision::Cancel => Ok(CloseOutcome::KeepOpen),
            CloseDecision::Discard => Ok(CloseOutcome::Close),
            CloseDecision::Save { untitled_path } => {
                if self.app.path().is_some() {
                    self.save()?;
                } else {
                    let path = untitled_path.ok_or("save panel returned no path".to_owned())?;
                    self.save_as(&path)?;
                }
                Ok(CloseOutcome::Close)
            }
        }
    }

    pub fn inspect_external_change(&mut self, now_ms: u64) -> Result<LinuxExternalOutcome, String> {
        let (Some(path), Some(saved)) = (
            self.app.path().map(Path::to_path_buf),
            self.app.saved_disk().cloned(),
        ) else {
            return Ok(LinuxExternalOutcome::Unchanged);
        };
        match self
            .file_service
            .inspect_external_change(
                &path,
                &saved,
                self.app.dirty(),
                feathermark_core::MAX_DOCUMENT_BYTES,
            )
            .map_err(|error| error.to_string())?
        {
            ExternalChange::Unchanged => Ok(LinuxExternalOutcome::Unchanged),
            ExternalChange::Reloaded(loaded) => {
                self.document = loaded.document;
                let revision = self.document.revision();
                for effect in self.app.reduce(AppMessage::DocumentOpened {
                    revision,
                    path,
                    disk: loaded.disk,
                }) {
                    if let AppEffect::ScheduleRender { revision } = effect {
                        self.submit_rope_render(revision, now_ms);
                    }
                }
                Ok(LinuxExternalOutcome::Reloaded { revision })
            }
            ExternalChange::Conflict { disk } => {
                let effects = self
                    .app
                    .reduce(AppMessage::ExternalConflictDetected { disk });
                Ok(
                    if effects
                        .iter()
                        .any(|effect| matches!(effect, AppEffect::PresentExternalConflict { .. }))
                    {
                        LinuxExternalOutcome::Conflict
                    } else {
                        LinuxExternalOutcome::Unchanged
                    },
                )
            }
        }
    }

    pub fn resolve_external_conflict(
        &mut self,
        resolution: ExternalResolution,
        now_ms: u64,
    ) -> Result<(), String> {
        for effect in self
            .app
            .reduce(AppMessage::ResolveExternalConflict(resolution))
        {
            match effect {
                AppEffect::ReloadExternal { path } => self.open(&path, now_ms)?,
                AppEffect::SaveExternalAs { path } => self.save_as(&path)?,
                _ => {}
            }
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn start_render(&mut self, now_ms: u64) -> Option<RenderPermit> {
        self.scheduler.start_ready(now_ms)
    }

    pub fn finish_render(
        &mut self,
        completed: CompletedRender,
        nonce: [u8; 16],
    ) -> Result<NativeRenderOutcome, String> {
        match self.scheduler.finish(completed, self.document.revision()) {
            Completion::Accepted(page) => {
                let revision = page.revision;
                let page_bytes = page.page.len();
                self.scroll = Some(LinuxScrollController::new(
                    revision,
                    self.document.len_bytes(),
                    page.blocks
                        .iter()
                        .map(|block| (block.start, block.end, block.ordinal)),
                    self.next_scroll_interaction_id,
                )?);
                let generated_source: Arc<str> = Arc::from(page.page);
                self.generated_source = Some((revision, Arc::clone(&generated_source)));
                let render_url = RenderUrl::new(revision, nonce);
                self.preview_host
                    .stage_document(
                        render_url.clone(),
                        Arc::from(generated_source.as_bytes().to_vec()),
                    )
                    .map_err(|error| error.to_string())?;
                self.app.reduce(AppMessage::RenderAccepted {
                    revision,
                    page_bytes,
                });
                Ok(NativeRenderOutcome::Navigate {
                    revision,
                    url: exact_render_url(&render_url),
                })
            }
            Completion::PreviewTooLarge { revision } => {
                self.app.reduce(AppMessage::RenderFailed {
                    revision,
                    error: RenderError::PreviewTooLarge,
                });
                Ok(NativeRenderOutcome::Failed { revision })
            }
            Completion::Failed { revision, error } => {
                self.app
                    .reduce(AppMessage::RenderFailed { revision, error });
                Ok(NativeRenderOutcome::Failed { revision })
            }
            Completion::DiscardedStale { revision } => {
                Ok(NativeRenderOutcome::DiscardedStale { revision })
            }
            Completion::UnknownJob { id } => Err(format!("unknown render job {id}")),
        }
    }

    pub fn handle_ipc(&mut self, bytes: &[u8]) -> Result<Vec<AppEffect>, String> {
        let event = self
            .preview_host
            .handle_ipc(bytes)
            .map_err(|error| error.to_string())?;
        Ok(self.app.reduce(AppMessage::PreviewEvent(event)))
    }

    pub fn source_user_scroll(
        &mut self,
        top_visible_byte: usize,
        clock: ScrollClock,
    ) -> Result<LinuxScrollDispatch, String> {
        let dispatch = self
            .scroll
            .as_mut()
            .ok_or_else(|| "scroll map is not rendered".to_owned())?
            .source_user(top_visible_byte, clock)?;
        self.next_scroll_interaction_id = self
            .scroll
            .as_ref()
            .expect("scroll controller exists")
            .next_interaction_id();
        if !matches!(dispatch, LinuxScrollDispatch::Suppressed) {
            let mut stats = self.stats.get();
            stats.scroll_events = stats.scroll_events.saturating_add(1);
            self.stats.set(stats);
        }
        Ok(dispatch)
    }

    pub fn preview_scroll(
        &mut self,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
        clock: ScrollClock,
    ) -> Result<LinuxScrollDispatch, String> {
        let dispatch = self
            .scroll
            .as_mut()
            .ok_or_else(|| "scroll map is not rendered".to_owned())?
            .preview(source_start, interaction_id, user, clock)?;
        self.next_scroll_interaction_id = self
            .scroll
            .as_ref()
            .expect("scroll controller exists")
            .next_interaction_id();
        if !matches!(dispatch, LinuxScrollDispatch::Suppressed) {
            let mut stats = self.stats.get();
            stats.scroll_events = stats.scroll_events.saturating_add(1);
            self.stats.set(stats);
        }
        Ok(dispatch)
    }

    pub fn source_programmatic_scroll(
        &mut self,
        revision: Revision,
        interaction_id: InteractionId,
        clock: ScrollClock,
    ) -> Result<LinuxScrollDispatch, String> {
        self.scroll
            .as_mut()
            .ok_or_else(|| "scroll map is not rendered".to_owned())?
            .source_programmatic(revision, interaction_id, clock)
    }

    pub fn apply_editor_event(
        &mut self,
        event: EditorEvent,
        now_ms: u64,
    ) -> Result<Option<(AdapterCommitId, ChangeSet)>, String> {
        let EditorEvent::CommitRequested {
            adapter_commit_id,
            commit,
        } = event
        else {
            return Ok(None);
        };
        let change = apply_editor_commit(&mut self.document, adapter_commit_id, commit)
            .map_err(|error| error.to_string())?;
        self.schedule_changed_revision(change.after, now_ms);
        Ok(Some((adapter_commit_id, change)))
    }

    fn schedule_changed_revision(&mut self, revision: Revision, now_ms: u64) {
        for effect in self.app.reduce(AppMessage::DocumentEdited { revision }) {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.submit_rope_render(revision, now_ms);
            }
        }
    }

    fn submit_rope_render(&mut self, revision: Revision, now_ms: u64) {
        let snapshot = self.document.snapshot();
        debug_assert_eq!(snapshot.revision, revision);
        self.scheduler
            .submit(RenderRequest::from_snapshot(snapshot), now_ms);
        let mut stats = self.stats.get();
        stats.rope_snapshot_render_submissions =
            stats.rope_snapshot_render_submissions.saturating_add(1);
        self.stats.set(stats);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeRenderOutcome {
    Navigate { revision: Revision, url: String },
    Failed { revision: Revision },
    DiscardedStale { revision: Revision },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxExternalOutcome {
    Unchanged,
    Reloaded { revision: Revision },
    Conflict,
}

pub fn scroll_delivery_script(bytes: &[u8]) -> Result<String, HostError> {
    if bytes.is_empty() || bytes.len() > feathermark_protocol::MAX_SCROLL_CONTROL_BYTES {
        return Err(HostError::Platform(
            "invalid typed ScrollTo delivery length".to_owned(),
        ));
    }
    let encoded = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "window.__feathermarkReceiveScrollTo(new TextDecoder().decode(new Uint8Array([{encoded}])));"
    ))
}

struct WryScrollSink<'webview>(&'webview WebView);

impl PreviewControlSink for WryScrollSink<'_> {
    fn deliver_scroll_to(&mut self, delivery: ScrollDelivery) -> Result<(), HostError> {
        let script = scroll_delivery_script(delivery.as_bytes())?;
        self.0
            .evaluate_script(&script)
            .map_err(|error| HostError::Platform(error.to_string()))
    }
}

/// Owns the production WebKit context and view as one lifecycle unit. The
/// context intentionally outlives the view so custom-protocol state remains
/// valid until the native child is released.
struct NativeWebState {
    webview: Option<WebView>,
    context: Option<WebContext>,
    visible: bool,
    bounds: Option<(i32, i32)>,
}

impl NativeWebState {
    fn new() -> Self {
        Self {
            webview: None,
            context: Some(WebContext::new(None)),
            visible: true,
            bounds: None,
        }
    }

    fn context_mut(&mut self) -> Result<&mut WebContext, String> {
        self.context
            .as_mut()
            .ok_or_else(|| "WebContext is closed".to_owned())
    }

    fn attach(&mut self, webview: WebView) -> Result<(), String> {
        if self.webview.is_some() {
            return Err("a production WebView is already attached".to_owned());
        }
        self.webview = Some(webview);
        Ok(())
    }

    fn webview(&self) -> Result<&WebView, String> {
        self.webview
            .as_ref()
            .ok_or_else(|| "WebView is closed".to_owned())
    }

    fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        let bounds = (width.max(1), height.max(1));
        if self.bounds == Some(bounds) {
            return Ok(());
        }
        self.webview()?
            .set_bounds(full_bounds(bounds.0, bounds.1))
            .map_err(|error| format!("WebView resize failed: {error}"))?;
        self.bounds = Some(bounds);
        Ok(())
    }

    fn focus(&self) -> Result<(), String> {
        self.webview()?
            .focus()
            .map_err(|error| format!("WebView focus failed: {error}"))
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), String> {
        self.webview()?
            .set_visible(visible)
            .map_err(|error| format!("WebView visibility failed: {error}"))?;
        self.visible = visible;
        Ok(())
    }

    fn suspend(&mut self) -> Result<(), String> {
        self.set_visible(false)
    }

    fn resume(&mut self) -> Result<(), String> {
        self.set_visible(true)
    }

    fn healthy(&self) -> bool {
        self.webview.is_some() && self.context.is_some() && self.visible
    }

    fn is_closed(&self) -> bool {
        self.webview.is_none() && self.context.is_none() && !self.visible
    }

    fn load_url(&self, url: &str) -> Result<(), String> {
        self.webview()?
            .load_url(url)
            .map_err(|error| format!("WebView navigation failed: {error}"))
    }

    fn close(&mut self) {
        // Required order: WebView first, then its WebContext.
        drop(self.webview.take());
        drop(self.context.take());
        self.visible = false;
        self.bounds = None;
    }
}

impl Drop for NativeWebState {
    fn drop(&mut self) {
        self.close();
    }
}

struct RenderWorker {
    request: Option<SyncSender<RenderPermit>>,
    completion: Receiver<CompletedRender>,
    thread: Option<JoinHandle<()>>,
}

impl RenderWorker {
    fn new() -> Self {
        let (request_tx, request_rx) = sync_channel::<RenderPermit>(1);
        let (completion_tx, completion_rx) = sync_channel::<CompletedRender>(1);
        let thread = std::thread::Builder::new()
            .name("feathermark-render".to_owned())
            .spawn(move || {
                while let Ok(permit) = request_rx.recv() {
                    if completion_tx.send(permit.execute()).is_err() {
                        break;
                    }
                }
            })
            .expect("render worker thread must start");
        Self {
            request: Some(request_tx),
            completion: completion_rx,
            thread: Some(thread),
        }
    }

    fn submit(&self, permit: RenderPermit) -> Result<(), String> {
        self.request
            .as_ref()
            .ok_or_else(|| "render worker is closed".to_owned())?
            .try_send(permit)
            .map_err(|error| format!("bounded render queue rejected work: {error}"))
    }

    fn try_recv(&self) -> Result<Option<CompletedRender>, String> {
        match self.completion.try_recv() {
            Ok(completed) => Ok(Some(completed)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("render worker disconnected".to_owned()),
        }
    }
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        drop(self.request.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_application() -> Result<(), String> {
    #[cfg(feature = "test-control")]
    if std::env::var_os("FEATHERMARK_STARTUP_TRACE").is_some() {
        eprintln!("FEATHERMARK_STARTUP_TRACE before-gtk-init");
    }
    gtk::init().map_err(|error| format!("GTK initialization failed: {error}"))?;
    #[cfg(feature = "test-control")]
    if std::env::var_os("FEATHERMARK_STARTUP_TRACE").is_some() {
        eprintln!("FEATHERMARK_STARTUP_TRACE after-gtk-init");
    }
    #[cfg(feature = "test-control")]
    let lifecycle_control = std::env::var_os("FEATHERMARK_SMOKE_AUTOCLOSE_MS").is_some()
        || std::env::var_os("FEATHERMARK_PRODUCT_FUNCTIONAL_PATH").is_some();
    #[cfg(not(feature = "test-control"))]
    let lifecycle_control = false;
    if lifecycle_control && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        eprintln!("FEATHERMARK_ACTIVATION_FAILED reason=missing_session_bus");
        return Err(
            "test-control lifecycle launch requires a D-Bus session (use dbus-run-session)"
                .to_owned(),
        );
    }
    let application_id = if lifecycle_control {
        format!("{APP_ID}.p{}", std::process::id())
    } else {
        APP_ID.to_owned()
    };
    let app_flags = if lifecycle_control {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::empty()
    };
    let application = gtk::Application::new(Some(&application_id), app_flags);
    let startup_error = Rc::new(RefCell::new(None::<String>));
    let error_slot = Rc::clone(&startup_error);
    let activated = Arc::new(AtomicBool::new(false));
    let activation_flag = Arc::clone(&activated);

    application.connect_activate(move |application| {
        activation_flag.store(true, Ordering::Release);
        if lifecycle_control {
            eprintln!("FEATHERMARK_ACTIVATED pid={}", std::process::id());
        }
        if let Err(error) = build_window(application) {
            *error_slot.borrow_mut() = Some(error);
            application.quit();
        } else if lifecycle_control {
            if let Ok(cycle) = std::env::var("FEATHERMARK_LIFECYCLE_CYCLE") {
                println!(r#"{{"type":"ready","cycle":{cycle}}}"#);
                let _ = std::io::stdout().flush();
            }
        }
    });
    if lifecycle_control {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            if !activated.load(Ordering::Acquire) {
                eprintln!("FEATHERMARK_ACTIVATION_FAILED reason=timeout");
                std::process::exit(70);
            }
        });
    }
    #[cfg(feature = "test-control")]
    if std::env::var_os("FEATHERMARK_STARTUP_TRACE").is_some() {
        eprintln!("FEATHERMARK_STARTUP_TRACE before-run");
    }
    application.run_with_args(&["feathermark"]);
    startup_error.borrow_mut().take().map_or(Ok(()), Err)
}

fn build_window(application: &gtk::Application) -> Result<(), String> {
    #[cfg(feature = "test-control")]
    let trace = |stage: &str| {
        if std::env::var_os("FEATHERMARK_STARTUP_TRACE").is_some() {
            eprintln!("FEATHERMARK_STARTUP_TRACE {stage}");
        }
    };
    #[cfg(feature = "test-control")]
    trace("build-window");
    let started = Instant::now();
    let session = Rc::new(RefCell::new(LinuxProductSession::new()?));
    let worker = Rc::new(RenderWorker::new());
    let editor_events = Rc::new(RefCell::new(VecDeque::<EditorEvent>::new()));

    let window = gtk::ApplicationWindow::new(application);
    window.set_title("FeatherMark");
    window.set_default_size(INITIAL_WIDTH as i32, INITIAL_HEIGHT as i32);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_position((INITIAL_WIDTH / 2) as i32);
    let source_buffer = sourceview4::Buffer::builder().max_undo_levels(0).build();
    let source_view = sourceview4::View::with_buffer(&source_buffer);
    source_view.set_show_line_numbers(true);
    source_view.set_monospace(true);
    source_view.set_hexpand(true);
    source_view.set_vexpand(true);
    let mut editor_adapter = GtkSourceEditorAdapter::new(&source_buffer);
    editor_adapter
        .install_open_snapshot(&session.borrow().document.snapshot())
        .map_err(|error| error.to_string())?;
    editor_adapter.bind_view(&source_view);
    {
        let editor_events = Rc::clone(&editor_events);
        editor_adapter.set_event_sink(Box::new(move |event| {
            editor_events.borrow_mut().push_back(event);
        }));
    }
    let editor_adapter = Rc::new(RefCell::new(editor_adapter));
    let source_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    source_scroll.add(&source_view);

    let preview_container = gtk::Fixed::new();
    preview_container.set_hexpand(true);
    preview_container.set_vexpand(true);
    preview_container.set_can_focus(true);
    paned.pack1(&source_scroll, true, false);
    paned.pack2(&preview_container, true, false);
    window.add(&paned);
    window.show_all();
    #[cfg(feature = "test-control")]
    trace("window-shown");

    let initial = session
        .borrow_mut()
        .start_render(crate::render_scheduler::DEBOUNCE_MS)
        .ok_or_else(|| "initial render did not become ready".to_owned())?
        .execute();
    let initial_url = match session
        .borrow_mut()
        .finish_render(initial, random_nonce()?)?
    {
        NativeRenderOutcome::Navigate { url, .. } => url,
        _ => return Err("initial render failed".to_owned()),
    };
    #[cfg(feature = "test-control")]
    trace("initial-rendered");

    let protocol_session = Rc::clone(&session);
    let navigation_session = Rc::clone(&session);
    let download_session = Rc::clone(&session);
    let new_window_session = Rc::clone(&session);
    let ipc_session = Rc::clone(&session);
    let ipc_editor_adapter = Rc::clone(&editor_adapter);
    let ipc_window = window.clone();
    let mut native_web = NativeWebState::new();
    let builder = WebViewBuilder::new_with_web_context(native_web.context_mut()?)
        .with_bounds(full_bounds(
            (INITIAL_WIDTH / 2) as i32,
            INITIAL_HEIGHT as i32,
        ))
        .with_custom_protocol("feathermark".to_owned(), move |_id, request| {
            let request = SchemeRequest::new(request.method().as_str(), request.uri().to_string());
            scheme_response(protocol_session.borrow().preview_host().serve(&request))
        })
        .with_navigation_handler(move |url| {
            navigation_session
                .borrow_mut()
                .preview_host_mut()
                .allow_navigation(&url, NavigationKind::AppInitiated)
        })
        .with_download_started_handler(move |url, _destination| {
            download_session
                .borrow()
                .preview_host()
                .allow_download(&url)
        })
        .with_new_window_req_handler(move |url, _features| {
            if new_window_session
                .borrow()
                .preview_host()
                .allow_new_window(&url)
            {
                NewWindowResponse::Allow
            } else {
                NewWindowResponse::Deny
            }
        })
        .with_ipc_handler(move |request| {
            let effects = match ipc_session
                .borrow_mut()
                .handle_ipc(request.body().as_bytes())
            {
                Ok(effects) => effects,
                Err(error) => {
                    ipc_window.set_title(&format!("FeatherMark — preview rejected: {error}"));
                    return;
                }
            };
            for effect in effects {
                match effect {
                    AppEffect::ScrollSource {
                        revision: _,
                        source_start,
                        interaction_id,
                        user,
                    } => {
                        match ipc_session.borrow_mut().preview_scroll(
                            source_start,
                            interaction_id,
                            user,
                            ScrollClock {
                                monotonic_ms: elapsed_ms(started),
                                preview_frame: 0,
                            },
                        ) {
                            Ok(LinuxScrollDispatch::Source {
                                revision,
                                source_start,
                                interaction_id,
                            }) => {
                                if let Err(error) = ipc_editor_adapter.borrow_mut().scroll_to_byte(
                                    revision,
                                    source_start,
                                    interaction_id,
                                ) {
                                    ipc_window.set_title(&format!(
                                        "FeatherMark — preview scroll rejected: {error}"
                                    ));
                                }
                            }
                            Ok(LinuxScrollDispatch::Suppressed) => {}
                            Ok(LinuxScrollDispatch::Preview { .. }) => {}
                            Err(error) => ipc_window.set_title(&format!(
                                "FeatherMark — stale preview scroll rejected: {error}"
                            )),
                        }
                    }
                    AppEffect::PresentLink(_) => {
                        ipc_window.set_title("FeatherMark — external link blocked");
                    }
                    _ => {}
                }
            }
        })
        .with_url(&initial_url);

    let native_webview = builder
        .build_gtk(&preview_container)
        .map_err(|error| format!("WebKitGTK creation failed: {error}"))?;
    #[cfg(feature = "test-control")]
    trace("webview-built");
    native_web.attach(native_webview)?;
    native_web.resize(
        preview_container.allocated_width(),
        preview_container.allocated_height(),
    )?;
    let native_web = Rc::new(RefCell::new(native_web));

    {
        let session = Rc::clone(&session);
        let editor_adapter = Rc::clone(&editor_adapter);
        let window = window.clone();
        source_view.connect_key_press_event(move |_view, event| {
            if !event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return gtk::glib::Propagation::Proceed;
            }
            let key = event.keyval();
            let shifted = event.state().contains(gtk::gdk::ModifierType::SHIFT_MASK);
            if shifted && key == gtk::gdk::keys::constants::i {
                if let Some((revision, generated)) = session.borrow().generated_source() {
                    if let Err(error) = editor_adapter
                        .borrow_mut()
                        .set_read_only_generated(revision, generated)
                    {
                        window.set_title(&format!(
                            "FeatherMark — generated source inspection failed: {error}"
                        ));
                    } else {
                        window.set_title("FeatherMark — Generated Source (read only)");
                    }
                }
                return gtk::glib::Propagation::Stop;
            }
            if shifted && key == gtk::gdk::keys::constants::r {
                let result = session
                    .borrow_mut()
                    .resolve_external_conflict(ExternalResolution::ReloadDisk, elapsed_ms(started));
                let result = result.and_then(|()| {
                    let snapshot = session.borrow().document.snapshot();
                    editor_adapter
                        .borrow_mut()
                        .install_open_snapshot(&snapshot)
                        .map_err(|error| error.to_string())
                });
                if let Err(error) = result {
                    window.set_title(&format!("FeatherMark — reload failed: {error}"));
                }
                return gtk::glib::Propagation::Stop;
            }
            if shifted && key == gtk::gdk::keys::constants::k {
                if let Err(error) = session
                    .borrow_mut()
                    .resolve_external_conflict(ExternalResolution::KeepBuffer, elapsed_ms(started))
                {
                    window.set_title(&format!("FeatherMark — keep buffer failed: {error}"));
                }
                return gtk::glib::Propagation::Stop;
            }
            let change = if key == gtk::gdk::keys::constants::z {
                session.borrow_mut().undo_change(elapsed_ms(started))
            } else if key == gtk::gdk::keys::constants::y {
                session.borrow_mut().redo_change(elapsed_ms(started))
            } else {
                if key == gtk::gdk::keys::constants::s {
                    let result = if let Some(path) = session.borrow().path().map(Path::to_path_buf)
                    {
                        session.borrow_mut().save_as(&path)
                    } else {
                        match prompt_save_path(Some(&window), None) {
                            Some(path) => session.borrow_mut().save_as(&path),
                            None => Ok(()),
                        }
                    };
                    if let Err(error) = result {
                        window.set_title(&format!("FeatherMark — save failed: {error}"));
                    }
                    return gtk::glib::Propagation::Stop;
                }
                return gtk::glib::Propagation::Proceed;
            };
            if let Some(change) = change
                && let Err(error) = editor_adapter.borrow_mut().apply_external_change(&change)
            {
                window.set_title(&format!("FeatherMark — history failed: {error}"));
            }
            gtk::glib::Propagation::Stop
        });
    }

    {
        let native_web = Rc::clone(&native_web);
        let window = window.clone();
        preview_container.connect_size_allocate(move |_container, allocation| {
            if let Err(error) = native_web
                .borrow_mut()
                .resize(allocation.width(), allocation.height())
            {
                window.set_title(&format!("FeatherMark — {error}"));
            }
        });
    }

    {
        let native_web = Rc::clone(&native_web);
        let window = window.clone();
        preview_container.connect_focus_in_event(move |_container, _event| {
            if let Err(error) = native_web.borrow().focus() {
                window.set_title(&format!("FeatherMark — {error}"));
            }
            gtk::glib::Propagation::Proceed
        });
    }

    {
        let paned = paned.clone();
        let last_width = Rc::new(Cell::new(0));
        paned
            .clone()
            .connect_size_allocate(move |_paned, allocation| {
                if last_width.replace(allocation.width()) != allocation.width() {
                    paned.set_position(allocation.width() / 2);
                }
            });
    }

    {
        let native_web = Rc::clone(&native_web);
        let window = window.clone();
        preview_container.connect_map(move |_container| {
            if let Err(error) = native_web.borrow_mut().resume() {
                window.set_title(&format!("FeatherMark — resume failed: {error}"));
            }
        });
    }

    {
        let native_web = Rc::clone(&native_web);
        let window = window.clone();
        preview_container.connect_unmap(move |_container| {
            if let Err(error) = native_web.borrow_mut().suspend() {
                window.set_title(&format!("FeatherMark — suspend failed: {error}"));
            }
        });
    }

    {
        let editor_adapter = Rc::clone(&editor_adapter);
        let session = Rc::clone(&session);
        source_scroll
            .vadjustment()
            .connect_value_changed(move |_adjustment| {
                let _ = editor_adapter.borrow().observe_viewport(true);
                if let Some(programmatic) = editor_adapter.borrow().take_programmatic_viewport() {
                    let _ = session.borrow_mut().source_programmatic_scroll(
                        programmatic.revision,
                        programmatic.interaction_id,
                        ScrollClock {
                            monotonic_ms: elapsed_ms(started),
                            preview_frame: 0,
                        },
                    );
                }
            });
    }

    {
        let session = Rc::clone(&session);
        let worker = Rc::clone(&worker);
        let native_web = Rc::clone(&native_web);
        let editor_adapter = Rc::clone(&editor_adapter);
        let editor_events = Rc::clone(&editor_events);
        let frame_seq = Rc::new(RefCell::new(0_u64));
        let last_external_poll_ms = Rc::new(Cell::new(0_u64));
        let window = window.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(RENDER_POLL_MS), move || {
            if session.borrow().is_closed() {
                return gtk::glib::ControlFlow::Break;
            }
            while let Some(event) = editor_events.borrow_mut().pop_front() {
                match event {
                    EditorEvent::CommitRequested {
                        adapter_commit_id,
                        commit,
                    } => {
                        let event = EditorEvent::CommitRequested {
                            adapter_commit_id,
                            commit,
                        };
                        match session
                            .borrow_mut()
                            .apply_editor_event(event, elapsed_ms(started))
                        {
                            Ok(Some((commit_id, change))) => {
                                if let Err(error) = editor_adapter
                                    .borrow_mut()
                                    .acknowledge_local_commit(commit_id, &change)
                                {
                                    window.set_title(&format!(
                                        "FeatherMark — editor acknowledgement failed: {error}"
                                    ));
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                let snapshot = session.borrow().document.snapshot();
                                let _ = editor_adapter.borrow_mut().reject_local_commit(
                                    adapter_commit_id,
                                    LocalCommitRejection::InvalidEdit,
                                    &snapshot,
                                );
                                window.set_title(&format!("FeatherMark — edit rejected: {error}"));
                            }
                        }
                    }
                    EditorEvent::ViewportChanged {
                        revision: _,
                        top_visible_byte,
                        user: true,
                    } => {
                        let webview_slot = native_web.borrow();
                        let dispatch = session.borrow_mut().source_user_scroll(
                            top_visible_byte,
                            ScrollClock {
                                monotonic_ms: elapsed_ms(started),
                                preview_frame: *frame_seq.borrow(),
                            },
                        );
                        if let (
                            Ok(webview),
                            Ok(LinuxScrollDispatch::Preview {
                                revision,
                                source_start,
                                interaction_id,
                            }),
                        ) = (webview_slot.webview(), dispatch)
                        {
                            let mut sink = WryScrollSink(webview);
                            let _ = session.borrow().preview_host().deliver_scroll_to(
                                &mut sink,
                                revision,
                                source_start,
                                interaction_id,
                            );
                        }
                    }
                    _ => {}
                }
            }
            let next_frame = frame_seq.borrow().saturating_add(1);
            *frame_seq.borrow_mut() = next_frame;
            editor_adapter.borrow().native_layout(next_frame);
            let now_ms = elapsed_ms(started);
            if now_ms.saturating_sub(last_external_poll_ms.get()) >= 500 {
                last_external_poll_ms.set(now_ms);
                match session.borrow_mut().inspect_external_change(now_ms) {
                    Ok(LinuxExternalOutcome::Reloaded { .. }) => {
                        let snapshot = session.borrow().document.snapshot();
                        if let Err(error) =
                            editor_adapter.borrow_mut().install_open_snapshot(&snapshot)
                        {
                            window.set_title(&format!(
                                "FeatherMark — external reload mirror failed: {error}"
                            ));
                        }
                    }
                    Ok(LinuxExternalOutcome::Conflict | LinuxExternalOutcome::Unchanged) => {}
                    Err(error) => {
                        window.set_title(&format!(
                            "FeatherMark — external change check failed: {error}"
                        ));
                    }
                }
            }
            if let Some(permit) = session.borrow_mut().start_render(elapsed_ms(started))
                && let Err(error) = worker.submit(permit)
            {
                window.set_title(&format!("FeatherMark — render queue failed: {error}"));
                return gtk::glib::ControlFlow::Break;
            }
            match worker.try_recv() {
                Ok(Some(completed)) => match random_nonce()
                    .and_then(|nonce| session.borrow_mut().finish_render(completed, nonce))
                {
                    Ok(NativeRenderOutcome::Navigate { url, .. }) => {
                        if let Err(error) = native_web.borrow().load_url(&url) {
                            window.set_title(&format!("FeatherMark — {error}"));
                            return gtk::glib::ControlFlow::Break;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        window.set_title(&format!("FeatherMark — render failed: {error}"));
                    }
                },
                Ok(None) => {}
                Err(error) => {
                    window.set_title(&format!("FeatherMark — renderer stopped: {error}"));
                    return gtk::glib::ControlFlow::Break;
                }
            }
            window.set_title(if session.borrow().has_external_conflict() {
                "FeatherMark — File changed on disk (Ctrl+Shift+R reload, Ctrl+Shift+K keep)"
            } else if session.borrow().dirty() {
                "FeatherMark — Modified"
            } else {
                "FeatherMark"
            });
            gtk::glib::ControlFlow::Continue
        });
    }

    {
        let session = Rc::clone(&session);
        let native_web = Rc::clone(&native_web);
        window.connect_delete_event(move |window, _event| {
            #[cfg(feature = "test-control")]
            let automated_close = std::env::var_os("FEATHERMARK_SMOKE_AUTOCLOSE_MS").is_some()
                || std::env::var_os("FEATHERMARK_PRODUCT_FUNCTIONAL_PATH").is_some();
            #[cfg(not(feature = "test-control"))]
            let automated_close = false;
            if !automated_close && session.borrow().dirty() {
                match prompt_dirty_close(window) {
                    CloseDecision::Cancel => return gtk::glib::Propagation::Stop,
                    CloseDecision::Discard => {}
                    CloseDecision::Save { .. } => {
                        let untitled_path = if session.borrow().path().is_some() {
                            None
                        } else {
                            prompt_save_path(Some(window), Some("Untitled.md"))
                        };
                        match session
                            .borrow_mut()
                            .decide_close(CloseDecision::Save { untitled_path })
                        {
                            Ok(CloseOutcome::Close) => {}
                            Ok(CloseOutcome::KeepOpen) | Err(_) => {
                                return gtk::glib::Propagation::Stop;
                            }
                        }
                    }
                }
            }
            session.borrow_mut().close();
            native_web.borrow_mut().close();
            #[cfg(feature = "test-control")]
            if std::env::var_os("FEATHERMARK_SMOKE_AUTOCLOSE_MS").is_some()
                || std::env::var_os("FEATHERMARK_PRODUCT_FUNCTIONAL_PATH").is_some()
            {
                let closed = native_web.borrow().is_closed();
                eprintln!("FEATHERMARK_NATIVE_CLOSE webview_first=true closed={closed}");
                if let Ok(cycle) = std::env::var("FEATHERMARK_LIFECYCLE_CYCLE") {
                    if closed {
                        println!(r#"{{"type":"closed","cycle":{cycle},"webview_first":true,"closed":true}}"#);
                    } else {
                        println!(r#"{{"type":"closed","cycle":{cycle},"webview_first":false,"closed":false}}"#);
                    }
                    let _ = std::io::stdout().flush();
                }
            }
            gtk::glib::Propagation::Proceed
        });
    }

    source_view.grab_focus();
    #[cfg(feature = "test-control")]
    if let Ok(path) = std::env::var("FEATHERMARK_PRODUCT_FUNCTIONAL_PATH") {
        const EXPECTED: &str = "# FeatherMark Linux\n\nNative edit, save, and reopen.\n";
        let buffer = source_buffer.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(100), move || {
            let mut end = buffer.end_iter();
            buffer.insert(&mut end, EXPECTED);
        });

        let lifecycle_web = Rc::clone(&native_web);
        let lifecycle_window = window.clone();
        let lifecycle_view = source_view.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let result = (|| {
                lifecycle_web.borrow_mut().suspend()?;
                lifecycle_web.borrow_mut().resume()?;
                lifecycle_web.borrow_mut().resize(
                    lifecycle_window.allocated_width() / 2,
                    lifecycle_window.allocated_height(),
                )?;
                lifecycle_web.borrow().focus()
            })();
            if let Err(error) = result {
                lifecycle_window.set_title(&format!("FeatherMark — lifecycle failed: {error}"));
            }
            lifecycle_view.grab_focus();
        });

        let save_session = Rc::clone(&session);
        let save_window = window.clone();
        let save_path = path.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Err(error) = save_session.borrow_mut().save_as(Path::new(&save_path)) {
                save_window.set_title(&format!("FeatherMark — functional save failed: {error}"));
            }
        });

        let reopen_session = Rc::clone(&session);
        let reopen_adapter = Rc::clone(&editor_adapter);
        let reopen_window = window.clone();
        let reopen_path = path.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(700), move || {
            let result = reopen_session
                .borrow_mut()
                .open(Path::new(&reopen_path), elapsed_ms(started));
            let result = result.and_then(|()| {
                let snapshot = reopen_session.borrow().document.snapshot();
                reopen_adapter
                    .borrow_mut()
                    .install_open_snapshot(&snapshot)
                    .map_err(|error| error.to_string())
            });
            if let Err(error) = result {
                reopen_window
                    .set_title(&format!("FeatherMark — functional reopen failed: {error}"));
            }
        });

        let scroll_session = Rc::clone(&session);
        let scroll_adapter = Rc::clone(&editor_adapter);
        let scroll_window = window.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(800), move || {
            let result = (|| {
                let interaction_id = scroll_session.borrow().next_scroll_interaction_id;
                let mut session = scroll_session.borrow_mut();
                let dispatch = session.preview_scroll(
                    1,
                    interaction_id,
                    true,
                    ScrollClock {
                        monotonic_ms: elapsed_ms(started),
                        preview_frame: 0,
                    },
                )?;
                if let LinuxScrollDispatch::Source {
                    revision,
                    source_start,
                    interaction_id,
                } = dispatch
                {
                    scroll_adapter
                        .borrow_mut()
                        .scroll_to_byte(revision, source_start, interaction_id)
                        .map_err(|error| error.to_string())?;
                }
                Ok::<(), String>(())
            })();
            if let Err(error) = result {
                scroll_window
                    .set_title(&format!("FeatherMark — functional scroll failed: {error}"));
            }
        });

        let inspect_session = Rc::clone(&session);
        let inspect_adapter = Rc::clone(&editor_adapter);
        let inspect_window = window.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(1_050), move || {
            if let Some((revision, generated)) = inspect_session.borrow().generated_source()
                && let Err(error) = inspect_adapter
                    .borrow_mut()
                    .set_read_only_generated(revision, generated)
            {
                inspect_window.set_title(&format!(
                    "FeatherMark — functional generated source failed: {error}"
                ));
            }
        });

        let receipt_session = Rc::clone(&session);
        let receipt_web = Rc::clone(&native_web);
        let receipt_window = window.clone();
        let receipt_view = source_view.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(1_400), move || {
            let disk = std::fs::read_to_string(&path);
            let session = receipt_session.borrow();
            let passed = disk.as_deref().ok() == Some(EXPECTED)
                && !session.dirty()
                && session.preview_ready()
                && receipt_web.borrow().healthy()
                && !receipt_view.is_editable()
                && session.stats().ui_full_source_flattens == 0
                && session.stats().scroll_events > 0;
            eprintln!(
                "FEATHERMARK_PRODUCT_FUNCTIONAL passed={passed} revision={} dirty={} preview_ready={} web_lifecycle={} generated_read_only={} ui_flattens={} scroll_events={} bytes={}",
                session.revision(),
                session.dirty(),
                session.preview_ready(),
                receipt_web.borrow().healthy(),
                !receipt_view.is_editable(),
                session.stats().ui_full_source_flattens,
                session.stats().scroll_events,
                disk.as_ref().map_or(0, String::len),
            );
            drop(session);
            receipt_window.close();
        });
    }
    #[cfg(feature = "test-control")]
    if let Ok(milliseconds) = std::env::var("FEATHERMARK_SMOKE_AUTOCLOSE_MS")
        && let Ok(milliseconds) = milliseconds.parse::<u64>()
    {
        let smoke_window = window.clone();
        let smoke_session = Rc::clone(&session);
        gtk::glib::timeout_add_local_once(Duration::from_millis(milliseconds), move || {
            eprintln!(
                "FEATHERMARK_SMOKE_READY revision={} dirty={} preview_ready={}",
                smoke_session.borrow().revision(),
                smoke_session.borrow().dirty(),
                smoke_session.borrow().preview_ready()
            );
            smoke_window.close();
        });
    }
    #[cfg(feature = "test-control")]
    trace("callbacks-installed");
    Ok(())
}

fn scheme_response(response: crate::preview_host::SchemeResponse) -> Response<Cow<'static, [u8]>> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Cow::Owned(response.body.as_ref().to_vec()))
        .expect("PreviewHost emits valid static headers")
}

fn full_bounds(width: i32, height: i32) -> Rect {
    Rect {
        position: wry::dpi::LogicalPosition::new(0, 0).into(),
        size: wry::dpi::LogicalSize::new(width.max(1) as u32, height.max(1) as u32).into(),
    }
}

fn exact_render_url(render_url: &RenderUrl) -> String {
    format!("feathermark://preview{}", render_url.document_path())
}

fn prompt_dirty_close(parent: &gtk::ApplicationWindow) -> CloseDecision {
    let dialog = gtk::MessageDialog::new(
        Some(parent),
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        "Save changes to this document before closing?",
    );
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Discard", gtk::ResponseType::No);
    dialog.add_button("Save", gtk::ResponseType::Yes);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let response = dialog.run();
    dialog.close();
    match response {
        gtk::ResponseType::Yes => CloseDecision::Save {
            untitled_path: None,
        },
        gtk::ResponseType::No => CloseDecision::Discard,
        _ => CloseDecision::Cancel,
    }
}

fn prompt_save_path(
    parent: Option<&gtk::ApplicationWindow>,
    current_name: Option<&str>,
) -> Option<PathBuf> {
    let dialog = gtk::FileChooserNative::new(
        Some("Save As"),
        parent,
        gtk::FileChooserAction::Save,
        Some("_Save"),
        Some("_Cancel"),
    );
    if let Some(name) = current_name {
        dialog.set_current_name(name);
    }
    let response = dialog.run();
    let path = dialog.filename();
    dialog.destroy();
    if response == gtk::ResponseType::Accept {
        path
    } else {
        None
    }
}

fn random_nonce() -> Result<[u8; 16], String> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| format!("secure preview nonce generation failed: {error}"))?;
    Ok(nonce)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
