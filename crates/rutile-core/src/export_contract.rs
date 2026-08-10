//! Typed contracts for recipient-grade self-contained HTML export
//! (SPEC §7, LD-4).
//!
//! Wave 0 freezes the request/response shape only; the (Wave 1) engine
//! `export::render_export_page` fills it in. The export invariants — zero
//! scripts, zero external requests — are expressed as a validating newtype:
//! an [`ExportPage`] cannot exist unless its HTML passed the self-containment
//! inspection, mirroring the preview inspector's philosophy.

use rutile_types::Revision;
use thiserror::Error;

use crate::{MAX_RENDERED_PAGE_BYTES, RenderError};

/// Maximum bytes for the document title carried in an [`ExportRequest`].
pub const MAX_EXPORT_TITLE_BYTES: usize = 512;

/// Byte-size gate for a rendered export page.
///
/// Kept equal to the preview's page cap so any document the preview can
/// render is also exportable; the export template adds only bounded inline
/// styles.
pub const MAX_EXPORT_PAGE_BYTES: usize = MAX_RENDERED_PAGE_BYTES;

/// Content-Security-Policy for the exported file: deny by default, permit only
/// the inline stylesheet and (future) inline data-URI images. Belt-and-suspenders
/// over the [`ExportPage`] inspection for recipients whose viewer honors meta CSP.
pub const EXPORT_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";

/// A request to render the current document as a self-contained HTML page.
///
/// Fields are private so every request is validated on construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportRequest {
    revision: Revision,
    title: Option<String>,
}

impl ExportRequest {
    pub fn new(revision: Revision, title: Option<String>) -> Result<Self, ExportError> {
        if let Some(title) = &title
            && title.len() > MAX_EXPORT_TITLE_BYTES
        {
            return Err(ExportError::TitleTooLarge {
                len: title.len(),
                max: MAX_EXPORT_TITLE_BYTES,
            });
        }
        Ok(Self { revision, title })
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
}

/// A violation of the export page's self-containment invariants.
///
/// The export template is trusted output (the renderer escapes document
/// text), so this inspection is a belt-and-suspenders gate over generated
/// markup, not a sanitizer for hostile input.
///
/// `#[non_exhaustive]` and the absence of `Copy` are deliberate: the Wave-1/3
/// sanitizer will grow this enum (e.g. inline event handlers, relative
/// references) and some variants will carry owned data, so downstream matches
/// keep a wildcard arm and the shape never has to be reopened after the freeze.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExportViolation {
    #[error("export page contains a <script> element")]
    Script,
    #[error("export page contains a <link> element (external stylesheet or resource hint)")]
    LinkElement,
    #[error("export page embeds a frame, object, or plugin element")]
    FrameOrObject,
    #[error("export page references an external URL (src/srcset/@import/url())")]
    ExternalReference,
    #[error("export page contains a javascript: URL")]
    JavascriptUrl,
    #[error("export page contains an inline event handler ({attr}=)")]
    EventHandler { attr: String },
    #[error("export page contains a relative or root-relative reference ({reference})")]
    RelativeReference { reference: String },
    #[error("export page contains a data:text/html URL")]
    DataHtmlUrl,
    #[error("export page contains a CSS expression()")]
    CssExpression,
    #[error("export page contains a <meta http-equiv=\"refresh\"> redirect")]
    MetaRefresh,
    #[error("export page contains navigation-capable metadata ({tag})")]
    NavigationCapableMeta { tag: String },
    #[error("export page contains duplicate CSP meta tags")]
    DuplicateCsp,
    #[error("export page is missing the required CSP meta tag")]
    MissingCsp,
    #[error("export page CSP meta has an unexpected value")]
    CspMismatch,
    #[error("export page exceeds {max} bytes")]
    TooLarge { max: usize },
}

/// Errors from building or validating an export.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("export title is {len} bytes; the maximum is {max}")]
    TitleTooLarge { len: usize, max: usize },
    #[error(transparent)]
    Violation(#[from] ExportViolation),
}

/// A validated, self-contained export page.
///
/// The only constructor is [`ExportPage::from_html`], which enforces the
/// zero-JS / zero-external-request invariants, so holding an `ExportPage`
/// *is* the proof that the page passed inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportPage {
    html: String,
}

impl ExportPage {
    pub fn from_html(html: String) -> Result<Self, ExportViolation> {
        if html.len() > MAX_EXPORT_PAGE_BYTES {
            return Err(ExportViolation::TooLarge {
                max: MAX_EXPORT_PAGE_BYTES,
            });
        }
        inspect(&html)?;
        Ok(Self { html })
    }

