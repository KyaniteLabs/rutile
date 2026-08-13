use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
#[cfg(feature = "test-control")]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gtk::gdk::keys::constants as keys;
use gtk::gio::prelude::ApplicationExtManual;
use gtk::prelude::*;
use rutile_core::{
    AdapterCommitId, ChangeSet, CompositionCancelReason, CompositionId, CompositionTracker,
    Document, Edit, EditTransaction, EditorAdapter, EditorCommit, EditorError, EditorEvent,
    EditorEventSink, ExternalChange, ExternalResolution, FileService, LocalCommitRejection,
    LocalFileService, RenderError, ScrollAnchorView, ScrollClock, ScrollGeometry, ScrollMap,
    ScrollOutcome, ScrollPosition, ScrollSynchronizer, ScrollTarget, StaleRevision,
    TransactionKind, apply_editor_commit,
};
use rutile_core::{
    AutosaveEntryV1, AutosaveRecordOutcome, AutosaveStore, Counts, FindDirection, FindQuery,
    FormatCommand, MatchMode, RecoveredDocument, Selection, SessionStateV1, SessionWindowV1,
    html_to_markdown,
};
use rutile_protocol::RenderUrl;
use rutile_types::{InteractionId, Revision};
use sourceview4::prelude::*;
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder, WebViewBuilderExtUnix};

use super::PlatformAdapter;
use crate::actions::{
    ExportOutput, FindSession, FormatApplied, InsertApplied, ReplaceApplied, SessionRestore,
};
use crate::app::{AppEffect, AppMessage, AppState, CloseDecision, CloseOutcome};
use crate::brand::{PRODUCT_NAME, SOURCE_EDITOR_LABEL, STARTER_DOCUMENT, status_title};
use crate::preview_host::{
    HostError, NavigationKind, PreviewControlSink, PreviewHost, SchemeRequest, ScrollDelivery,
};
use crate::render_scheduler::{
    CompletedRender, Completion, RenderPermit, RenderRequest, RenderScheduler,
};

const APP_ID: &str = "tech.kyanitelabs.rutile";
const INITIAL_WIDTH: u32 = 1100;
const INITIAL_HEIGHT: u32 = 760;
/// Bounded, debounced external-disk polling cadence (LNX-004). The previous
/// design ran a single 10 ms (100 Hz) omnibus timer that drained editor events,
/// polled the render worker, inspected the disk, and refreshed chrome. Wave 2-C
/// replaces that with channel-driven wakeups (editor events and render
/// completions arrive on a `glib::MainContext` channel and wake the loop only
/// when there is work) plus this standalone, debounced disk inspector. 1 Hz
/// keeps idle CPU well under the 1%-of-one-core budget while still surfacing
/// external edits promptly.
const DISK_POLL_MS: u64 = 1_000;
/// Clipboard `(target_name, info_id)` pairs published by copy-as-HTML (LNX-005).
/// HTML is first so rich pastes win; the plain-text fallbacks cover every
/// conventional `text/plain`/`STRING` requestor.
const CLIPBOARD_HTML_TARGETS: [(&str, u32); 5] = [
    ("text/html", 0),
    ("UTF8_STRING", 1),
    ("text/plain", 2),
    ("text/plain;charset=utf-8", 3),
    ("STRING", 4),
];

/// Wakes the GTK main-loop handler when editor or render work is pending.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoopWakeup {
    Work,
}

/// Result of applying the Linux multi-file open policy (INT-001): exactly one
/// primary path is opened; additional paths are counted and surfaced as a
/// warning rather than opened in parallel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinuxOpenDelivery {
    pub primary: Option<PathBuf>,
    pub ignored_extra_count: usize,
    pub warning: Option<String>,
}

/// Plans which path the shared open command should load. Non-file paths are
/// rejected; when multiple files arrive (cold launch or second-instance `%f`
/// delivery) only the first is opened and the rest are reported.
pub fn plan_open_delivery(paths: Vec<PathBuf>) -> LinuxOpenDelivery {
    let mut iter = paths.into_iter();
    let primary = iter.next();
    let extras: Vec<PathBuf> = iter.collect();
    let ignored_extra_count = extras.len();
    let warning = if ignored_extra_count == 0 {
        None
    } else {
        Some(format!(
            "Opened only the first of {} files; additional paths were ignored",
            ignored_extra_count.saturating_add(1)
        ))
    };
    LinuxOpenDelivery {
        primary,
        ignored_extra_count,
        warning,
    }
}

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
    id: CompositionId,
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
    next_composition_id: CompositionId,
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
        self.next_commit_id = AdapterCommitId::new(self.next_commit_id.get().saturating_add(1));
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
                        id: commit_id.get(),
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
            revision: Revision::new(0),
            next_commit_id: AdapterCommitId::new(0),
            next_composition_id: CompositionId::new(0),
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
                inner.next_composition_id =
                    CompositionId::new(inner.next_composition_id.get().saturating_add(1));
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

    /// Current native selection in document byte coordinates. A collapsed cursor
    /// yields an empty [`Selection`]; the anchor is the selection bound and the
    /// head is the insertion point so the format engine sees the caret side.
    pub fn selection(&self) -> Result<Selection, EditorError> {
        let inner = self.inner.borrow();
        if let Some((start, end)) = self.buffer.selection_bounds() {
            let anchor = inner.index.byte_at(start.line(), start.line_index())?;
            let head = inner.index.byte_at(end.line(), end.line_index())?;
            return Ok(Selection { anchor, head });
        }
        let insert = self
            .buffer
            .get_insert()
            .map(|mark| self.buffer.iter_at_mark(&mark))
            .unwrap_or_else(|| self.buffer.start_iter());
        let byte = inner.index.byte_at(insert.line(), insert.line_index())?;
        Ok(Selection::collapsed(byte))
    }

    /// Whether an IME composition (preedit) is currently active. Smart Enter and
    /// other key interceptions must defer to the IME while composing so a CJK
    /// commit-on-Enter is never stolen.
    pub fn is_composing(&self) -> bool {
        self.inner.borrow().composition.is_some()
    }

    /// Installs a document-byte [`Selection`] into the native buffer and scrolls
    /// the caret into view. Used after a programmatic edit (format, smart paste,
    /// find/replace) resyncs the mirror.
    pub fn set_selection(&self, selection: Selection) -> Result<(), EditorError> {
        let anchor = self.iter_at_byte(selection.anchor)?;
        let head = self.iter_at_byte(selection.head)?;
        self.buffer.select_range(&head, &anchor);
        if let Some(view) = self.inner.borrow().view.clone()
            && let Some(mark) = self.buffer.get_insert()
        {
            view.scroll_to_mark(&mark, 0.1, false, 0.0, 0.0);
        }
        Ok(())
    }
}

impl EditorAdapter for GtkSourceEditorAdapter {
    fn set_event_sink(&mut self, sink: EditorEventSink) {
        self.inner.borrow_mut().sink = Some(sink);
    }

