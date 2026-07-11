//! Golden-corpus and hostile-input tests for the smart-paste converter.

use feathermark_core::{
    HtmlToMarkdownError, MAX_HTML_INPUT_BYTES, MAX_NESTING_DEPTH, html_to_markdown, render_markdown,
};

fn convert(html: &str) -> String {
    html_to_markdown(html).expect("conversion should succeed")
}

/// Every conversion must produce Markdown that the real renderer accepts and
/// that yields no executable HTML.
fn assert_safe_roundtrip(markdown: &str) {
    let lower = markdown.to_ascii_lowercase();
    assert!(!lower.contains("<script"), "markdown leaked a script tag");
    assert!(
        !lower.contains("javascript:"),
        "markdown leaked a javascript: url"
    );
    let rendered = render_markdown(markdown, 1).expect("rendered markdown");
    let body = rendered.body.to_ascii_lowercase();
    assert!(!body.contains("<script"), "rendered body leaked a script");
    assert!(!body.contains("<iframe"), "rendered body leaked an iframe");
    assert!(
        !body.contains(" onerror") && !body.contains(" onload") && !body.contains(" onclick"),
        "rendered body leaked an event handler"
    );
}

// ---------------------------------------------------------------------------
// Golden corpus — realistic clipboard payloads
// ---------------------------------------------------------------------------

