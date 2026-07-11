use feathermark_core::{
    ExportError, ExportPage, ExportRequest, ExportViolation, MAX_EXPORT_PAGE_BYTES,
    MAX_EXPORT_TITLE_BYTES, MAX_RENDERED_PAGE_BYTES, RenderError,
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
    let html = CLEAN_PAGE.replace("<h1>", "<script>alert(1)</script><h1>");
    assert_eq!(ExportPage::from_html(html), Err(ExportViolation::Script));
    let sneaky = CLEAN_PAGE.replace("<h1>", "<SCRIPT src=x><h1>");
    assert_eq!(ExportPage::from_html(sneaky), Err(ExportViolation::Script));
}

#[test]
fn export_page_rejects_external_stylesheets_and_links() {
    let html = CLEAN_PAGE.replace(
        "<style>",
        "<link rel=\"stylesheet\" href=\"https://cdn.example/x.css\"><style>",
    );
    assert_eq!(
        ExportPage::from_html(html),
        Err(ExportViolation::LinkElement)
    );
}

#[test]
fn export_page_rejects_frames_and_embedded_objects() {
    for tag in [
        "<iframe src=\"x\">",
        "<object data=\"x\">",
        "<embed src=\"x\">",
    ] {
        let html = CLEAN_PAGE.replace("<h1>", &format!("{tag}<h1>"));
        assert_eq!(
            ExportPage::from_html(html),
            Err(ExportViolation::FrameOrObject),
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
        assert_eq!(
            ExportPage::from_html(html),
            Err(ExportViolation::ExternalReference),
            "fragment {fragment} must be rejected"
        );
    }
    for css in [
        "@import url(x.css);",
        "background:url(https://x/y.png);",
        "background:url(//x/y.png);",
    ] {
        let html = CLEAN_PAGE.replace(":root{color-scheme:light dark}", css);
        assert_eq!(
            ExportPage::from_html(html),
            Err(ExportViolation::ExternalReference),
            "css {css} must be rejected"
        );
    }
}

#[test]
fn export_page_rejects_javascript_urls() {
    let html = CLEAN_PAGE.replace("https://example.com", "javascript:alert(1)");
    assert_eq!(
        ExportPage::from_html(html),
        Err(ExportViolation::JavascriptUrl)
    );
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
    assert_eq!(
        ExportPage::from_html(html),
        Err(ExportViolation::TooLarge {
            max: MAX_EXPORT_PAGE_BYTES,
        })
    );
}

#[test]
fn export_error_wraps_render_errors_and_violations() {
    let from_render: ExportError = RenderError::PreviewTooLarge.into();
    assert!(matches!(
        from_render,
        ExportError::Render(RenderError::PreviewTooLarge)
    ));
    let from_violation: ExportError = ExportViolation::Script.into();
    assert!(matches!(
        from_violation,
        ExportError::Violation(ExportViolation::Script)
    ));
}

// The export page cap never exceeds the preview page cap; enforced at compile
// time so the export template can reuse the render pipeline's bounds.
const _: () = assert!(MAX_EXPORT_PAGE_BYTES <= MAX_RENDERED_PAGE_BYTES);
