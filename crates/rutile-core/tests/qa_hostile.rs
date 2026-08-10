//! QA round 1 hostile-input harness (qa/ultraqa-round1).
//!
//! Drives the public product APIs with adversarial inputs beyond the fuzzer's
//! reach. Assertions target invariants: no panic, sanitizer holds (no
//! executable vectors in rendered body), source-block contract holds, editor
//! and document caps hold. These tests are diagnostic; failures are findings.

use rutile_core::{
    Document, Edit, EditTransaction, HistoryContext, MAX_DOCUMENT_BYTES, RenderLimits, Selection,
    TransactionKind, TypingDirection, build_source_blocks, render_markdown,
    render_markdown_with_limits, validate_source_blocks,
};

const REV: u64 = 7;

/// The only literal `<` in rendered output come from the fixed typed-tag set in
/// security.rs; every byte of untrusted input is HTML-escaped (`<` -> `&lt;`).
/// So the strong sanitizer check is: every raw tag opening must be an allowed
/// tag name. Escaped text like `&lt;img ... onerror=...&gt;` is inert and must
/// NOT be flagged (that was a false positive in an earlier revision).
const ALLOWED_TAGS: &[&str] = &[
    "main",
    "section",
    "div",
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "code",
    "em",
    "strong",
    "del",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tr",
    "th",
    "td",
    "hr",
    "br",
    "sup",
    "a",
    "span",
];

fn assert_body_sanitized(body: &str, label: &str) {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            if j < bytes.len() && bytes[j] == b'/' {
                j += 1;
            }
            let name_start = j;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric()) {
                j += 1;
            }
            if j > name_start {
                let name = body[name_start..j].to_ascii_lowercase();
                assert!(
                    ALLOWED_TAGS.contains(&name.as_str()),
                    "[{label}] disallowed raw tag <{name}> survived sanitizer:\n{}",
                    &body[i..body.len().min(i + 200)]
                );
            }
        }
        i += 1;
    }
    // Any data-rutile-url must be a safe scheme (SafeLinkTarget invariant).
    for (idx, _) in body.match_indices("data-rutile-url=\"") {
        let start = idx + "data-rutile-url=\"".len();
        let val = &body[start..];
        let end = val.find('"').unwrap_or(val.len());
        let url = &val[..end];
        let ok =
            url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:");
        assert!(ok, "[{label}] unsafe link target survived: {url:?}");
    }
}

fn render_ok(src: &str, label: &str) {
    match render_markdown(src, REV) {
        Ok(page) => {
            assert_body_sanitized(&page.body, label);
            // Rendered blocks must satisfy the published contract.
            validate_source_blocks(src, REV, &page.blocks)
                .unwrap_or_else(|e| panic!("[{label}] blocks violate contract: {e}"));
        }
        Err(e) => {
            // Bounded rejection is acceptable; a panic is not (would abort here).
            eprintln!("[{label}] rejected (acceptable): {e}");
        }
    }
}

// ---------- Lane 1: hostile render inputs ----------

#[test]
fn deeply_nested_blockquotes() {
    for depth in [64usize, 256, 1024, 4096, 16384] {
        let src = "> ".repeat(depth) + "x";
        render_ok(&src, &format!("nested-blockquote-{depth}"));
    }
}

#[test]
fn deeply_nested_lists() {
    for depth in [64usize, 256, 1024, 4096] {
        let mut src = String::new();
        for i in 0..depth {
            src.push_str(&"  ".repeat(i));
            src.push_str("- x\n");
        }
        render_ok(&src, &format!("nested-list-{depth}"));
    }
}

#[test]
fn deeply_nested_emphasis() {
    for depth in [64usize, 512, 4096] {
        let src = "*".repeat(depth) + "x" + &"*".repeat(depth);
        render_ok(&src, &format!("nested-emphasis-{depth}"));
    }
}

#[test]
fn pathological_link_reference_definitions() {
    // Many undefined reference uses + cyclic-ish definitions.
    let mut src = String::new();
    for i in 0..5000 {
        src.push_str(&format!("[a{i}]: /x{i}\n"));
    }
    for i in 0..5000 {
        src.push_str(&format!("[link][a{i}] "));
    }
    render_ok(&src, "link-ref-defs");
    // The specific fuzz-crash seeds, re-exercised.
    render_ok("-\t[`]:I\r\t\t", "fuzz-seed-1");
    render_ok("[ =5(]:$#\n\t", "fuzz-seed-2");
}

