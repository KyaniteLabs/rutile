//! Typed contracts for recipient-grade self-contained HTML export
//! (SPEC §7, LD-4).
//!
//! Wave 0 freezes the request/response shape only; the (Wave 1) engine
//! `export::render_export_page` fills it in. The export invariants — zero
//! scripts, zero external requests — are expressed as a validating newtype:
//! an [`ExportPage`] cannot exist unless its HTML passed the self-containment
//! inspection, mirroring the preview inspector's philosophy.

use feathermark_types::Revision;
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
fn inspect(html: &str) -> Result<(), ExportViolation> {
    let lowered = html.to_ascii_lowercase();
    if lowered.contains("<script") {
        return Err(ExportViolation::Script);
    }
    if lowered.contains("<link") {
        return Err(ExportViolation::LinkElement);
    }
    if ["<iframe", "<frame", "<object", "<embed", "<applet"]
        .iter()
        .any(|tag| lowered.contains(tag))
    {
        return Err(ExportViolation::FrameOrObject);
    }
    if lowered.contains("javascript:") {
        return Err(ExportViolation::JavascriptUrl);
    }
    let external_fetch_markers = [
        "src=\"http",
        "src='http",
        "src=\"//",
        "src='//",
        "srcset=",
        "@import",
        "url(http",
        "url(//",
        "url(\"http",
        "url('http",
        "url(\"//",
        "url('//",
    ];
    if external_fetch_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err(ExportViolation::ExternalReference);
    }
    Ok(())
}
