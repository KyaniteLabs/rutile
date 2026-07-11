//! Recipient-grade, self-contained HTML export (SPEC §7, LD-4).
//!
//! [`render_export_page`] reuses the trusted [`render_markdown`] pipeline for the
//! document body (typed nodes → escaped HTML) and wraps it in a single, fully
//! self-contained document: `<!doctype>`, `<html lang>`, a `<head>` carrying the
//! title, charset/viewport meta, a restrictive CSP, and an **inline** stylesheet
//! derived from `design/tokens.css` — no web fonts, no external requests, no
//! JavaScript. The result is validated by [`ExportPage::from_html`], so a returned
//! page *is* the proof that it carries nothing executable and fetches nothing.
//!
//! The theme is the "mineral editorial" core from `DESIGN-SYSTEM.md`: a
//! system-serif document on smoky/oatmeal ground (light + dark via
//! `prefers-color-scheme`), with rutile-gold accents kept at needle weight only —
//! the H1 gold hairline underline (the geniculated signature), the sixling `*`
//! divider, the blockquote spine, list markers, and link underlines. Corners are
//! 2px, the measure caps at 68ch, and a print stylesheet keeps the needles.
//!
//! 0.2 export is text-only: Markdown images render as their alt text (SPEC
//! §7-OQ1 defers data-URI image embedding to 0.3), so a conforming export never
//! carries an image `src` at all.

use crate::export_contract::{ExportError, ExportPage, ExportRequest};
use crate::render::render_markdown;

/// Title used when an [`ExportRequest`] carries none.
const DEFAULT_TITLE: &str = "FeatherMark document";

/// Content-Security-Policy for the exported file: deny by default, permit only
/// the inline stylesheet and (future) inline data-URI images. Belt-and-suspenders
/// over the [`ExportPage`] inspection for recipients whose viewer honors meta CSP.
const EXPORT_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";

/// The renderer emits hyperlinks as inert `data-feathermark-url` carriers for the
/// live preview bridge; the exported file has no bridge, so the marker is rewritten
/// to a real `href`. The attribute order is fixed by the renderer, and the URL is
/// already a canonical, attribute-escaped `SafeLinkTarget` (http/https/mailto only),
/// so the rewrite is a precise literal substitution.
const LINK_MARKER: &str = "<a role=\"link\" tabindex=\"0\" data-feathermark-url=\"";
const LINK_REPLACEMENT: &str = "<a href=\"";

/// Inline "mineral editorial" stylesheet, derived from `design/tokens.css`.
/// Self-contained by construction: no `url()`, no `@import`, no web fonts.
const EXPORT_CSS: &str = r#":root{color-scheme:light dark;--font-body:Charter,'Bitstream Charter','Sitka Text',Cambria,Georgia,serif;--font-mono:ui-monospace,'SF Mono','Cascadia Code',Menlo,Consolas,monospace;--measure:68ch;--radius:2px;--rule-needle:1px;--accent:#c9921e;--bg:#f6f2e9;--surface:#e9e2d6;--ink:#241e19;--muted:#6b6157;--border:#d8cfbe}
@media (prefers-color-scheme:dark){:root{--accent:#f0c24b;--bg:#241e19;--surface:#2e2822;--ink:#e9e2d6;--muted:#a69b8c;--border:#3a332b}}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--ink);font-family:var(--font-body);font-size:1.125rem;line-height:1.65;text-rendering:optimizeLegibility}
main{max-width:var(--measure);margin:0 auto;padding:4rem 1.5rem 6rem}
h1,h2,h3,h4,h5,h6{font-family:var(--font-body);font-weight:700;line-height:1.2;margin:2.2em 0 .6em}
h1{font-size:2rem;margin-top:0;padding-bottom:.24em;border-bottom:var(--rule-needle) solid var(--accent)}
h2{font-size:1.5rem}
h3{font-size:1.25rem}
h4{font-size:1.05rem}
p{margin:0 0 1.2em}
a{color:inherit;text-decoration:underline;text-decoration-color:var(--accent);text-decoration-thickness:var(--rule-needle);text-underline-offset:.18em}
a:hover,a:focus-visible{text-decoration-thickness:2px}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
blockquote{margin:1.4em 0;padding:.2em 0 .2em 1.1em;border-left:var(--rule-needle) solid var(--accent);color:var(--muted)}
code{font-family:var(--font-mono);font-size:.9em;background:var(--surface);padding:.1em .3em;border-radius:var(--radius)}
pre{background:var(--surface);border:var(--rule-needle) solid var(--border);border-radius:var(--radius);padding:1em 1.2em;overflow-x:auto}
pre code{background:none;padding:0;font-size:.95em}
hr{border:0;margin:2.6em 0;text-align:center;overflow:visible;line-height:1}
hr::before{content:"\2736";color:var(--accent);font-size:1.15rem}
ul,ol{padding-left:1.4em}
li{margin:.3em 0}
li::marker{color:var(--accent)}
table{border-collapse:collapse;width:100%;margin:1.4em 0;font-size:.95em}
th,td{border:var(--rule-needle) solid var(--border);padding:.5em .7em;text-align:left}
th{font-weight:700}
.align-left{text-align:left}
.align-center{text-align:center}
.align-right{text-align:right}
.image-alt{color:var(--muted);font-style:italic}
.task-checkbox{margin-right:.4em}
@media (prefers-reduced-motion:reduce){*{transition:none!important;animation:none!important}}
@media print{:root{--bg:#fff;--surface:#fff;--ink:#000;--muted:#333;--border:#999;--accent:#c9921e}body{font-size:12pt}main{max-width:none;padding:0}pre,blockquote,table{break-inside:avoid}}"#;

/// Renders `source` as a validated, self-contained export page titled per
/// `request`.
///
/// # Errors
///
/// Returns [`ExportError::Render`] if the body exceeds the render pipeline's
/// bounds, or [`ExportError::Violation`] if the assembled page fails the
/// self-containment allowlist (which, for trusted output, would indicate a bug in
/// the template rather than hostile input).
pub fn render_export_page(
    source: &str,
    request: &ExportRequest,
) -> Result<ExportPage, ExportError> {
    let rendered = render_markdown(source, request.revision())?;
    let body = rendered.body.replace(LINK_MARKER, LINK_REPLACEMENT);
    let title = request.title().unwrap_or(DEFAULT_TITLE);
    let document = assemble_document(title, &body);
    Ok(ExportPage::from_html(document)?)
}

/// Builds the full HTML document around the rendered `<main>` body.
fn assemble_document(title: &str, body: &str) -> String {
    let mut escaped_title = String::with_capacity(title.len());
    escape_html_text(title, &mut escaped_title);
    format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<meta http-equiv=\"Content-Security-Policy\" content=\"{EXPORT_CSP}\">\n\
<title>{escaped_title}</title>\n\
<style>{EXPORT_CSS}</style>\n\
</head>\n\
<body>\n{body}\n</body>\n\
</html>\n"
    )
}

/// Escapes text for safe inclusion in element content (e.g. the `<title>`).
fn escape_html_text(value: &str, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            other => output.push(other),
        }
    }
}
