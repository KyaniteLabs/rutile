use rutile_core::{
    Document, Edit, EditTransaction, HistoryBoundary, HistoryContext, MAX_DOCUMENT_BYTES,
    Selection, TransactionKind, TypingDirection,
};

fn insertion(document: &Document, id: u64, byte: usize, text: &str) -> EditTransaction {
    EditTransaction {
        base_revision: document.revision(),
        id,
        kind: TransactionKind::Typing,
        edits: vec![Edit {
            byte_range: byte..byte,
            replacement: text.into(),
        }],
    }
}

fn typing_context(
    elapsed_ms: u64,
    direction: TypingDirection,
    before: usize,
    after: usize,
) -> HistoryContext {
    HistoryContext::typing(
        elapsed_ms,
        direction,
        Selection::collapsed(before),
        Selection::collapsed(after),
    )
}

#[test]
fn adjacent_typing_coalesces_by_time_direction_and_selection() {
    let mut document = Document::new("").unwrap();
    document
        .apply_with_history(
            insertion(&document, 1, 0, "a"),
            typing_context(1_000, TypingDirection::Forward, 0, 1),
        )
        .unwrap();
    document
        .apply_with_history(
            insertion(&document, 2, 1, "b"),
            typing_context(1_200, TypingDirection::Forward, 1, 2),
        )
        .unwrap();
    document
        .apply_with_history(
            insertion(&document, 3, 2, "c"),
            typing_context(1_499, TypingDirection::Forward, 2, 3),
        )
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "abc");
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "");
    assert!(document.undo().is_none());
    document.redo().unwrap();
    assert_eq!(document.snapshot().to_string(), "abc");
}

#[test]
fn timeout_direction_and_selection_each_break_a_typing_group() {
    let mut document = Document::new("").unwrap();
    document
        .apply_with_history(
            insertion(&document, 1, 0, "a"),
            typing_context(0, TypingDirection::Forward, 0, 1),
        )
        .unwrap();
    document
        .apply_with_history(
            insertion(&document, 2, 1, "b"),
            typing_context(501, TypingDirection::Forward, 1, 2),
        )
        .unwrap();
    document
        .apply_with_history(
            insertion(&document, 3, 2, "c"),
            typing_context(600, TypingDirection::Backward, 2, 3),
        )
        .unwrap();
    document
        .apply_with_history(
            insertion(&document, 4, 3, "d"),
            typing_context(700, TypingDirection::Backward, 1, 4),
        )
        .unwrap();

    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "abc");
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "ab");
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "a");
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "");
}

#[test]
fn save_focus_newline_and_explicit_boundary_close_groups() {
    let mut document = Document::new("").unwrap();
    let boundaries = [
        HistoryBoundary::Save,
        HistoryBoundary::FocusLost,
        HistoryBoundary::CursorRelocated,
    ];
    let mut byte = 0;
    let mut id = 1;

    for boundary in boundaries {
        document
            .apply_with_history(
                insertion(&document, id, byte, "x"),
                typing_context(id * 10, TypingDirection::Forward, byte, byte + 1),
            )
            .unwrap();
        byte += 1;
        id += 1;
        document.close_history_group(boundary);
    }
    document
        .apply_with_history(
            insertion(&document, id, byte, "\n"),
            typing_context(id * 10, TypingDirection::Forward, byte, byte + 1),
        )
        .unwrap();
    byte += 1;
    id += 1;
    document
        .apply_with_history(
            insertion(&document, id, byte, "z"),
            typing_context(id * 10, TypingDirection::Forward, byte, byte + 1),
        )
        .unwrap();

    for expected in ["xxx\n", "xxx", "xx", "x", ""] {
        document.undo().unwrap();
        assert_eq!(document.snapshot().to_string(), expected);
    }
    assert!(document.undo().is_none());
}

#[test]
fn new_edit_invalidates_redo_even_when_it_starts_a_group() {
    let mut document = Document::new("").unwrap();
    document
        .apply_with_history(
            insertion(&document, 1, 0, "a"),
            typing_context(0, TypingDirection::Forward, 0, 1),
        )
        .unwrap();
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "");

    document
        .apply_with_history(
            insertion(&document, 2, 0, "b"),
            typing_context(1, TypingDirection::Forward, 0, 1),
        )
        .unwrap();

    assert!(document.redo().is_none());
    assert_eq!(document.snapshot().to_string(), "b");
}

#[test]
fn rejected_edit_does_not_mutate_the_open_history_group() {
    let mut document = Document::new("").unwrap();
    document
        .apply_with_history(
            insertion(&document, 1, 0, "a"),
            typing_context(0, TypingDirection::Forward, 0, 1),
        )
        .unwrap();
    let rejected = insertion(&document, 2, 1, &"x".repeat(MAX_DOCUMENT_BYTES + 1));
    assert!(document.apply(rejected).is_err());
    document
        .apply_with_history(
            insertion(&document, 3, 1, "b"),
            typing_context(100, TypingDirection::Forward, 1, 2),
        )
        .unwrap();

    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), "");
    assert!(document.undo().is_none());
}
