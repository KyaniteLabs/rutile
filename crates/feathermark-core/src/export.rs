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
//! `prefers-color-scheme`, plus a `forced-colors` pass and a print sheet), with
//! rutile-gold accents kept at needle weight only — the H1 title's geniculated
//! (elbow-bent) gold underline, the sixling `*` divider, the blockquote spine,
//! list markers, and link underlines. The geniculated bend is drawn with a
//! `data:`-URI SVG `mask-image` (self-contained, no external fetch) gated behind
//! an `@supports` query, so a browser without CSS masking falls back to a plain
//! hairline rather than a filled slab. Corners are 2px, type runs on a fluid
//! `clamp()` scale, the measure caps at 68ch, directional CSS uses logical
//! properties (`padding-inline-start`, `border-block-end`, `text-align: start`,
//! …) for i18n resilience, and hover "edge-break" transitions (150ms, CSS-only)
//! warm blockquote/code/table borders toward a break-tan, honoring
//! `prefers-reduced-motion`.
//!
//! 0.2 export is text-only: Markdown images render as their alt text (SPEC
//! §7-OQ1 defers data-URI image embedding to 0.3), so a conforming export never
//! carries an image `src` at all today. The `img` rule below (2px hairline
//! frame, per `DESIGN-SYSTEM.md`'s imagery section) is forward-compatible with
//! that future without costing anything now.

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
/// Self-contained by construction: no `url()`, no `@import`, no web fonts. The
/// one exception is the `data:` mask image below that draws the geniculated
/// (elbow-bent) H1 signature — `data:` URIs load nothing external, and the
/// self-containment inspector (`export_contract::inspect`) allowlists them
/// explicitly. It is applied only inside an `@supports` feature query, so a
/// browser without CSS masking falls back to the plain gold hairline instead
/// of a filled slab (the fire budget's hard "never a slab" rule holds either
/// way).
const EXPORT_CSS: &str = r#":root{color-scheme:light dark;--font-body:Charter,'Bitstream Charter','Sitka Text',Cambria,Georgia,serif;--font-mono:ui-monospace,'SF Mono','Cascadia Code',Menlo,Consolas,monospace;--measure:68ch;--radius:2px;--rule-needle:1px;--dur-fast:150ms;--ease-out:cubic-bezier(.2,0,0,1);--step--1:clamp(.95rem,.92rem + .12vw,1rem);--step-0:clamp(1rem,.96rem + .2vw,1.125rem);--step-1:clamp(1.2rem,1.1rem + .4vw,1.375rem);--step-2:clamp(1.4rem,1.24rem + .7vw,1.75rem);--step-3:clamp(1.7rem,1.42rem + 1.25vw,2.1875rem);--step-4:clamp(2.05rem,1.6rem + 2vw,2.75rem);--accent:#c9921e;--bg:#f6f2e9;--surface:#e9e2d6;--ink:#241e19;--muted:#6b6157;--border:#d8cfbe;--edge:#9f7655}
@media (prefers-color-scheme:dark){:root{--accent:#f0c24b;--bg:#241e19;--surface:#332d26;--ink:#e9e2d6;--muted:#a69b8c;--border:#3a332b}}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
html,body{overflow-x:hidden}
body{margin:0;background:var(--bg);color:var(--ink);font-family:var(--font-body);font-size:var(--step-0);line-height:1.65;text-rendering:optimizeLegibility;overflow-wrap:break-word}
main{max-width:var(--measure);margin-inline:auto;padding-block:clamp(2.5rem,7vw,4rem) clamp(3rem,9vw,6rem);padding-inline:clamp(1rem,4vw,1.5rem)}
h1,h2,h3,h4,h5,h6{font-family:var(--font-body);font-weight:700;line-height:1.2;margin-block:2.2em .6em;text-wrap:balance}
h1{font-size:var(--step-4);margin-block-start:0;padding-block-end:.28em;border-block-end:var(--rule-needle) solid var(--accent)}
h2{font-size:var(--step-3)}
h3{font-size:var(--step-2)}
h4{font-size:var(--step-1)}
h5{font-size:var(--step-0)}
h6{font-size:var(--step--1);text-transform:uppercase;letter-spacing:.06em;color:var(--muted)}
@supports (mask-image:none) or (-webkit-mask-image:none){
h1{border-block-end:0;padding-block-end:0}
h1::after{content:"";display:block;width:min(100%,26rem);height:.5rem;margin-block-start:.4em;background-color:var(--accent);-webkit-mask-repeat:no-repeat;mask-repeat:no-repeat;-webkit-mask-size:100% 100%;mask-size:100% 100%;-webkit-mask-image:url("data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 240 12'><path d='M0 9 L150 9 L169 3 L240 3' fill='none' stroke='black' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'/></svg>");mask-image:url("data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 240 12'><path d='M0 9 L150 9 L169 3 L240 3' fill='none' stroke='black' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'/></svg>")}
}
p{margin-block:0 1.2em}
a{color:inherit;text-decoration:underline;text-decoration-color:var(--accent);text-decoration-thickness:var(--rule-needle);text-underline-offset:.18em;transition:text-decoration-thickness var(--dur-fast) var(--ease-out)}
a:hover,a:focus-visible{text-decoration-thickness:2px}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
blockquote{margin-block:1.4em;padding-block:.2em;padding-inline-start:1.1em;border-inline-start:var(--rule-needle) solid var(--accent);color:var(--muted);transition:border-color var(--dur-fast) var(--ease-out)}
blockquote:hover{border-inline-start-color:var(--edge)}
code{font-family:var(--font-mono);font-size:.9em;background:var(--surface);padding-block:.1em;padding-inline:.3em;border-radius:var(--radius)}
pre{background:var(--surface);border:var(--rule-needle) solid var(--border);border-radius:var(--radius);padding-block:1em;padding-inline:1.2em;overflow-x:auto;transition:border-color var(--dur-fast) var(--ease-out)}
pre:hover{border-color:var(--edge)}
pre code{background:none;padding:0;font-size:.95em}
hr{border:0;margin-block:2.6em;text-align:center;overflow:visible;line-height:1}
hr::before{content:"\2736";color:var(--accent);font-size:1.15rem}
ul,ol{padding-inline-start:1.4em}
li{margin-block:.3em}
li::marker{color:var(--accent)}
img{max-width:100%;height:auto;border:var(--rule-needle) solid var(--border);border-radius:var(--radius)}
table{display:block;overflow-x:auto;border-collapse:collapse;width:100%;margin-block:1.4em;font-size:var(--step--1)}
th,td{border:var(--rule-needle) solid var(--border);padding-block:.5em;padding-inline:.7em;text-align:start;transition:border-color var(--dur-fast) var(--ease-out)}
table:hover th,table:hover td{border-color:var(--edge)}
th{font-weight:700}
.align-left{text-align:start}
.align-center{text-align:center}
.align-right{text-align:end}
.image-alt{color:var(--muted);font-style:italic}
.task-checkbox{margin-inline-end:.4em}
@media (prefers-reduced-motion:reduce){*{transition:none!important;animation:none!important}}
@media (forced-colors:active){
code,pre{border:var(--rule-needle) solid CanvasText}
hr::before,h1::after{forced-color-adjust:none}
:focus-visible{outline-color:Highlight}
}
@media print{
:root{--bg:#fff;--surface:#fff;--ink:#000;--muted:#333;--border:#999;--accent:#c9921e;--edge:#9f7655}
*{-webkit-print-color-adjust:exact;print-color-adjust:exact}
body{font-size:12pt;overflow-wrap:normal}
main{max-width:none;padding:0}
table{display:table;overflow-x:visible}
pre,blockquote,table{break-inside:avoid}
}"#;

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
