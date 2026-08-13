use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rutile_core::CompositionId;
use rutile_core::{
    ChangeSet, CompositionCancelReason, CompositionTracker, Document, DocumentSnapshot, Edit,
    EditTransaction, EditorAdapter, EditorCommit, EditorError, EditorEvent, EditorEventSink,
    HistoryContext, ImeCommit, LocalCommitRejection, Selection, StaleRevision, TransactionKind,
    TypingDirection, ViewportState, apply_editor_commit,
};
use rutile_types::{InteractionId, Revision};

struct TraceAdapter {
    sink: Option<EditorEventSink>,
    composition: CompositionTracker,
    mirror: String,
    revision: Revision,
    pending_commit: Option<u64>,
    pending_paint: Option<Revision>,
    mirror_replacements: usize,
    acknowledgements: usize,
    paints: usize,
}

impl TraceAdapter {
    fn new() -> Self {
        Self {
            sink: None,
            composition: CompositionTracker::default(),
            mirror: String::new(),
            revision: Revision::new(0),
            pending_commit: None,
            pending_paint: None,
            mirror_replacements: 0,
            acknowledgements: 0,
            paints: 0,
        }
    }

    fn emit(&mut self, event: EditorEvent) {
        self.sink.as_mut().expect("event sink installed")(event);
    }

    fn composition_started(&mut self, id: CompositionId, range: std::ops::Range<usize>) {
        let event = self
            .composition
            .start(id, self.revision, range)
            .expect("composition starts");
        self.emit(event);
    }

    fn composition_updated(&mut self, id: CompositionId, preedit: &str) {
        let event = self
            .composition
            .update(id, self.revision, preedit)
            .expect("matching composition update");
        self.emit(event);
    }

    fn native_ime_commit(
        &mut self,
        id: CompositionId,
        adapter_commit_id: u64,
        replacement: &str,
    ) -> bool {
        let Some(event) =
            self.composition
                .commit(id, self.revision, adapter_commit_id, replacement)
        else {
            return false;
        };
        let EditorEvent::CommitRequested {
            commit: EditorCommit::Ime(ime),
            ..
        } = &event
        else {
            panic!("native IME must emit an IME commit")
        };
        self.mirror
            .replace_range(ime.byte_range.clone(), &ime.replacement);
        self.mirror_replacements += 1;
        self.pending_commit = Some(adapter_commit_id);
        self.emit(event);
        true
    }

    fn native_layout(&mut self, frame_seq: u64) {
        let Some(revision) = self.pending_paint.take() else {
            return;
        };
        if revision != self.revision {
            return;
        }
        self.paints += 1;
        self.emit(EditorEvent::SourcePainted {
            revision,
            frame_seq,
        });
    }
}

impl EditorAdapter for TraceAdapter {
    fn set_event_sink(&mut self, sink: EditorEventSink) {
        self.sink = Some(sink);
    }

    fn install_open_snapshot(&mut self, snapshot: &DocumentSnapshot) -> Result<(), EditorError> {
        self.mirror = snapshot.to_string();
        self.revision = snapshot.revision;
        self.pending_commit = None;
        self.pending_paint = None;
        Ok(())
    }

    fn acknowledge_local_commit(
        &mut self,
        adapter_commit_id: u64,
        change: &ChangeSet,
    ) -> Result<(), EditorError> {
        if self.pending_commit != Some(adapter_commit_id) || change.before != self.revision {
            return Err(EditorError::Platform("unexpected acknowledgement".into()));
        }
        self.pending_commit = None;
        self.revision = change.after;
        self.pending_paint = Some(change.after);
        self.acknowledgements += 1;
        Ok(())
    }

    fn reject_local_commit(
        &mut self,
        adapter_commit_id: u64,
        _reason: LocalCommitRejection,
        authoritative: &DocumentSnapshot,
    ) -> Result<(), EditorError> {
        if self.pending_commit != Some(adapter_commit_id) {
            return Err(EditorError::Platform("unexpected rejection".into()));
        }
        self.mirror = authoritative.to_string();
        self.revision = authoritative.revision;
        self.pending_commit = None;
        self.pending_paint = None;
        Ok(())
    }

    fn apply_external_change(&mut self, change: &ChangeSet) -> Result<(), EditorError> {
        if let Some(cancelled) = self.composition.invalidate_for_revision(change.after) {
            self.emit(cancelled);
        }
        for edit in change.edits.iter().rev() {
            self.mirror
                .replace_range(edit.byte_range.clone(), &edit.replacement);
        }
        self.mirror_replacements += 1;
        self.revision = change.after;
        Ok(())
    }