#[test]
fn giant_table() {
    let cols = 200;
    let mut src = String::new();
    src.push('|');
    for c in 0..cols {
        src.push_str(&format!(" h{c} |"));
    }
    src.push('\n');
    src.push('|');
    for _ in 0..cols {
        src.push_str(" --- |");
    }
    src.push('\n');
    for r in 0..500 {
        src.push('|');
        for c in 0..cols {
            src.push_str(&format!(" r{r}c{c} |"));
        }
        src.push('\n');
    }
    render_ok(&src, "giant-table");
}

#[test]
fn footnote_cycles() {
    let mut src = String::from("text[^a][^b][^c]\n\n");
    src.push_str("[^a]: refers [^b]\n");
    src.push_str("[^b]: refers [^c]\n");
    src.push_str("[^c]: refers [^a]\n");
    render_ok(&src, "footnote-cycle");
}

#[test]
fn mixed_crlf_tabs() {
    let src = "a\r\n\tb\r\n\t\tc\r\n> \tq\r\n\r\n```\r\n\tcode\r\n```\r\n";
    render_ok(src, "crlf-tabs");
    let src2 = "\r\r\r\n\n\r\t\t\r\n";
    render_ok(src2, "crlf-only");
}

#[test]
fn unicode_bidi_zalgo_zwj() {
    // Bidi overrides, zalgo combining marks, ZWJ floods, emoji ZWJ sequences.
    let bidi = "\u{202E}reversed\u{202D} \u{2066}iso\u{2069}";
    render_ok(bidi, "bidi");
    let zalgo: String = "e"
        .chars()
        .chain(std::iter::repeat_n('\u{0301}', 4000))
        .collect();
    render_ok(&zalgo, "zalgo");
    let zwj: String = std::iter::repeat_n("\u{200D}", 8000).collect();
    render_ok(&zwj, "zwj-flood");
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}".repeat(2000);
    render_ok(&family, "emoji-zwj");
}

#[test]
fn html_injection_attempts() {
    let vectors = [
        "<script>alert(1)</script>",
        "<img src=x onerror=alert(1)>",
        "<a href=\"javascript:alert(1)\">x</a>",
        "[click](javascript:alert(1))",
        "[click](data:text/html,<script>alert(1)</script>)",
        "[click](vbscript:msgbox(1))",
        "<svg/onload=alert(1)>",
        "<iframe src=\"javascript:alert(1)\"></iframe>",
        "![x](javascript:alert(1))",
        "<a href=\"HtTpS://ok\" onclick=\"x\">y</a>",
        "<style>*{}</style>",
        "<!-- comment --><script>x</script>",
        "&#106;avascript:alert(1)",
        "[a](java\tscript:alert(1))",
        "[a](  javascript:alert(1)  )",
        "[a](JAVASCRIPT:alert(1))",
        "[a](%6Aavascript:alert(1))",
    ];
    for (i, v) in vectors.iter().enumerate() {
        let page = render_markdown(v, REV)
            .unwrap_or_else(|e| panic!("injection {i} rejected/panicked: {e}"));
        assert_body_sanitized(&page.body, &format!("injection-{i}"));
    }
}

#[test]
fn one_giant_code_block() {
    let inner = "a".repeat(2 * 1024 * 1024);
    let src = format!("```\n{inner}\n```\n");
    render_ok(&src, "giant-code-block");
    // Code block full of would-be HTML.
    let hostile = format!(
        "```\n{}\n```\n",
        "<script>alert(1)</script>\n".repeat(10000)
    );
    render_ok(&hostile, "code-block-html");
}

#[test]
fn document_cap_boundary_renders() {
    // Just under, at, and over the 20 MiB doc cap (render has its own caps).
    for (size, label) in [
        (MAX_DOCUMENT_BYTES - 100, "19.9MiB"),
        (MAX_DOCUMENT_BYTES, "20MiB"),
        (MAX_DOCUMENT_BYTES + 1, "20MiB+1"),
    ] {
        let src = "a".repeat(size);
        render_ok(&src, label);
    }
}

#[test]
fn tiny_render_limits_reject_gracefully() {
    let src = "# ".to_string() + &"word ".repeat(1000);
    // Force the page cap tiny; must reject, not panic.
    let limits = RenderLimits {
        max_body_bytes: 10,
        max_page_bytes: 10,
    };
    let _ = render_markdown_with_limits(&src, REV, limits);
}

#[test]
fn source_blocks_never_violate_own_contract() {
    let quotes = "> ".repeat(2000);
    let zalgo = "\u{0301}".repeat(5000);
    let inputs = [
        "-\t[`]:I\r\t\t",
        "[ =5(]:$#\n\t",
        quotes.as_str(),
        "```\n```\n",
        zalgo.as_str(),
        "| a | b |\n| - | - |\n| 1 | 2 |\n",
    ];
    for (i, src) in inputs.iter().enumerate() {
        match build_source_blocks(src, REV) {
            Ok(blocks) => validate_source_blocks(src, REV, &blocks)
                .unwrap_or_else(|e| panic!("blocks-{i} violate contract: {e}")),
            Err(e) => eprintln!("blocks-{i} rejected (acceptable): {e}"),
        }
    }
}

