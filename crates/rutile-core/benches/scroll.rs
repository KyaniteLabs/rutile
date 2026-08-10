use std::hint::black_box;
use std::time::{Duration, Instant};

use rutile_core::{ScrollAnchorView, ScrollGeometry, ScrollMap};

const BLOCK_COUNT: usize = 100_000;
const LOOKUPS_PER_DIRECTION: usize = 200_000;
const WARMUPS_PER_DIRECTION: usize = 10_000;
const P95_GATE: Duration = Duration::from_micros(100);

#[derive(Clone, Copy)]
struct BenchBlock {
    start: usize,
    ordinal: u32,
}

impl ScrollAnchorView for BenchBlock {
    fn revision(&self) -> u64 {
        1
    }

    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.start + 16
    }

    fn ordinal(&self) -> u32 {
        self.ordinal
    }

    fn preview_top(&self) -> f64 {
        self.ordinal as f64
    }
}

fn main() {
    let document_len = BLOCK_COUNT * 16;
    let blocks = (0..BLOCK_COUNT).map(|ordinal| BenchBlock {
        start: ordinal * 16,
        ordinal: ordinal as u32,
    });

    let build_started = Instant::now();
    let map = ScrollMap::new(
        ScrollGeometry {
            revision: 1,
            document_len,
            source_max_top: document_len - 1,
            preview_max_y: BLOCK_COUNT as f64,
        },
        blocks,
    )
    .expect("100k-block map");
    let build_elapsed = build_started.elapsed();

    let mut checksum = 0_u128;
    for sample in 0..WARMUPS_PER_DIRECTION {
        let byte = black_box(sample.wrapping_mul(7_919) % document_len);
        checksum ^= black_box(map.source_to_preview(1, byte).expect("source warmup")) as u128;
        let y = black_box((sample.wrapping_mul(1_009) % BLOCK_COUNT) as f64);
        checksum ^= black_box(map.preview_to_source(1, y).expect("preview warmup")) as u128;
    }

    let mut source_durations = Vec::with_capacity(LOOKUPS_PER_DIRECTION);
    for sample in 0..LOOKUPS_PER_DIRECTION {
        let byte = black_box(sample.wrapping_mul(7_919) % document_len);
        let started = Instant::now();
        let mapped = map.source_to_preview(1, byte).expect("source map");
        source_durations.push(started.elapsed());
        checksum ^= black_box(mapped) as u128;
    }

    let mut preview_durations = Vec::with_capacity(LOOKUPS_PER_DIRECTION);
    for sample in 0..LOOKUPS_PER_DIRECTION {
        let y = black_box((sample.wrapping_mul(1_009) % BLOCK_COUNT) as f64);
        let started = Instant::now();
        let mapped = map.preview_to_source(1, y).expect("preview map");
        preview_durations.push(started.elapsed());
        checksum ^= black_box(mapped) as u128;
    }

    source_durations.sort_unstable();
    preview_durations.sort_unstable();
    let source_p95 = nearest_rank_p95(&source_durations);
    let preview_p95 = nearest_rank_p95(&preview_durations);

    black_box(checksum);
    eprintln!(
        "100k scroll blocks: build={build_elapsed:?}, source-to-preview p95={source_p95:?}, preview-to-source p95={preview_p95:?}"
    );
    assert!(
        source_p95 < P95_GATE,
        "source-to-preview p95 must be below {P95_GATE:?}, got {source_p95:?}"
    );
    assert!(
        preview_p95 < P95_GATE,
        "preview-to-source p95 must be below {P95_GATE:?}, got {preview_p95:?}"
    );
}

fn nearest_rank_p95(samples: &[Duration]) -> Duration {
    assert!(!samples.is_empty(), "p95 requires retained samples");
    let index = (95 * samples.len()).div_ceil(100) - 1;
    samples[index]
}
