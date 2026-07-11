use std::ops::Range;
use std::path::{Path, PathBuf};

use feathermark_core::{
    AutosaveEntryV1, AutosaveError, AutosaveStore, ChangeSet, Counts, DiskVersion, Document,
    EditError, EditPlan, ExportError, ExportRequest, ExternalResolution, FindDirection, FindQuery,
    FormatCommand, RecoveredDocument, RenderError, ReplaceSpec, SESSION_SCHEMA_V1, Selection,
    SessionSelectionV1, SessionStateV1, SessionWindowV1, apply_format, render_export_page,
    smart_enter,
};
use feathermark_protocol::PreviewEventV1;
use feathermark_types::{InteractionId, Revision, SafeLinkTarget};

use crate::actions::{
    ActionError, ExportOutput, FindSession, FormatApplied, ReplaceApplied, SessionRestore,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PreviewState {
    #[default]
    Empty,
    Waiting {
        revision: Revision,
    },
    Navigating {
        revision: Revision,
    },
    Ready {
        revision: Revision,
    },
    TooLarge {
        revision: Revision,
    },
    Failed {
        revision: Revision,
        error: RenderError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloseDecision {
    Save { untitled_path: Option<PathBuf> },
    Discard,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseOutcome {
    Close,
    KeepOpen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppMessage {
    NewDocument,
    DocumentOpened {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    DocumentEdited {
        revision: Revision,
    },
    SaveCompleted {
        revision: Revision,
        path: PathBuf,
        disk: DiskVersion,
    },
    SaveFailed {
        revision: Revision,
    },
    ExternalConflictDetected {
        disk: DiskVersion,
    },
    ResolveExternalConflict(ExternalResolution),
    RenderAccepted {
        revision: Revision,
        page_bytes: usize,
    },
    RenderFailed {
        revision: Revision,
        error: RenderError,
    },
    PreviewEvent(PreviewEventV1),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    ScheduleRender {
        revision: Revision,
    },
    NavigatePreview {
        revision: Revision,
        page_bytes: usize,
    },
    ScrollSource {
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
        user: bool,
    },
    PresentLink(SafeLinkTarget),
    PresentExternalConflict {
        path: PathBuf,
        disk: DiskVersion,
    },
    ReloadExternal {
        path: PathBuf,
    },
    SaveExternalAs {
        path: PathBuf,
    },
    IgnoredStale {
        revision: Revision,
    },
}

#[derive(Debug, Default)]
pub struct AppState {
    revision: Revision,
    dirty: bool,
    preview: PreviewState,
    path: Option<PathBuf>,
    saved_disk: Option<DiskVersion>,
    external_conflict: Option<DiskVersion>,
    // Wave 2S shared shell-integration state.
    find: Option<FindSession>,
    autosave: Option<AutosaveStore>,
    autosave_sequence: u64,
    next_transaction_id: u64,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn preview(&self) -> &PreviewState {
        &self.preview
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn saved_disk(&self) -> Option<&DiskVersion> {
        self.saved_disk.as_ref()
    }

    pub fn external_conflict(&self) -> Option<&DiskVersion> {
        self.external_conflict.as_ref()
    }

    pub fn reduce(&mut self, message: AppMessage) -> Vec<AppEffect> {
        match message {
            AppMessage::NewDocument => {
                self.revision = 0;
                self.dirty = false;
                self.path = None;
                self.saved_disk = None;
                self.external_conflict = None;
                self.preview = PreviewState::Waiting { revision: 0 };
                vec![AppEffect::ScheduleRender { revision: 0 }]
            }
            AppMessage::DocumentOpened {
                revision,
                path,
                disk,
            } => {
                self.revision = revision;
                self.dirty = false;
                self.path = Some(path);
                self.saved_disk = Some(disk);
                self.external_conflict = None;
                self.preview = PreviewState::Waiting { revision };
                vec![AppEffect::ScheduleRender { revision }]
            }
            AppMessage::DocumentEdited { revision } if revision <= self.revision => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::DocumentEdited { revision } => {
                self.revision = revision;
                self.dirty = true;
                self.preview = PreviewState::Waiting { revision };
                vec![AppEffect::ScheduleRender { revision }]
            }
            AppMessage::SaveCompleted {
                revision,
                path,
                disk,
            } if revision == self.revision => {
                self.dirty = false;
                self.path = Some(path);
                self.saved_disk = Some(disk);
                self.external_conflict = None;
                vec![]
            }
            AppMessage::SaveCompleted { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::SaveFailed { revision } if revision == self.revision => {
                // A failed save leaves the document dirty and any conflict
                // unresolved; the platform shell must present the error and
                // keep the document open.
                vec![]
            }
            AppMessage::SaveFailed { revision } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::ExternalConflictDetected { disk } => {
                if self.saved_disk.as_ref() == Some(&disk) {
                    return vec![];
                }
                let Some(path) = self.path.clone() else {
                    return vec![];
                };
                self.external_conflict = Some(disk.clone());
                vec![AppEffect::PresentExternalConflict { path, disk }]
            }
            AppMessage::ResolveExternalConflict(resolution) => {
                let Some(disk) = self.external_conflict.take() else {
                    return vec![];
                };
                match resolution {
                    ExternalResolution::ReloadDisk => self
                        .path
                        .clone()
                        .map(|path| vec![AppEffect::ReloadExternal { path }])
                        .unwrap_or_default(),
                    ExternalResolution::KeepBuffer => {
                        self.saved_disk = Some(disk);
                        vec![]
                    }
                    ExternalResolution::SaveBufferAs(path) => {
                        vec![AppEffect::SaveExternalAs { path }]
                    }
                }
            }
            AppMessage::RenderAccepted {
                revision,
                page_bytes,
            } if revision == self.revision => {
                self.preview = PreviewState::Navigating { revision };
                vec![AppEffect::NavigatePreview {
                    revision,
                    page_bytes,
                }]
            }
            AppMessage::RenderAccepted { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::RenderFailed { revision, error } if revision == self.revision => {
                self.preview = match error {
                    RenderError::PreviewTooLarge => PreviewState::TooLarge { revision },
                    error => PreviewState::Failed { revision, error },
                };
                vec![]
            }
            AppMessage::RenderFailed { revision, .. } => {
                vec![AppEffect::IgnoredStale { revision }]
            }
            AppMessage::PreviewEvent(event) => self.reduce_preview_event(event),
        }
    }

    fn reduce_preview_event(&mut self, event: PreviewEventV1) -> Vec<AppEffect> {
        let revision = event_revision(&event);
        if revision != self.revision {
            return vec![AppEffect::IgnoredStale { revision }];
        }

        match event {
            PreviewEventV1::BridgeReady { .. } => vec![],
            PreviewEventV1::Painted {
                revision,
                frame_seq,
            } => {
                if frame_seq >= 2 {
                    self.preview = PreviewState::Ready { revision };
                }
                vec![]
            }
            PreviewEventV1::Scroll {
                revision,
                source_start,
                interaction_id,
                user,
            } => vec![AppEffect::ScrollSource {
                revision,
                source_start,
                interaction_id,
                user,
            }],
            PreviewEventV1::LinkActivated { target, .. } => {
                vec![AppEffect::PresentLink(target)]
            }
        }
    }
}

/// Wave 2S: the shared shell-integration action surface.
///
/// These methods are the vocabulary both platform lanes bind native input to.
/// Every mutating action takes `&mut Document` and routes its
/// [`EditPlan`](feathermark_core::EditPlan)s through the existing transaction
/// path (`into_transaction` → `Document::apply`), then advances the reducer via
/// [`AppMessage::DocumentEdited`] so the render pipeline behaves exactly as it
/// does for typed edits. See [`crate::actions`] for the state-authority note.
impl AppState {
    // --- formatting ---------------------------------------------------------

    /// Applies a [`FormatCommand`] to `document` at `selection`.
    ///
    /// `SmartEnter` is routed through [`smart_enter`] so the decided
    /// [`SmartEnterAction`](feathermark_core::SmartEnterAction) is reported;
    /// every other command goes through [`apply_format`]. On any
    /// [`EditPlanError`](feathermark_core::EditPlanError) (including an empty
    /// plan — e.g. a list toggle over a blank line) the document is left
    /// untouched and a typed [`ActionError`] is returned: a clean no-op, never
    /// a panic and never a whole-buffer rewrite.
    pub fn apply_format_command(
        &mut self,
        document: &mut Document,
        selection: Selection,
        command: FormatCommand,
    ) -> Result<FormatApplied, ActionError> {
        let (action, plan) = if matches!(command, FormatCommand::SmartEnter) {
            let outcome = smart_enter(document, selection)?;
            (Some(outcome.action), outcome.plan)
        } else {
            (None, apply_format(document, selection, command)?)
        };
        let (selection_after, changes, effects) = self.apply_edit_plans(document, vec![plan])?;
        Ok(FormatApplied {
            action,
            selection_after,
            revision: document.revision(),
            changes,
            effects,
        })
    }

    /// Convenience wrapper: a smart-Enter keystroke. Equivalent to
    /// [`apply_format_command`](Self::apply_format_command) with
    /// [`FormatCommand::SmartEnter`].
    pub fn smart_enter(
        &mut self,
        document: &mut Document,
        selection: Selection,
    ) -> Result<FormatApplied, ActionError> {
        self.apply_format_command(document, selection, FormatCommand::SmartEnter)
    }

    // --- find / replace -----------------------------------------------------

    /// Opens (or replaces) the active find session.
    pub fn start_find(&mut self, query: FindQuery, direction: FindDirection, wrap: bool) {
        self.find = Some(FindSession::new(query, direction, wrap));
    }

    /// Borrows the active find session, if any.
    pub fn find_session(&self) -> Option<&FindSession> {
        self.find.as_ref()
    }

    /// Closes the active find session.
    pub fn end_find(&mut self) {
        self.find = None;
    }

    /// Finds the next match in the session's direction from `from_byte`,
    /// recording it as the session's current match.
    pub fn find_next(
        &mut self,
        document: &Document,
        from_byte: usize,
    ) -> Result<Option<Range<usize>>, ActionError> {
        let direction = self
            .find
            .as_ref()
            .ok_or(ActionError::NoFindSession)?
            .direction;
        Ok(self.locate(document, from_byte, direction))
    }

    /// Finds the previous match (the session's direction, inverted) from
    /// `from_byte`, recording it as the session's current match.
    pub fn find_prev(
        &mut self,
        document: &Document,
        from_byte: usize,
    ) -> Result<Option<Range<usize>>, ActionError> {
        let direction = self
            .find
            .as_ref()
            .ok_or(ActionError::NoFindSession)?
            .direction;
        let reversed = match direction {
            FindDirection::Forward => FindDirection::Backward,
            FindDirection::Backward => FindDirection::Forward,
        };
        Ok(self.locate(document, from_byte, reversed))
    }

    /// Replaces the session's current match with `replacement`. A missing
    /// current match is a clean no-op.
    pub fn replace_current(
        &mut self,
        document: &mut Document,
        replacement: String,
    ) -> Result<ReplaceApplied, ActionError> {
        let (query, current) = {
            let session = self.find.as_ref().ok_or(ActionError::NoFindSession)?;
            (session.query.clone(), session.current.clone())
        };
        let Some(current) = current else {
            return Ok(ReplaceApplied {
                replaced: 0,
                selection_after: None,
                revision: document.revision(),
                changes: Vec::new(),
                effects: Vec::new(),
            });
        };
        let spec = ReplaceSpec::new(query, replacement)?;
        let text = document.snapshot().to_string();
        let plan = feathermark_core::replace_current(document.revision(), &text, &spec, current)?;
        let (selection_after, changes, effects) = self.apply_edit_plans(document, vec![plan])?;
        if let Some(session) = self.find.as_mut() {
            session.current = None;
        }
        Ok(ReplaceApplied {
            replaced: 1,
            selection_after: Some(selection_after),
            revision: document.revision(),
            changes,
            effects,
        })
    }

    /// Replaces every match of the session's query with `replacement`.
    ///
    /// The engine chunks large replace-alls into a sequence of bounded
    /// [`EditPlan`]s; they are applied in order through the transaction path so
    /// a replace-all touching more than one plan still commits fully. Zero
    /// matches is a clean no-op.
    pub fn replace_all(
        &mut self,
        document: &mut Document,
        replacement: String,
    ) -> Result<ReplaceApplied, ActionError> {
        let query = self
            .find
            .as_ref()
            .ok_or(ActionError::NoFindSession)?
            .query
            .clone();
        let spec = ReplaceSpec::new(query, replacement)?;
        let text = document.snapshot().to_string();
        let plans = feathermark_core::replace_all(document.revision(), &text, &spec)?;
        if plans.is_empty() {
            return Ok(ReplaceApplied {
                replaced: 0,
                selection_after: None,
                revision: document.revision(),
                changes: Vec::new(),
                effects: Vec::new(),
            });
        }
        let replaced = feathermark_core::match_count(&text, spec.query());
        let (selection_after, changes, effects) = self.apply_edit_plans(document, plans)?;
        if let Some(session) = self.find.as_mut() {
            session.current = None;
        }
        Ok(ReplaceApplied {
            replaced,
            selection_after: Some(selection_after),
            revision: document.revision(),
            changes,
            effects,
        })
    }

    // --- export -------------------------------------------------------------

    /// Produces a validated, self-contained export page for `document` plus a
    /// suggested file name, for both shell gestures (save-as-HTML and
    /// copy-as-HTML). The actual file write / clipboard set is the platform
    /// lane's job. When `title` is `None` the document's file stem is used.
    pub fn export_html(
        &self,
        document: &Document,
        title: Option<String>,
    ) -> Result<ExportOutput, ExportError> {
        let stem = self
            .path()
            .and_then(Path::file_stem)
            .map(|stem| stem.to_string_lossy().into_owned());
        let title = title.or_else(|| stem.clone());
        let request = ExportRequest::new(document.revision(), title)?;
        let source = document.snapshot().to_string();
        let page = render_export_page(&source, &request)?;
        let suggested_file_name = match stem {
            Some(name) => format!("{name}.html"),
            None => "untitled.html".to_owned(),
        };
        Ok(ExportOutput {
            html: page.into_html(),
            suggested_file_name,
        })
    }

    // --- counts -------------------------------------------------------------

    /// Live word/character/reading-time counts for `document`'s current text,
    /// for the native status bar.
    pub fn counts(&self, document: &Document) -> Counts {
        feathermark_core::counts(&document.snapshot().to_string())
    }

    // --- autosave / session -------------------------------------------------

    /// Binds the autosave/session store this state owns, priming the next
    /// autosave sequence from any existing journal so recovery-then-continue
    /// keeps monotonically increasing sequences.
    pub fn bind_autosave(&mut self, store: AutosaveStore) -> Result<(), AutosaveError> {
        self.autosave_sequence = store.next_sequence()?;
        self.autosave = Some(store);
        Ok(())
    }

    /// Borrows the bound autosave store, if any.
    pub fn autosave_store(&self) -> Option<&AutosaveStore> {
        self.autosave.as_ref()
    }

    /// Writes one autosave entry for `document`'s current snapshot. Returns
    /// `Ok(None)` when no store is bound.
    pub fn autosave_tick(
        &mut self,
        document: &Document,
        captured_at_unix_ms: u64,
    ) -> Result<Option<AutosaveEntryV1>, AutosaveError> {
        let Some(store) = self.autosave.clone() else {
            return Ok(None);
        };
        let sequence = self.autosave_sequence;
        let snapshot = document.snapshot();
        let document_path = self.path().map(|path| path.to_string_lossy().into_owned());
        let entry = store.record(
            sequence,
            &snapshot,
            document_path.as_deref(),
            captured_at_unix_ms,
        )?;
        self.autosave_sequence = self.autosave_sequence.saturating_add(1);
        Ok(Some(entry))
    }

    /// Recovery-on-startup: returns the highest verifiable autosaved document,
    /// or `None` when there is nothing to recover / no store is bound.
    pub fn recover(&self) -> Result<Option<RecoveredDocument>, AutosaveError> {
        match &self.autosave {
            Some(store) => store.recover(),
            None => Ok(None),
        }
    }

    /// Adopts a crash-recovered [`Document`] as the working buffer.
    ///
    /// The reducer takes on `document`'s own revision and is associated with the
    /// recovered `document_path` (the file the autosave entry was capturing), so
    /// saves and session-restore target the original file rather than an
    /// untitled buffer. The buffer is marked dirty because the recovered
    /// snapshot is newer than any on-disk copy, and `saved_disk` is cleared (the
    /// recovered content has never been written). Returns the render effect to
    /// schedule.
    ///
    /// This preserves what the app *can* preserve without touching the frozen
    /// core contracts: unlike inserting the recovered text as a fresh edit off a
    /// new untitled document (which advanced the revision by one, added a
    /// spurious undo entry, and dropped the path), the caller swaps the recovered
    /// document in directly. The original numeric revision recorded in the
    /// journal entry (`document_revision`) cannot be restored into the `Document`
    /// itself — core reconstructs every recovered snapshot at revision 0 and
    /// exposes no revision setter — so that part is a core follow-up; a
    /// freshly-loaded document already starts at revision 0, so this matches the
    /// open-a-file baseline.
    pub fn adopt_recovered(
        &mut self,
        document: &Document,
        document_path: Option<PathBuf>,
    ) -> Vec<AppEffect> {
        self.revision = document.revision();
        self.dirty = true;
        self.path = document_path;
        self.saved_disk = None;
        self.external_conflict = None;
        self.preview = PreviewState::Waiting {
            revision: self.revision,
        };
        vec![AppEffect::ScheduleRender {
            revision: self.revision,
        }]
    }

    /// Captures session-restore state from the current document path plus the
    /// platform-supplied selection, viewport, and window frame.
    pub fn capture_session_state(
        &self,
        saved_at_unix_ms: u64,
        selection: Option<Selection>,
        top_visible_byte: Option<usize>,
        window: Option<SessionWindowV1>,
    ) -> SessionStateV1 {
        let last_file = self.path().map(|path| path.to_string_lossy().into_owned());
        let recent_files = last_file.clone().into_iter().collect();
        SessionStateV1 {
            schema: SESSION_SCHEMA_V1.to_owned(),
            v: 1,
            saved_at_unix_ms,
            last_file,
            selection: selection.map(|selection| SessionSelectionV1 {
                anchor: selection.anchor as u64,
                head: selection.head as u64,
            }),
            top_visible_byte: top_visible_byte.map(|byte| byte as u64),
            window,
            recent_files,
        }
    }

    /// Persists session-restore `state` through the bound store. A no-op when
    /// no store is bound.
    pub fn save_session_state(&self, state: &SessionStateV1) -> Result<(), AutosaveError> {
        match &self.autosave {
            Some(store) => store.save_session_state(state),
            None => Ok(()),
        }
    }

    /// Loads and re-validates persisted session state, or `None` when absent /
    /// no store is bound.
    pub fn load_session_state(&self) -> Result<Option<SessionStateV1>, AutosaveError> {
        match &self.autosave {
            Some(store) => store.load_session_state(),
            None => Ok(None),
        }
    }

    /// Lowers a persisted [`SessionStateV1`] into the platform-actionable parts
    /// a shell restores. Offsets are advisory: the shell re-validates them
    /// against the loaded document.
    pub fn restore_session(&self, state: &SessionStateV1) -> SessionRestore {
        SessionRestore {
            last_file: state.last_file.as_ref().map(PathBuf::from),
            selection: state.selection.map(|selection| Selection {
                anchor: selection.anchor as usize,
                head: selection.head as usize,
            }),
            top_visible_byte: state.top_visible_byte.map(|byte| byte as usize),
            window: state.window,
        }
    }

    // --- shared internals ---------------------------------------------------

    /// Locates a match with the active session's query and records it as the
    /// session's current match. Returns `None` with no active session.
    fn locate(
        &mut self,
        document: &Document,
        from_byte: usize,
        direction: FindDirection,
    ) -> Option<Range<usize>> {
        let session = self.find.as_mut()?;
        let text = document.snapshot().to_string();
        let found =
            feathermark_core::find_next(&text, &session.query, from_byte, direction, session.wrap);
        session.current = found.clone();
        found
    }

    /// Applies a non-empty sequence of edit plans through the existing
    /// transaction path and advances the reducer once at the final revision.
    /// Returns the last plan's selection, the per-plan [`ChangeSet`]s (in commit
    /// order, so a shell can follow the mutation incrementally), and the reducer
    /// effects.
    fn apply_edit_plans(
        &mut self,
        document: &mut Document,
        plans: Vec<EditPlan>,
    ) -> Result<(Selection, Vec<ChangeSet>, Vec<AppEffect>), EditError> {
        debug_assert!(
            !plans.is_empty(),
            "apply_edit_plans requires a non-empty batch"
        );
        // All-or-nothing size guard. The plans are applied in sequence and each
        // `Document::apply` rejects a post-edit document over the cap. If a
        // *middle* plan failed that way, earlier plans would already be committed
        // to the document while the reducer and adapter stayed behind — wedging
        // every later edit with a StaleRevision until reload. Project the running
        // document size across the whole batch up front and reject the entire
        // batch (before mutating anything) if it would ever cross the cap, so a
        // cap-crossing replace-all can't half-apply.
        let mut projected = document.len_bytes() as isize;
        let mut peak = projected;
        for plan in &plans {
            for edit in plan.edits() {
                let span = (edit.byte_range.end - edit.byte_range.start) as isize;
                projected += edit.replacement.len() as isize - span;
            }
            peak = peak.max(projected);
        }
        if peak > feathermark_core::MAX_DOCUMENT_BYTES as isize {
            return Err(EditError::TooLarge);
        }
        let mut selection_after = Selection::collapsed(0);
        let mut changes = Vec::with_capacity(plans.len());
        for plan in plans {
            selection_after = plan.selection_after();
            let id = self.next_transaction_id;
            self.next_transaction_id = self.next_transaction_id.saturating_add(1);
            changes.push(document.apply(plan.into_transaction(id))?);
        }
        let effects = self.reduce(AppMessage::DocumentEdited {
            revision: document.revision(),
        });
        Ok((selection_after, changes, effects))
    }
}

fn event_revision(event: &PreviewEventV1) -> Revision {
    match event {
        PreviewEventV1::BridgeReady { revision }
        | PreviewEventV1::Painted { revision, .. }
        | PreviewEventV1::Scroll { revision, .. }
        | PreviewEventV1::LinkActivated { revision, .. } => *revision,
    }
}
