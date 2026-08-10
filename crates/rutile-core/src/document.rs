use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::ops::Range;

use rutile_types::Revision;
use ropey::Rope;
use thiserror::Error;

pub const MAX_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_UNDO_BYTES: usize = 64 * 1024 * 1024;
const HISTORY_EDIT_OVERHEAD: usize = 96;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub byte_range: Range<usize>,
    pub replacement: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionKind {
    Typing,
    Delete,
    Paste,
    Cut,
    ImeCommit,
    Programmatic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypingDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub const fn collapsed(byte: usize) -> Self {
        Self {
            anchor: byte,
            head: byte,
        }
    }

    pub const fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryContext {
    pub elapsed_ms: u64,
    pub direction: TypingDirection,
    pub selection_before: Selection,
    pub selection_after: Selection,
}

impl HistoryContext {
    pub const fn typing(
        elapsed_ms: u64,
        direction: TypingDirection,
        selection_before: Selection,
        selection_after: Selection,
    ) -> Self {
        Self {
            elapsed_ms,
            direction,
            selection_before,
            selection_after,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryBoundary {
    Command,
    Composition,
    FocusLost,
    Newline,
    Save,
    SelectionChanged,
    CursorRelocated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditTransaction {
    pub base_revision: Revision,
    pub id: u64,
    pub kind: TransactionKind,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSet {
    pub before: Revision,
    pub after: Revision,
    pub edits: Vec<Edit>,
    pub changed_bytes_after: Range<usize>,
}

#[derive(Clone)]
pub struct DocumentSnapshot {
    pub revision: Revision,
    rope: Rope,
}

impl DocumentSnapshot {
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    pub fn is_char_boundary(&self, byte: usize) -> bool {
        byte <= self.len_bytes() && is_char_boundary(&self.rope, byte)
    }

    pub fn write_to<W: Write>(&self, mut sink: W) -> io::Result<()> {
        for chunk in self.rope.chunks() {
            sink.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }
}

impl fmt::Display for DocumentSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.rope.chunks() {
            formatter.write_str(chunk)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    transaction: EditTransaction,
    inverse: Vec<Edit>,
    charged_bytes: usize,
    context: Option<HistoryContext>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum DocumentError {
    #[error("document is larger than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum EditError {
    #[error("transaction revision {actual} does not match document revision {expected}")]
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    #[error("edit range {start}..{end} is reversed")]
    ReversedRange { start: usize, end: usize },
    #[error("edit range {start}..{end} exceeds document length {len}")]
    OutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    #[error("edit endpoint {offset} is not a UTF-8 boundary")]
    NotCharBoundary { offset: usize },
    #[error("edit range starts at {start} before the prior range ends at {prior_end}")]
    OverlappingEdits { start: usize, prior_end: usize },
    #[error("transaction has no edits")]
    EmptyTransaction,
    #[error("post-edit document is larger than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    #[error("document revision overflow")]
    RevisionOverflow,
}

pub struct Document {
    rope: Rope,
    revision: Revision,
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
    undo_bytes: usize,
    history_group_open: bool,
}

impl Document {
    pub fn new(text: &str) -> Result<Self, DocumentError> {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge);
        }
        Ok(Self {
            rope: Rope::from_str(text),
            revision: 0,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            undo_bytes: 0,
            history_group_open: false,
        })
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            revision: self.revision,
            rope: self.rope.clone(),
        }
    }

    pub fn apply(&mut self, tx: EditTransaction) -> Result<ChangeSet, EditError> {
        self.apply_internal(tx, None)
    }

    pub fn apply_with_history(
        &mut self,
        tx: EditTransaction,
        context: HistoryContext,
    ) -> Result<ChangeSet, EditError> {
        self.apply_internal(tx, Some(context))
    }

    pub fn close_history_group(&mut self, _boundary: HistoryBoundary) {
        self.history_group_open = false;
    }

    fn apply_internal(
        &mut self,
        tx: EditTransaction,
        context: Option<HistoryContext>,
    ) -> Result<ChangeSet, EditError> {
        if tx.base_revision != self.revision {
            return Err(EditError::StaleRevision {
                expected: self.revision,
                actual: tx.base_revision,
            });
        }
        let after = self
            .revision
            .checked_add(1)
            .ok_or(EditError::RevisionOverflow)?;
        self.validate_edits(&tx.edits)?;
        let post_edit_len = post_edit_len(self.len_bytes(), &tx.edits)?;
        if post_edit_len > MAX_DOCUMENT_BYTES {
            return Err(EditError::TooLarge);
        }

        let inverse = build_inverse(&self.rope, &tx.edits);
        let charged_bytes = history_charge(&tx.edits);
        apply_edits(&mut self.rope, &tx.edits);
        let changed_bytes_after = changed_range_after(&tx.edits);
        let change = ChangeSet {
            before: self.revision,
            after,
            edits: tx.edits.clone(),
            changed_bytes_after,
        };
        self.revision = after;
        self.redo.clear();
        self.undo_bytes = self.undo_bytes.saturating_add(charged_bytes);
        let typing_group_eligible = context.is_some_and(|history| coalescible_typing(&tx, history));
        let coalesced = self.history_group_open
            && context.is_some_and(|history| {
                self.undo.back_mut().is_some_and(|entry| {
                    coalesce_typing(entry, &tx, &inverse, charged_bytes, history)
                })
            });
        if !coalesced {
            self.undo.push_back(HistoryEntry {
                transaction: tx,
                inverse,
                charged_bytes,
                context,
            });
        }
        self.history_group_open = typing_group_eligible;
        self.enforce_undo_budget();
        Ok(change)
    }

    pub fn undo(&mut self) -> Option<ChangeSet> {
        let after = self.revision.checked_add(1)?;
        let entry = self.undo.pop_back()?;
        self.undo_bytes = self.undo_bytes.saturating_sub(entry.charged_bytes);
        let edits = entry.inverse.clone();
        let changed_bytes_after = changed_range_after(&edits);
        apply_edits(&mut self.rope, &edits);
        let change = ChangeSet {
            before: self.revision,
            after,
            edits,
            changed_bytes_after,
        };
        self.revision = after;
        self.redo.push_back(entry);
        self.history_group_open = false;
        Some(change)
    }

    pub fn redo(&mut self) -> Option<ChangeSet> {
        let after = self.revision.checked_add(1)?;
        let entry = self.redo.pop_back()?;
        let edits = entry.transaction.edits.clone();
        let changed_bytes_after = changed_range_after(&edits);
        apply_edits(&mut self.rope, &edits);
        let change = ChangeSet {
            before: self.revision,
            after,
            edits,
            changed_bytes_after,
        };
        self.revision = after;
        self.undo_bytes = self.undo_bytes.saturating_add(entry.charged_bytes);
        self.undo.push_back(entry);
        self.enforce_undo_budget();
        self.history_group_open = false;
        Some(change)
    }

    pub fn write_to<W: Write>(&self, mut sink: W) -> io::Result<()> {
        for chunk in self.rope.chunks() {
            sink.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }

    fn validate_edits(&self, edits: &[Edit]) -> Result<(), EditError> {
        if edits.is_empty() {
            return Err(EditError::EmptyTransaction);
        }
        let len = self.len_bytes();
        let mut prior_end = 0;
        for (index, edit) in edits.iter().enumerate() {
            let Range { start, end } = edit.byte_range;
            if start > end {
                return Err(EditError::ReversedRange { start, end });
            }
            if end > len {
                return Err(EditError::OutOfBounds { start, end, len });
            }
            if !is_char_boundary(&self.rope, start) {
                return Err(EditError::NotCharBoundary { offset: start });
            }
            if !is_char_boundary(&self.rope, end) {
                return Err(EditError::NotCharBoundary { offset: end });
            }
            if index > 0 && start < prior_end {
                return Err(EditError::OverlappingEdits { start, prior_end });
            }
            prior_end = end;
        }
        Ok(())
    }

    fn enforce_undo_budget(&mut self) {
        while self.undo_bytes > MAX_UNDO_BYTES {
            let Some(evicted) = self.undo.pop_front() else {
                self.undo_bytes = 0;
                break;
            };
            self.undo_bytes = self.undo_bytes.saturating_sub(evicted.charged_bytes);
        }
    }
}

fn coalescible_typing(transaction: &EditTransaction, context: HistoryContext) -> bool {
    transaction.kind == TransactionKind::Typing
        && transaction.edits.len() == 1
        && transaction.edits[0].byte_range.is_empty()
        && !transaction.edits[0].replacement.is_empty()
        && !transaction.edits[0].replacement.contains('\n')
        && context.selection_before.is_collapsed()
        && context.selection_after.is_collapsed()
}

fn coalesce_typing(
    entry: &mut HistoryEntry,
    transaction: &EditTransaction,
    inverse: &[Edit],
    charged_bytes: usize,
    context: HistoryContext,
) -> bool {
    let Some(previous_context) = entry.context else {
        return false;
    };
    if !coalescible_typing(&entry.transaction, previous_context)
        || !coalescible_typing(transaction, context)
        || context.direction != previous_context.direction
        || context.selection_before != previous_context.selection_after
        || context.elapsed_ms < previous_context.elapsed_ms
        || context.elapsed_ms - previous_context.elapsed_ms > 500
    {
        return false;
    }

    let previous = &mut entry.transaction.edits[0];
    let next = &transaction.edits[0];
    let contiguous = match context.direction {
        TypingDirection::Forward => {
            next.byte_range.start == previous.byte_range.start + previous.replacement.len()
        }
        TypingDirection::Backward => next.byte_range.start == previous.byte_range.start,
    };
    if !contiguous || inverse.len() != 1 || !inverse[0].replacement.is_empty() {
        return false;
    }

    match context.direction {
        TypingDirection::Forward => previous.replacement.push_str(&next.replacement),
        TypingDirection::Backward => {
            previous.replacement.insert_str(0, &next.replacement);
            previous.byte_range = next.byte_range.clone();
        }
    }
    entry.inverse[0].byte_range.end = entry.inverse[0]
        .byte_range
        .end
        .saturating_add(next.replacement.len());
    entry.charged_bytes = entry.charged_bytes.saturating_add(charged_bytes);
    entry.context = Some(context);
    true
}

fn post_edit_len(len: usize, edits: &[Edit]) -> Result<usize, EditError> {
    edits.iter().try_fold(len, |current, edit| {
        current
            .checked_sub(edit.byte_range.end - edit.byte_range.start)
            .and_then(|value| value.checked_add(edit.replacement.len()))
            .ok_or(EditError::TooLarge)
    })
}

fn history_charge(edits: &[Edit]) -> usize {
    edits.iter().fold(0usize, |total, edit| {
        total
            .saturating_add(edit.byte_range.end - edit.byte_range.start)
            .saturating_add(edit.replacement.len())
            .saturating_add(HISTORY_EDIT_OVERHEAD)
    })
}

fn build_inverse(rope: &Rope, edits: &[Edit]) -> Vec<Edit> {
    let mut displacement = 0isize;
    edits
        .iter()
        .map(|edit| {
            let start = edit.byte_range.start.saturating_add_signed(displacement);
            let replaced = rope.byte_slice(edit.byte_range.clone()).to_string();
            let inverse = Edit {
                byte_range: start..start + edit.replacement.len(),
                replacement: replaced,
            };
            displacement = displacement.saturating_add(
                edit.replacement.len() as isize
                    - (edit.byte_range.end - edit.byte_range.start) as isize,
            );
            inverse
        })
        .collect()
}

fn changed_range_after(edits: &[Edit]) -> Range<usize> {
    let start = edits[0].byte_range.start;
    let mut displacement = 0isize;
    let mut end = start;
    for edit in edits {
        let edit_start = edit.byte_range.start.saturating_add_signed(displacement);
        end = edit_start + edit.replacement.len();
        displacement = displacement.saturating_add(
            edit.replacement.len() as isize
                - (edit.byte_range.end - edit.byte_range.start) as isize,
        );
    }
    start..end
}

fn apply_edits(rope: &mut Rope, edits: &[Edit]) {
    for edit in edits.iter().rev() {
        let start = rope.byte_to_char(edit.byte_range.start);
        let end = rope.byte_to_char(edit.byte_range.end);
        rope.remove(start..end);
        rope.insert(start, &edit.replacement);
    }
}

fn is_char_boundary(rope: &Rope, byte: usize) -> bool {
    rope.char_to_byte(rope.byte_to_char(byte)) == byte
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(document: &Document, id: u64) -> EditTransaction {
        EditTransaction {
            base_revision: document.revision,
            id,
            kind: TransactionKind::Programmatic,
            edits: vec![Edit {
                byte_range: 0..1,
                replacement: "b".into(),
            }],
        }
    }

    #[test]
    fn revision_overflow_rejects_apply_without_mutation() {
        let mut document = Document::new("a").unwrap();
        document.revision = Revision::MAX;
        let before = document.snapshot().to_string();

        assert_eq!(
            document.apply(replace(&document, 1)),
            Err(EditError::RevisionOverflow)
        );
        assert_eq!(document.snapshot().to_string(), before);
        assert_eq!(document.revision, Revision::MAX);
        assert!(document.undo.is_empty());
    }

    #[test]
    fn revision_overflow_leaves_undo_entry_available() {
        let mut document = Document::new("a").unwrap();
        document.apply(replace(&document, 1)).unwrap();
        document.revision = Revision::MAX;

        assert!(document.undo().is_none());
        assert_eq!(document.snapshot().to_string(), "b");
        assert_eq!(document.undo.len(), 1);
    }

    #[test]
    fn revision_overflow_leaves_redo_entry_available() {
        let mut document = Document::new("a").unwrap();
        document.apply(replace(&document, 1)).unwrap();
        document.undo().unwrap();
        document.revision = Revision::MAX;

        assert!(document.redo().is_none());
        assert_eq!(document.snapshot().to_string(), "a");
        assert_eq!(document.redo.len(), 1);
    }
}
