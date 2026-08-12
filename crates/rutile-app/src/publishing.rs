//! Publishing presets and print workflow contract (roadmap 09).
//!
//! A [`PublishingPreset`] is bounded presentation data that produces a print
//! stylesheet consumed by the export integrator. The frozen
//! [`render_export_page`](rutile_core::render_export_page) still owns the safe,
//! self-contained HTML; this module layers print presentation on top without
//! touching the renderer or constructing HTML outside a `<style>` block.
//!
//! # Design
//!
//! See `docs/plan/publishing-presets-design.md`. Key decisions: presets wrap
//! the safe export (not replace it), bounded clamped fields, fixed system font
//! stacks (no remote fonts), `@media print` CSS, no hosted service.
//!
//! # Security-core fence
//!
//! The generated CSS contains no `url()`, `@import`, or `expression()` — the
//! exact constructs the export validator rejects — so an injected preset block
//! re-validates cleanly. No raw HTML is constructed outside `<style>`.

/// Supported page formats for publishing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PageFormat {
    #[default]
    A4,
    Letter,
    Legal,
    A5,
}

impl PageFormat {
    /// The CSS `@page` size keyword.
    pub const fn css_size(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::Letter => "Letter",
            Self::Legal => "Legal",
            Self::A5 => "A5",
        }
    }

    /// A short, filesystem-safe slug for export filenames.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::Letter => "letter",
            Self::Legal => "legal",
            Self::A5 => "a5",
        }
    }
}

/// Body font family, mapped to a fixed system-available stack (never remote).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FontFamily {
    #[default]
    Serif,
    SansSerif,
    Monospace,
}

impl FontFamily {
    /// The CSS font-family stack. System fonts only — no `url()` / `@font-face`.
    pub const fn css_stack(self) -> &'static str {
        match self {
            Self::Serif => "Georgia, \"Times New Roman\", serif",
            Self::SansSerif => "system-ui, -apple-system, sans-serif",
            Self::Monospace => "ui-monospace, \"SF Mono\", Menlo, monospace",
        }
    }
}

/// Bounded bounds for preset fields.
const MIN_MARGIN_MM: u16 = 5;
const MAX_MARGIN_MM: u16 = 50;
const MIN_FONT_PT: u8 = 8;
const MAX_FONT_PT: u8 = 18;

/// A bounded publishing preset producing a print stylesheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishingPreset {
    format: PageFormat,
    margin_mm: u16,
    font: FontFamily,
    base_font_size_pt: u8,
}

impl Default for PublishingPreset {
    fn default() -> Self {
        Self::print_ready()
    }
}

impl PublishingPreset {
    /// Builds a preset, clamping every field to its safe range.
    pub fn new(
        format: PageFormat,
        margin_mm: u16,
        font: FontFamily,
        base_font_size_pt: u8,
    ) -> Self {
        Self {
            format,
            margin_mm: margin_mm.clamp(MIN_MARGIN_MM, MAX_MARGIN_MM),
            font,
            base_font_size_pt: base_font_size_pt.clamp(MIN_FONT_PT, MAX_FONT_PT),
        }
    }

    /// Draft: A4, comfortable sans, generous margins (PP5).
    pub fn draft() -> Self {
        Self::new(PageFormat::A4, 25, FontFamily::SansSerif, 12)
    }

    /// Manuscript: Letter, 12pt serif, 1in (≈25mm) margins — standard format.
    pub fn manuscript() -> Self {
        Self::new(PageFormat::Letter, 25, FontFamily::Serif, 12)
    }

    /// Print-ready: A4, 11pt serif, balanced margins (default).
    pub fn print_ready() -> Self {
        Self::new(PageFormat::A4, 18, FontFamily::Serif, 11)
    }

    pub fn format(&self) -> PageFormat {
        self.format
    }

    pub fn margin_mm(&self) -> u16 {
        self.margin_mm
    }

    pub fn font(&self) -> FontFamily {
        self.font
    }

    pub fn base_font_size_pt(&self) -> u8 {
        self.base_font_size_pt
    }

    /// A filesystem-safe slug for this preset (format only — never user input).
    pub fn slug(&self) -> String {
        self.format.slug().to_owned()
    }

    /// Generates the `@media print { … }` stylesheet for this preset.
    ///
    /// Contains only `@page` size/margin and body font declarations — no
    /// `url()`, `@import`, or `expression()` — so it re-validates as a safe
    /// export block.
    pub fn print_stylesheet(&self) -> String {
        format!(
            "@media print{{@page{{size:{size};margin:{margin}mm}}\
             body{{font-family:{font};font-size:{size_pt}pt;line-height:1.5}}}}",
            size = self.format.css_size(),
            margin = self.margin_mm,
            font = self.font.css_stack(),
            size_pt = self.base_font_size_pt,
        )
    }

    /// The ready-to-inject `<style>` block wrapping the print stylesheet.
    pub fn print_style_block(&self) -> String {
        format!("<style>{}</style>", self.print_stylesheet())
    }

