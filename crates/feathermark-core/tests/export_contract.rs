use feathermark_core::{
    ExportError, ExportPage, ExportRequest, ExportViolation, MAX_EXPORT_PAGE_BYTES,
    MAX_EXPORT_TITLE_BYTES, MAX_RENDERED_PAGE_BYTES, RenderError, render_export_page,
};

const CLEAN_PAGE: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Note</title>\n<style>:root{color-scheme:light dark}@media print{a{color:#000}}</style>\n</head>\n<body><h1>Note</h1><p>Hello <a href=\"https://example.com\">there</a>.</p></body>\n</html>\n";

#[test]
fn export_request_accepts_bounded_titles() {
    let request = ExportRequest::new(3, Some("My Note".into())).unwrap();
    assert_eq!(request.revision(), 3);
    assert_eq!(request.title(), Some("My Note"));

    let untitled = ExportRequest::new(0, None).unwrap();
    assert_eq!(untitled.title(), None);
}

#[test]
fn export_request_rejects_oversized_titles() {
    let title = "x".repeat(MAX_EXPORT_TITLE_BYTES + 1);
    assert!(matches!(
        ExportRequest::new(0, Some(title)),
        Err(ExportError::TitleTooLarge {
            len,
            max: MAX_EXPORT_TITLE_BYTES,
        }) if len == MAX_EXPORT_TITLE_BYTES + 1
    ));
}

#[test]
fn export_page_accepts_a_clean_self_contained_page() {
    let page = ExportPage::from_html(CLEAN_PAGE.into()).unwrap();
    assert_eq!(page.as_html(), CLEAN_PAGE);
    assert_eq!(page.into_html(), CLEAN_PAGE);
}

#[test]
fn export_page_rejects_scripts() {
    // `ExportViolation` is `#[non_exhaustive]` (it grows in Wave 1/3), so tests
    // match variants rather than constructing them.
    let html = CLEAN_PAGE.replace("<h1>", "<script>alert(1)</script><h1>");
    assert!(matches!(
        ExportPage::from_html(html),
        Err(ExportViolation::Script)
    ));
    let sneaky = CLEAN_PAGE.replace("<h1>", "<SCRIPT src=x><h1>");
    assert!(matches!(
        ExportPage::from_html(sneaky),
        Err(ExportViolation::Script)
    ));
}

#[test]
fn export_page_rejects_external_stylesheets_and_links() {
    let html = CLEAN_PAGE.replace(
        "<style>",
        "<link rel=\"stylesheet\" href=\"https://cdn.example/x.css\"><style>",
    );
    assert!(matches!(
        ExportPage::from_html(html),
        Err(ExportViolation::LinkElement)
    ));
}

#[test]
fn export_page_rejects_frames_and_embedded_objects() {
    for tag in [
        "<iframe src=\"x\">",
        "<object data=\"x\">",
        "<embed src=\"x\">",
    ] {
        let html = CLEAN_PAGE.replace("<h1>", &format!("{tag}<h1>"));
        assert!(
            matches!(
                ExportPage::from_html(html),
                Err(ExportViolation::FrameOrObject)
            ),
            "tag {tag} must be rejected"
        );
    }
}

#[test]
fn export_page_rejects_external_url_references() {
    for fragment in [
        "<img src=\"https://example.com/x.png\">",
        "<img src=\"//example.com/x.png\">",
        "<img srcset=\"https://example.com/x.png 1x\">",
    ] {
        let html = CLEAN_PAGE.replace("<h1>", &format!("{fragment}<h1>"));
        assert!(
            matches!(
                ExportPage::from_html(html),
                Err(ExportViolation::ExternalReference)
            ),
            "fragment {fragment} must be rejected"
        );
    }
    for css in [
        "@import url(x.css);",
        "background:url(https://x/y.png);",
        "background:url(//x/y.png);",
    ] {
        let html = CLEAN_PAGE.replace(":root{color-scheme:light dark}", css);
        assert!(
            matches!(
                ExportPage::from_html(html),
                Err(ExportViolation::ExternalReference)
            ),
            "css {css} must be rejected"
        );
    }
}

#[test]
fn export_page_rejects_javascript_urls() {
    let html = CLEAN_PAGE.replace("https://example.com", "javascript:alert(1)");
    assert!(matches!(
        ExportPage::from_html(html),
        Err(ExportViolation::JavascriptUrl)
    ));
}

#[test]
fn export_page_allows_plain_hyperlinks_and_data_free_text() {
    // Hyperlinks trigger no request on open; they are explicitly allowed in a
    // recipient-grade export.
    assert!(CLEAN_PAGE.contains("href=\"https://example.com\""));
    assert!(ExportPage::from_html(CLEAN_PAGE.into()).is_ok());
}

#[test]
fn export_page_rejects_oversized_pages() {
    // Construct an oversized page without allocating 96 MiB of pattern
    // replacements: pad with a comment.
    let mut html = String::with_capacity(MAX_EXPORT_PAGE_BYTES + 128);
    html.push_str("<!doctype html>\n<html><head><title>x</title></head><body><!--");
    html.push_str(&"y".repeat(MAX_EXPORT_PAGE_BYTES));
    html.push_str("--></body></html>");
    assert!(matches!(
        ExportPage::from_html(html),
        Err(ExportViolation::TooLarge {
            max: MAX_EXPORT_PAGE_BYTES,
        })
    ));
}

#[test]
fn export_error_wraps_render_errors_and_violations() {
    let from_render: ExportError = RenderError::PreviewTooLarge.into();
    assert!(matches!(
        from_render,
        ExportError::Render(RenderError::PreviewTooLarge)
    ));
    // Route through a library-produced violation: `ExportViolation` is
    // `#[non_exhaustive]`, so the test crate can't construct a variant directly.
    let violation = ExportPage::from_html(CLEAN_PAGE.replace("<h1>", "<script></script><h1>"))
        .expect_err("script must be rejected");
    let from_violation: ExportError = violation.into();
    assert!(matches!(
        from_violation,
        ExportError::Violation(ExportViolation::Script)
    ));
}

// The export page cap never exceeds the preview page cap; enforced at compile
// time so the export template can reuse the render pipeline's bounds.
const _: () = assert!(MAX_EXPORT_PAGE_BYTES <= MAX_RENDERED_PAGE_BYTES);

const REPRESENTATIVE_DOC: &str = "# Field Notes\n\nA paragraph with **bold**, *italic*, \
`inline code`, and a [link](https://example.com/path) plus a mail [contact](mailto:a@b.co).\n\n\
> A blockquote spine.\n\n---\n\n## Table\n\n| Mineral | Habit |\n| --- | --- |\n| Rutile | Acicular |\n\n\
- one\n- two\n\n1. first\n2. second\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";

fn export(source: &str, title: Option<&str>) -> ExportPage {
    let request = ExportRequest::new(7, title.map(str::to_owned)).unwrap();
    render_export_page(source, &request).expect("representative document must export")
}

#[test]
fn render_export_page_produces_a_self_contained_themed_document() {
    let page = export(REPRESENTATIVE_DOC, Some("Field Notes"));
    let html = page.as_html();

    // Structural self-containment.
    assert!(
        html.starts_with("<!doctype html>"),
        "must open with a doctype"
    );
    assert!(
        html.contains("<html lang=\"en\">"),
        "must set a document language"
    );
    assert!(html.contains("<meta charset=\"utf-8\">"));
    assert!(html.contains("name=\"viewport\""));
    assert!(html.contains("<title>Field Notes</title>"));

    // Inlined stylesheet, no external or executable surface.
    assert!(html.contains("<style>"), "styles must be inlined");
    assert!(!html.contains("<script"), "export must carry no script");
    assert!(
        !html.contains("<link"),
        "export must carry no external stylesheet link"
    );
    assert!(!html.contains("://cdn"), "no external hosts");
    // The only permitted "http://" substring is the inert XML namespace URI
    // inside the geniculated H1 underline's `data:` mask image — a static
    // identifier every SVG root carries, never dereferenced over the network.
    // Any other occurrence would be a real external reference.
    let external_http_occurrences = html.matches("http://").count();
    let inert_xmlns_occurrences = html.matches("xmlns='http://www.w3.org/2000/svg'").count();
    assert_eq!(
        external_http_occurrences, inert_xmlns_occurrences,
        "every http:// substring must be the inert SVG xmlns, not a live external reference"
    );
    // Every CSS `url(...)` must be a `data:` reference (self-containment's own
    // allowlist re-verified from the test side, not just trusted).
    for (offset, _) in html.match_indices("url(") {
        let after = &html[offset + 4..];
        let end = after.find(')').expect("url(...) must be closed");
        let argument = after[..end].trim().trim_matches(['"', '\'']);
        assert!(
            argument.starts_with("data:"),
            "url() argument must be a data: URI, got {argument}"
        );
    }
    assert!(!html.contains("@import"), "no CSS @import fetches");

    // Theme signatures: light + dark, forced-colors, print, gold underline,
    // sixling hr.
    assert!(
        html.contains("prefers-color-scheme:dark"),
        "must theme dark mode"
    );
    assert!(
        html.contains("forced-colors:active"),
        "must carry a forced-colors (Windows High Contrast) pass"
    );
    assert!(
        html.contains("@media print"),
        "must carry a print stylesheet"
    );
    assert!(html.contains("h1{"), "must style the document title");
    // The geniculated (elbow-bent) signature: a plain gold hairline as the
    // universal baseline, enhanced to a bent line via a self-contained `data:`
    // SVG mask wherever CSS masking is supported.
    assert!(
        html.contains("border-block-end:var(--rule-needle) solid var(--accent)"),
        "must carry the plain-hairline fallback under the title"
    );
    assert!(
        html.contains("@supports (mask-image:none) or (-webkit-mask-image:none)"),
        "must gate the geniculated mask behind a masking feature query"
    );
    assert!(
        html.contains("h1::after") && html.contains("mask-image:url(\"data:image/svg+xml,"),
        "must draw the geniculated bend via a self-contained data: SVG mask"
    );
    assert!(
        html.contains("hr::before"),
        "must style the sixling divider"
    );
    assert!(
        html.contains("--measure:68ch"),
        "must cap the measure at 68ch"
    );
    assert!(
        html.contains("clamp("),
        "type scale must be fluid (web-typography)"
    );
    assert!(
        html.contains("padding-inline-start")
            && html.contains("border-inline-start")
            && html.contains("text-align:start"),
        "directional CSS must use logical properties (i18n-ready)"
    );
    assert!(
        html.contains("table{display:block;overflow-x:auto"),
        "wide tables must scroll in their own container, not the page"
    );
    assert!(
        html.contains("overflow-wrap:break-word"),
        "long unbroken tokens must wrap instead of forcing page scroll"
    );
    assert!(
        html.contains("prefers-reduced-motion:reduce"),
        "hover edge-break transitions must respect reduced motion"
    );

    // Plain links are preserved as real, click-to-open hyperlinks.
    assert!(
        html.contains("href=\"https://example.com/path\""),
        "http link preserved"
    );
    assert!(
        html.contains("href=\"mailto:a@b.co\""),
        "mailto link preserved"
    );
    assert!(
        !html.contains("data-feathermark-url"),
        "preview bridge marker rewritten"
    );
}

#[test]
fn render_export_page_uses_a_default_title_when_absent() {
    let page = export("plain body", None);
    assert!(
        page.as_html()
            .contains("<title>FeatherMark document</title>")
    );
}

#[test]
fn render_export_page_escapes_the_title() {
    let page = export("body", Some("<b>&\"risky\""));
    let html = page.as_html();
    assert!(html.contains("<title>&lt;b&gt;&amp;&quot;risky&quot;</title>"));
    assert!(
        !html.contains("<title><b>"),
        "raw markup must not reach the title"
    );
}

#[test]
fn render_export_page_survives_hostile_but_renderable_documents() {
    // Every hostile token here renders to escaped, inert text — the export must
    // neither inject it nor be falsely rejected by the inspector.
    let hostile = "# <script>alert(1)</script>\n\n\
        Text with onerror=alert(1) and onclick=steal() written literally.\n\n\
        A CSS-looking string: background:url(http://evil.example/x.png) and @import trickery.\n\n\
        <img src=\"x.png\" onerror=\"alert(1)\"> as raw markup.\n\n\
        A javascript link: [click](javascript:alert(1)) and a data page \
        [doc](data:text/html,<h1>x</h1>).\n\n\
        A protocol-relative reference //evil.example/a and a relative one ./b.png.\n";
    // `export` unwraps the Result, so reaching this point already proves the
    // inspector neither rejected the document nor let anything executable through.
    let page = export(hostile, Some("Hostile"));
    let html = page.as_html();
    assert!(!html.contains("<script"), "no live script element");
    assert!(
        !html.contains("<img"),
        "raw img markup must be escaped, not live"
    );
    assert!(
        !html.contains("href=\"javascript:"),
        "unsafe link scheme dropped by the renderer"
    );
    assert!(
        !html.contains("data:text/html"),
        "data:text/html link dropped by the renderer"
    );
    // The hostile tokens survive only as inert, entity-escaped text.
    assert!(
        html.contains("&lt;script&gt;"),
        "hostile markup survives as escaped text"
    );
    assert!(
        html.contains("onerror=alert(1)"),
        "escaped handler text is preserved verbatim"
    );
}

#[test]
fn render_export_page_output_round_trips_through_from_html() {
    let page = export(REPRESENTATIVE_DOC, Some("Round Trip"));
    let reparsed = ExportPage::from_html(page.as_html().to_owned())
        .expect("a rendered export page must pass from_html unchanged");
    assert_eq!(reparsed, page);
}

#[test]
fn export_page_rejects_inline_event_handlers() {
    let html = CLEAN_PAGE.replace("<h1>", "<h1 onclick=\"steal()\">");
    assert!(matches!(
        ExportPage::from_html(html),
        Err(ExportViolation::EventHandler { attr }) if attr == "onclick"
    ));
    let body = CLEAN_PAGE.replace("<body>", "<body onload=\"go()\">");
    assert!(matches!(
        ExportPage::from_html(body),
        Err(ExportViolation::EventHandler { attr }) if attr == "onload"
    ));
}

#[test]
fn export_page_rejects_relative_references() {
    for fragment in ["<img src=\"x.png\">", "<img src=\"/root/x.png\">"] {
        let html = CLEAN_PAGE.replace("<h1>", &format!("{fragment}<h1>"));
        assert!(
            matches!(
                ExportPage::from_html(html),
                Err(ExportViolation::RelativeReference { .. })
            ),
            "fragment {fragment} must be a relative reference"
        );
    }
    let css = CLEAN_PAGE.replace(
        ":root{color-scheme:light dark}",
        "body{background:url(local.png)}",
    );
    assert!(matches!(
        ExportPage::from_html(css),
        Err(ExportViolation::RelativeReference { .. })
    ));
}

#[test]
fn export_page_rejects_data_text_html_and_css_expression() {
    let data_html = CLEAN_PAGE.replace("https://example.com", "data:text/html,<h1>x</h1>");
    assert!(matches!(
        ExportPage::from_html(data_html),
        Err(ExportViolation::DataHtmlUrl)
    ));
    let expression = CLEAN_PAGE.replace(
        ":root{color-scheme:light dark}",
        "body{width:expression(alert(1))}",
    );
    assert!(matches!(
        ExportPage::from_html(expression),
        Err(ExportViolation::CssExpression)
    ));
}

#[test]
fn export_page_allows_escaped_hostile_text_outside_tags() {
    // A page whose *text* mentions handlers, url(http…), and <script> — all
    // entity-escaped — is inert and must be accepted.
    let page = CLEAN_PAGE.replace(
        "Hello ",
        "Hello &lt;script&gt; onerror=x url(http://evil) &lt;img src=y&gt; ",
    );
    assert!(
        ExportPage::from_html(page).is_ok(),
        "escaped text must not trip the tag-aware inspector"
    );
}
