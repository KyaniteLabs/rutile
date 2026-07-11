//! UltraQA round 2 (Wave 4) — hostile/novel-input campaign over the 0.2
//! feature surfaces: format engine, html_to_markdown smart-paste, themed
//! export, find/replace, autosave/recovery, counts.
//!
//! Method: execution with adversarial inputs through the public APIs, not a
//! re-run of the happy-path suites. Two findings from this campaign are fixed
//! and carry a red-green regression here:
//!   * `list_conversion_does_not_blow_up_quadratically` — a ~2 MiB paste of many
//!     `<li>`s hung the converter for minutes (O(n²) per-item re-join).
//!   * `format_*_selection_never_panics` — out-of-range / mid-scalar selections
//!     panicked (and aborted the process under `panic = "abort"`).

use std::time::{Duration, Instant};

use feathermark_core::{
    AutosaveStore, Document, ExportRequest, FindDirection, FindQuery, FormatCommand,
    HtmlToMarkdownError, MatchMode, ReplaceSpec, Selection, SmartEnterAction, apply_format, counts,
    find_next, html_to_markdown, match_count, render_export_page, render_markdown, replace_all,
    smart_enter,
};

// ---------------------------------------------------------------------------
// Lane 1 — format engine edge cases
// ---------------------------------------------------------------------------

fn apply(text: &str, selection: Selection, command: FormatCommand) -> String {
    let mut document = Document::new(text).unwrap();
    let plan = apply_format(&document, selection, command).unwrap();
    document.apply(plan.into_transaction(1)).unwrap();
    document.snapshot().to_string()
}

#[test]
fn format_bold_on_empty_document_is_a_clean_insert() {
    let out = apply("", Selection::collapsed(0), FormatCommand::ToggleBold);
    assert_eq!(out, "****");
}

#[test]
fn format_out_of_range_collapsed_selection_never_panics() {
    // Regression: word_at()/line_start() sliced the raw offset and panicked.
    let out = apply("abc", Selection::collapsed(100), FormatCommand::ToggleBold);
    assert_eq!(out, "**abc**");
}

#[test]
fn format_mid_scalar_selection_never_panics() {
    // Offset 4 falls inside 'é' (bytes 3..5) of "café"; must snap, not panic.
    let out = apply("café", Selection::collapsed(4), FormatCommand::ToggleBold);
    assert_eq!(out, "**café**");
}

#[test]
fn format_out_of_range_range_selection_never_panics() {
    // Both endpoints past EOF and reversed order.
    let doc = Document::new("hello").unwrap();
    let plan = apply_format(
        &doc,
        Selection {
            anchor: 999,
            head: 400,
        },
        FormatCommand::ToggleItalic,
    );
    // Clamped to EOF: an empty target inserts an empty emphasis pair.
    assert!(plan.is_ok());
}

#[test]
fn format_combining_char_selection_round_trips() {
    // "é" as e + combining acute (U+0301) is 3 bytes; select all of it.
    let text = "e\u{301}x";
    let out = apply(
        text,
        Selection { anchor: 0, head: 3 },
        FormatCommand::ToggleBold,
    );
    assert_eq!(out, "**e\u{301}**x");
}

#[test]
fn smart_enter_out_of_range_selection_never_panics() {
    let doc = Document::new("- item").unwrap();
    let outcome = smart_enter(&doc, Selection::collapsed(999)).unwrap();
    assert_eq!(
        outcome.action,
        SmartEnterAction::ContinueBullet {
            marker: feathermark_core::ListMarker::Dash
        }
    );
}

#[test]
fn smart_enter_continues_deeply_nested_quote() {
    let text = "> > > deep";
    let doc = Document::new(text).unwrap();
    let outcome = smart_enter(&doc, Selection::collapsed(text.len())).unwrap();
    assert_eq!(outcome.action, SmartEnterAction::ContinueQuote { depth: 3 });
    let mut d = Document::new(text).unwrap();
    d.apply(outcome.plan.into_transaction(1)).unwrap();
    assert_eq!(d.snapshot().to_string(), "> > > deep\n> > > ");
}

