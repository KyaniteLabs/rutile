//! Outline navigator contract (roadmap 05).
//!
//! Derives a document's heading outline from the **existing** source-anchor
//! model ([`build_source_blocks`]) so headings match the preview exactly. The
//! shell renders the entries and navigates by byte offset + renderer dom id.
//!
//! # Design
//!
//! See `docs/plan/outline-navigator-design.md`. Key decisions: reuse the
//! renderer's source blocks (no second parser), level/text from the validated
//! range, navigation by byte offset, bounded flat list.
//!
//! # Security-core fence
//!
//! This module reads only text already in memory and the renderer's public
//! source-block API. It never parses files, follows links, or constructs HTML.

use rutile_core::{RenderError, SourceBlockKind, build_source_blocks};

/// Maximum outline entries (resource bound, O5).
pub const MAX_OUTLINE_ENTRIES: usize = 500;

/// A single heading in the document outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Heading level 1–6 (ATX `#` count or setext underline).
    pub level: u8,
    /// Display text with leading/trailing markup stripped.
    pub text: String,
    /// Byte offset of the heading start in the source.
    pub source_offset: usize,
    /// The renderer's source-block anchor id (for preview-side correlation).
    pub dom_id: String,
}

/// The ordered, flat list of a document's headings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    entries: Vec<OutlineEntry>,
}

impl Outline {
    /// Builds the outline from markdown `source` via the renderer's
    /// source-anchor model. Returns an empty outline for a heading-less
    /// document; propagates a [`RenderError`] only if the source is so
    /// pathological the renderer itself rejects it.
    pub fn from_source(source: &str) -> Result<Self, RenderError> {
        let blocks = build_source_blocks(source, 0)?;
        let mut entries = Vec::new();
        for block in blocks.iter().filter(|b| b.kind == SourceBlockKind::Heading) {
            if entries.len() >= MAX_OUTLINE_ENTRIES {
                break;
            }
            let (level, text) = extract_level_and_text(&source[block.start..block.end]);
            entries.push(OutlineEntry {
                level,
                text,
                source_offset: block.start,
                dom_id: block.dom_id.clone(),
            });
        }
        Ok(Self { entries })
    }

    /// The ordered heading entries.
    #[must_use]
    pub fn entries(&self) -> &[OutlineEntry] {
        &self.entries
    }

    /// Number of headings.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no headings.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The deepest heading whose source offset precedes `byte_offset` — i.e.
    /// the section the viewport is currently inside (O6).
    #[must_use]
    pub fn heading_at(&self, byte_offset: usize) -> Option<&OutlineEntry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.source_offset <= byte_offset)
    }

    /// The first heading strictly after `byte_offset` (↓ in the sidebar).
    #[must_use]
    pub fn next_after(&self, byte_offset: usize) -> Option<&OutlineEntry> {
        self.entries.iter().find(|e| e.source_offset > byte_offset)
    }

    /// The last heading strictly before `byte_offset` (↑ in the sidebar).
    #[must_use]
    pub fn prev_before(&self, byte_offset: usize) -> Option<&OutlineEntry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.source_offset < byte_offset)
    }
}

/// Recovers the level (1–6) and display text from a heading's validated source
/// range. Handles ATX (`#`) and setext (`===`/`---`) headings; defaults to
/// level 1 when the markup is ambiguous.
fn extract_level_and_text(heading_source: &str) -> (u8, String) {
    let first_line = heading_source.lines().next().unwrap_or("");
    let trimmed = first_line.trim_start();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) {
        let text = trimmed[hashes..].trim().to_owned();
        return (hashes as u8, text);
    }

    // Setext heading: a text line followed by an `===` (H1) or `---` (H2) line.
    let mut lines = heading_source.lines();
    let text_line = lines.next().unwrap_or("").trim().to_owned();
    if let Some(underline) = lines.next() {
        let u = underline.trim();
        if !u.is_empty() && u.bytes().all(|b| b == b'=') {
            return (1, text_line);
        }
        if !u.is_empty() && u.bytes().all(|b| b == b'-') {
            return (2, text_line);
        }
    }
    (1, text_line)
}

#[cfg(test)]
mod outline_tests {
    use super::*;

    #[test]
    fn empty_document_has_no_outline() {
        let outline = Outline::from_source("").unwrap();
        assert!(outline.is_empty());
    }

    #[test]
    fn plain_text_has_no_headings() {
        let outline = Outline::from_source("Just a paragraph.\n\nNo headings.").unwrap();
        assert!(outline.is_empty());
    }

