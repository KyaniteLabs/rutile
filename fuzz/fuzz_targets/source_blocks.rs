#![no_main]

use rutile_core::{
    MAX_SOURCE_BLOCK_BYTES, SourceBlockKind, build_source_blocks, validate_source_blocks,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(source) = std::str::from_utf8(input) else {
        return;
    };
    let blocks = build_source_blocks(source, 23).unwrap();
    validate_source_blocks(source, 23, &blocks).unwrap();
    for window in blocks.windows(2) {
        assert!(window[0].start <= window[1].start);
        assert!(window[0].end <= window[1].start);
    }
    for block in &blocks {
        if block.start != block.end {
            assert!(block.end - block.start <= MAX_SOURCE_BLOCK_BYTES);
        }
        if block.segment_index > 0 {
            assert_eq!(block.kind, SourceBlockKind::Continuation);
        }
    }
});
