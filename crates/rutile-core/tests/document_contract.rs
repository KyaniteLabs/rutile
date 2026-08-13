use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use rutile_core::{
    Document, Edit, EditError, EditTransaction, MAX_DOCUMENT_BYTES, TransactionKind,
};
use rutile_types::Revision;

struct CountingAllocator;

thread_local! {
    static COUNT_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT_ALLOCATIONS.with(|enabled| {
            if enabled.get() {
                ALLOCATIONS.with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn transaction(
    document: &Document,
    id: u64,
    range: std::ops::Range<usize>,
    text: &str,
) -> EditTransaction {
    EditTransaction {
        base_revision: document.revision(),
        id,
        kind: TransactionKind::Typing,
        edits: vec![Edit {
            byte_range: range,
            replacement: text.to_owned(),
        }],
    }
}

#[test]
fn utf8_split_edit_is_atomic() {
    let mut document = Document::new("a🪶b").unwrap();
    let before = document.snapshot().to_string();

    let error = document
        .apply(transaction(&document, 1, 2..3, "x"))
        .unwrap_err();

    assert!(matches!(error, EditError::NotCharBoundary { .. }));
    assert_eq!(document.revision(), Revision::new(0));
    assert_eq!(document.snapshot().to_string(), before);
    assert!(document.undo().is_none());
}

#[test]
fn multi_edit_transactions_match_a_string_oracle() {
    let mut document = Document::new("alpha βeta 🪶 omega").unwrap();
    let mut oracle = document.snapshot().to_string();
    let mut state = 0x6a09_e667_f3bc_c909_u64;

    for id in 1..=400 {
        let boundaries = oracle
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(oracle.len()))
            .collect::<Vec<_>>();
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let first = (state as usize) % boundaries.len();
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let second = (state as usize) % boundaries.len();
        let (start_index, end_index) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        let range = boundaries[start_index]..boundaries[end_index];
        let replacement = match id % 5 {
            0 => "",
            1 => "x",
            2 => "é",
            3 => "日本",
            _ => "\n🪶",
        };

        oracle.replace_range(range.clone(), replacement);
        document
            .apply(transaction(&document, id, range, replacement))
            .unwrap();

        assert_eq!(document.snapshot().to_string(), oracle, "transaction {id}");
    }
}

#[test]
fn snapshot_clone_is_constant_allocation_and_keeps_the_old_root() {
    let mut document = Document::new(&"🪶".repeat(300_000)).unwrap();
    let snapshot = document.snapshot();

    ALLOCATIONS.with(|count| count.set(0));
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
    let cloned = snapshot.clone();
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
    let clone_allocations = ALLOCATIONS.with(Cell::get);

    document
        .apply(transaction(&document, 1, 0..0, "prefix"))
        .unwrap();

    assert!(
        clone_allocations <= 1,
        "snapshot clone allocated {clone_allocations} times"
    );
    assert_eq!(snapshot.to_string(), cloned.to_string());
    assert!(!snapshot.to_string().starts_with("prefix"));
    assert!(document.snapshot().to_string().starts_with("prefix"));
}

#[test]
fn post_edit_limit_rejection_preserves_document_and_history() {
    let mut document = Document::new("kept").unwrap();
    let oversized = "x".repeat(MAX_DOCUMENT_BYTES + 1);

    let error = document
        .apply(transaction(&document, 1, 0..4, &oversized))
        .unwrap_err();

    assert_eq!(error, EditError::TooLarge);
    assert_eq!(document.revision(), Revision::new(0));
    assert_eq!(document.snapshot().to_string(), "kept");
    assert!(document.undo().is_none());
}

#[test]
fn undo_budget_evicts_only_complete_transactions() {
    const CHUNK: usize = 1024 * 1024;
    let mut document = Document::new(&"a".repeat(CHUNK)).unwrap();

    for id in 1..=34 {
        let next = if id % 2 == 0 { "a" } else { "b" }.repeat(CHUNK);
        document
            .apply(transaction(&document, id, 0..CHUNK, &next))
            .unwrap();
    }

    let mut undo_count = 0;
    while document.undo().is_some() {
        undo_count += 1;
    }

    assert_eq!(undo_count, 31);
    assert_eq!(document.revision(), Revision::new(34 + undo_count as u64));
}

#[test]
fn undo_and_redo_emit_incremental_changes() {
    let mut document = Document::new("one 🪶 three").unwrap();
    let change = document
        .apply(transaction(&document, 1, 4..8, "two"))
        .unwrap();
    assert_eq!(change.before, Revision::new(0));
    assert_eq!(change.after, Revision::new(1));
    assert_eq!(change.changed_bytes_after, 4..7);
    assert_eq!(document.snapshot().to_string(), "one two three");

    let undo = document.undo().unwrap();
    assert_eq!(
        (undo.before, undo.after),
        (Revision::new(1), Revision::new(2))
    );
    assert_eq!(document.snapshot().to_string(), "one 🪶 three");

    let redo = document.redo().unwrap();
    assert_eq!(
        (redo.before, redo.after),
        (Revision::new(2), Revision::new(3))
    );
    assert_eq!(document.snapshot().to_string(), "one two three");
}