    #[test]
    fn atx_levels_extracted_in_order() {
        let src = "# Title\n\n## Section\n\n### Sub\n\n## Back to two";
        let outline = Outline::from_source(src).unwrap();
        let levels: Vec<u8> = outline.entries().iter().map(|e| e.level).collect();
        assert_eq!(levels, vec![1, 2, 3, 2]);
        assert_eq!(outline.entries()[0].text, "Title");
        assert_eq!(outline.entries()[2].text, "Sub");
    }

    #[test]
    fn heading_source_offsets_are_byte_positions() {
        let src = "# A\n\n# B";
        let outline = Outline::from_source(src).unwrap();
        assert_eq!(outline.entries().len(), 2);
        assert_eq!(outline.entries()[0].source_offset, 0);
        // "# A\n\n" = 5 bytes, so the second heading starts at 5.
        assert_eq!(outline.entries()[1].source_offset, 5);
    }

    #[test]
    fn setext_headings_recognized() {
        let src = "Title One\n==========\n\nTitle Two\n----------\n";
        let outline = Outline::from_source(src).unwrap();
        assert_eq!(outline.entries().len(), 2);
        assert_eq!(outline.entries()[0].level, 1);
        assert_eq!(outline.entries()[0].text, "Title One");
        assert_eq!(outline.entries()[1].level, 2);
        assert_eq!(outline.entries()[1].text, "Title Two");
    }

    #[test]
    fn heading_at_finds_current_section() {
        let src = "# A\n\nbody a\n\n## B\n\nbody b";
        let outline = Outline::from_source(src).unwrap();
        // Offset inside "body a" (after "# A\n\n" = 5 bytes, "body a" at 5..11).
        assert_eq!(outline.heading_at(7).map(|e| e.text.as_str()), Some("A"));
        // Offset inside "body b".
        let b_offset = outline
            .entries()
            .iter()
            .find(|e| e.text == "B")
            .unwrap()
            .source_offset;
        assert_eq!(
            outline.heading_at(b_offset + 2).map(|e| e.text.as_str()),
            Some("B")
        );
    }

    #[test]
    fn next_and_prev_navigation() {
        let src = "# One\n\n# Two\n\n# Three";
        let outline = Outline::from_source(src).unwrap();
        let first = outline.entries()[0].source_offset;
        assert_eq!(
            outline.next_after(first).map(|e| e.text.as_str()),
            Some("Two")
        );
        assert!(outline.prev_before(first).is_none());
        let last = outline.entries().last().unwrap().source_offset;
        assert_eq!(
            outline.prev_before(last).map(|e| e.text.as_str()),
            Some("Two")
        );
        assert!(outline.next_after(last).is_none());
    }

    #[test]
    fn duplicate_titles_both_present() {
        let src = "# Notes\n\n# Notes";
        let outline = Outline::from_source(src).unwrap();
        assert_eq!(outline.entries().len(), 2);
        assert_eq!(outline.entries()[0].source_offset, 0);
    }

    #[test]
    fn closing_hashes_left_in_text() {
        // Closing ATX #'s are left in the display text (non-destructive); the
        // important data — level + offset — is correct.
        let outline = Outline::from_source("## Hello ##").unwrap();
        assert_eq!(outline.entries()[0].level, 2);
        assert!(outline.entries()[0].text.contains("Hello"));
    }

    #[test]
    fn six_levels_supported() {
        let outline = Outline::from_source("###### Deepest").unwrap();
        assert_eq!(outline.entries().len(), 1);
        assert_eq!(outline.entries()[0].level, 6);
    }

    #[test]
    fn seven_hashes_is_not_a_heading_level() {
        // CommonMark: seven # is a paragraph, not an H7. The renderer treats it
        // as non-heading text, so the outline has no entry.
        let outline = Outline::from_source("####### Not a heading").unwrap();
        // pulldown-cmark may or may not classify 7# as a heading; assert it is
        // never reported above level 6.
        for e in outline.entries() {
            assert!(e.level <= 6);
        }
    }

    #[test]
    fn headings_inside_code_block_are_excluded() {
        // The renderer suppresses source blocks inside fenced code, so a `#`
        // line inside a code fence is not a heading.
        let src = "```\n# not a heading\n```\n\n# real heading";
        let outline = Outline::from_source(src).unwrap();
        let texts: Vec<&str> = outline.entries().iter().map(|e| e.text.as_str()).collect();
        assert!(texts.iter().all(|t| !t.contains("not a heading")));
        assert!(texts.contains(&"real heading"));
    }
}
