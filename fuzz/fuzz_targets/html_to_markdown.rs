#![no_main]

//! Fuzz the clipboard-HTML → Markdown smart-paste converter.
//!
//! Invariants asserted on every input:
//! * `html_to_markdown` never panics (hostile clipboard content is untrusted).
//! * Output is bounded by `MAX_OUTPUT_BYTES`.
//! * The produced Markdown is *safe by construction*: it feeds the real
//!   renderer without panicking and the rendered HTML contains no `<script`,
//!   no `<iframe`, no `javascript:` URL, and no inline event handler. Because
//!   `render_markdown` treats every HTML event as escaped text, the true
//!   safety property is checked against the rendered output rather than a naive
//!   substring scan of the Markdown (a fenced code block may legitimately
//!   contain the text `<script>`, which is inert once rendered).

use rutile_core::{MAX_OUTPUT_BYTES, html_to_markdown, render_markdown};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(html) = std::str::from_utf8(input) else {
        return;
    };
    let Ok(markdown) = html_to_markdown(html) else {
        return;
    };

    assert!(
        markdown.len() <= MAX_OUTPUT_BYTES,
        "markdown output exceeded its byte cap"
    );

    // The converter must never emit a raw executable tag or scheme.
    let lower = markdown.to_ascii_lowercase();
    assert!(!lower.contains("<script"), "markdown leaked a <script tag");
    assert!(!lower.contains("<iframe"), "markdown leaked an <iframe tag");
    assert!(
        !lower.contains("javascript:"),
        "markdown leaked a javascript: URL"
    );

    // The Markdown must render safely through the existing pipeline.
    let Ok(rendered) = render_markdown(&markdown, 1) else {
        return;
    };
    let body = rendered.body.to_ascii_lowercase();
    assert!(!body.contains("<script"), "rendered body leaked a script");
    assert!(!body.contains("<iframe"), "rendered body leaked an iframe");
    assert!(
        !body.contains("javascript:"),
        "rendered body leaked a javascript: URL"
    );
    assert!(
        !body.contains(" onerror=")
            && !body.contains(" onload=")
            && !body.contains(" onclick=")
            && !body.contains(" onmouseover="),
        "rendered body leaked an event handler"
    );
});