    pub fn as_html(&self) -> &str {
        &self.html
    }

    pub fn into_html(self) -> String {
        self.html
    }
}

/// Rejects markup that would execute code or trigger a network request when
/// the exported file is opened. Plain `href` hyperlinks are allowed: they
/// fetch nothing until the recipient deliberately follows them.
///
/// This is an **allowlist** inspection, not a substring denylist. The generated
/// export escapes every byte of document text, so a literal `<` only ever begins
/// a real element and a literal `>` only ever ends one (values are entity-escaped
/// and never carry either). That invariant lets the inspector walk the markup as
/// a stream of tags, `<style>` blocks, comments, and inert text — and apply its
/// checks *only inside real element tags and stylesheet blocks*. Escaped document
/// text (e.g. a note that literally contains `<img onerror=…>` or `url(http…)`)
/// is left alone, so hostile-but-renderable documents export without either
/// injecting anything or being falsely rejected.
///
/// Inside a real tag the rules are: forbidden elements (`script`, `link`,
/// frames/objects) are rejected by name; `on*=` attributes, `javascript:` and
/// `data:text/html` URLs, and `srcset` are rejected; `src`/`poster` and CSS
/// `url()` are allowlisted to `data:` only (anything else is an external or
/// relative reference); `@import` and `expression()` are rejected. `href` keeps
/// http/https/mailto/relative anchors — they fetch nothing on open.
///
/// Complete HTML pages (those with a `<!doctype html>` or `<html>` element) must
/// carry exactly one CSP meta tag whose `content` equals [`EXPORT_CSP`].
fn inspect(html: &str) -> Result<(), ExportViolation> {
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0;
    let mut csp_count = 0;
    let mut is_complete_page = false;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        // HTML comment: inert, skip to the terminator.
        if lower[index..].starts_with("<!--") {
            index = match lower[index + 4..].find("-->") {
                Some(offset) => index + 4 + offset + 3,
                None => bytes.len(),
            };
            continue;
        }
        // Declaration such as `<!doctype html>`: no attributes of concern.
        if bytes.get(index + 1) == Some(&b'!') {
            if lower[index..].starts_with("<!doctype html") {
                is_complete_page = true;
            }
            index = match lower[index..].find('>') {
                Some(offset) => index + offset + 1,
                None => bytes.len(),
            };
            continue;
        }
        // Real element: the tag body runs from `<` to the next `>`.
        let (inner, after) = match lower[index..].find('>') {
            Some(offset) => (&lower[index + 1..index + offset], index + offset + 1),
            None => (&lower[index + 1..], bytes.len()),
        };
        let closing = inner.starts_with('/');
        let name = tag_name(inner);
        if name == "html" {
            is_complete_page = true;
        }
        match name {
            "script" => return Err(ExportViolation::Script),
            "link" => return Err(ExportViolation::LinkElement),
            "iframe" | "frame" | "object" | "embed" | "applet" => {
                return Err(ExportViolation::FrameOrObject);
            }
            "base" | "form" => {
                return Err(ExportViolation::NavigationCapableMeta {
                    tag: name.to_owned(),
                });
            }
            "meta" => {
                inspect_attributes(inner)?;
                if inspect_meta(inner)? {
                    csp_count += 1;
                }
            }
            _ => {
                if !closing {
                    inspect_attributes(inner)?;
                }
            }
        }
        index = after;
        // A `<style>` element's content is trusted CSS, but the allowlist still
        // proves it carries no external or executable reference.
        if !closing && name == "style" {
            let css_end = match lower[index..].find("</style>") {
                Some(offset) => index + offset,
                None => bytes.len(),
            };
            inspect_css(&lower[index..css_end])?;
            index = css_end;
        }
    }
    if is_complete_page && csp_count != 1 {
        return if csp_count == 0 {
            Err(ExportViolation::MissingCsp)
        } else {
            Err(ExportViolation::DuplicateCsp)
        };
    }
    Ok(())
}

/// The element name from a tag body (`inner` is the slice between `<` and `>`).
fn tag_name(inner: &str) -> &str {
    inner
        .strip_prefix('/')
        .unwrap_or(inner)
        .split(|character: char| character.is_ascii_whitespace() || character == '/')
        .next()
        .unwrap_or("")
}

