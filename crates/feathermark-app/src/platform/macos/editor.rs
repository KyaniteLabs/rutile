use std::ops::Range;
use std::sync::Arc;

use feathermark_core::{
    AdapterCommitId, ChangeSet, CompositionCancelReason, CompositionTracker, DocumentSnapshot,
    Edit, EditTransaction, EditorAdapter, EditorCommit, EditorError, EditorEvent, EditorEventSink,
    LocalCommitRejection, StaleRevision, TransactionKind,
};
use feathermark_types::{InteractionId, Revision};
use iced_widget::text_editor;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IcedAdapterStats {
    pub full_snapshot_installs: u64,
    pub incremental_native_edits: u64,
    pub acknowledgements: u64,
    pub whole_buffer_reads_during_native_edits: u64,
    pub whole_buffer_replacements_during_native_edits: u64,
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

    fn byte_at(&self, line: usize, column: usize) -> Result<usize, EditorError> {
        let start = *self
            .starts
            .get(line)
            .ok_or_else(|| EditorError::Platform("Iced cursor line is outside mirror".into()))?;
        start
            .checked_add(column)
            .filter(|byte| *byte <= self.len)
            .ok_or_else(|| EditorError::Platform("Iced cursor byte is outside mirror".into()))
    }

    fn position_at(&self, byte: usize) -> Result<text_editor::Position, EditorError> {
        if byte > self.len {
            return Err(EditorError::Platform(
                "requested byte is outside Iced mirror".into(),
            ));
        }
        let line = self.starts.partition_point(|start| *start <= byte) - 1;
        Ok(text_editor::Position {
            line,
            column: byte - self.starts[line],
        })
    }

    fn line_start(&self, line: usize) -> Result<usize, EditorError> {
        self.starts
            .get(line)
            .copied()
            .ok_or_else(|| EditorError::Platform("Iced line is outside mirror".into()))
    }

    fn line_end(&self, line: usize) -> Result<usize, EditorError> {
        Ok(self
            .starts
            .get(line + 1)
            .map_or(self.len, |next| next.saturating_sub(1)))
    }

    fn apply(&mut self, range: Range<usize>, replacement: &str) {
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
struct ActiveComposition {
    id: u64,
    base_revision: Revision,
    range: Range<usize>,
}

/// Incremental adapter for Iced's native text editor. Ordinary edits are
/// translated from cursor/selection state into exact byte transactions; the
/// authoritative revision advances only after the core acknowledges the same
/// adapter commit id.
pub struct IcedEditorAdapter {
    content: text_editor::Content<iced_renderer::Renderer>,
    mirror: String,
    index: LineByteIndex,
    revision: Revision,
    next_commit_id: AdapterCommitId,
    next_composition_id: u64,
    pending_commit: Option<AdapterCommitId>,
    sink: Option<EditorEventSink>,
    composition_tracker: CompositionTracker,
    composition: Option<ActiveComposition>,
    top_visible_byte: usize,
    stats: IcedAdapterStats,
}

impl Default for IcedEditorAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl IcedEditorAdapter {
    pub fn new() -> Self {
        Self {
            content: text_editor::Content::new(),
            mirror: String::new(),
            index: LineByteIndex::from_text(""),
            revision: 0,
            next_commit_id: 0,
            next_composition_id: 0,
            pending_commit: None,
            sink: None,
            composition_tracker: CompositionTracker::default(),
            composition: None,
            top_visible_byte: 0,
            stats: IcedAdapterStats::default(),
        }
    }

    pub fn content(&self) -> &text_editor::Content<iced_renderer::Renderer> {
        &self.content
    }

    pub fn mirror(&self) -> &str {
        &self.mirror
    }

    pub fn mirror_len(&self) -> usize {
        self.mirror.len()
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn stats(&self) -> IcedAdapterStats {
        self.stats
    }

    pub fn perform(&mut self, action: text_editor::Action) -> Result<bool, EditorError> {
        let text_editor::Action::Edit(edit) = &action else {
            if let text_editor::Action::Scroll { lines } = action {
                self.observe_scroll(lines);
                self.content.perform(text_editor::Action::Scroll { lines });
            } else {
                self.content.perform(action);
            }
            return Ok(false);
        };
        if self.pending_commit.is_some() {
            return Err(EditorError::Platform(
                "Iced editor received an edit before core acknowledgement".into(),
            ));
        }

        let cursor = self.content.cursor();
        let caret = self
            .index
            .byte_at(cursor.position.line, cursor.position.column)?;
        let selection = cursor
            .selection
            .map(|position| self.index.byte_at(position.line, position.column))
            .transpose()?;
        let selected =
            selection.map_or(caret..caret, |anchor| anchor.min(caret)..anchor.max(caret));
        let (range, replacement) = self.edit_delta(edit, selected, &cursor)?;

        self.content.perform(action);
        self.mirror.replace_range(range.clone(), &replacement);
        self.index.apply(range.clone(), &replacement);
        self.stats.incremental_native_edits = self.stats.incremental_native_edits.saturating_add(1);
        let adapter_commit_id = self.next_commit_id();
        self.pending_commit = Some(adapter_commit_id);
        self.emit(EditorEvent::CommitRequested {
            adapter_commit_id,
            commit: EditorCommit::Edit {
                transaction: EditTransaction {
                    base_revision: self.revision,
                    id: adapter_commit_id,
                    kind: TransactionKind::Typing,
                    edits: vec![Edit {
                        byte_range: range,
                        replacement,
                    }],
                },
                history: None,
            },
        });
        Ok(true)
    }

    fn edit_delta(
        &self,
        edit: &text_editor::Edit,
        selected: Range<usize>,
        cursor: &text_editor::Cursor,
    ) -> Result<(Range<usize>, String), EditorError> {
        use text_editor::Edit;
        let delta = match edit {
            Edit::Insert(character) => (selected, character.to_string()),
            Edit::Paste(text) => (selected, text.as_str().to_owned()),
            Edit::Enter => (selected, "\n".to_owned()),
            Edit::Backspace if !selected.is_empty() => (selected, String::new()),
            Edit::Delete if !selected.is_empty() => (selected, String::new()),
            Edit::Backspace => {
                let start = self.mirror[..selected.start]
                    .char_indices()
                    .next_back()
                    .map_or(selected.start, |(offset, _)| offset);
                (start..selected.start, String::new())
            }
            Edit::Delete => {
                let end = self.mirror[selected.start..]
                    .chars()
                    .next()
                    .map_or(selected.start, |character| {
                        selected.start + character.len_utf8()
                    });
                (selected.start..end, String::new())
            }
            Edit::Indent | Edit::Unindent => {
                let anchor_line = cursor.selection.map_or(cursor.position.line, |p| p.line);
                let first_line = anchor_line.min(cursor.position.line);
                let last_line = anchor_line.max(cursor.position.line);
                let start = self.index.line_start(first_line)?;
                let end = self.index.line_end(last_line)?;
                let source = &self.mirror[start..end];
                let replacement = match edit {
                    Edit::Indent => source
                        .split_inclusive('\n')
                        .map(|line| format!("    {line}"))
                        .collect(),
                    Edit::Unindent => source
                        .split_inclusive('\n')
                        .map(|line| line.strip_prefix("    ").unwrap_or(line))
                        .collect(),
                    _ => unreachable!(),
                };
                (start..end, replacement)
            }
        };
        Ok(delta)
    }

    pub fn start_composition(&mut self) -> Result<u64, EditorError> {
        if self.pending_commit.is_some() || self.composition.is_some() {
            return Err(EditorError::Platform(
                "Iced composition cannot start while another edit is pending".into(),
            ));
        }
        let cursor = self.content.cursor();
        let caret = self
            .index
            .byte_at(cursor.position.line, cursor.position.column)?;
        let anchor = cursor
            .selection
            .map(|position| self.index.byte_at(position.line, position.column))
            .transpose()?
            .unwrap_or(caret);
        self.next_composition_id = self.next_composition_id.saturating_add(1);
        let active = ActiveComposition {
            id: self.next_composition_id,
            base_revision: self.revision,
            range: anchor.min(caret)..anchor.max(caret),
        };
        if let Some(event) =
            self.composition_tracker
                .start(active.id, active.base_revision, active.range.clone())
        {
            self.emit(event);
        }
        let id = active.id;
        self.composition = Some(active);
        Ok(id)
    }

    pub fn update_composition(&mut self, preedit: &str) -> Result<(), EditorError> {
        let active = self
            .composition
            .as_ref()
            .ok_or_else(|| EditorError::Platform("Iced composition is not active".into()))?;
        if let Some(event) =
            self.composition_tracker
                .update(active.id, active.base_revision, preedit)
        {
            self.emit(event);
        }
        Ok(())
    }

    pub fn commit_composition(&mut self, replacement: &str) -> Result<bool, EditorError> {
        if self.pending_commit.is_some() {
            return Err(EditorError::Platform(
                "Iced composition commit is waiting for acknowledgement".into(),
            ));
        }
        let active = self
            .composition
            .take()
            .ok_or_else(|| EditorError::Platform("Iced composition is not active".into()))?;
        self.content
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                Arc::new(replacement.to_owned()),
            )));
        self.mirror.replace_range(active.range.clone(), replacement);
        self.index.apply(active.range.clone(), replacement);
        self.stats.incremental_native_edits = self.stats.incremental_native_edits.saturating_add(1);
        let adapter_commit_id = self.next_commit_id();
        let event = self
            .composition_tracker
            .commit(
                active.id,
                active.base_revision,
                adapter_commit_id,
                replacement,
            )
            .ok_or_else(|| EditorError::Platform("Iced composition state diverged".into()))?;
        self.pending_commit = Some(adapter_commit_id);
        self.emit(event);
        Ok(true)
    }

    pub fn cancel_composition(&mut self, reason: CompositionCancelReason) {
        self.composition = None;
        if let Some(event) = self.composition_tracker.cancel(reason) {
            self.emit(event);
        }
    }

    fn next_commit_id(&mut self) -> AdapterCommitId {
        self.next_commit_id = self.next_commit_id.saturating_add(1);
        self.next_commit_id
    }

    fn observe_scroll(&mut self, lines: i32) {
        let current_line = self
            .index
            .starts
            .partition_point(|start| *start <= self.top_visible_byte)
            .saturating_sub(1);
        let line = current_line
            .saturating_add_signed(lines as isize)
            .min(self.index.starts.len().saturating_sub(1));
        self.top_visible_byte = self.index.starts[line];
        self.emit(EditorEvent::ViewportChanged {
            revision: self.revision,
            top_visible_byte: self.top_visible_byte,
            user: true,
        });
    }

    fn emit(&mut self, event: EditorEvent) {
        if let Some(sink) = self.sink.as_mut() {
            sink(event);
        }
    }
}