    fn install_open_snapshot(
        &mut self,
        snapshot: &rutile_core::DocumentSnapshot,
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
        authoritative: &rutile_core::DocumentSnapshot,
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
                rutile_core::Pane::Source,
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
                    rutile_core::Pane::Preview,
                    ScrollPosition::PreviewY(source_start as f64),
                    clock,
                )
                .map_err(|error| error.to_string())?
        } else {
            self.synchronizer
                .handle_programmatic(revision, rutile_core::Pane::Preview, interaction_id, clock)
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
            .handle_programmatic(revision, rutile_core::Pane::Source, interaction_id, clock)
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
    /// Sole AppState/Document authority (Wave 2-A DocumentSessionCore).
    core: crate::session_core::DocumentSessionCore,
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
        let document = Document::new(STARTER_DOCUMENT).map_err(|error| error.to_string())?;
        let mut app = AppState::new();
        let mut scheduler = RenderScheduler::new();
        for effect in app.reduce(AppMessage::NewDocument) {
            if let AppEffect::ScheduleRender { revision } = effect {
                scheduler.submit(RenderRequest::new(revision, Arc::from(STARTER_DOCUMENT)), 0);
            }
        }
        Ok(Self {
            core: crate::session_core::DocumentSessionCore::from_parts(app, document),
            scheduler,
            preview_host: PreviewHost::new(),
            generated_source: None,
            scroll: None,
            file_service: LocalFileService::new(),
            next_transaction_id: 0,
            next_scroll_interaction_id: InteractionId::new(1),
            stats: Cell::new(LinuxSessionStats::default()),
            closed: false,
        })
    }

    pub fn revision(&self) -> Revision {
        self.core.document().revision()
    }

    pub fn dirty(&self) -> bool {
        self.core.app().dirty()
    }

    pub fn has_external_conflict(&self) -> bool {
        self.core.app().external_conflict().is_some()
    }

    pub fn preview_ready(&self) -> bool {
        matches!(
            self.core.app().preview(),
            crate::app::PreviewState::Ready { .. }
        )
    }

    pub fn source(&self) -> String {
        let mut stats = self.stats.get();
        stats.ui_full_source_flattens = stats.ui_full_source_flattens.saturating_add(1);
        self.stats.set(stats);
        self.core.document().snapshot().to_string()
    }

    pub fn stats(&self) -> LinuxSessionStats {
        self.stats.get()
    }

    pub fn path(&self) -> Option<&Path> {
        self.core.app().path()
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
        let revision = self.core.document().revision();
        let document_len = self.core.document().len_bytes();
        let change = self
            .core
            .document_mut()
            .apply(EditTransaction {
                base_revision: revision,
                id: self.next_transaction_id,
                kind: TransactionKind::Programmatic,
                edits: vec![Edit {
                    byte_range: 0..document_len,
                    replacement: replacement.to_owned(),
                }],
            })
            .map_err(|error| error.to_string())?;
        for effect in self.core.reduce(AppMessage::DocumentEdited {
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
        let change = self.core.document_mut().undo()?;
        self.schedule_changed_revision(change.after, now_ms);
        Some(change)
    }

    pub fn redo_change(&mut self, now_ms: u64) -> Option<ChangeSet> {
        let change = self.core.document_mut().redo()?;
        self.schedule_changed_revision(change.after, now_ms);
        Some(change)
    }

    pub fn new_document(&mut self, now_ms: u64) -> Result<(), String> {
        self.core
            .set_document(Document::new("").map_err(|error| error.to_string())?);
        self.closed = false;
        for effect in self.core.reduce(AppMessage::NewDocument) {
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
            .load(path, rutile_core::MAX_DOCUMENT_BYTES)
            .map_err(|error| error.to_string())?;
        self.core.set_document(loaded.document);
        self.closed = false;
        let revision = self.core.document().revision();
        for effect in self.core.reduce(AppMessage::DocumentOpened {
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
            .core
            .app_mut()
            .path()
            .map(Path::to_path_buf)
            .ok_or_else(|| "save requires a document path".to_owned())?;
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        let revision = self.core.document().revision();
        let path_buf = path.to_path_buf();
        let outcome = self
            .file_service
            .save_atomic(path, &self.core.document().snapshot());
        match outcome {
            rutile_core::SaveOutcome::Committed { disk } => {
                self.core.reduce(AppMessage::SaveCompleted {
                    revision,
                    path: path_buf,
                    disk,
                });
            }
            rutile_core::SaveOutcome::CommittedDurabilityUnknown { disk, .. } => {
                self.core.reduce(AppMessage::SaveDurabilityUnknown {
                    revision,
                    path: path_buf,
                    disk,
                });
            }
            rutile_core::SaveOutcome::NotCommitted { reason } => {
                return Err(reason.to_string());
            }
        };
        Ok(())
    }

    pub fn decide_close(&mut self, decision: CloseDecision) -> Result<CloseOutcome, String> {
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
                    let path = untitled_path.ok_or("save panel returned no path".to_owned())?;
                    self.save_as(&path)?;
                }
                // Durability-unknown saves leave dirty set; keep the window open
                // so the user can re-save to flush the parent directory.
                if self.core.app().dirty() {
                    Ok(CloseOutcome::KeepOpen)
                } else {
                    Ok(CloseOutcome::Close)
                }
            }
        }
    }

    pub fn inspect_external_change(&mut self, now_ms: u64) -> Result<LinuxExternalOutcome, String> {
        let (Some(path), Some(saved)) = (
            self.core.app().path().map(Path::to_path_buf),
            self.core.app().saved_disk().cloned(),
        ) else {
            return Ok(LinuxExternalOutcome::Unchanged);
        };
        match self
            .file_service
            .inspect_external_change(
                &path,
                &saved,
                self.core.app().dirty(),
                rutile_core::MAX_DOCUMENT_BYTES,
            )
            .map_err(|error| error.to_string())?
        {
            ExternalChange::Unchanged => Ok(LinuxExternalOutcome::Unchanged),
            ExternalChange::Reloaded(loaded) => {
                self.core.set_document(loaded.document);
                let revision = self.core.document().revision();
                for effect in self.core.reduce(AppMessage::DocumentOpened {
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
                    .core
                    .app_mut()
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
            .core
            .app_mut()
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
        match self
            .scheduler
            .finish(completed, self.core.document().revision())
        {
            Completion::Accepted(page) => {
                let revision = page.revision;
                let page_bytes = page.page.len();
                self.scroll = Some(LinuxScrollController::new(
                    revision,
                    self.core.document().len_bytes(),
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
                self.core.reduce(AppMessage::RenderAccepted {
                    revision,
                    page_bytes,
                });
                Ok(NativeRenderOutcome::Navigate {
                    revision,
                    url: exact_render_url(&render_url),
                })
            }
            Completion::PreviewTooLarge { revision } => {
                self.core.reduce(AppMessage::RenderFailed {
                    revision,
                    error: RenderError::PreviewTooLarge,
                });
                Ok(NativeRenderOutcome::Failed { revision })
            }
            Completion::Failed { revision, error } => {
                self.core
                    .app_mut()
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
        Ok(self.core.reduce(AppMessage::PreviewEvent(event)))
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
        let change = apply_editor_commit(self.core.document_mut(), adapter_commit_id, commit)
            .map_err(|error| error.to_string())?;
        self.schedule_changed_revision(change.after, now_ms);
        Ok(Some((adapter_commit_id, change)))
    }

    fn schedule_changed_revision(&mut self, revision: Revision, now_ms: u64) {
        for effect in self.core.reduce(AppMessage::DocumentEdited { revision }) {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.submit_rope_render(revision, now_ms);
            }
        }
    }

    fn submit_rope_render(&mut self, revision: Revision, now_ms: u64) {
        let snapshot = self.core.document().snapshot();
        debug_assert_eq!(snapshot.revision, revision);
        self.scheduler
            .submit(RenderRequest::from_snapshot(snapshot), now_ms);
        let mut stats = self.stats.get();
        stats.rope_snapshot_render_submissions =
            stats.rope_snapshot_render_submissions.saturating_add(1);
        self.stats.set(stats);
    }
}

/// Wave 2L shell-integration wiring: native GTK input routed to the frozen
/// Wave-2S [`AppState`] action surface. Every mutating method routes through the
/// shared action (format engine / find-replace / arbitrary insert), then submits
/// the resulting render through the same scheduler path typed edits use. The
/// caller resyncs the native mirror from [`Self::snapshot`] afterwards.
impl LinuxProductSession {
    pub fn snapshot(&self) -> rutile_core::DocumentSnapshot {
        self.core.document().snapshot()
    }

    fn schedule_effects(&mut self, effects: &[AppEffect], now_ms: u64) {
        for effect in effects {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.submit_rope_render(*revision, now_ms);
            }
        }
    }

    /// Applies a [`FormatCommand`] (bold/italic/link/heading/code/quote/list/
    /// checklist, or `SmartEnter`) at `selection` through the shared engine.
    pub fn apply_format(
        &mut self,
        selection: Selection,
        command: FormatCommand,
        now_ms: u64,
    ) -> Result<FormatApplied, String> {
        if self.closed {
            return Err("document session is closed".to_owned());
        }
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.apply_format_command(document, selection, command)
                .map_err(|error| error.to_string())?
        };
        self.schedule_effects(&applied.effects, now_ms);
        Ok(applied)
    }

    /// Smart-Enter keystroke: continues/exits lists, quotes, and checklists.
    pub fn smart_enter(
        &mut self,
        selection: Selection,
        now_ms: u64,
    ) -> Result<FormatApplied, String> {
        self.apply_format(selection, FormatCommand::SmartEnter, now_ms)
    }

    /// Replaces `selection` with `text` as a bounded programmatic edit through
    /// the shared [`AppState::insert_text`] primitive, so the paste advances the
    /// reducer exactly like every other shared edit and returns the
    /// [`ChangeSet`] a shell follows incrementally (viewport-preserving). The
    /// smart-paste path lowers converted-clipboard markdown through here.
    pub fn insert_text(
        &mut self,
        selection: Selection,
        text: &str,
        now_ms: u64,
    ) -> Result<InsertApplied, String> {
        if self.closed {
            return Err("document session is closed".to_owned());
        }
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.insert_text(document, selection, text)
                .map_err(|error| error.to_string())?
        };
        self.schedule_effects(&applied.effects, now_ms);
        Ok(applied)
    }

    /// Smart paste: converts clipboard HTML to markdown via the bounded core
    /// converter, then inserts it over `selection`. On any converter rejection
    /// the caller falls back to plain-text paste.
    pub fn paste_html(
        &mut self,
        selection: Selection,
        html: &str,
        now_ms: u64,
    ) -> Result<InsertApplied, String> {
        let markdown = html_to_markdown(html).map_err(|error| error.to_string())?;
        self.insert_text(selection, &markdown, now_ms)
    }

    // --- find / replace -----------------------------------------------------

    pub fn start_find(&mut self, query: FindQuery, direction: FindDirection, wrap: bool) {
        self.core.app_mut().start_find(query, direction, wrap);
    }

    pub fn end_find(&mut self) {
        self.core.app_mut().end_find();
    }

    pub fn find_session(&self) -> Option<&FindSession> {
        self.core.app().find_session()
    }

    pub fn find_next(
        &mut self,
        from_byte: usize,
    ) -> Result<Option<std::ops::Range<usize>>, String> {
        let (app, document) = self.core.app_mut_and_document();
        app.find_next(document, from_byte)
            .map_err(|error| error.to_string())
    }

    pub fn find_prev(
        &mut self,
        from_byte: usize,
    ) -> Result<Option<std::ops::Range<usize>>, String> {
        let (app, document) = self.core.app_mut_and_document();
        app.find_prev(document, from_byte)
            .map_err(|error| error.to_string())
    }

    pub fn replace_current_match(
        &mut self,
        replacement: String,
        now_ms: u64,
    ) -> Result<ReplaceApplied, String> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.replace_current(document, replacement)
                .map_err(|error| error.to_string())?
        };
        self.schedule_effects(&applied.effects, now_ms);
        Ok(applied)
    }

    pub fn replace_all_matches(
        &mut self,
        replacement: String,
        now_ms: u64,
    ) -> Result<ReplaceApplied, String> {
        let applied = {
            let (app, document) = self.core.app_and_document_mut();
            app.replace_all(document, replacement)
                .map_err(|error| error.to_string())?
        };
        self.schedule_effects(&applied.effects, now_ms);
        Ok(applied)
    }

    // --- export -------------------------------------------------------------

    /// Produces a validated, self-contained export page for Save-as-HTML and
    /// Copy-as-HTML.
    pub fn export_html(&self, title: Option<String>) -> Result<ExportOutput, String> {
        {
            let (app, document) = self.core.app_and_document();
            app.export_html(document, title)
                .map_err(|error| error.to_string())
        }
    }

    /// Writes the export page to `path` for the Save-as-HTML gesture.
    pub fn save_html(&self, path: &Path, title: Option<String>) -> Result<(), String> {
        let output = self.export_html(title)?;
        std::fs::write(path, output.html).map_err(|error| error.to_string())
    }

    // --- counts -------------------------------------------------------------

    pub fn counts(&self) -> Counts {
        self.core.app().counts(self.core.document())
    }

    // --- autosave / recovery / session --------------------------------------

    pub fn bind_autosave(&mut self, store: AutosaveStore) -> Result<(), String> {
        self.core
            .app_mut()
            .bind_autosave(store)
            .map_err(|error| error.to_string())
    }

    pub fn autosave_tick(
        &mut self,
        captured_at_unix_ms: u64,
    ) -> Result<Option<AutosaveEntryV1>, String> {
        let (app, document) = self.core.app_mut_and_document();
        app.autosave_tick(document, captured_at_unix_ms)
            .map_err(|error| error.to_string())
    }

    pub fn recover(&self) -> Result<Option<RecoveredDocument>, String> {
        self.core.app().recover().map_err(|error| error.to_string())
    }

    /// Adopts a recovered document (crash-recovery accept) as the live buffer.
    ///
    /// The recovered document is swapped in directly at its own revision and
    /// associated with its former path, so it becomes a *dirty* buffer editing
    /// that file (a save targets the recovered path). This preserves the
    /// `document_path` the previous insert-as-fresh-edit path dropped, and keeps
    /// the document's revision instead of advancing it by a spurious edit.
    pub fn adopt_recovered(
        &mut self,
        recovered: RecoveredDocument,
        now_ms: u64,
    ) -> Result<(), String> {
        let hint = recovered.entry.document_path.clone().map(PathBuf::from);
        self.core.set_document(recovered.document);
        self.closed = false;
        let effects = {
            let (app, document) = self.core.app_mut_and_document();
            app.adopt_recovered(document, hint)
        };
        self.schedule_effects(&effects, now_ms);
        Ok(())
    }

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

    pub fn save_session_state(&self, state: &SessionStateV1) -> Result<(), String> {
        self.core
            .app()
            .save_session_state(state)
            .map_err(|error| error.to_string())
    }

    pub fn load_session_state(&self) -> Result<Option<SessionStateV1>, String> {
        self.core
            .app()
            .load_session_state()
            .map_err(|error| error.to_string())
    }

    pub fn restore_session(&self, state: &SessionStateV1) -> SessionRestore {
        self.core.app().restore_session(state)
    }

    // --- Wave 2-C: durable notices + shared open/mirror/autosave bus --------

    /// Borrows the active durable notices (LNX-001/LNX-002). Notices persist in
    /// the shared status model across render and disk polling until the user
    /// dismisses them, so a transient poll never clears a persistent error.
    pub fn notices(&self) -> &[crate::app::UserNotice] {
        self.core.app().notices()
    }

    /// Returns the most recent undismissed notice, if any, for chrome rendering.
    pub fn latest_notice(&self) -> Option<crate::app::UserNotice> {
        self.core
            .app()
            .notices()
            .iter()
            .rev()
            .find(|notice| !notice.dismissed)
            .cloned()
    }

    /// Dismisses the notice with `id` through the shared reducer.
    pub fn dismiss_notice(&mut self, id: usize) {
        let effects = self.core.reduce(AppMessage::NoticeDismissed { id });
        // Notice dismissal does not schedule renders, but stay consistent with
        // the shared bus: honour any effect the reducer emits.
        for effect in effects {
            if let AppEffect::ScheduleRender { revision } = effect {
                self.submit_rope_render(revision, 0);
            }
        }
    }

    /// Requests opening `path` through the shared open command bus (INT-001).
    /// Returns the reducer effects for the shell to act on (a `PerformOpen`).
    pub fn request_open(&mut self, path: PathBuf) -> Vec<AppEffect> {
        self.core.reduce(AppMessage::OpenDocument { path })
    }

    /// Delivers an application-open or positional path through the shared open
    /// command bus (INT-001): request → perform the load → report completion.
    /// On a load failure the error becomes a durable notice instead of a fatal
    /// exit, and the existing document is preserved. On success the loaded
    /// [`Document`] is swapped in and bound to `path`.
    pub fn open_via_shared_command(&mut self, path: PathBuf, now_ms: u64) -> Vec<AppEffect> {
        let _ = self
            .core
            .app_mut()
            .reduce(AppMessage::OpenDocument { path: path.clone() });
        let result = self
            .file_service
            .load(&path, rutile_core::MAX_DOCUMENT_BYTES)
            .map(|loaded| {
                self.core.set_document(loaded.document);
                self.closed = false;
                (self.core.document().revision(), path, loaded.disk)
            })
            .map_err(|error| error.to_string());
        self.complete_open_request(result, now_ms)
    }

    /// Records a failed save as a durable error notice while keeping the
    /// document dirty (LNX-006). Uses [`AppMessage::SurfaceNotice`] so the
    /// message is not mislabeled as an open failure.
    pub fn report_save_failure(&mut self, error: impl Into<String>) -> Vec<AppEffect> {
        let error = error.into();
        let revision = self.core.document().revision();
        let _ = self.core.reduce(AppMessage::SaveFailed { revision });
        self.core.reduce(AppMessage::SurfaceNotice {
            severity: crate::app::NoticeSeverity::Error,
            message: format!("Save failed: {error}"),
            source_error: error,
        })
    }

    /// Surfaces a generic, non-save product failure (for example export or
    /// clipboard) as a durable error notice (G002). Unlike
    /// [`report_save_failure`] this records no save-specific reducer state: it
    /// routes only through [`AppMessage::SurfaceNotice`] so any surface can
    /// report failure without a dedicated method. The user-facing message is
    /// `"{context}: {error}"` and the original `error` is preserved verbatim as
    /// `source_error`.
    pub fn report_surface_failure(
        &mut self,
        context: impl Into<String>,
        error: impl Into<String>,
    ) -> Vec<AppEffect> {
        let context = context.into();
        let error = error.into();
        self.core.reduce(AppMessage::SurfaceNotice {
            severity: crate::app::NoticeSeverity::Error,
            message: format!("{context}: {error}"),
            source_error: error,
        })
    }

    /// Surfaces a non-fatal open-delivery warning (for example multi-file
    /// `%f` requests) as a dismissible warning notice.
    pub fn report_open_warning(&mut self, message: impl Into<String>) -> Vec<AppEffect> {
        let message = message.into();
        self.core.reduce(AppMessage::SurfaceNotice {
            severity: crate::app::NoticeSeverity::Warning,
            message: message.clone(),
            source_error: message,
        })
    }

    /// Completes a shared open request with the reducer-ready `(revision, path,
    /// disk)` tuple produced by a successful load, or an error string. The
    /// caller is responsible for swapping the loaded [`Document`] in before
    /// calling this. Success schedules the render; failure pushes a durable
    /// error notice (LNX-002) instead of exiting. Returns the reducer effects so
    /// the shell can render the resulting notice.
    pub fn complete_open_request(
        &mut self,
        result: Result<(Revision, PathBuf, rutile_core::DiskVersion), String>,
        now_ms: u64,
    ) -> Vec<AppEffect> {
        let effects = self
            .core
            .reduce(AppMessage::OpenRequestCompleted { result });
        self.schedule_effects(&effects, now_ms);
        effects
    }

    /// Reports an autosave result through the shared bus. A failure pushes a
    /// durable warning notice (LNX-002) instead of being lost in a transient
    /// status line.
    pub fn complete_autosave(
        &mut self,
        result: Result<AutosaveRecordOutcome, String>,
    ) -> Vec<AppEffect> {
        self.core.reduce(AppMessage::AutosaveCompleted { result })
    }

    /// Reports a session-restore failure as a durable, non-fatal notice
    /// (LNX-002): degraded restore never exits the process.
    pub fn report_restore_failure(&mut self, error: String) -> Vec<AppEffect> {
        self.core.reduce(AppMessage::SurfaceNotice {
            severity: crate::app::NoticeSeverity::Warning,
            message: format!("session restore degraded: {error}"),
            source_error: error,
        })
    }

    /// Drives the shared mirror-failure contract (LNX-003): the first failure
    /// triggers exactly one full authoritative resync; a second failure while a
    /// resync is outstanding surfaces a durable error notice instead of looping.
    pub fn record_mirror_failure(&mut self, error: String) -> Vec<AppEffect> {
        self.core.reduce(AppMessage::MirrorFailed { error })
    }

    /// Completes a one-shot mirror resync. A failed resync pushes a durable
    /// error notice; success clears the pending flag and allows a later failure
    /// to request another single resync (LNX-003).
    pub fn complete_mirror_resync(&mut self, result: Result<(), String>) -> Vec<AppEffect> {
        self.core
            .app_mut()
            .reduce(AppMessage::MirrorResyncCompleted { result })
    }

    /// Performs a full authoritative editor mirror resync after an incremental
    /// failure (the `PerformMirrorResync` effect's shell side, LNX-003). Returns
    /// `Ok(())` once the native mirror is reinstalled; the caller then reports
    /// the outcome through [`complete_mirror_resync`].
    pub fn resync_editor_mirror(
        &mut self,
        adapter: &mut GtkSourceEditorAdapter,
    ) -> Result<(), String> {
        let snapshot = self.core.document().snapshot();
        adapter
            .install_open_snapshot(&snapshot)
            .map_err(|error| error.to_string())
    }
}

/// Dual-target clipboard payload for copy-as-HTML (LNX-005): both `text/html`
/// and a tag-stripped plain-text fallback. GTK publishes every target in
/// [`CLIPBOARD_HTML_TARGETS`] so rich and plain pastes both receive content.
///
/// The payload is constructed from the already-validated, self-contained export
/// page; the plain-text fallback is a bounded tag strip over the same bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinuxClipboardPayload {
    /// The inert, self-contained HTML document (the export page).
    pub html: String,
    /// A tag-stripped plain-text rendering of [`html`](Self::html).
    pub plain: String,
}

impl LinuxClipboardPayload {
    /// Builds the dual-target payload from an export page.
    pub fn for_html_export(html: String) -> Self {
        Self {
            plain: html_to_plain_text(&html),
            html,
        }
    }

    /// Returns the bytes to publish for the target identified by `info_id`
    /// (the `u32` paired with each name in [`CLIPBOARD_HTML_TARGETS`]). `info_id
    /// == 0` is `text/html`; every other id is served the plain-text fallback.
    pub fn bytes_for_info(&self, info_id: u32) -> &[u8] {
        if info_id == 0 {
            self.html.as_bytes()
        } else {
            self.plain.as_bytes()
        }
    }

    /// Returns the target name paired with `info_id`, or `None` if it is not a
    /// published target.
    pub fn target_name(info_id: u32) -> Option<&'static str> {
        CLIPBOARD_HTML_TARGETS
            .iter()
            .find(|(_, id)| *id == info_id)
            .map(|(name, _)| *name)
    }
}

/// Bounded tag stripper that produces a plain-text fallback for clipboard
/// publishing. It removes `<…>` tags, decodes the four character references a
/// self-contained export page emits (`&amp;`, `&lt;`, `&gt;`, `&quot;`,
/// `&#39;`), collapses intra-whitespace runs, and keeps block boundaries
/// readable. It is intentionally conservative: the export page is already
/// scriptless and self-contained, so a perfect renderer is not required, only a
/// faithful plain-text shadow.
fn html_to_plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut last_was_space = true;
    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                // Skip until the matching `>`, tolerating a missing close at EOF.
                for inner in chars.by_ref() {
                    if inner == '>' {
                        break;
                    }
                }
                // Treat a tag boundary as a word separator.
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            '&' => {
                let mut entity = String::new();
                let mut matched = false;
                for _ in 0..8 {
                    match chars.peek() {
                        Some(&next) if next != ';' => {
                            entity.push(next);
                            chars.next();
                        }
                        Some(&';') => {
                            chars.next();
                            matched = true;
                            break;
                        }
                        _ => break,
                    }
                }
                let decoded = match entity.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "#39" => Some('\''),
                    _ => None,
                };
                if let Some(decoded) = decoded {
                    push_collapsed(&mut out, decoded, &mut last_was_space);
                } else if matched {
                    push_collapsed(&mut out, '&', &mut last_was_space);
                    for decoded in entity.chars() {
                        push_collapsed(&mut out, decoded, &mut last_was_space);
                    }
                } else {
                    push_collapsed(&mut out, '&', &mut last_was_space);
                }
            }
            _ => push_collapsed(&mut out, ch, &mut last_was_space),
        }
    }
    out.trim().trim_end_matches('\n').to_owned()
}