/// Parses the attributes of a tag body into `(name, value)` pairs.
///
/// `inner` is expected to be lowercased; values are returned as they appear in
/// `inner` (also lowercased because of the caller's preprocessing).
fn parse_attributes(inner: &str) -> Vec<(&str, &str)> {
    let bytes = inner.as_bytes();
    let mut index = 0;
    // Skip the tag name.
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut attributes = Vec::new();
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let name_start = index;
        while index < bytes.len()
            && bytes[index] != b'='
            && bytes[index] != b'/'
            && !bytes[index].is_ascii_whitespace()
        {
            index += 1;
        }
        let name = &inner[name_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let mut value = "";
        if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < bytes.len() && (bytes[index] == b'"' || bytes[index] == b'\'') {
                let quote = bytes[index];
                index += 1;
                let value_start = index;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                value = &inner[value_start..index];
                if index < bytes.len() {
                    index += 1;
                }
            } else {
                let value_start = index;
                while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                value = &inner[value_start..index];
            }
        }
        attributes.push((name, value));
    }
    attributes
}

/// Walks the attributes of an opening tag and applies the allowlist to each.
fn inspect_attributes(inner: &str) -> Result<(), ExportViolation> {
    for (name, value) in parse_attributes(inner) {
        inspect_attribute(name, value)?;
    }
    Ok(())
}

/// Inspects a `<meta>` tag body. Returns `true` if it is the single allowed CSP
/// meta, `false` for inert meta tags, and errors for navigation-capable metadata.
fn inspect_meta(inner: &str) -> Result<bool, ExportViolation> {
    let mut http_equiv = None;
    let mut content = None;
    for (name, value) in parse_attributes(inner) {
        match name {
            "http-equiv" => http_equiv = Some(value),
            "content" => content = Some(value),
            _ => {}
        }
    }
    match http_equiv {
        Some("refresh") => Err(ExportViolation::MetaRefresh),
        Some("content-security-policy") => {
            if content == Some(EXPORT_CSP) {
                Ok(true)
            } else {
                Err(ExportViolation::CspMismatch)
            }
        }
        Some(_) => Err(ExportViolation::NavigationCapableMeta {
            tag: "meta http-equiv".to_owned(),
        }),
        None => Ok(false),
    }
}

/// Applies the allowlist to a single `name="value"` attribute pair.
fn inspect_attribute(name: &str, value: &str) -> Result<(), ExportViolation> {
    let name_bytes = name.as_bytes();
    if name_bytes.len() > 2
        && name_bytes[0] == b'o'
        && name_bytes[1] == b'n'
        && name_bytes[2..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
    {
        return Err(ExportViolation::EventHandler {
            attr: name.to_owned(),
        });
    }
    if value.contains("javascript:") {
        return Err(ExportViolation::JavascriptUrl);
    }
    if value.contains("data:text/html") {
        return Err(ExportViolation::DataHtmlUrl);
    }
    match name {
        "src" | "poster" | "background" => classify_reference(value)?,
        "srcset" => return Err(ExportViolation::ExternalReference),
        "href" => classify_href(value)?,
        "style" => inspect_css(value)?,
        _ => {}
    }
    Ok(())
}

/// Allowlists an `href` value to http/https/mailto targets and relative anchors.
fn classify_href(reference: &str) -> Result<(), ExportViolation> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(ExportViolation::RelativeReference {
            reference: reference.to_owned(),
        });
    }
    if reference.starts_with("http:")
        || reference.starts_with("https:")
        || reference.starts_with("mailto:")
        || reference.starts_with('#')
    {
        return Ok(());
    }
    Err(ExportViolation::RelativeReference {
        reference: reference.to_owned(),
    })
}

/// Allowlists a resource reference: only `data:` URLs load nothing external and
/// nothing relative to the recipient's filesystem, so everything else is barred.
fn classify_reference(reference: &str) -> Result<(), ExportViolation> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Ok(());
    }
    if reference.contains("data:text/html") {
        return Err(ExportViolation::DataHtmlUrl);
    }
    if reference.starts_with("data:") {
        return Ok(());
    }
    if reference.starts_with("http:")
        || reference.starts_with("https:")
        || reference.starts_with("//")
    {
        return Err(ExportViolation::ExternalReference);
    }
    Err(ExportViolation::RelativeReference {
        reference: reference.to_owned(),
    })
}

/// Applies the allowlist to a CSS fragment (a `<style>` body or a `style="…"`).
fn inspect_css(css: &str) -> Result<(), ExportViolation> {
    if css.contains("@import") {
        return Err(ExportViolation::ExternalReference);
    }
    if css.contains("expression(") {
        return Err(ExportViolation::CssExpression);
    }
    let mut rest = css;
    while let Some(offset) = rest.find("url(") {
        let after = &rest[offset + 4..];
        let end = after.find(')').unwrap_or(after.len());
        let argument = after[..end].trim().trim_matches(['"', '\'']).trim();
        classify_reference(argument)?;
        rest = &after[end..];
    }
    Ok(())
}
