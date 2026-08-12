#![no_main]

use libfuzzer_sys::fuzz_target;
use rutile_core::{DESIGN_TOKENS, DesignTokenSet};

fuzz_target!(|data: &[u8]| {
    // Try to interpret the fuzz input as (id_bytes, value_bytes) split on
    // the first newline. This exercises the CSS-injection defense chain.
    let Some(split) = data.iter().position(|&b| b == b'\n') else {
        return;
    };
    let id = std::str::from_utf8(&data[..split]).unwrap_or("");
    let value = std::str::from_utf8(&data[split + 1..]).unwrap_or("");

    if id.is_empty() || value.is_empty() {
        return;
    }

    let mut set = DesignTokenSet::default();
    // The set rejects unknown ids and injection-carrying values.
    let _ = set.set(id, value);

    // Verify that only frozen ids are present.
    for (token_id, _) in set.iter() {
        debug_assert!(DESIGN_TOKENS.contains(&token_id));
    }
});