impl EditorAdapter for IcedEditorAdapter {
    fn set_event_sink(&mut self, sink: EditorEventSink) {
        self.sink = Some(sink);
    }

    fn install_open_snapshot(&mut self, snapshot: &DocumentSnapshot) -> Result<(), EditorError> {
        let text = snapshot.to_string();
        self.content = text_editor::Content::with_text(&text);
        self.mirror = text;
        self.index = LineByteIndex::from_text(&self.mirror);
        self.revision = snapshot.revision;
        self.pending_commit = None;
        self.composition = None;
        self.top_visible_byte = 0;
        self.stats.full_snapshot_installs = self.stats.full_snapshot_installs.saturating_add(1);
        Ok(())
    }

    fn acknowledge_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        change: &ChangeSet,
    ) -> Result<(), EditorError> {
        if self.pending_commit != Some(adapter_commit_id) || change.before != self.revision {
            return Err(EditorError::Platform(
                "unexpected Iced adapter acknowledgement".into(),
            ));
        }
        self.pending_commit = None;
        self.revision = change.after;
        self.stats.acknowledgements = self.stats.acknowledgements.saturating_add(1);
        Ok(())
    }

    fn reject_local_commit(
        &mut self,
        adapter_commit_id: AdapterCommitId,
        _reason: LocalCommitRejection,
        authoritative: &DocumentSnapshot,
    ) -> Result<(), EditorError> {
        if self.pending_commit != Some(adapter_commit_id) {
            return Err(EditorError::Platform(
                "unexpected Iced adapter rejection".into(),
            ));
        }
        self.install_open_snapshot(authoritative)
    }

    fn apply_external_change(&mut self, change: &ChangeSet) -> Result<(), EditorError> {
        if change.before != self.revision || self.pending_commit.is_some() {
            return Err(EditorError::Platform(
                "external Iced change does not match acknowledged revision".into(),
            ));
        }
        if let Some(event) = self
            .composition_tracker
            .invalidate_for_revision(change.after)
        {
            self.composition = None;
            self.emit(event);
        }
        for edit in change.edits.iter().rev() {
            let start = self.index.position_at(edit.byte_range.start)?;
            let end = self.index.position_at(edit.byte_range.end)?;
            self.content.move_to(text_editor::Cursor {
                position: end,
                selection: Some(start),
            });
            self.content
                .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                    Arc::new(edit.replacement.clone()),
                )));
            self.mirror
                .replace_range(edit.byte_range.clone(), &edit.replacement);
            self.index.apply(edit.byte_range.clone(), &edit.replacement);
        }
        self.revision = change.after;
        Ok(())
    }

    fn top_visible_byte(&self, revision: Revision) -> Result<usize, StaleRevision> {
        if revision != self.revision {
            return Err(StaleRevision {
                expected: self.revision,
                actual: revision,
            });
        }
        Ok(self.top_visible_byte)
    }

    fn scroll_to_byte(
        &mut self,
        revision: Revision,
        byte: usize,
        _id: InteractionId,
    ) -> Result<(), EditorError> {
        self.top_visible_byte(revision)?;
        let position = self.index.position_at(byte)?;
        self.content.move_to(text_editor::Cursor {
            position,
            selection: None,
        });
        self.top_visible_byte = byte;
        Ok(())
    }

    fn set_read_only_generated(
        &mut self,
        revision: Revision,
        _html: Arc<str>,
    ) -> Result<(), EditorError> {
        self.top_visible_byte(revision)?;
        Err(EditorError::Platform(
            "Iced source editor cannot display generated HTML".into(),
        ))
    }
}
