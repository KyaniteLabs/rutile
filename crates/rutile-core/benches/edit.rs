use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::time::{Duration, Instant};

use rutile_core::{Document, Edit, EditTransaction, TransactionKind};

struct GateAllocator;

thread_local! {
    static MEASURING: Cell<bool> = const { Cell::new(false) };
    static LARGEST_ALLOCATION: Cell<usize> = const { Cell::new(0) };
    static ALLOCATED_BYTES: Cell<usize> = const { Cell::new(0) };
}

fn record_allocation(bytes: usize) {
    MEASURING.with(|measuring| {
        if measuring.get() {
            LARGEST_ALLOCATION.with(|largest| largest.set(largest.get().max(bytes)));
            ALLOCATED_BYTES.with(|total| total.set(total.get().saturating_add(bytes)));
        }
    });
}

unsafe impl GlobalAlloc for GateAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: GateAllocator = GateAllocator;

fn main() {
    let mut document = Document::new(&"a".repeat(5 * 1024 * 1024)).expect("fixture");
    let mut samples = Vec::with_capacity(1_000);
    for id in 1..=1_000 {
        let start = 2 * 1024 * 1024 + (id as usize % 1024);
        let transaction = EditTransaction {
            base_revision: document.revision(),
            id,
            kind: TransactionKind::Typing,
            edits: vec![Edit {
                byte_range: start..start + 1,
                replacement: if id % 2 == 0 { "a" } else { "b" }.into(),
            }],
        };
        LARGEST_ALLOCATION.with(|largest| largest.set(0));
        ALLOCATED_BYTES.with(|total| total.set(0));
        MEASURING.with(|measuring| measuring.set(true));
        let now = Instant::now();
        document.apply(transaction).expect("ordinary edit");
        samples.push(now.elapsed());
        MEASURING.with(|measuring| measuring.set(false));
        let largest = LARGEST_ALLOCATION.with(Cell::get);
        let allocated = ALLOCATED_BYTES.with(Cell::get);
        assert!(
            largest < 1024 * 1024,
            "ordinary edit allocated a {largest}-byte buffer; full-buffer copies are forbidden"
        );
        assert!(
            allocated < 1024 * 1024,
            "ordinary edit allocated {allocated} bytes total; full-buffer copies are forbidden"
        );
    }
    samples.sort_unstable();
    let p95 = samples[samples.len() * 95 / 100];
    assert!(
        p95 < Duration::from_millis(8),
        "5 MiB edit-preparation p95 {p95:?} exceeds 8 ms"
    );
    black_box(document);
    eprintln!("5 MiB ordinary-edit p95: {p95:?}");
}