#[test]
fn smart_enter_double_enter_exits_nested_list() {
    // Cursor on an empty continuation line exits the block cleanly.
    let text = "- a\n- ";
    let doc = Document::new(text).unwrap();
    let outcome = smart_enter(&doc, Selection::collapsed(text.len())).unwrap();
    assert_eq!(outcome.action, SmartEnterAction::ExitEmptyItem);
}

#[test]
fn cycle_heading_wraps_past_h6_to_paragraph() {
    let mut cur = "Title".to_string();
    for _ in 0..6 {
        cur = apply(&cur, Selection::collapsed(0), FormatCommand::CycleHeading);
    }
    assert_eq!(cur, "###### Title");
    // The 7th cycle drops back to a paragraph.
    let para = apply(&cur, Selection::collapsed(0), FormatCommand::CycleHeading);
    assert_eq!(para, "Title");
}

#[test]
fn quote_toggle_over_edit_cap_is_a_typed_error_not_a_panic() {
    use feathermark_core::{EditPlanError, MAX_PLAN_EDITS};
    let text = "x\n".repeat(MAX_PLAN_EDITS + 50);
    let doc = Document::new(&text).unwrap();
    let sel = Selection {
        anchor: 0,
        head: text.len(),
    };
    let result = apply_format(&doc, sel, FormatCommand::ToggleQuote);
    assert!(matches!(
        result,
        Err(EditPlanError::TooManyEdits { max, .. }) if max == MAX_PLAN_EDITS
    ));
}

#[test]
fn bold_inside_bold_wraps_not_strips() {
    // Selecting the inner word of ***?*** style overlap must not corrupt markers.
    let text = "**bold**";
    let out = apply(
        text,
        Selection { anchor: 2, head: 6 },
        FormatCommand::ToggleCodeSpan,
    );
    assert_eq!(out, "**`bold`**");
}

// ---------------------------------------------------------------------------
// Lane 2 — html_to_markdown hostile clipboard
// ---------------------------------------------------------------------------

#[test]
fn list_conversion_does_not_blow_up_quadratically() {
    // Regression: re-joining the accumulated list per item was O(n²); a big
    // paste hung for minutes. Linear now — a large list completes in well under
    // the generous bound even in a debug build.
    let count = 60_000usize;
    let mut html = String::with_capacity(count * 10 + 16);
    html.push_str("<ul>");
    for _ in 0..count {
        html.push_str("<li>a</li>");
    }
    html.push_str("</ul>");

    let start = Instant::now();
    let out = html_to_markdown(&html).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(15),
        "list conversion took {elapsed:?}; expected linear-time completion"
    );
    assert_eq!(out.matches("- a").count(), count);
}

#[test]
fn near_max_raw_text_paste_completes_fast() {
    // Round-1 O(n²) `skip_raw_text` guard still holds: ~1.9 MiB of adjacent
    // <style></style> pairs must complete quickly.
    let pair = "<style>x</style>";
    let repeats = (1_900_000usize) / pair.len();
    let html = pair.repeat(repeats);
    let start = Instant::now();
    let out = html_to_markdown(&html).unwrap();
    assert!(start.elapsed() < Duration::from_secs(10));
    assert!(out.is_empty(), "style content must be dropped");
}

#[test]
fn deeply_nested_inline_hits_depth_cap_not_stack_overflow() {
    let html = format!("{}x{}", "<b>".repeat(1000), "</b>".repeat(1000));
    assert_eq!(
        html_to_markdown(&html),
        Err(HtmlToMarkdownError::NestingTooDeep)
    );
}

