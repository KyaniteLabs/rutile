#![no_main]

//! Fuzz the export-page validator.
//!
//! Invariants asserted on every input:
//! * `ExportPage::from_html` never panics (the validator is hand-rolled and
//!   must be safe on arbitrary input).
//! * `render_export_page` never panics on a small, fixed source document.

use rutile_core::{ExportPage, ExportRequest, render_export_page};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(html) = std::str::from_utf8(input) else {
        return;
    };
    let _ = ExportPage::from_html(html.to_owned());

    // The render path must also stay panic-free for a fixed, benign source.
    let request = ExportRequest::new(rutile_types::Revision::new(1), None).unwrap();
    let _ = render_export_page("# Hello\n\nworld.", &request);
});
