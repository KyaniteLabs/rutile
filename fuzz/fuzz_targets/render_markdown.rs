#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|_data: &[u8]| {
    // Build-only placeholder. The rendering implementation is not owned by Task 1A.
});