#[test]
fn malformed_and_unclosed_everywhere_never_panics() {
    let corpus = [
        "<a href=<b><<<>>> <p unterminated",
        "<<<<<<<<<<<<<<",
        "<a href=\"http://x\" href='javascript:evil'>dup</a>",
        "<ul><li><ul><li><ul><li>no closes",
        "<strong><em><code>mixed</strong></code></em>",
        "<a href=\"  \t javascript:alert(1)\">x</a>",
        "text < not a tag & raw ampersand > done",
        "<p>&#60;script&#62;alert(1)&#60;/script&#62;</p>",
        "<blockquote><blockquote><p>quote",
        "<!-- <script>evil</script> --> after comment",
        "<?php echo 'x' ?> pi",
        "<style>body{background:url(http://evil)}</style>keep",
    ];
    for input in corpus {
        // Must return a bounded Result and never panic.
        let _ = html_to_markdown(input);
    }
}

#[test]
fn entity_obfuscated_javascript_href_is_dropped() {
    let html = "<a href=\"&#106;avascript:alert(1)\">click</a>";
    let out = html_to_markdown(html).unwrap();
    assert!(
        !out.to_ascii_lowercase().contains("javascript:"),
        "obfuscated javascript href leaked: {out:?}"
    );
    assert!(out.contains("click"));
    // No markdown link was emitted (href rejected), just the text.
    assert!(!out.contains("]("));
}