// ---------- Lane 2: editor / document contracts ----------

fn tx(
    base: u64,
    id: u64,
    range: std::ops::Range<usize>,
    repl: &str,
    kind: TransactionKind,
) -> EditTransaction {
    EditTransaction {
        base_revision: base,
        id,
        kind,
        edits: vec![Edit {
            byte_range: range,
            replacement: repl.into(),
        }],
    }
}

#[test]
fn undo_redo_storm() {
    let mut doc = Document::new("").unwrap();
    for i in 0..10_000u64 {
        let rev = doc.revision();
        let len = doc.len_bytes();
        doc.apply(tx(rev, i, len..len, "x", TransactionKind::Programmatic))
            .expect("insert");
    }
    assert_eq!(doc.len_bytes(), 10_000);
    let mut undos = 0;
    while doc.undo().is_some() {
        undos += 1;
    }
    let mut redos = 0;
    while doc.redo().is_some() {
        redos += 1;
    }
    // Undo budget may evict old entries, so undos <= edits; redos == undos.
    assert_eq!(undos, redos, "redo count must match undo count");
    assert!(undos <= 10_000);
}

#[test]
fn interleaved_undo_redo_edit() {
    let mut doc = Document::new("seed").unwrap();
    let mut id = 0u64;
    for round in 0..2000 {
        let rev = doc.revision();
        let len = doc.len_bytes();
        id += 1;
        doc.apply(tx(rev, id, len..len, "z", TransactionKind::Programmatic))
            .unwrap();
        if round % 3 == 0 {
            doc.undo();
        }
        if round % 7 == 0 {
            doc.redo();
        }
    }
    // Snapshot must always be internally consistent (valid UTF-8, revision monotonic).
    let snap = doc.snapshot();
    let _ = snap.to_string();
    assert_eq!(snap.revision, doc.revision());
}

#[test]
fn edit_cap_edge() {
    let mut doc = Document::new(&"a".repeat(MAX_DOCUMENT_BYTES - 1)).unwrap();
    let rev = doc.revision();
    let len = doc.len_bytes();
    // Grow to exactly the cap: OK.
    doc.apply(tx(rev, 1, len..len, "b", TransactionKind::Paste))
        .expect("fill to cap");
    assert_eq!(doc.len_bytes(), MAX_DOCUMENT_BYTES);
    // One more byte: must be rejected as TooLarge, no mutation.
    let rev = doc.revision();
    let len = doc.len_bytes();
    let before = doc.len_bytes();
    let err = doc.apply(tx(rev, 2, len..len, "c", TransactionKind::Paste));
    assert!(err.is_err(), "over-cap insert must be rejected");
    assert_eq!(doc.len_bytes(), before, "rejected edit must not mutate");
}

#[test]
fn new_document_over_cap_rejected() {
    assert!(Document::new(&"a".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
    assert!(Document::new(&"a".repeat(MAX_DOCUMENT_BYTES)).is_ok());
}

#[test]
fn empty_document_operations() {
    let mut doc = Document::new("").unwrap();
    assert!(doc.undo().is_none());
    assert!(doc.redo().is_none());
    assert_eq!(doc.len_bytes(), 0);
    // Empty transaction rejected.
    let rev = doc.revision();
    let empty = EditTransaction {
        base_revision: rev,
        id: 1,
        kind: TransactionKind::Programmatic,
        edits: vec![],
    };
    assert!(doc.apply(empty).is_err());
    // Render an empty document.
    render_ok("", "empty-doc");
}

#[test]
fn typing_coalescing_storm() {
    let mut doc = Document::new("").unwrap();
    for i in 0..5000u64 {
        let rev = doc.revision();
        let len = doc.len_bytes();
        let ctx = HistoryContext::typing(
            i * 10,
            TypingDirection::Forward,
            Selection::collapsed(len),
            Selection::collapsed(len + 1),
        );
        doc.apply_with_history(tx(rev, i, len..len, "a", TransactionKind::Typing), ctx)
            .unwrap();
    }
    assert_eq!(doc.len_bytes(), 5000);
    // Coalesced typing should undo as grouped chunks; must terminate and restore.
    let mut count = 0;
    while doc.undo().is_some() {
        count += 1;
        assert!(count < 5001, "undo did not terminate");
    }
    assert_eq!(doc.len_bytes(), 0);
}