    /// Derives an export filename `<stem>-<slug>.html` from a document stem.
    /// `stem` is expected to be a file name without extension; the slug is a
    /// fixed format identifier, so the result can't be spoofed.
    pub fn suggested_filename(&self, stem: &str) -> String {
        let clean = stem
            .trim()
            .trim_end_matches(".md")
            .trim_end_matches(".markdown");
        let base = if clean.is_empty() { "untitled" } else { clean };
        format!("{base}-{slug}.html", slug = self.format.slug())
    }
}

#[cfg(test)]
mod publishing_tests {
    use super::*;

    #[test]
    fn defaults_to_print_ready() {
        let p = PublishingPreset::default();
        assert_eq!(p, PublishingPreset::print_ready());
        assert_eq!(p.format(), PageFormat::A4);
        assert_eq!(p.font(), FontFamily::Serif);
        assert_eq!(p.base_font_size_pt(), 11);
    }

    #[test]
    fn built_in_presets_have_distinct_formats() {
        assert_eq!(PublishingPreset::draft().format(), PageFormat::A4);
        assert_eq!(PublishingPreset::manuscript().format(), PageFormat::Letter);
        assert_eq!(PublishingPreset::print_ready().format(), PageFormat::A4);
    }

    // -- Bounding (PP2) ------------------------------------------------------

    #[test]
    fn margins_clamp_to_safe_range() {
        let tiny = PublishingPreset::new(PageFormat::A4, 0, FontFamily::Serif, 11);
        assert_eq!(tiny.margin_mm(), MIN_MARGIN_MM);
        let huge = PublishingPreset::new(PageFormat::A4, 999, FontFamily::Serif, 11);
        assert_eq!(huge.margin_mm(), MAX_MARGIN_MM);
    }

    #[test]
    fn font_size_clamps_to_safe_range() {
        let tiny = PublishingPreset::new(PageFormat::A4, 20, FontFamily::Serif, 0);
        assert_eq!(tiny.base_font_size_pt(), MIN_FONT_PT);
        let huge = PublishingPreset::new(PageFormat::A4, 20, FontFamily::Serif, 99);
        assert_eq!(huge.base_font_size_pt(), MAX_FONT_PT);
    }

    // -- Print stylesheet safety (PP4) --------------------------------------

    #[test]
    fn print_stylesheet_is_media_print() {
        let css = PublishingPreset::manuscript().print_stylesheet();
        assert!(css.starts_with("@media print{"));
        assert!(css.contains("@page{size:Letter;margin:25mm}"));
    }

    #[test]
    fn print_stylesheet_never_contains_forbidden_constructs() {
        // The export validator rejects url(), @import, expression(). A preset's
        // print CSS must never produce these, or injection+revalidation fails.
        for preset in [
            PublishingPreset::draft(),
            PublishingPreset::manuscript(),
            PublishingPreset::print_ready(),
        ] {
            let css = preset.print_stylesheet();
            assert!(!css.contains("url("), "url() leaked: {css}");
            assert!(!css.contains("@import"), "@import leaked: {css}");
            assert!(!css.contains("expression("), "expression() leaked: {css}");
        }
    }

    #[test]
    fn style_block_wraps_in_style_element() {
        let block = PublishingPreset::draft().print_style_block();
        assert!(block.starts_with("<style>@media print"));
        assert!(block.ends_with("</style>"));
    }

    #[test]
    fn font_stacks_are_system_only() {
        assert!(FontFamily::Serif.css_stack().contains("serif"));
        assert!(FontFamily::SansSerif.css_stack().contains("sans-serif"));
        assert!(FontFamily::Monospace.css_stack().contains("monospace"));
        // No remote font reference.
        for f in [
            FontFamily::Serif,
            FontFamily::SansSerif,
            FontFamily::Monospace,
        ] {
            assert!(!f.css_stack().contains("http"));
        }
    }

    // -- Filename derivation (PP6) ------------------------------------------

    #[test]
    fn suggested_filename_includes_slug() {
        let p = PublishingPreset::manuscript();
        assert_eq!(p.suggested_filename("chapter-1"), "chapter-1-letter.html");
    }

    #[test]
    fn suggested_filename_strips_markdown_extension() {
        let p = PublishingPreset::print_ready();
        assert_eq!(p.suggested_filename("notes.md"), "notes-a4.html");
        assert_eq!(p.suggested_filename("draft.markdown"), "draft-a4.html");
    }

    #[test]
    fn suggested_filename_handles_empty_stem() {
        let p = PublishingPreset::print_ready();
        assert_eq!(p.suggested_filename(""), "untitled-a4.html");
        assert_eq!(p.suggested_filename("   "), "untitled-a4.html");
    }

    // -- Page format coverage ------------------------------------------------

    #[test]
    fn all_formats_have_css_size_and_slug() {
        for f in [
            PageFormat::A4,
            PageFormat::Letter,
            PageFormat::Legal,
            PageFormat::A5,
        ] {
            assert!(!f.css_size().is_empty());
            assert!(!f.slug().is_empty());
        }
    }
}