#[test]
fn blockquote_amplification_is_bounded_not_hanging() {
    // Deep nesting × wide content: must terminate (bounded output/nesting), fast.
    let inner = "line\n".repeat(2000);
    let html = format!(
        "{}{}{}",
        "<blockquote>".repeat(200),
        inner,
        "</blockquote>".repeat(200)
    );
    let start = Instant::now();
    let result = html_to_markdown(&html);
    assert!(start.elapsed() < Duration::from_secs(10));
    // Either it fit under the output cap or it was rejected — never a hang/panic.
    match result {
        Ok(out) => assert!(out.len() <= feathermark_core::MAX_OUTPUT_BYTES),
        Err(HtmlToMarkdownError::OutputTooLarge) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn smart_paste_output_is_safe_by_construction() {
    // Every hostile input's Markdown must render through the real pipeline with
    // no executable vector surviving.
    let corpus = [
        "<script>alert(1)</script><p>hi</p>",
        "<img src=x onerror=alert(1)>",
        "<a href=\"javascript:alert(1)\">x</a>",
        "<a href=\"data:text/html,<script>alert(1)</script>\">y</a>",
        "<svg onload=alert(1)></svg>",
        "<iframe src=\"https://evil\"></iframe>",
        "<div onclick=\"steal()\">z</div>",
        "<a href=\"vbscript:msgbox(1)\">v</a>",
        "<p>plain &amp; safe</p>",
        "<a href=\"jav\tascript:alert(1)\">tabbed</a>",
    ];
    for input in corpus {
        let Ok(markdown) = html_to_markdown(input) else {
            continue;
        };
        let lower = markdown.to_ascii_lowercase();
        assert!(
            !lower.contains("<script"),
            "md leaked <script: {markdown:?}"
        );
        assert!(
            !lower.contains("<iframe"),
            "md leaked <iframe: {markdown:?}"
        );
        assert!(
            !lower.contains("javascript:"),
            "md leaked javascript: {markdown:?}"
        );
        let Ok(rendered) = render_markdown(&markdown, 1) else {
            continue;
        };
        let body = rendered.body.to_ascii_lowercase();
        assert!(!body.contains("<script"), "body leaked <script: {input:?}");
        assert!(!body.contains("<iframe"), "body leaked <iframe: {input:?}");
        assert!(
            !body.contains("javascript:"),
            "body leaked javascript: {input:?}"
        );
        assert!(
            !body.contains(" onerror=")
                && !body.contains(" onload=")
                && !body.contains(" onclick="),
            "body leaked an event handler: {input:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Lane 3 — themed export sanitizer / self-containment
// ---------------------------------------------------------------------------

fn export(source: &str) -> String {
    let request = ExportRequest::new(1, Some("QA".to_owned())).unwrap();
    render_export_page(source, &request)
        .expect("hostile-but-renderable doc exports")
        .into_html()
}

#[test]
fn export_of_hostile_but_renderable_doc_is_self_contained() {
    // Each of these carries a would-be vector as *document text*. The renderer
    // escapes it, so the export inspector accepts a page that fetches/executes
    // nothing. `render_export_page` returning Ok is itself the proof (ExportPage
    // cannot exist unless it passed the self-containment allowlist).
    let sources = [
        "# Title\n\n<script>alert(1)</script>",
        "[click](javascript:alert(1))",
        "![alt](data:text/html,<script>alert(1)</script>)",
        "text with url(http://evil.example/x) inline",
        "<img src=x onerror=alert(1)>",
        "> quote\n\n```\n<script>while(1){}</script>\n```",
        "[link](https://ok.example/path?q=1)",
        "Contact <me@example.com> please",
    ];
    for source in sources {
        let html = export(source);
        // No real executable tag or external fetch in the produced markup.
        // (Document text is escaped, so these substrings can only come from a
        // real tag/attribute the template emitted — of which there are none.)
        assert!(
            !html.contains("<script"),
            "export carried <script: {source:?}"
        );
        assert!(
            !html.contains("<iframe"),
            "export carried <iframe: {source:?}"
        );
        assert!(
            !html.contains("src=\"http"),
            "export carried an external src: {source:?}"
        );
        assert!(
            !html.contains("src=\"//"),
            "export carried a protocol-relative src: {source:?}"
        );
        assert!(
            !html.to_ascii_lowercase().contains("<a href=\"javascript:"),
            "export carried a javascript: link: {source:?}"
        );
    }
}

// The following directly exercise the export self-containment inspector
// (`ExportPage::from_html`) with hostile markup that the current template never
// emits (document text is renderer-escaped), to confirm the allowlist would
// still catch these if the template ever broadened.

fn inspect_rejects(html: &str) -> bool {
    use feathermark_core::ExportPage;
    ExportPage::from_html(html.to_owned()).is_err()
}

#[test]
fn inspector_rejects_event_handlers_with_odd_case_and_spacing() {
    assert!(inspect_rejects("<div OnLoad = \"x()\">a</div>"));
    assert!(inspect_rejects("<body onLoad='x()'>a</body>"));
    assert!(inspect_rejects("<img src=\"data:,\" ONERROR=\"x()\">"));
}

#[test]
fn inspector_rejects_protocol_relative_and_relative_src() {
    assert!(inspect_rejects("<img src=\"//evil.example/x.png\">"));
    assert!(inspect_rejects("<img src=\"x.png\">"));
    assert!(inspect_rejects("<img src=\"/root/x.png\">"));
    assert!(inspect_rejects("<img src=\"http://evil.example/x\">"));
}

#[test]
fn inspector_rejects_data_html_and_css_obfuscation() {
    assert!(inspect_rejects("<img src=\"data:text/html,<b>\">"));
    assert!(inspect_rejects(
        "<div style=\"background:expression(alert(1))\">a</div>"
    ));
    assert!(inspect_rejects(
        "<style>@import url(http://evil.example/x.css)</style>"
    ));
    assert!(inspect_rejects(
        "<style>body{background:url('http://evil.example/x')}</style>"
    ));
}

#[test]
fn inspector_accepts_data_uri_and_plain_hyperlinks() {
    // Self-contained references and inert hyperlinks are allowed.
    assert!(!inspect_rejects(
        "<!doctype html><html><body><img src=\"data:image/png;base64,AAAA\"><a href=\"https://ok.example\">x</a></body></html>"
    ));
}

// ---------------------------------------------------------------------------
// Lane 4 — find/replace at boundaries
// ---------------------------------------------------------------------------

fn q(pattern: &str, mode: MatchMode, cs: bool) -> FindQuery {
    FindQuery::new(pattern.to_owned(), mode, cs).unwrap()
}

fn apply_plans_sequential(text: &str, spec: &ReplaceSpec) -> String {
    let plans = replace_all(0, text, spec).unwrap();
    let mut document = Document::new(text).unwrap();
    for plan in plans {
        let tx = plan.into_transaction(0);
        document.apply(tx).unwrap();
    }
    document.snapshot().to_string()
}

#[test]
fn replace_all_over_ten_thousand_matches_fully_replaces() {
    let count = 10_000usize;
    let text = "x ".repeat(count);
    let spec = ReplaceSpec::new(q("x", MatchMode::Plain, true), "yy".to_owned()).unwrap();
    let plans = replace_all(0, &text, &spec).unwrap();
    assert!(plans.len() >= 2, "expected chunking, got {}", plans.len());
    let out = apply_plans_sequential(&text, &spec);
    assert_eq!(out, "yy ".repeat(count));
    assert_eq!(match_count(&text, spec.query()), count);
}

#[test]
fn whole_word_respects_unicode_word_boundaries() {
    let text = "naïve naïveté";
    let query = q("naïve", MatchMode::WholeWord, true);
    // First "naïve" is whole (space after); the one inside "naïveté" is not.
    let first = find_next(text, &query, 0, FindDirection::Forward, false).unwrap();
    assert_eq!(&text[first.clone()], "naïve");
    assert_eq!(
        find_next(text, &query, first.end, FindDirection::Forward, false),
        None
    );
}

#[test]
fn case_insensitive_folding_edge_cases_never_panic() {
    // Length-changing foldings (ß, İ) are documented non-matches, not panics.
    let straed = "Straße";
    // Same scalars, different case → matches.
    assert!(
        find_next(
            straed,
            &q("straße", MatchMode::Plain, false),
            0,
            FindDirection::Forward,
            false
        )
        .is_some()
    );
    // ß→ss expansion is not matched (documented limitation).
    assert!(
        find_next(
            straed,
            &q("strasse", MatchMode::Plain, false),
            0,
            FindDirection::Forward,
            false
        )
        .is_none()
    );
    // Turkish dotted capital İ vs ascii i: folds to i + combining dot → no match.
    let istanbul = "İstanbul";
    assert!(
        find_next(
            istanbul,
            &q("istanbul", MatchMode::Plain, false),
            0,
            FindDirection::Forward,
            false
        )
        .is_none()
    );
}

#[test]
fn overlapping_pattern_matches_are_non_overlapping() {
    let text = "aaaa";
    let spec = ReplaceSpec::new(q("aa", MatchMode::Plain, true), "b".to_owned()).unwrap();
    assert_eq!(match_count(text, spec.query()), 2);
    assert_eq!(apply_plans_sequential(text, &spec), "bb");
}

#[test]
fn find_from_mid_multibyte_offset_is_nudged() {
    let text = "áíóú"; // each char two bytes
    let query = q("ó", MatchMode::Plain, true);
    // from_byte 3 is mid-scalar; must nudge, not panic.
    let found = find_next(text, &query, 3, FindDirection::Forward, false).unwrap();
    assert_eq!(&text[found], "ó");
}

// ---------------------------------------------------------------------------
// Lane 5 — autosave / recovery corruption
// ---------------------------------------------------------------------------

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "fm-qa-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snap(text: &str) -> feathermark_core::DocumentSnapshot {
    Document::new(text).unwrap().snapshot()
}

fn append_journal_line(dir: &std::path::Path, line: &[u8]) {
    let journal = dir.join(feathermark_core::AUTOSAVE_JOURNAL_FILE);
    let mut bytes = std::fs::read(&journal).unwrap_or_default();
    bytes.extend_from_slice(line);
    std::fs::write(&journal, &bytes).unwrap();
}

#[test]
fn wrong_schema_journal_entry_is_skipped() {
    let dir = TempDir::new("wrong-schema");
    let store = AutosaveStore::new(dir.0.clone());
    store.record(0, &snap("good"), None, 1).unwrap();
    append_journal_line(
        &dir.0,
        format!(
            "{{\"schema\":\"evil.v1\",\"v\":1,\"sequence\":9,\"captured_at_unix_ms\":9,\"document_path\":null,\"document_revision\":0,\"snapshot_file\":\"autosave-9.md\",\"snapshot_bytes\":1,\"snapshot_blake3\":\"{}\"}}\n",
            "0".repeat(64)
        )
        .as_bytes(),
    );
    let recovered = store.recover().unwrap().unwrap();
    assert_eq!(recovered.entry.sequence, 0);
    assert_eq!(recovered.document.snapshot().to_string(), "good");
}

#[test]
fn unsupported_version_journal_entry_is_skipped() {
    let dir = TempDir::new("bad-version");
    let store = AutosaveStore::new(dir.0.clone());
    store.record(0, &snap("alive"), None, 1).unwrap();
    append_journal_line(
        &dir.0,
        format!(
            "{{\"schema\":\"feathermark.autosave.v1\",\"v\":2,\"sequence\":9,\"captured_at_unix_ms\":9,\"document_path\":null,\"document_revision\":0,\"snapshot_file\":\"autosave-9.md\",\"snapshot_bytes\":1,\"snapshot_blake3\":\"{}\"}}\n",
            "0".repeat(64)
        )
        .as_bytes(),
    );
    let recovered = store.recover().unwrap().unwrap();
    assert_eq!(recovered.entry.sequence, 0);
}

#[test]
fn path_traversal_snapshot_reference_is_unrecoverable() {
    let dir = TempDir::new("traversal");
    let store = AutosaveStore::new(dir.0.clone());
    // A journal entry whose snapshot_file tries to escape the store dir. The
    // session contract makes a non-bare name undecodable, so it is skipped.
    append_journal_line(
        &dir.0,
        format!(
            "{{\"schema\":\"feathermark.autosave.v1\",\"v\":1,\"sequence\":5,\"captured_at_unix_ms\":5,\"document_path\":null,\"document_revision\":0,\"snapshot_file\":\"../../etc/passwd\",\"snapshot_bytes\":1,\"snapshot_blake3\":\"{}\"}}\n",
            "0".repeat(64)
        )
        .as_bytes(),
    );
    assert!(store.recover().unwrap().is_none());
}

#[test]
fn missing_snapshot_file_falls_back_to_lower_sequence() {
    let dir = TempDir::new("missing-snap");
    let store = AutosaveStore::new(dir.0.clone());
    store.record(0, &snap("keep"), None, 1).unwrap();
    let latest = store.record(1, &snap("gone"), None, 2).unwrap();
    std::fs::remove_file(dir.0.join(&latest.snapshot_file)).unwrap();
    let recovered = store.recover().unwrap().unwrap();
    assert_eq!(recovered.entry.sequence, 0);
    assert_eq!(recovered.document.snapshot().to_string(), "keep");
}

#[test]
fn highest_sequence_wins_under_out_of_order_append() {
    let dir = TempDir::new("out-of-order");
    let store = AutosaveStore::new(dir.0.clone());
    // Recorded out of order; recovery must still pick the highest sequence.
    store.record(5, &snap("five"), None, 1).unwrap();
    store.record(2, &snap("two"), None, 2).unwrap();
    store.record(9, &snap("nine"), None, 3).unwrap();
    store.record(3, &snap("three"), None, 4).unwrap();
    let recovered = store.recover().unwrap().unwrap();
    assert_eq!(recovered.entry.sequence, 9);
    assert_eq!(recovered.document.snapshot().to_string(), "nine");
    assert_eq!(store.next_sequence().unwrap(), 10);
}

// ---------------------------------------------------------------------------
// Counts — quick hostile sanity
// ---------------------------------------------------------------------------

#[test]
fn counts_on_multibyte_and_combining_are_scalar_exact() {
    let text = "e\u{301} 世界 \u{1f600}\u{200d}\u{1f4bb}";
    let c = counts(text);
    assert_eq!(c.chars, text.chars().count());
    assert_eq!(c.words, text.split_whitespace().count());
}
