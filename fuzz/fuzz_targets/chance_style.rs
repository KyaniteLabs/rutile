#![no_main]

use libfuzzer_sys::fuzz_target;
use rutile_core::{ChanceRoll, ContentType, render_chance_css};

fuzz_target!(|data: &[u8]| {
    // Seed from the first 8 bytes; content type from the 9th byte.
    if data.len() < 9 {
        return;
    }
    let seed = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    let content_type = match data[8] % 5 {
        0 => ContentType::Journal,
        1 => ContentType::Spec,
        2 => ContentType::Letter,
        3 => ContentType::Recipe,
        _ => ContentType::Note,
    };

    let mut roll = ChanceRoll::new(seed, content_type);

    // Exercise lock/reroll cycles with fuzz-derived token ids.
    if let Ok(token) = std::str::from_utf8(&data[9..]) {
        let _ = roll.lock(token);
    }

    // Reroll a few times — locked dimensions must survive.
    for _ in 0..4 {
        roll.reroll();
    }

    // Render the CSS — verify no injection vectors.
    let css = render_chance_css(roll.current());
    debug_assert!(!css.contains("url("));
    debug_assert!(!css.contains("@import"));
    debug_assert!(!css.contains("</"));
    debug_assert!(!css.contains("javascript:"));
});
