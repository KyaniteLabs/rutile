#![no_main]

use feathermark_protocol::decode_preview_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_preview_event(data, 0);
});