    fn top_visible_byte(&self, revision: Revision) -> Result<usize, StaleRevision> {
        if revision == self.revision {
            Ok(0)
        } else {
            Err(StaleRevision {
                expected: self.revision,
                actual: revision,
            })
        }
    }

    fn scroll_to_byte(
        &mut self,
        revision: Revision,
        _byte: usize,
        _id: InteractionId,
    ) -> Result<(), EditorError> {
        self.top_visible_byte(revision)?;
        Ok(())
    }

    fn set_read_only_generated(
        &mut self,
        revision: Revision,
        _html: Arc<str>,
    ) -> Result<(), EditorError> {
        self.top_visible_byte(revision)?;
        Ok(())
    }
}

fn capture_sink() -> (EditorEventSink, Rc<RefCell<Vec<EditorEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = Rc::clone(&events);
    (
        Box::new(move |event| captured.borrow_mut().push(event)),
        events,
    )
}

#[test]
fn japanese_ime_trace_mutates_mirror_and_rope_once_then_paints_once() {
    let mut document = Document::new("A B").unwrap();
    let mut adapter = TraceAdapter::new();
    adapter.install_open_snapshot(&document.snapshot()).unwrap();
    let (sink, events) = capture_sink();
    adapter.set_event_sink(sink);

    adapter.composition_started(CompositionId::new(7), 2..2);
    adapter.composition_updated(CompositionId::new(7), "に");
    assert!(adapter.native_ime_commit(CompositionId::new(7), 11, "日本"));
    let requested = events.borrow()[2].clone();
    let EditorEvent::CommitRequested {
        adapter_commit_id,
        commit,
    } = requested
    else {
        panic!("third event must request the commit")
    };
    let change = apply_editor_commit(&mut document, adapter_commit_id, commit).unwrap();
    adapter
        .acknowledge_local_commit(adapter_commit_id, &change)
        .unwrap();
    assert_eq!(
        adapter.mirror, "A 日本B",
        "acknowledge must not apply twice"
    );
    adapter.native_layout(42);
    adapter.native_layout(42);

    let events = events.borrow();
    assert!(matches!(events[0], EditorEvent::CompositionStarted { .. }));
    assert!(matches!(events[1], EditorEvent::CompositionUpdated { .. }));
    assert!(matches!(events[2], EditorEvent::CommitRequested { .. }));
    assert_eq!(
        events[3],
        EditorEvent::SourcePainted {
            revision: Revision::new(1),
            frame_seq: 42,
        }
    );
    assert_eq!(events.len(), 4);
    assert_eq!(document.snapshot().to_string(), adapter.mirror);
    assert_eq!(document.revision(), Revision::new(1));
    assert_eq!(adapter.mirror_replacements, 1);
    assert_eq!(adapter.acknowledgements, 1);
    assert_eq!(adapter.paints, 1);
    assert!(!adapter.native_ime_commit(CompositionId::new(7), 12, "duplicate"));
    assert!(document.undo().is_some());
    assert!(document.undo().is_none());
    assert!(!include_str!("../src/editor_contract.rs").contains("CompositionCommitted"));
}

#[test]
fn revision_change_cancels_preedit_before_external_change_and_late_commit_is_inert() {
    let mut document = Document::new("A B").unwrap();
    let mut adapter = TraceAdapter::new();
    adapter.install_open_snapshot(&document.snapshot()).unwrap();
    let (sink, events) = capture_sink();
    adapter.set_event_sink(sink);
    adapter.composition_started(CompositionId::new(9), 2..2);
    adapter.composition_updated(CompositionId::new(9), "に");

    let change = document
        .apply(EditTransaction {
            base_revision: Revision::new(0),
            id: 1,
            kind: TransactionKind::Programmatic,
            edits: vec![Edit {
                byte_range: 0..0,
                replacement: "X".into(),
            }],
        })
        .unwrap();
    adapter.apply_external_change(&change).unwrap();
    let mirror_after_external = adapter.mirror.clone();

    assert!(matches!(
        events.borrow()[2],
        EditorEvent::CompositionCancelled {
            reason: CompositionCancelReason::StaleRevision,
            ..
        }
    ));
    assert!(
        adapter
            .composition
            .update(CompositionId::new(9), Revision::new(0), "late")
            .is_none()
    );
    assert!(!adapter.native_ime_commit(CompositionId::new(9), 3, "late"));
    assert_eq!(adapter.mirror, mirror_after_external);
    assert_eq!(adapter.mirror, document.snapshot().to_string());
    assert_eq!(adapter.acknowledgements, 0);
    assert_eq!(adapter.paints, 0);
}