#[test]
fn headings_and_paragraphs() {
    let html = "<h1>Title</h1><h2>Subtitle</h2><p>A paragraph of text.</p>";
    let markdown = convert(html);
    assert_eq!(markdown, "# Title\n\n## Subtitle\n\nA paragraph of text.");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn inline_emphasis_and_code() {
    let html = "<p>Some <strong>bold</strong> and <em>italic</em> and <code>code()</code>.</p>";
    let markdown = convert(html);
    assert_eq!(markdown, "Some **bold** and *italic* and `code()`.");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn b_and_i_alias_to_strong_and_em() {
    let html = "<p><b>bold</b> then <i>italic</i></p>";
    assert_eq!(convert(html), "**bold** then *italic*");
}

#[test]
fn safari_style_span_soup_paragraph() {
    // Safari wraps runs in styled spans with Apple-specific styles.
    let html = "<p style=\"margin:0\"><span style=\"font-weight:600\">Hello</span> \
        <span style=\"font-style:italic\">world</span></p>";
    let markdown = convert(html);
    assert_eq!(markdown, "Hello world");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn chrome_bold_via_span_is_unwrapped() {
    // Chrome copies bold as a span with inline style, not <strong>. Styling is
    // not part of the allowlist, so the span is unwrapped to its text.
    let html = "<span style=\"font-weight:700\">Bold text</span>";
    assert_eq!(convert(html), "Bold text");
}

#[test]
fn ms_word_mso_junk_is_stripped() {
    let html = "<!--StartFragment--><o:p></o:p>\
        <p class=MsoNormal style='margin:0in'>\
        <span style='font-family:\"Calibri\",sans-serif'>Report body</span></p>\
        <!--EndFragment-->";
    let markdown = convert(html);
    assert_eq!(markdown, "Report body");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn google_docs_span_soup_with_links() {
    let html = "<p dir=\"ltr\"><span>See the </span>\
        <a href=\"https://example.com/docs\"><span>documentation</span></a>\
        <span> for details.</span></p>";
    let markdown = convert(html);
    assert_eq!(
        markdown,
        "See the [documentation](https://example.com/docs) for details."
    );
    assert_safe_roundtrip(&markdown);
}

#[test]
fn unordered_list() {
    let html = "<ul><li>First</li><li>Second</li><li>Third</li></ul>";
    let markdown = convert(html);
    assert_eq!(markdown, "- First\n- Second\n- Third");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn ordered_list_renumbers() {
    let html = "<ol><li>alpha</li><li>beta</li></ol>";
    let markdown = convert(html);
    assert_eq!(markdown, "1. alpha\n2. beta");
}

#[test]
fn nested_list_indentation() {
    let html = "<ul><li>Parent<ul><li>Child</li></ul></li></ul>";
    let markdown = convert(html);
    assert_eq!(markdown, "- Parent\n  - Child");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn blockquote() {
    let html = "<blockquote><p>Quoted line one</p><p>Quoted line two</p></blockquote>";
    let markdown = convert(html);
    assert_eq!(markdown, "> Quoted line one\n>\n> Quoted line two");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn preformatted_code_block() {
    let html = "<pre>let x = 1;\nlet y = 2;</pre>";
    let markdown = convert(html);
    assert_eq!(markdown, "```\nlet x = 1;\nlet y = 2;\n```");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn thematic_break() {
    let html = "<p>Above</p><hr><p>Below</p>";
    assert_eq!(convert(html), "Above\n\n---\n\nBelow");
}

#[test]
fn hard_break_inside_paragraph() {
    let html = "<p>Line one<br>Line two</p>";
    assert_eq!(convert(html), "Line one  \nLine two");
}

#[test]
fn entities_are_decoded() {
    let html =
        "<p>Fish &amp; chips &lt; tea &gt; coffee &quot;quoted&quot; &#39;apos&#39; &nbsp;end</p>";
    let markdown = convert(html);
    assert_eq!(
        markdown,
        "Fish &amp; chips &lt; tea &gt; coffee \"quoted\" 'apos' end"
    );
    // The entity-encoded text renders back to the original literal characters.
    let rendered = render_markdown(&markdown, 1).unwrap();
    assert!(
        rendered
            .body
            .contains("Fish &amp; chips &lt; tea &gt; coffee")
    );
    assert_safe_roundtrip(&markdown);
}

#[test]
fn whitespace_is_collapsed() {
    let html = "<p>lots\n\n   of    \t whitespace</p>";
    assert_eq!(convert(html), "lots of whitespace");
}

#[test]
fn link_with_non_http_scheme_is_dropped() {
    let html = "<p><a href=\"ftp://files.example.com/x\">download</a></p>";
    // ftp is not allowlisted; the link is dropped, the text remains.
    assert_eq!(convert(html), "download");
}

#[test]
fn mailto_link_is_kept() {
    let html = "<p>Email <a href=\"mailto:hi@example.com\">us</a></p>";
    assert_eq!(convert(html), "Email [us](mailto:hi@example.com)");
}

#[test]
fn javascript_link_is_dropped() {
    let html = "<a href=\"javascript:alert(1)\">click</a>";
    let markdown = convert(html);
    assert_eq!(markdown, "click");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn empty_input_is_empty() {
    assert_eq!(convert(""), "");
    assert_eq!(convert("   \n\t "), "");
}

#[test]
fn plain_text_passthrough() {
    assert_eq!(convert("just some text"), "just some text");
}

#[test]
fn markdown_special_chars_in_prose_are_escaped() {
    let html = "<p>Use *asterisks* and _underscores_ and [brackets] literally.</p>";
    let markdown = convert(html);
    assert_eq!(
        markdown,
        "Use \\*asterisks\\* and \\_underscores\\_ and \\[brackets\\] literally."
    );
    // Round-trip: escaped text must render as literal, not emphasis.
    let rendered = render_markdown(&markdown, 1).unwrap();
    assert!(rendered.body.contains("*asterisks*"));
}

#[test]
fn leading_block_marker_in_prose_is_escaped() {
    let html = "<p># not a heading</p>";
    let markdown = convert(html);
    assert_eq!(markdown, "\\# not a heading");
    let rendered = render_markdown(&markdown, 1).unwrap();
    assert!(!rendered.body.contains("<h1"));
}

// ---------------------------------------------------------------------------
// Hostile input
// ---------------------------------------------------------------------------

#[test]
fn script_tag_is_stripped_entirely() {
    let html = "<p>before</p><script>alert('xss')</script><p>after</p>";
    let markdown = convert(html);
    assert_eq!(markdown, "before\n\nafter");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn style_tag_is_stripped_entirely() {
    let html = "<style>body{display:none}</style><p>visible</p>";
    let markdown = convert(html);
    assert_eq!(markdown, "visible");
    assert_safe_roundtrip(&markdown);
}

#[test]
fn event_handler_attributes_never_survive() {
    let html = "<p onclick=\"steal()\">text</p><img src=x onerror=\"alert(1)\">";
    let markdown = convert(html);
    assert_safe_roundtrip(&markdown);
    assert!(!markdown.contains("onclick"));
    assert!(!markdown.contains("onerror"));
}

#[test]
fn entity_encoded_script_does_not_execute() {
    // Decoded, this reads "<script>", but it must stay literal text.
    let html = "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>";
    let markdown = convert(html);
    assert_safe_roundtrip(&markdown);
    assert!(!markdown.to_ascii_lowercase().contains("<script"));
}

#[test]
fn unclosed_tags_do_not_panic() {
    let html = "<p><strong>bold <em>and italic <a href=\"https://x.example\">link";
    let markdown = convert(html);
    assert_safe_roundtrip(&markdown);
    assert!(markdown.contains("bold"));
    assert!(markdown.contains("link"));
}

#[test]
fn stray_close_tags_are_ignored() {
    let html = "</div></p>text</strong>";
    assert_eq!(convert(html), "text");
}

#[test]
fn deeply_nested_input_yields_typed_error_not_panic() {
    let depth = MAX_NESTING_DEPTH + 50;
    let mut html = String::new();
    for _ in 0..depth {
        html.push_str("<div>");
    }
    html.push_str("deep");
    for _ in 0..depth {
        html.push_str("</div>");
    }
    assert_eq!(
        html_to_markdown(&html),
        Err(HtmlToMarkdownError::NestingTooDeep)
    );
}

#[test]
fn deeply_nested_blockquotes_are_bounded() {
    let depth = 100;
    let mut html = String::new();
    for _ in 0..depth {
        html.push_str("<blockquote>");
    }
    html.push_str("<p>bottom</p>");
    for _ in 0..depth {
        html.push_str("</blockquote>");
    }
    let result = html_to_markdown(&html);
    // Either a bounded conversion or a typed cap error — never a panic.
    match result {
        Ok(markdown) => assert_safe_roundtrip(&markdown),
        Err(HtmlToMarkdownError::OutputTooLarge | HtmlToMarkdownError::NestingTooDeep) => {}
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn oversized_input_is_rejected() {
    let html = "a".repeat(MAX_HTML_INPUT_BYTES + 1);
    assert_eq!(
        html_to_markdown(&html),
        Err(HtmlToMarkdownError::InputTooLarge)
    );
}

#[test]
fn input_at_the_cap_is_accepted() {
    let html = "a".repeat(MAX_HTML_INPUT_BYTES);
    assert!(html_to_markdown(&html).is_ok());
}

#[test]
fn multibyte_utf8_is_preserved() {
    let html = "<p>café — naïve — 日本語 — 🦀</p>";
    let markdown = convert(html);
    assert!(markdown.contains("café"));
    assert!(markdown.contains("日本語"));
    assert!(markdown.contains("🦀"));
    assert_safe_roundtrip(&markdown);
}

#[test]
fn malformed_angle_brackets_are_literal_text() {
    let html = "a < b and c > d";
    let markdown = convert(html);
    assert_safe_roundtrip(&markdown);
    assert!(markdown.contains("a"));
    assert!(markdown.contains("b"));
}

#[test]
fn comment_only_input_is_empty() {
    assert_eq!(convert("<!-- just a comment -->"), "");
    assert_eq!(convert("<!-- unterminated comment"), "");
}

#[test]
fn code_span_with_backticks_is_fenced() {
    let html = "<p>Use <code>a`b</code> here</p>";
    let markdown = convert(html);
    assert!(markdown.contains("a`b"));
    let rendered = render_markdown(&markdown, 1).unwrap();
    assert!(rendered.body.contains("a`b"));
}

#[test]
fn code_block_preserves_angle_brackets_safely() {
    let html = "<pre>if (a &lt; b) { return &lt;T&gt;; }</pre>";
    let markdown = convert(html);
    assert_safe_roundtrip(&markdown);
    let rendered = render_markdown(&markdown, 1).unwrap();
    assert!(rendered.body.contains("&lt;T&gt;"));
}

#[test]
fn huge_flat_paragraph_count_is_bounded() {
    let html = "<p>x</p>".repeat(10_000);
    let markdown = html_to_markdown(&html).unwrap();
    assert!(markdown.len() <= feathermark_core::MAX_OUTPUT_BYTES);
    assert_safe_roundtrip(&markdown);
}