fn push_collapsed(out: &mut String, ch: char, last_was_space: &mut bool) {
    let is_space = ch.is_ascii_whitespace();
    if is_space && *last_was_space {
        return;
    }
    out.push(ch);
    *last_was_space = is_space;
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
    if bytes.is_empty() || bytes.len() > rutile_protocol::MAX_SCROLL_CONTROL_BYTES {
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
        "window.__rutileReceiveScrollTo(new TextDecoder().decode(new Uint8Array([{encoded}])));"
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

    #[cfg(feature = "test-control")]
    fn healthy(&self) -> bool {
        self.webview.is_some() && self.context.is_some() && self.visible
    }

    #[cfg(feature = "test-control")]
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
    fn new(wakeup: gtk::glib::Sender<LoopWakeup>) -> Self {
        let (request_tx, request_rx) = sync_channel::<RenderPermit>(1);
        let (completion_tx, completion_rx) = sync_channel::<CompletedRender>(1);
        let main_context = gtk::glib::MainContext::default();
        let thread = std::thread::Builder::new()
            .name("rutile-render".to_owned())
            .spawn(move || {
                while let Ok(permit) = request_rx.recv() {
                    let completed = permit.execute();
                    if completion_tx.send(completed).is_err() {
                        break;
                    }
                    let wake = wakeup.clone();
                    main_context.invoke(move || {
                        let _ = wake.send(LoopWakeup::Work);
                    });
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

fn gio_files_to_paths(files: &[gtk::gio::File]) -> Vec<PathBuf> {
    files
        .iter()
        .filter_map(|file| file.path())
        .filter(|path| path.is_file())
        .collect()
}

fn refresh_window_chrome(
    session: &Rc<RefCell<LinuxProductSession>>,
    window: &gtk::ApplicationWindow,
    status_bar: &gtk::Label,
    last_counts_revision: &Cell<u64>,
) {
    let session_ref = session.borrow();
    let title = if session_ref.has_external_conflict() {
        status_title("File changed on disk (Ctrl+Shift+R reload, Ctrl+Shift+K keep)")
    } else if session_ref.dirty() {
        status_title("Modified")
    } else {
        PRODUCT_NAME.to_owned()
    };
    window.set_title(&title);

    if let Some(notice) = session_ref.latest_notice() {
        status_bar.set_text(&notice.message);
        return;
    }

    let revision = session_ref.revision();
    if last_counts_revision.get() != revision {
        last_counts_revision.set(revision);
        let counts = session_ref.counts();
        let minutes = counts.reading_time_seconds().div_ceil(60);
        status_bar.set_text(&format!(
            "{} words   {} characters   {} min read",
            counts.words, counts.chars, minutes
        ));
    }
}

fn execute_shared_open(
    session: &Rc<RefCell<LinuxProductSession>>,
    editor_adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
    window: &gtk::ApplicationWindow,
    announcement_bar: &gtk::Label,
    path: PathBuf,
    started: Instant,
) {
    let effects = session
        .borrow_mut()
        .open_via_shared_command(path, elapsed_ms(started));
    for effect in effects {
        if let AppEffect::ScheduleRender { revision } = effect {
            session
                .borrow_mut()
                .submit_rope_render(revision, elapsed_ms(started));
        }
    }
    if session.borrow().is_closed() {
        return;
    }
    let snapshot = session.borrow().snapshot();
    if let Err(error) = editor_adapter.borrow_mut().install_open_snapshot(&snapshot) {
        let mirror_effects = session
            .borrow_mut()
            .record_mirror_failure(error.to_string());
        handle_session_effects(
            &mirror_effects,
            session,
            editor_adapter,
            window,
            announcement_bar,
            started,
        );
    }
}

#[allow(clippy::only_used_in_recursion)] // window only feeds the recursion; notice presentation goes through announcement_bar
fn handle_session_effects(
    effects: &[AppEffect],
    session: &Rc<RefCell<LinuxProductSession>>,
    editor_adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
    window: &gtk::ApplicationWindow,
    announcement_bar: &gtk::Label,
    started: Instant,
) {
    for effect in effects {
        match effect {
            AppEffect::PerformMirrorResync => {
                let result = session
                    .borrow_mut()
                    .resync_editor_mirror(&mut editor_adapter.borrow_mut());
                let follow_up = session
                    .borrow_mut()
                    .complete_mirror_resync(result.map_err(|error| error.to_string()));
                handle_session_effects(
                    &follow_up,
                    session,
                    editor_adapter,
                    window,
                    announcement_bar,
                    started,
                );
            }
            AppEffect::PresentNotice { notice } => {
                // A11Y (G006 gap 4): surface the notice message in the dedicated
                // Notification-role live region so AT-SPI/Orca announces it
                // without disturbing the word-count status_bar.
                announcement_bar.set_text(&notice.message);
                announcement_bar.show();
            }
            AppEffect::ScheduleRender { revision } => {
                session
                    .borrow_mut()
                    .submit_rope_render(*revision, elapsed_ms(started));
            }
            _ => {}
        }
    }
}

// Platform pump carries session + render + chrome handles; packing them into a
// struct would obscure the existing shell shape for little gain.
#[allow(clippy::too_many_arguments)]
fn pump_ui_work(
    session: &Rc<RefCell<LinuxProductSession>>,
    worker: &Rc<RenderWorker>,
    native_web: &Rc<RefCell<NativeWebState>>,
    editor_adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
    editor_events: &Rc<RefCell<VecDeque<EditorEvent>>>,
    frame_seq: &Rc<RefCell<u64>>,
    window: &gtk::ApplicationWindow,
    status_bar: &gtk::Label,
    announcement_bar: &gtk::Label,
    last_counts_revision: &Cell<u64>,
    started: Instant,
) -> gtk::glib::ControlFlow {
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
                            let effects = session
                                .borrow_mut()
                                .record_mirror_failure(error.to_string());
                            handle_session_effects(
                                &effects,
                                session,
                                editor_adapter,
                                window,
                                announcement_bar,
                                started,
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let snapshot = session.borrow().snapshot();
                        let _ = editor_adapter.borrow_mut().reject_local_commit(
                            adapter_commit_id,
                            LocalCommitRejection::InvalidEdit,
                            &snapshot,
                        );
                        let _ = session
                            .borrow_mut()
                            .report_open_warning(format!("edit rejected: {error}"));
                    }
                }
            }
            EditorEvent::ViewportChanged {
                revision: _,
                top_visible_byte,
                user: true,
            } => {
                let _ = editor_adapter.borrow().observe_viewport(true);
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

    if let Some(permit) = session.borrow_mut().start_render(elapsed_ms(started))
        && let Err(error) = worker.submit(permit)
    {
        // Recoverable: keep the pump attached so later editor commits still drain.
        let effects = session
            .borrow_mut()
            .report_open_warning(format!("render queue failed: {error}"));
        handle_session_effects(
            &effects,
            session,
            editor_adapter,
            window,
            announcement_bar,
            started,
        );
    }

    match worker.try_recv() {
        Ok(Some(completed)) => match random_nonce()
            .and_then(|nonce| session.borrow_mut().finish_render(completed, nonce))
        {
            Ok(NativeRenderOutcome::Navigate { url, .. }) => {
                if let Err(error) = native_web.borrow().load_url(&url) {
                    let effects = session.borrow_mut().report_open_warning(error.to_string());
                    handle_session_effects(
                        &effects,
                        session,
                        editor_adapter,
                        window,
                        announcement_bar,
                        started,
                    );
                }
            }
            Ok(_) => {}
            Err(error) => {
                let effects = session
                    .borrow_mut()
                    .report_open_warning(format!("render failed: {error}"));
                handle_session_effects(
                    &effects,
                    session,
                    editor_adapter,
                    window,
                    announcement_bar,
                    started,
                );
            }
        },
        Ok(None) => {}
        Err(error) => {
            // Worker death is still recoverable enough to keep the editor open
            // with a durable notice; re-attachable wakeups can continue to drain
            // editor commits and disk polls even if preview is dead.
            let effects = session
                .borrow_mut()
                .report_open_warning(format!("renderer stopped: {error}"));
            handle_session_effects(
                &effects,
                session,
                editor_adapter,
                window,
                announcement_bar,
                started,
            );
        }
    }

    refresh_window_chrome(session, window, status_bar, last_counts_revision);
    gtk::glib::ControlFlow::Continue
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseSaveRetryAction {
    Retry,
    SaveAs,
    Cancel,
}

fn prompt_close_save_retry(parent: &gtk::ApplicationWindow) -> CloseSaveRetryAction {
    let dialog = gtk::MessageDialog::new(
        Some(parent),
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Error,
        gtk::ButtonsType::None,
        "Could not save changes before closing.",
    );
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Save As…", gtk::ResponseType::Apply);
    dialog.add_button("Retry", gtk::ResponseType::Yes);
    dialog.set_default_response(gtk::ResponseType::Yes);
    let response = dialog.run();
    dialog.close();
    match response {
        gtk::ResponseType::Yes => CloseSaveRetryAction::Retry,
        gtk::ResponseType::Apply => CloseSaveRetryAction::SaveAs,
        _ => CloseSaveRetryAction::Cancel,
    }
}

fn run_application() -> Result<(), String> {
    #[cfg(feature = "test-control")]
    if std::env::var_os("RUTILE_STARTUP_TRACE").is_some() {
        eprintln!("RUTILE_STARTUP_TRACE before-gtk-init");
    }
    gtk::init().map_err(|error| format!("GTK initialization failed: {error}"))?;
    #[cfg(feature = "test-control")]
    if std::env::var_os("RUTILE_STARTUP_TRACE").is_some() {
        eprintln!("RUTILE_STARTUP_TRACE after-gtk-init");
    }
    #[cfg(feature = "test-control")]
    let lifecycle_control = std::env::var_os("RUTILE_SMOKE_AUTOCLOSE_MS").is_some()
        || std::env::var_os("RUTILE_PRODUCT_FUNCTIONAL_PATH").is_some();
    #[cfg(not(feature = "test-control"))]
    let lifecycle_control = false;
    if lifecycle_control && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        eprintln!("RUTILE_ACTIVATION_FAILED reason=missing_session_bus");
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
    let mut app_flags = gtk::gio::ApplicationFlags::HANDLES_OPEN;
    if lifecycle_control {
        app_flags.insert(gtk::gio::ApplicationFlags::NON_UNIQUE);
    }
    let application = gtk::Application::new(Some(&application_id), app_flags);
    let startup_error = Rc::new(RefCell::new(None::<String>));
    let error_slot = Rc::clone(&startup_error);
    let activated = Arc::new(AtomicBool::new(false));
    let activation_flag = Arc::clone(&activated);
    let pending_open_paths = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let pending_open_warnings = Rc::new(RefCell::new(Vec::<String>::new()));
    let window_ready = Rc::new(Cell::new(false));
    let open_session = Rc::new(RefCell::new(None::<OpenSessionHandles>));

    {
        let pending_open_paths = Rc::clone(&pending_open_paths);
        let pending_open_warnings = Rc::clone(&pending_open_warnings);
        let window_ready = Rc::clone(&window_ready);
        let open_session = Rc::clone(&open_session);
        application.connect_open(move |_application, files, _hint| {
            let paths = gio_files_to_paths(files);
            if paths.is_empty() {
                return;
            }
            let delivery = plan_open_delivery(paths);
            if let Some(primary) = delivery.primary {
                pending_open_paths.borrow_mut().push(primary);
            }
            if let Some(warning) = delivery.warning {
                pending_open_warnings.borrow_mut().push(warning);
            }
            if window_ready.get() {
                if let Some(handles) = open_session.borrow_mut().take() {
                    drain_pending_open_paths(
                        &pending_open_paths,
                        &pending_open_warnings,
                        &handles.session,
                        &handles.editor_adapter,
                        &handles.window,
                        &handles.announcement_bar,
                        handles.started,
                    );
                    *open_session.borrow_mut() = Some(handles);
                }
            }
        });
    }

    application.connect_activate(move |application| {
        activation_flag.store(true, Ordering::Release);
        if lifecycle_control {
            eprintln!("RUTILE_ACTIVATED pid={}", std::process::id());
        }
        if let Err(error) = build_window(
            application,
            Rc::clone(&pending_open_paths),
            Rc::clone(&pending_open_warnings),
            Rc::clone(&window_ready),
            Rc::clone(&open_session),
        ) {
            *error_slot.borrow_mut() = Some(error);
            application.quit();
        } else {
            #[cfg(feature = "test-control")]
            if lifecycle_control && let Ok(cycle) = std::env::var("RUTILE_LIFECYCLE_CYCLE") {
                println!(r#"{{"type":"ready","cycle":{cycle}}}"#);
                let _ = std::io::stdout().flush();
            }
        }
    });
    if lifecycle_control {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            if !activated.load(Ordering::Acquire) {
                eprintln!("RUTILE_ACTIVATION_FAILED reason=timeout");
                std::process::exit(70);
            }
        });
    }
    #[cfg(feature = "test-control")]
    if std::env::var_os("RUTILE_STARTUP_TRACE").is_some() {
        eprintln!("RUTILE_STARTUP_TRACE before-run");
    }
    let args: Vec<String> = std::env::args().collect();
    application.run_with_args(&args);
    startup_error.borrow_mut().take().map_or(Ok(()), Err)
}

struct OpenSessionHandles {
    session: Rc<RefCell<LinuxProductSession>>,
    editor_adapter: Rc<RefCell<GtkSourceEditorAdapter>>,
    window: gtk::ApplicationWindow,
    announcement_bar: gtk::Label,
    started: Instant,
}

fn drain_pending_open_paths(
    pending_paths: &Rc<RefCell<Vec<PathBuf>>>,
    pending_warnings: &Rc<RefCell<Vec<String>>>,
    session: &Rc<RefCell<LinuxProductSession>>,
    editor_adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
    window: &gtk::ApplicationWindow,
    announcement_bar: &gtk::Label,
    started: Instant,
) {
    for warning in pending_warnings.borrow_mut().drain(..) {
        let _ = session.borrow_mut().report_open_warning(warning);
    }
    for path in pending_paths.borrow_mut().drain(..) {
        execute_shared_open(
            session,
            editor_adapter,
            window,
            announcement_bar,
            path,
            started,
        );
    }
}

fn build_window(
    application: &gtk::Application,
    pending_open_paths: Rc<RefCell<Vec<PathBuf>>>,
    pending_open_warnings: Rc<RefCell<Vec<String>>>,
    window_ready: Rc<Cell<bool>>,
    open_session: Rc<RefCell<Option<OpenSessionHandles>>>,
) -> Result<(), String> {
    #[cfg(feature = "test-control")]
    let trace = |stage: &str| {
        if std::env::var_os("RUTILE_STARTUP_TRACE").is_some() {
            eprintln!("RUTILE_STARTUP_TRACE {stage}");
        }
    };
    #[cfg(feature = "test-control")]
    trace("build-window");
    let started = Instant::now();
    let session = Rc::new(RefCell::new(LinuxProductSession::new()?));
    // gtk 0.18 marks MainContext::channel deprecated in favor of async-channel +
    // spawn_future_local. That rewrite is out of scope for W2-C; the wake path
    // is still the supported local-main-context pattern for shell event pumps.
    #[allow(deprecated)]
    let (wakeup_tx, wakeup_rx) =
        gtk::glib::MainContext::channel::<LoopWakeup>(gtk::glib::Priority::DEFAULT);
    let worker = Rc::new(RenderWorker::new(wakeup_tx.clone()));
    let editor_events = Rc::new(RefCell::new(VecDeque::<EditorEvent>::new()));

    let window = gtk::ApplicationWindow::new(application);
    window.set_title(PRODUCT_NAME);
    window.set_default_size(INITIAL_WIDTH as i32, INITIAL_HEIGHT as i32);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_position((INITIAL_WIDTH / 2) as i32);
    let source_buffer = sourceview4::Buffer::builder().max_undo_levels(0).build();
    let source_view = sourceview4::View::with_buffer(&source_buffer);
    source_view.set_show_line_numbers(true);
    source_view.set_monospace(true);
    source_view.set_hexpand(true);
    source_view.set_vexpand(true);
    if let Some(accessible) = source_view.accessible() {
        accessible.set_name(SOURCE_EDITOR_LABEL);
    }
    let mut editor_adapter = GtkSourceEditorAdapter::new(&source_buffer);
    editor_adapter
        .install_open_snapshot(&session.borrow().snapshot())
        .map_err(|error| error.to_string())?;
    editor_adapter.bind_view(&source_view);
    {
        let editor_events = Rc::clone(&editor_events);
        let wakeup_tx = wakeup_tx.clone();
        editor_adapter.set_event_sink(Box::new(move |event| {
            editor_events.borrow_mut().push_back(event);
            let _ = wakeup_tx.send(LoopWakeup::Work);
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

    // Specimen-case visuals (DESIGN-SYSTEM app-chrome): gold needle caret + warm
    // gold selection (CSS) and quiet gold syntax staining on a smoky ground
    // (GtkSourceView style scheme + markdown language). Best-effort; the
    // light-mode oatmeal ground flip is deferred to Wave 3.
    install_app_css();
    stain_source(&source_buffer);
    source_view.set_monospace(false);
    source_view.style_context().add_class("rutile-source");

    // Shared format dispatch: native input (menu + toolbar + accelerators)
    // routes every FormatCommand through the Wave-2S action surface, then
    // resyncs the native mirror and reinstalls the decided selection.
    let format_action: Rc<dyn Fn(FormatCommand)> = {
        let session = Rc::clone(&session);
        let editor_adapter = Rc::clone(&editor_adapter);
        Rc::new(move |command: FormatCommand| {
            let selection = match editor_adapter.borrow().selection() {
                Ok(selection) => selection,
                Err(error) => {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("Format failed", error.to_string());
                    return;
                }
            };
            let result = session
                .borrow_mut()
                .apply_format(selection, command, elapsed_ms(started));
            let applied = match result {
                Ok(applied) => applied,
                Err(error) => {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("Format rejected", error.to_string());
                    return;
                }
            };
            let snapshot = session.borrow().snapshot();
            if let Err(error) = follow_shared_edit(
                &editor_adapter,
                &snapshot,
                &applied.changes,
                applied.selection_after,
            ) {
                let _ = session
                    .borrow_mut()
                    .report_surface_failure("Format mirror failed", error.to_string());
            }
        })
    };

    // Formatting toolbar — text-only, borderless, DEFAULT OFF behind a View
    // toggle (DESIGN-SYSTEM: toolbar default-off, no icons in chrome).
    let toolbar = build_format_toolbar(&format_action);
    // Find/replace bar — hidden until Ctrl+F / Edit ▸ Find.
    let find_bar = FindBar::new(&session, &editor_adapter, &window, &source_view, started);
    // Live-counts status bar.
    let status_bar = gtk::Label::new(Some(""));
    status_bar.set_xalign(0.0);
    status_bar.set_margin_start(8);
    status_bar.set_margin_end(8);
    status_bar.set_margin_top(2);
    status_bar.set_margin_bottom(2);
    status_bar.style_context().add_class("rutile-statusbar");
    // Dedicated status announcement region (G006 gap 4): a Notification-role
    // live region for AppEffect::PresentNotice messages, separate from the
    // word-count status_bar so AT-SPI/Orca announces notices without speaking
    // count churn. Hidden until the first notice arrives; set_no_show_all keeps
    // the initial show_all() from revealing it.
    let announcement_bar = gtk::Label::new(Some(""));
    announcement_bar.set_xalign(0.0);
    announcement_bar.set_margin_start(8);
    announcement_bar.set_margin_end(8);
    announcement_bar.set_margin_top(2);
    announcement_bar.set_margin_bottom(2);
    announcement_bar.set_no_show_all(true);
    announcement_bar
        .style_context()
        .add_class("rutile-announce");
    let announcement_spec = announcement_accessible_spec();
    if let Some(accessible) = announcement_bar.accessible() {
        accessible.set_role(announcement_spec.1);
        accessible.set_name(announcement_spec.0);
    }

    let menubar = build_menu_bar(
        &window,
        &format_action,
        &toolbar,
        &find_bar.container,
        &find_bar.search,
        &session,
    );

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.pack_start(&menubar, false, false, 0);
    root.pack_start(&toolbar, false, false, 0);
    root.pack_start(&find_bar.container, false, false, 0);
    root.pack_start(&paned, true, true, 0);
    root.pack_start(&announcement_bar, false, false, 0);
    root.pack_start(&status_bar, false, false, 0);
    window.add(&root);
    window.show_all();
    // Default-off chrome: hide after the initial show_all so the first frame
    // never flashes them.
    toolbar.hide();
    find_bar.container.hide();
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
    let mut native_web = NativeWebState::new();
    let builder = WebViewBuilder::new_with_web_context(native_web.context_mut()?)
        .with_bounds(full_bounds(
            (INITIAL_WIDTH / 2) as i32,
            INITIAL_HEIGHT as i32,
        ))
        .with_custom_protocol("rutile".to_owned(), move |_id, request| {
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
            let result = ipc_session
                .borrow_mut()
                .handle_ipc(request.body().as_bytes());
            let effects = match result {
                Ok(effects) => effects,
                Err(error) => {
                    let _ = ipc_session
                        .borrow_mut()
                        .report_surface_failure("Preview rejected", error.to_string());
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
                        let result = ipc_session.borrow_mut().preview_scroll(
                            source_start,
                            interaction_id,
                            user,
                            ScrollClock {
                                monotonic_ms: elapsed_ms(started),
                                preview_frame: 0,
                            },
                        );
                        match result {
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
                                    let _ = ipc_session.borrow_mut().report_surface_failure(
                                        "Preview scroll rejected",
                                        error.to_string(),
                                    );
                                }
                            }
                            Ok(LinuxScrollDispatch::Suppressed) => {}
                            Ok(LinuxScrollDispatch::Preview { .. }) => {}
                            Err(error) => {
                                let _ = ipc_session.borrow_mut().report_surface_failure(
                                    "Stale preview scroll rejected",
                                    error.to_string(),
                                );
                            }
                        }
                    }
                    AppEffect::PresentLink(_) => {
                        let _ = ipc_session.borrow_mut().report_surface_failure(
                            "External link blocked",
                            "navigation policy rejected the link",
                        );
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
                let generated_source = session.borrow().generated_source();
                if let Some((revision, generated)) = generated_source {
                    if let Err(error) = editor_adapter
                        .borrow_mut()
                        .set_read_only_generated(revision, generated)
                    {
                        let _ = session.borrow_mut().report_surface_failure(
                            "Generated source inspection failed",
                            error.to_string(),
                        );
                    } else {
                        window.set_title(&status_title("Generated Source (read only)"));
                    }
                }
                return gtk::glib::Propagation::Stop;
            }
            if shifted && key == gtk::gdk::keys::constants::r {
                let result = session
                    .borrow_mut()
                    .resolve_external_conflict(ExternalResolution::ReloadDisk, elapsed_ms(started));
                let result = result.and_then(|()| {
                    let snapshot = session.borrow().snapshot();
                    editor_adapter
                        .borrow_mut()
                        .install_open_snapshot(&snapshot)
                        .map_err(|error| error.to_string())
                });
                if let Err(error) = result {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("Reload failed", error.to_string());
                }
                return gtk::glib::Propagation::Stop;
            }
            if shifted && key == gtk::gdk::keys::constants::k {
                let result = session
                    .borrow_mut()
                    .resolve_external_conflict(ExternalResolution::KeepBuffer, elapsed_ms(started));
                if let Err(error) = result {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("Keep buffer failed", error.to_string());
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
                        // Surface the save failure as a durable notice (matching the
                        // close-path's report_save_failure), not a transient title.
                        let _ = session.borrow_mut().report_save_failure(&error);
                    }
                    return gtk::glib::Propagation::Stop;
                }
                return gtk::glib::Propagation::Proceed;
            };
            if let Some(change) = change
                && let Err(error) = editor_adapter.borrow_mut().apply_external_change(&change)
            {
                let _ = session
                    .borrow_mut()
                    .report_surface_failure("History failed", error.to_string());
            }
            gtk::glib::Propagation::Stop
        });
    }

    // Format accelerators + smart Enter + find toggle. A second handler so the
    // existing edit/scroll handler is untouched: it returns Proceed for these
    // keys, then this handler claims them.
    {
        let session = Rc::clone(&session);
        let editor_adapter = Rc::clone(&editor_adapter);
        let format_action = Rc::clone(&format_action);
        let find_container = find_bar.container.clone();
        let find_search = find_bar.search.clone();
        source_view.connect_key_press_event(move |_view, event| {
            let key = event.keyval();
            let ctrl = event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = event.state().contains(gtk::gdk::ModifierType::SHIFT_MASK);

            // Smart Enter (no Ctrl): continue/exit lists, quotes, checklists.
            // Deferred while an IME composition is active so a CJK commit-on-
            // Enter is never stolen from the input method.
            if !ctrl
                && (key == keys::Return || key == keys::KP_Enter)
                && !editor_adapter.borrow().is_composing()
            {
                let selection = match editor_adapter.borrow().selection() {
                    Ok(selection) => selection,
                    Err(_) => return gtk::glib::Propagation::Proceed,
                };
                let applied = match session
                    .borrow_mut()
                    .smart_enter(selection, elapsed_ms(started))
                {
                    Ok(applied) => applied,
                    Err(_) => return gtk::glib::Propagation::Proceed,
                };
                let snapshot = session.borrow().snapshot();
                let _ = follow_shared_edit(
                    &editor_adapter,
                    &snapshot,
                    &applied.changes,
                    applied.selection_after,
                );
                return gtk::glib::Propagation::Stop;
            }

            if !ctrl {
                return gtk::glib::Propagation::Proceed;
            }

            // Ctrl+F reveals the find/replace bar.
            if !shift && key == keys::f {
                find_container.show_all();
                find_search.grab_focus();
                return gtk::glib::Propagation::Stop;
            }

            let command = if !shift {
                if key == keys::b {
                    Some(FormatCommand::ToggleBold)
                } else if key == keys::i {
                    Some(FormatCommand::ToggleItalic)
                } else if key == keys::k {
                    Some(FormatCommand::InsertLink { url: None })
                } else if key == keys::e {
                    Some(FormatCommand::ToggleCodeSpan)
                } else {
                    None
                }
            } else if key == keys::c || key == keys::C {
                Some(FormatCommand::ToggleCodeBlock)
            } else if key == keys::h || key == keys::H {
                Some(FormatCommand::CycleHeading)
            } else if key == keys::q || key == keys::Q {
                Some(FormatCommand::ToggleQuote)
            } else if key == keys::u || key == keys::U {
                Some(FormatCommand::ToggleBulletList)
            } else if key == keys::o || key == keys::O {
                Some(FormatCommand::ToggleOrderedList)
            } else if key == keys::l || key == keys::L {
                Some(FormatCommand::ToggleChecklist)
            } else {
                None
            };

            if let Some(command) = command {
                format_action(command);
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
    }

    // Smart paste: convert clipboard HTML → markdown via the bounded core
    // converter before insertion; fall back to the native plain-text paste when
    // there is no HTML flavour or the converter rejects the input.
    {
        let session = Rc::clone(&session);
        let editor_adapter = Rc::clone(&editor_adapter);
        source_view.connect_paste_clipboard(move |view| {
            let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
            let Some(data) = clipboard.wait_for_contents(&gtk::gdk::Atom::intern("text/html"))
            else {
                return;
            };
            let bytes = data.data();
            if bytes.is_empty() {
                return;
            }
            let html = String::from_utf8_lossy(&bytes).into_owned();
            let markdown = match rutile_core::html_to_markdown(&html) {
                Ok(markdown) => markdown,
                Err(_) => return,
            };
            let selection = match editor_adapter.borrow().selection() {
                Ok(selection) => selection,
                Err(_) => return,
            };
            // Claim the paste only once we can honour it.
            view.stop_signal_emission_by_name("paste-clipboard");
            let result =
                session
                    .borrow_mut()
                    .insert_text(selection, &markdown, elapsed_ms(started));
            let applied = match result {
                Ok(applied) => applied,
                Err(error) => {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("Smart paste failed", error.to_string());
                    return;
                }
            };
            // Follow the insert incrementally so the viewport is preserved,
            // instead of reinstalling the whole buffer (which reset scroll).
            let snapshot = session.borrow().snapshot();
            let _ = follow_shared_edit(
                &editor_adapter,
                &snapshot,
                &applied.changes,
                applied.selection_after,
            );
        });
    }

    {
        let native_web = Rc::clone(&native_web);
        let session = Rc::clone(&session);
        preview_container.connect_size_allocate(move |_container, allocation| {
            if let Err(error) = native_web
                .borrow_mut()
                .resize(allocation.width(), allocation.height())
            {
                let _ = session
                    .borrow_mut()
                    .report_surface_failure("Preview resize failed", error.to_string());
            }
        });
    }

    {
        let native_web = Rc::clone(&native_web);
        let session = Rc::clone(&session);
        preview_container.connect_focus_in_event(move |_container, _event| {
            if let Err(error) = native_web.borrow().focus() {
                let _ = session
                    .borrow_mut()
                    .report_surface_failure("Preview focus failed", error.to_string());
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
        let session = Rc::clone(&session);
        preview_container.connect_map(move |_container| {
            if let Err(error) = native_web.borrow_mut().resume() {
                let _ = session
                    .borrow_mut()
                    .report_surface_failure("Preview resume failed", error.to_string());
            }
        });
    }

    {
        let native_web = Rc::clone(&native_web);
        let session = Rc::clone(&session);
        preview_container.connect_unmap(move |_container| {
            if let Err(error) = native_web.borrow_mut().suspend() {
                let _ = session
                    .borrow_mut()
                    .report_surface_failure("Preview suspend failed", error.to_string());
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
        let window = window.clone();
        let status_bar = status_bar.clone();
        let announcement_bar = announcement_bar.clone();
        let last_counts_revision = Cell::new(u64::MAX);
        wakeup_rx.attach(None, move |_wakeup| {
            pump_ui_work(
                &session,
                &worker,
                &native_web,
                &editor_adapter,
                &editor_events,
                &frame_seq,
                &window,
                &status_bar,
                &announcement_bar,
                &last_counts_revision,
                started,
            )
        });
        let _ = wakeup_tx.send(LoopWakeup::Work);
    }

    {
        let session = Rc::clone(&session);
        let editor_adapter = Rc::clone(&editor_adapter);
        let window = window.clone();
        let status_bar = status_bar.clone();
        let announcement_bar = announcement_bar.clone();
        let last_counts_revision = Cell::new(u64::MAX);
        gtk::glib::timeout_add_local(Duration::from_millis(DISK_POLL_MS), move || {
            if session.borrow().is_closed() {
                return gtk::glib::ControlFlow::Break;
            }
            let now_ms = elapsed_ms(started);
            match session.borrow_mut().inspect_external_change(now_ms) {
                Ok(LinuxExternalOutcome::Reloaded { .. }) => {
                    let snapshot = session.borrow().snapshot();
                    if let Err(error) = editor_adapter.borrow_mut().install_open_snapshot(&snapshot)
                    {
                        let effects = session
                            .borrow_mut()
                            .record_mirror_failure(error.to_string());
                        handle_session_effects(
                            &effects,
                            &session,
                            &editor_adapter,
                            &window,
                            &announcement_bar,
                            started,
                        );
                    }
                }
                Ok(LinuxExternalOutcome::Conflict | LinuxExternalOutcome::Unchanged) => {}
                Err(error) => {
                    let _ = session
                        .borrow_mut()
                        .report_open_warning(format!("external change check failed: {error}"));
                }
            }
            refresh_window_chrome(&session, &window, &status_bar, &last_counts_revision);
            gtk::glib::ControlFlow::Continue
        });
    }

    {
        let session = Rc::clone(&session);
        let native_web = Rc::clone(&native_web);
        let editor_adapter = Rc::clone(&editor_adapter);
        window.connect_delete_event(move |window, _event| {
            #[cfg(feature = "test-control")]
            let automated_close = std::env::var_os("RUTILE_SMOKE_AUTOCLOSE_MS").is_some()
                || std::env::var_os("RUTILE_PRODUCT_FUNCTIONAL_PATH").is_some();
            #[cfg(not(feature = "test-control"))]
            let automated_close = false;
            if !automated_close && session.borrow().dirty() {
                match prompt_dirty_close(window) {
                    CloseDecision::Cancel => return gtk::glib::Propagation::Stop,
                    CloseDecision::Discard => {}
                    CloseDecision::Save { .. } => loop {
                        let untitled_path = if session.borrow().path().is_some() {
                            None
                        } else {
                            prompt_save_path(Some(window), Some("Untitled.md"))
                        };
                        match session
                            .borrow_mut()
                            .decide_close(CloseDecision::Save { untitled_path })
                        {
                            Ok(CloseOutcome::Close) => break,
                            Ok(CloseOutcome::KeepOpen) => {
                                return gtk::glib::Propagation::Stop;
                            }
                            Err(error) => {
                                let _ = session.borrow_mut().report_save_failure(&error);
                                match prompt_close_save_retry(window) {
                                    CloseSaveRetryAction::Retry => continue,
                                    CloseSaveRetryAction::SaveAs => {
                                        let Some(path) = prompt_save_path(Some(window), None)
                                        else {
                                            return gtk::glib::Propagation::Stop;
                                        };
                                        if let Err(save_error) =
                                            session.borrow_mut().save_as(&path)
                                        {
                                            let _ = session
                                                .borrow_mut()
                                                .report_save_failure(save_error);
                                            continue;
                                        }
                                        if session.borrow().dirty() {
                                            return gtk::glib::Propagation::Stop;
                                        }
                                        break;
                                    }
                                    CloseSaveRetryAction::Cancel => {
                                        return gtk::glib::Propagation::Stop;
                                    }
                                }
                            }
                        }
                    },
                }
            }
            // Persist session-restore state (last file, selection, window
            // frame). A no-op unless an autosave store is bound — i.e. only in
            // the real user session, never under the automated harness.
            let selection = editor_adapter.borrow().selection().ok();
            let (x, y) = window.position();
            let (width, height) = window.size();
            let state = session.borrow().capture_session_state(
                unix_millis(),
                selection,
                None,
                Some(SessionWindowV1 {
                    x,
                    y,
                    width: width.max(0) as u32,
                    height: height.max(0) as u32,
                }),
            );
            if let Err(error) = session.borrow().save_session_state(&state) {
                eprintln!("rutile: session-state save failed: {error}");
            }

            session.borrow_mut().close();
            native_web.borrow_mut().close();
            #[cfg(feature = "test-control")]
            if std::env::var_os("RUTILE_SMOKE_AUTOCLOSE_MS").is_some()
                || std::env::var_os("RUTILE_PRODUCT_FUNCTIONAL_PATH").is_some()
            {
                let closed = native_web.borrow().is_closed();
                eprintln!("RUTILE_NATIVE_CLOSE webview_first=true closed={closed}");
                if let Ok(cycle) = std::env::var("RUTILE_LIFECYCLE_CYCLE") {
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

    // Wave 2L QoL: autosave journal, crash recovery, and session restore. The
    // whole block is skipped under the automated lifecycle/functional harness so
    // the modal recovery prompt never blocks a headless cycle.
    #[cfg(feature = "test-control")]
    let automated_qol = std::env::var_os("RUTILE_SMOKE_AUTOCLOSE_MS").is_some()
        || std::env::var_os("RUTILE_PRODUCT_FUNCTIONAL_PATH").is_some();
    #[cfg(not(feature = "test-control"))]
    let automated_qol = false;
    if !automated_qol {
        let mut recovered_adopted = false;
        if let Some(dir) = autosave_dir()
            && std::fs::create_dir_all(&dir).is_ok()
        {
            if let Err(error) = session.borrow_mut().bind_autosave(AutosaveStore::new(dir)) {
                let _ = session
                    .borrow_mut()
                    .report_restore_failure(format!("autosave disabled: {error}"));
            }
            // Crash recovery: offer the highest verifiable autosave.
            let recovered = session.borrow().recover();
            if let Ok(Some(recovered)) = recovered
                && prompt_recover(&window)
                && session
                    .borrow_mut()
                    .adopt_recovered(recovered, elapsed_ms(started))
                    .is_ok()
            {
                let snapshot = session.borrow().snapshot();
                let _ = editor_adapter.borrow_mut().install_open_snapshot(&snapshot);
                recovered_adopted = true;
            }
        }

        // Session restore: window frame always; last file + selection only when
        // we did not just adopt a recovered buffer.
        if let Ok(Some(state)) = session.borrow().load_session_state() {
            let restore = session.borrow().restore_session(&state);
            if let Some(frame) = restore.window {
                window.move_(frame.x, frame.y);
                window.resize(frame.width.max(1) as i32, frame.height.max(1) as i32);
            }
            if !recovered_adopted && let Some(path) = restore.last_file.clone() {
                execute_shared_open(
                    &session,
                    &editor_adapter,
                    &window,
                    &announcement_bar,
                    path,
                    started,
                );
                if let Some(selection) = restore.selection {
                    if let Err(error) = editor_adapter.borrow().set_selection(selection) {
                        eprintln!(
                            "rutile: set_selection failed after restore; view resyncs on next render: {error}"
                        );
                    }
                }
            }
        }

        // Autosave timer: journal the dirty buffer on a quiet cadence.
        let autosave_session = Rc::clone(&session);
        gtk::glib::timeout_add_local(Duration::from_secs(4), move || {
            if autosave_session.borrow().is_closed() {
                return gtk::glib::ControlFlow::Break;
            }
            if autosave_session.borrow().dirty() {
                if let Err(error) = autosave_session.borrow_mut().autosave_tick(unix_millis()) {
                    let _ = autosave_session.borrow_mut().complete_autosave(Err(error));
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    #[cfg(feature = "test-control")]
    if let Ok(path) = std::env::var("RUTILE_PRODUCT_FUNCTIONAL_PATH") {
        const EXPECTED: &str = "# Rutile Linux\n\nNative edit, save, and reopen.\n";
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
                lifecycle_window.set_title(&status_title(&format!("lifecycle failed: {error}")));
            }
            lifecycle_view.grab_focus();
        });

        let save_session = Rc::clone(&session);
        let save_window = window.clone();
        let save_path = path.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Err(error) = save_session.borrow_mut().save_as(Path::new(&save_path)) {
                save_window.set_title(&status_title(&format!("functional save failed: {error}")));
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
                let snapshot = reopen_session.borrow().snapshot();
                reopen_adapter
                    .borrow_mut()
                    .install_open_snapshot(&snapshot)
                    .map_err(|error| error.to_string())
            });
            if let Err(error) = result {
                reopen_window
                    .set_title(&status_title(&format!("functional reopen failed: {error}")));
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
                    .set_title(&status_title(&format!("functional scroll failed: {error}")));
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
                inspect_window.set_title(&status_title(&format!(
                    "functional generated source failed: {error}"
                )));
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
                "RUTILE_PRODUCT_FUNCTIONAL passed={passed} revision={} dirty={} preview_ready={} web_lifecycle={} generated_read_only={} ui_flattens={} scroll_events={} bytes={}",
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
    if let Ok(milliseconds) = std::env::var("RUTILE_SMOKE_AUTOCLOSE_MS")
        && let Ok(milliseconds) = milliseconds.parse::<u64>()
    {
        let smoke_window = window.clone();
        let smoke_session = Rc::clone(&session);
        gtk::glib::timeout_add_local_once(Duration::from_millis(milliseconds), move || {
            eprintln!(
                "RUTILE_SMOKE_READY revision={} dirty={} preview_ready={}",
                smoke_session.borrow().revision(),
                smoke_session.borrow().dirty(),
                smoke_session.borrow().preview_ready()
            );
            smoke_window.close();
        });
    }
    window_ready.set(true);
    *open_session.borrow_mut() = Some(OpenSessionHandles {
        session: Rc::clone(&session),
        editor_adapter: Rc::clone(&editor_adapter),
        window: window.clone(),
        announcement_bar: announcement_bar.clone(),
        started,
    });
    drain_pending_open_paths(
        &pending_open_paths,
        &pending_open_warnings,
        &session,
        &editor_adapter,
        &window,
        &announcement_bar,
        started,
    );

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
    format!("rutile://preview{}", render_url.document_path())
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

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `$XDG_DATA_HOME/rutile/autosave` (or `~/.local/share/...`) — the
/// per-user journal + session-state directory.
fn autosave_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("rutile").join("autosave"))
}

/// Cache directory for the runtime-materialised GtkSourceView style scheme.
fn scheme_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("rutile").join("styles"))
}

/// Gold needle caret + warm gold selection, applied screen-wide (DESIGN-SYSTEM
/// app-chrome fire budget). The style scheme also sets these; the CSS is the
/// belt-and-braces path for themes that ignore scheme cursor colours.
fn install_app_css() {
    const CSS: &[u8] = b"textview { caret-color: #C9921E; }\n\
                         textview text selection { background-color: rgba(201,146,30,0.28); }\n";
    let provider = gtk::CssProvider::new();
    if provider.load_from_data(CSS).is_ok()
        && let Some(screen) = gtk::gdk::Screen::default()
    {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Materialises the bundled "quartz" style scheme into the user cache dir and
/// resolves it through the manager (the 0.5 binding has no from-string loader).
fn source_style_scheme() -> Option<sourceview4::StyleScheme> {
    const SCHEME_XML: &[u8] = include_bytes!("../../assets/rutile-quartz.xml");
    const SCHEME_ID: &str = "rutile-quartz";
    let dir = scheme_cache_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("rutile-quartz.xml");
    if std::fs::read(&path).ok().as_deref() != Some(SCHEME_XML) {
        std::fs::write(&path, SCHEME_XML).ok()?;
    }
    let manager = sourceview4::StyleSchemeManager::default()?;
    manager.append_search_path(dir.to_str()?);
    manager.scheme(SCHEME_ID)
}

/// Quiet gold syntax staining: markdown language (when installed) + the quartz
/// scheme. When either is absent the buffer simply stays unstained.
fn stain_source(buffer: &sourceview4::Buffer) {
    if let Some(language) =
        sourceview4::LanguageManager::default().and_then(|manager| manager.language("markdown"))
    {
        buffer.set_language(Some(&language));
        buffer.set_highlight_syntax(true);
    }
    if let Some(scheme) = source_style_scheme() {
        buffer.set_style_scheme(Some(&scheme));
    }
}

fn prompt_recover(parent: &gtk::ApplicationWindow) -> bool {
    let dialog = gtk::MessageDialog::new(
        Some(parent),
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        "Recover unsaved changes from the last session?",
    );
    dialog.add_button("Discard", gtk::ResponseType::No);
    dialog.add_button("Recover", gtk::ResponseType::Yes);
    dialog.set_default_response(gtk::ResponseType::Yes);
    let response = dialog.run();
    dialog.close();
    response == gtk::ResponseType::Yes
}

/// Follows a shared (AppState-driven) mutation on the GTK adapter incrementally
/// by replaying its `changes` through `apply_external_change` (the same path
/// external edits and undo/redo use), which preserves the viewport, then
/// installs `selection`. Falls back to a full `install_open_snapshot` from
/// `authoritative` only when a `ChangeSet` cannot be applied incrementally — the
/// fallback replaces the whole buffer, so it recovers correctly even from a
/// partially-applied change sequence.
fn follow_shared_edit(
    adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
    authoritative: &rutile_core::DocumentSnapshot,
    changes: &[ChangeSet],
    selection: Selection,
) -> Result<(), EditorError> {
    let applied_incrementally = !changes.is_empty() && {
        let mut adapter = adapter.borrow_mut();
        changes
            .iter()
            .all(|change| adapter.apply_external_change(change).is_ok())
    };
    if !applied_incrementally {
        adapter.borrow_mut().install_open_snapshot(authoritative)?;
    }
    adapter.borrow().set_selection(selection)
}

/// Text-only, borderless, no-icon formatting toolbar (DESIGN-SYSTEM: default-off
/// chrome). Each button routes through the shared format action.
fn build_format_toolbar(format_action: &Rc<dyn Fn(FormatCommand)>) -> gtk::Box {
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    toolbar.style_context().add_class("rutile-toolbar");
    let specs: [(&str, FormatCommand); 8] = [
        ("Bold", FormatCommand::ToggleBold),
        ("Italic", FormatCommand::ToggleItalic),
        ("Link", FormatCommand::InsertLink { url: None }),
        ("Code", FormatCommand::ToggleCodeSpan),
        ("Heading", FormatCommand::CycleHeading),
        ("Quote", FormatCommand::ToggleQuote),
        ("List", FormatCommand::ToggleBulletList),
        ("Check", FormatCommand::ToggleChecklist),
    ];
    for (label, command) in specs {
        let button = gtk::Button::with_label(label);
        button.set_relief(gtk::ReliefStyle::None);
        // A11Y (toolbar keyboard-focusable + accessible names): the format
        // buttons stay in the focus chain with their visible label as the
        // accessible name, so keyboard-only and AT-SPI users can reach and
        // identify every control. The previous `set_can_focus(false)` removed
        // them from keyboard navigation entirely.
        button.set_can_focus(true);
        button.set_focus_on_click(true);
        if let Some(accessible) = button.accessible() {
            accessible.set_name(label);
            accessible.set_role(gtk::atk::Role::PushButton);
        }
        let action = Rc::clone(format_action);
        button.connect_clicked(move |_| action(command.clone()));
        toolbar.pack_start(&button, false, false, 0);
    }
    toolbar
}

/// The menu bar: File (HTML export), Edit (find), Format (all commands), View
/// (toolbar toggle). Accelerators are handled by the key handler; the labels
/// carry the shortcut hint for discoverability.
fn build_menu_bar(
    window: &gtk::ApplicationWindow,
    format_action: &Rc<dyn Fn(FormatCommand)>,
    toolbar: &gtk::Box,
    find_container: &gtk::Box,
    find_search: &gtk::Entry,
    session: &Rc<RefCell<LinuxProductSession>>,
) -> gtk::MenuBar {
    let menubar = gtk::MenuBar::new();

    let file_menu = gtk::Menu::new();
    let file_root = gtk::MenuItem::with_label("File");
    file_root.set_submenu(Some(&file_menu));
    {
        let item = gtk::MenuItem::with_label("Save as HTML…");
        let session = Rc::clone(session);
        let window = window.clone();
        item.connect_activate(move |_| {
            if let Some(path) = prompt_save_path(Some(&window), Some("export.html")) {
                let result = session.borrow().save_html(&path, None);
                if let Err(error) = result {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("HTML export failed", error.to_string());
                }
            }
        });
        file_menu.append(&item);
    }
    {
        let item = gtk::MenuItem::with_label("Copy as HTML");
        let session = Rc::clone(session);
        item.connect_activate(move |_| {
            let result = session.borrow().export_html(None);
            match result {
                Ok(output) => {
                    // LNX-005: publish both text/html and text/plain so rich and
                    // plain pastes both receive content. The payload holds an HTML
                    // copy and a tag-stripped fallback; `set_with_data` lazily
                    // serves whichever target the requestor asks for.
                    let payload = Rc::new(LinuxClipboardPayload::for_html_export(output.html));
                    let targets: Vec<gtk::TargetEntry> = CLIPBOARD_HTML_TARGETS
                        .iter()
                        .map(|(name, info)| {
                            gtk::TargetEntry::new(name, gtk::TargetFlags::empty(), *info)
                        })
                        .collect();
                    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
                    let provider = Rc::clone(&payload);
                    clipboard.set_with_data(&targets, move |_clip, selection, info| {
                        let bytes = provider.bytes_for_info(info);
                        // The type atom must match the target the requestor asked
                        // for so paste-side target detection (text/html vs plain)
                        // succeeds. `target_name` maps the info id back to its atom.
                        let type_name =
                            LinuxClipboardPayload::target_name(info).unwrap_or("UTF8_STRING");
                        let type_atom = gtk::gdk::Atom::intern(type_name);
                        selection.set(&type_atom, 8, bytes);
                    });
                }
                Err(error) => {
                    let _ = session
                        .borrow_mut()
                        .report_surface_failure("HTML copy failed", error.to_string());
                }
            }
        });
        file_menu.append(&item);
    }
    menubar.append(&file_root);

    let edit_menu = gtk::Menu::new();
    let edit_root = gtk::MenuItem::with_label("Edit");
    edit_root.set_submenu(Some(&edit_menu));
    {
        let item = gtk::MenuItem::with_label("Find / Replace   Ctrl+F");
        let find_container = find_container.clone();
        let find_search = find_search.clone();
        item.connect_activate(move |_| {
            // A11Y (G006 gap 1): Edit ▸ Find must reveal AND focus the search
            // field, matching the Ctrl+F accelerator (keyboard/menu parity).
            find_container.show_all();
            find_search.grab_focus();
        });
        edit_menu.append(&item);
    }

    menubar.append(&edit_root);

    let format_menu = gtk::Menu::new();
    let format_root = gtk::MenuItem::with_label("Format");
    format_root.set_submenu(Some(&format_menu));
    let format_items: [(&str, FormatCommand); 11] = [
        ("Bold   Ctrl+B", FormatCommand::ToggleBold),
        ("Italic   Ctrl+I", FormatCommand::ToggleItalic),
        ("Link   Ctrl+K", FormatCommand::InsertLink { url: None }),
        ("Inline Code   Ctrl+E", FormatCommand::ToggleCodeSpan),
        ("Code Block   Ctrl+Shift+C", FormatCommand::ToggleCodeBlock),
        ("Cycle Heading   Ctrl+Shift+H", FormatCommand::CycleHeading),
        ("Quote   Ctrl+Shift+Q", FormatCommand::ToggleQuote),
        (
            "Bullet List   Ctrl+Shift+U",
            FormatCommand::ToggleBulletList,
        ),
        (
            "Numbered List   Ctrl+Shift+O",
            FormatCommand::ToggleOrderedList,
        ),
        ("Checklist   Ctrl+Shift+L", FormatCommand::ToggleChecklist),
        ("Smart New Line   Enter", FormatCommand::SmartEnter),
    ];
    for (label, command) in format_items {
        let item = gtk::MenuItem::with_label(label);
        let action = Rc::clone(format_action);
        item.connect_activate(move |_| action(command.clone()));
        format_menu.append(&item);
    }
    menubar.append(&format_root);

    let view_menu = gtk::Menu::new();
    let view_root = gtk::MenuItem::with_label("View");
    view_root.set_submenu(Some(&view_menu));
    {
        let item = gtk::CheckMenuItem::with_label("Formatting Toolbar");
        item.set_active(false);
        let toolbar = toolbar.clone();
        item.connect_toggled(move |item| {
            if item.is_active() {
                toolbar.show_all();
            } else {
                toolbar.hide();
            }
        });
        view_menu.append(&item);
    }
    menubar.append(&view_root);

    menubar
}

/// Accessible name + ATK role for the find/replace entries (G006 gap 1). Pure
/// and widget-free so the contract is headlessly testable without a running
/// GTK/AT-SPI stack; the entries previously carried only placeholder text.
pub fn find_entry_accessible_specs() -> [(&'static str, gtk::atk::Role); 2] {
    [
        ("Find", gtk::atk::Role::Entry),
        ("Replace", gtk::atk::Role::Entry),
    ]
}

/// Accessible name + ATK role for the dedicated status announcement region
/// (G006 gap 4). Pure and widget-free so the Notification-role contract is
/// headlessly testable; the live region consumes [`AppEffect::PresentNotice`]
/// messages without disturbing the word-count status bar.
pub fn announcement_accessible_spec() -> (&'static str, gtk::atk::Role) {
    ("Rutile status", gtk::atk::Role::Notification)
}

/// Find/replace bar. Owns its widgets and routes every gesture to the Wave-2S
/// find/replace actions on the session, resyncing the native mirror after any
/// mutation.
struct FindBar {
    container: gtk::Box,
    search: gtk::Entry,
}

impl FindBar {
    fn new(
        session: &Rc<RefCell<LinuxProductSession>>,
        adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
        window: &gtk::ApplicationWindow,
        source_view: &sourceview4::View,
        started: Instant,
    ) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        container.style_context().add_class("rutile-findbar");
        let search = gtk::Entry::new();
        search.set_placeholder_text(Some("Find"));
        let replace = gtk::Entry::new();
        replace.set_placeholder_text(Some("Replace with"));
        // A11Y (G006 gap 1): explicit accessible name + Entry role on the
        // find/replace entries so AT-SPI users can identify them. Previously
        // only the placeholder text labelled these fields.
        let entry_specs = find_entry_accessible_specs();
        if let Some(accessible) = search.accessible() {
            accessible.set_name(entry_specs[0].0);
            accessible.set_role(entry_specs[0].1);
        }
        if let Some(accessible) = replace.accessible() {
            accessible.set_name(entry_specs[1].0);
            accessible.set_role(entry_specs[1].1);
        }
        let prev = gtk::Button::with_label("Prev");
        let next = gtk::Button::with_label("Next");
        let replace_one = gtk::Button::with_label("Replace");
        let replace_all = gtk::Button::with_label("All");
        let close = gtk::Button::with_label("Close");
        container.pack_start(&search, true, true, 0);
        container.pack_start(&prev, false, false, 0);
        container.pack_start(&next, false, false, 0);
        container.pack_start(&replace, true, true, 0);
        container.pack_start(&replace_one, false, false, 0);
        container.pack_start(&replace_all, false, false, 0);
        container.pack_end(&close, false, false, 0);

        // Start (or restart) the session for the current pattern and select the
        // located match, if any.
        let locate: Rc<dyn Fn(bool)> = {
            let session = Rc::clone(session);
            let adapter = Rc::clone(adapter);
            let window = window.clone();
            let search = search.clone();
            Rc::new(move |forward: bool| {
                let pattern = search.text().to_string();
                if pattern.is_empty() {
                    return;
                }
                let query = match FindQuery::new(pattern, MatchMode::Plain, false) {
                    Ok(query) => query,
                    Err(error) => {
                        let _ = session
                            .borrow_mut()
                            .report_surface_failure("Find rejected", error.to_string());
                        return;
                    }
                };
                session
                    .borrow_mut()
                    .start_find(query, FindDirection::Forward, true);
                let from = adapter
                    .borrow()
                    .selection()
                    .map(|selection| selection.head)
                    .unwrap_or(0);
                let result = if forward {
                    session.borrow_mut().find_next(from)
                } else {
                    session.borrow_mut().find_prev(from)
                };
                match result {
                    Ok(Some(range)) => {
                        let _ = adapter.borrow().set_selection(Selection {
                            anchor: range.start,
                            head: range.end,
                        });
                    }
                    Ok(None) => window.set_title(&status_title("No matches")),
                    Err(error) => {
                        let _ = session
                            .borrow_mut()
                            .report_surface_failure("Find failed", error.to_string());
                    }
                }
            })
        };

        {
            let locate = Rc::clone(&locate);
            next.connect_clicked(move |_| locate(true));
        }
        {
            let locate = Rc::clone(&locate);
            prev.connect_clicked(move |_| locate(false));
        }
        {
            let locate = Rc::clone(&locate);
            search.connect_activate(move |_| locate(true));
        }
        {
            let session = Rc::clone(session);
            let adapter = Rc::clone(adapter);
            let replace = replace.clone();
            let locate = Rc::clone(&locate);
            replace_one.connect_clicked(move |_| {
                locate(true);
                let replacement = replace.text().to_string();
                let result = session
                    .borrow_mut()
                    .replace_current_match(replacement, elapsed_ms(started));
                let applied = match result {
                    Ok(applied) => applied,
                    Err(error) => {
                        let _ = session
                            .borrow_mut()
                            .report_surface_failure("Replace failed", error.to_string());
                        return;
                    }
                };
                if let Some(selection) = applied.selection_after {
                    let snapshot = session.borrow().snapshot();
                    let _ = follow_shared_edit(&adapter, &snapshot, &applied.changes, selection);
                }
            });
        }
        {
            let session = Rc::clone(session);
            let adapter = Rc::clone(adapter);
            let window = window.clone();
            let search = search.clone();
            let replace = replace.clone();
            replace_all.connect_clicked(move |_| {
                let pattern = search.text().to_string();
                if pattern.is_empty() {
                    return;
                }
                let query = match FindQuery::new(pattern, MatchMode::Plain, false) {
                    Ok(query) => query,
                    Err(error) => {
                        let _ = session
                            .borrow_mut()
                            .report_surface_failure("Find rejected", error.to_string());
                        return;
                    }
                };
                session
                    .borrow_mut()
                    .start_find(query, FindDirection::Forward, true);
                let replacement = replace.text().to_string();
                let result = session
                    .borrow_mut()
                    .replace_all_matches(replacement, elapsed_ms(started));
                let applied = match result {
                    Ok(applied) => applied,
                    Err(error) => {
                        let _ = session
                            .borrow_mut()
                            .report_surface_failure("Replace all failed", error.to_string());
                        return;
                    }
                };
                if let Some(selection) = applied.selection_after {
                    let snapshot = session.borrow().snapshot();
                    let _ = follow_shared_edit(&adapter, &snapshot, &applied.changes, selection);
                }
                window.set_title(&status_title(&format!("Replaced {}", applied.replaced)));
            });
        }
        {
            let session = Rc::clone(session);
            let container = container.clone();
            let source_view = source_view.clone();
            close.connect_clicked(move |_| {
                session.borrow_mut().end_find();
                container.hide();
                // A11Y (G006 gap 2): return keyboard focus to the source editor
                // when the find bar closes so the user is not stranded.
                source_view.grab_focus();
            });
        }

        Self { container, search }
    }
}