#[test]
fn rejected_local_ime_restores_authoritative_snapshot_and_never_paints() {
    let mut document = Document::new("kept").unwrap();
    let authoritative = document.snapshot();
    let mut adapter = TraceAdapter::new();
    adapter.install_open_snapshot(&authoritative).unwrap();
    let (sink, _events) = capture_sink();
    adapter.set_event_sink(sink);
    adapter.composition_started(CompositionId::new(3), 0..4);
    assert!(adapter.native_ime_commit(
        CompositionId::new(3),
        21,
        &"x".repeat(rutile_core::MAX_DOCUMENT_BYTES + 1)
    ));
    let commit = EditorCommit::Ime(ImeCommit {
        composition_id: CompositionId::new(3),
        base_revision: Revision::new(0),
        byte_range: 0..4,
        replacement: "x".repeat(rutile_core::MAX_DOCUMENT_BYTES + 1),
    });
    assert!(apply_editor_commit(&mut document, 21, commit).is_err());
    adapter
        .reject_local_commit(21, LocalCommitRejection::TooLarge, &authoritative)
        .unwrap();
    adapter.native_layout(1);

    assert_eq!(adapter.mirror, "kept");
    assert_eq!(document.snapshot().to_string(), "kept");
    assert_eq!(document.revision(), Revision::new(0));
    assert_eq!(adapter.acknowledgements, 0);
    assert_eq!(adapter.paints, 0);
}

#[test]
fn edit_commit_requires_the_adapter_commit_id() {
    let mut document = Document::new("abc").unwrap();
    let commit = EditorCommit::Edit {
        transaction: EditTransaction {
            base_revision: Revision::new(0),
            id: 40,
            kind: TransactionKind::Typing,
            edits: vec![Edit {
                byte_range: 1..2,
                replacement: "x".into(),
            }],
        },
        history: None,
    };

    assert!(apply_editor_commit(&mut document, 41, commit).is_err());
    assert_eq!(document.snapshot().to_string(), "abc");
}

#[test]
fn adjacent_typing_commit_requests_use_history_context_and_undo_as_one() {
    let mut document = Document::new("").unwrap();
    for (id, byte, text, elapsed_ms) in [(1, 0, "a", 10), (2, 1, "b", 100)] {
        let commit = EditorCommit::Edit {
            transaction: EditTransaction {
                base_revision: document.revision(),
                id,
                kind: TransactionKind::Typing,
                edits: vec![Edit {
                    byte_range: byte..byte,
                    replacement: text.into(),
                }],
            },
            history: Some(HistoryContext::typing(
                elapsed_ms,
                TypingDirection::Forward,
                Selection::collapsed(byte),
                Selection::collapsed(byte + 1),
            )),
        };
        apply_editor_commit(&mut document, id, commit).unwrap();
    }

    assert_eq!(document.snapshot().to_string(), "ab");
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "");
    assert!(document.undo().is_none());
}

#[test]
fn top_visible_byte_requires_exact_revision_and_utf8_boundary() {
    let mut document = Document::new("a🪶b").unwrap();
    let snapshot = document.snapshot();
    let mut viewport = ViewportState::new(&snapshot, 1).unwrap();

    assert_eq!(viewport.top_visible_byte(Revision::new(0)).unwrap(), 1);
    assert!(matches!(
        viewport.top_visible_byte(Revision::new(1)),
        Err(StaleRevision { .. })
    ));
    assert!(viewport.update(&snapshot, 2).is_err());
    assert_eq!(viewport.top_visible_byte(Revision::new(0)).unwrap(), 1);

    document
        .apply(EditTransaction {
            base_revision: Revision::new(0),
            id: 1,
            kind: TransactionKind::Programmatic,
            edits: vec![Edit {
                byte_range: 0..0,
                replacement: "x".into(),
            }],
        })
        .unwrap();
    viewport.update(&document.snapshot(), 6).unwrap();
    assert_eq!(viewport.top_visible_byte(Revision::new(1)).unwrap(), 6);
}
