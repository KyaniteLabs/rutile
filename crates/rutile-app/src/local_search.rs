//! Local search and backlinks contract (roadmap 12).
//!
//! Provides [`SearchIndex`] for cross-document substring search and
//! [`BacklinkGraph`] for tracking inter-document links. See
//! `docs/plan/local-search-design.md` for the resolved grilling questions.
//!
//! # Security-core fence
//!
//! The search index operates ONLY on text already loaded into memory by the
//! document engine. It never reads files from disk, never traverses paths,
//! and never interprets URLs. Link parsing uses the existing
//! [`SafeLinkTarget`] types where applicable.

use std::collections::BTreeMap;

use rutile_types::DocumentId;

/// Maximum snippet length for search result context (bytes).
pub const MAX_SNIPPET_BYTES: usize = 200;

/// A single search match within a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The document that contains the match.
    pub document_id: DocumentId,
    /// Byte offset of the match start.
    pub byte_offset: usize,
    /// Byte length of the matched text.
    pub byte_length: usize,
    /// The nearest preceding heading line (line starting with `#`), or `None`.
    pub heading: Option<String>,
    /// A short text snippet around the match (≤ `MAX_SNIPPET_BYTES`).
    pub snippet: String,
    /// The document revision when this result was indexed (for staleness check).
    pub indexed_revision: u64,
}

/// A backlink: document A links to document B.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    /// The document that contains the link.
    pub source: DocumentId,
    /// The document path that the link targets (resolved relative path).
    pub target_path: String,
    /// The link text (display label).
    pub label: String,
    /// Byte offset of the link in the source document.
    pub byte_offset: usize,
}

/// In-memory cross-document substring search index (roadmap 12).
///
/// Indexes the text of open documents. Search is case-insensitive substring
/// matching. Results are ranked by active-document-first, then frequency,
/// then recency.
pub struct SearchIndex {
    /// Document texts keyed by `DocumentId`, plus their revision.
    documents: BTreeMap<DocumentId, IndexedDoc>,
    /// The active document (boosted in ranking).
    active_id: DocumentId,
    /// Tab order for recency ranking (MRU first).
    tab_order: Vec<DocumentId>,
}

struct IndexedDoc {
    text: String,
    revision: u64,
    /// Pre-computed heading line starts for context extraction.
    heading_starts: Vec<usize>,
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self {
            documents: BTreeMap::new(),
            active_id: DocumentId::ROOT,
            tab_order: vec![DocumentId::ROOT],
        }
    }
}

impl SearchIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the active document and tab order for ranking.
    pub fn set_context(&mut self, active_id: DocumentId, tab_order: Vec<DocumentId>) {
        self.active_id = active_id;
        self.tab_order = tab_order;
    }

    /// Adds or updates a document in the index.
    pub fn index_document(&mut self, id: DocumentId, text: &str, revision: u64) {
        let heading_starts = extract_heading_starts(text);
        self.documents.insert(
            id,
            IndexedDoc {
                text: text.to_owned(),
                revision,
                heading_starts,
            },
        );
    }

    /// Removes a document from the index.
    pub fn remove_document(&mut self, id: DocumentId) {
        self.documents.remove(&id);
    }

    /// Searches all indexed documents for `query` (case-insensitive substring).
    /// Returns results ranked: active document first, then by tab order, then
    /// by frequency within each document.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }
        let q = query.to_lowercase();

        // Collect per-document results, ordered by tab_order (active first).
        let mut ordered_ids: Vec<DocumentId> = Vec::new();
        if self.documents.contains_key(&self.active_id) {
            ordered_ids.push(self.active_id);
        }
        for &id in &self.tab_order {
            if id != self.active_id && self.documents.contains_key(&id) {
                ordered_ids.push(id);
            }
        }
        // Include any docs not in tab_order (safety net).
        for &id in self.documents.keys() {
            if !ordered_ids.contains(&id) {
                ordered_ids.push(id);
            }
        }

        let mut results = Vec::new();
        for id in ordered_ids {
            let doc = &self.documents[&id];
            let text_lower = doc.text.to_lowercase();
            let mut count = 0;
            let mut start = 0;
            while let Some(pos) = text_lower[start..].find(&q) {
                let abs_pos = start + pos;
                let heading = nearest_heading(&doc.text, &doc.heading_starts, abs_pos);
                let snippet = make_snippet(&doc.text, abs_pos, q.len());
                results.push(SearchResult {
                    document_id: id,
                    byte_offset: abs_pos,
                    byte_length: q.len(),
                    heading,
                    snippet,
                    indexed_revision: doc.revision,
                });
                count += 1;
                start = abs_pos + q.len().max(1);
                // Cap matches per document to bound result size.
                if count >= 50 {
                    break;
                }
            }
        }
        results
    }

    /// Number of indexed documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Whether no documents are indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

/// Backlink graph tracking inter-document links (roadmap 12).
///
/// Parses markdown links `[text](path)` and wiki-links `[[path]]` from each
/// document's text, then answers "which documents link to `target_path`?"
#[derive(Default)]
pub struct BacklinkGraph {
    /// All known backlinks, keyed by source document.
    backlinks: BTreeMap<DocumentId, Vec<Backlink>>,
}

impl BacklinkGraph {
    /// Creates an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Indexes links from a document's text.
    pub fn index_document(&mut self, id: DocumentId, text: &str) {
        let mut links = Vec::new();
        links.extend(parse_markdown_links(id, text));
        links.extend(parse_wiki_links(id, text));
        self.backlinks.insert(id, links);
    }

    /// Removes a document from the graph.
    pub fn remove_document(&mut self, id: DocumentId) {
        self.backlinks.remove(&id);
    }

    /// Returns all backlinks pointing TO `target_path` from any document.
    #[must_use]
    pub fn backlinks_to(&self, target_path: &str) -> Vec<&Backlink> {
        self.backlinks
            .values()
            .flatten()
            .filter(|bl| bl.target_path == target_path)
            .collect()
    }

    /// Returns all outgoing links FROM `source` document.
    #[must_use]
    pub fn links_from(&self, source: DocumentId) -> &[Backlink] {
        self.backlinks
            .get(&source)
            .map_or(&[], std::vec::Vec::as_slice)
    }

    /// Total number of links in the graph.
    #[must_use]
    pub fn link_count(&self) -> usize {
        self.backlinks.values().map(std::vec::Vec::len).sum()
    }
}

// ---------------------------------------------------------------------------
// Link parsing helpers
// ---------------------------------------------------------------------------

/// Finds byte offsets of lines starting with `#` (headings).
fn extract_heading_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    for (offset, _) in text.match_indices('\n') {
        let line_start = offset + 1;
        if text[line_start..].starts_with('#') {
            starts.push(line_start);
        }
    }
    // Check first line
    if text.starts_with('#') {
        starts.insert(0, 0);
    }
    starts
}

/// Finds the nearest preceding heading for a byte offset.
fn nearest_heading(text: &str, heading_starts: &[usize], offset: usize) -> Option<String> {
    let start = heading_starts
        .iter()
        .rev()
        .find(|&&hs| hs <= offset)
        .copied()?;
    let line_end = text[start..].find('\n').map_or(text.len(), |e| start + e);
    let heading = &text[start..line_end];
    // Strip leading `#`s and whitespace
    Some(heading.trim_start_matches('#').trim().to_owned())
}

/// Creates a short snippet around a match.
fn make_snippet(text: &str, pos: usize, len: usize) -> String {
    let start = pos.saturating_sub(MAX_SNIPPET_BYTES / 3);
    let end = (pos + len + MAX_SNIPPET_BYTES / 3).min(text.len());
    let raw = &text[start..end];
    let mut snippet = raw.replace('\n', " ");
    if snippet.len() > MAX_SNIPPET_BYTES {
        snippet.truncate(MAX_SNIPPET_BYTES);
    }
    snippet.trim().to_owned()
}

/// Parses markdown links `[text](path)` from document text.
/// Case-insensitive suffix check (no allocation).
fn ends_with_ci(haystack: &str, needle: &str) -> bool {
    haystack.len() >= needle.len()
        && haystack[haystack.len() - needle.len()..].eq_ignore_ascii_case(needle)
}

fn parse_markdown_links(source: DocumentId, text: &str) -> Vec<Backlink> {
    let mut links = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find `[`
        if bytes[i] == b'[' {
            // Find closing `]`
            if let Some(bracket_end) = text[i + 1..].find(']') {
                let label = &text[i + 1..i + 1 + bracket_end];
                let after = i + 1 + bracket_end + 1; // Skip `]`
                if after < bytes.len() && bytes[after] == b'(' {
                    // Find closing `)`
                    if let Some(paren_end) = text[after + 1..].find(')') {
                        let path = &text[after + 1..after + 1 + paren_end];
                        // Only accept .md paths (basic filter)
                        if ends_with_ci(path, ".md") || ends_with_ci(path, ".markdown") {
                            links.push(Backlink {
                                source,
                                target_path: path.to_owned(),
                                label: label.to_owned(),
                                byte_offset: i,
                            });
                        }
                        i = after + 1 + paren_end + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    links
}

/// Parses wiki-links `[[path]]` from document text.
fn parse_wiki_links(source: DocumentId, text: &str) -> Vec<Backlink> {
    let mut links = Vec::new();
    let mut start = 0;
    while let Some(pos) = text[start..].find("[[") {
        let abs = start + pos;
        if let Some(end_rel) = text[abs + 2..].find("]]") {
            let end = abs + 2 + end_rel;
            let path = &text[abs + 2..end];
            if !path.is_empty() {
                links.push(Backlink {
                    source,
                    target_path: path.to_owned(),
                    label: path.to_owned(),
                    byte_offset: abs,
                });
            }
            start = end + 2;
        } else {
            break;
        }
    }
    links
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_substring_case_insensitive() {
        let mut idx = SearchIndex::new();
        idx.index_document(DocumentId::ROOT, "# Hello World\nHello again", 1);
        let results = idx.search("hello");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].byte_offset, 2); // After "# "
        assert_eq!(results[1].byte_offset, 14); // After "\n"
    }

    #[test]
    fn search_empty_query_returns_nothing() {
        let mut idx = SearchIndex::new();
        idx.index_document(DocumentId::ROOT, "test", 1);
        assert!(idx.search("").is_empty());
    }

    #[test]
    fn search_includes_heading_context() {
        let mut idx = SearchIndex::new();
        idx.index_document(DocumentId::ROOT, "# My Heading\n\nSome content here", 1);
        let results = idx.search("content");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].heading.as_deref(), Some("My Heading"));
    }

    #[test]
    fn search_ranks_active_document_first() {
        let mut idx = SearchIndex::new();
        idx.index_document(DocumentId::ROOT, "match", 1);
        idx.index_document(DocumentId::new(1), "match", 2);
        idx.set_context(
            DocumentId::new(1),
            vec![DocumentId::new(1), DocumentId::ROOT],
        );

        let results = idx.search("match");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].document_id, DocumentId::new(1)); // active first
        assert_eq!(results[1].document_id, DocumentId::ROOT);
    }

    #[test]
    fn search_caps_matches_per_document() {
        let mut idx = SearchIndex::new();
        let text = "x ".repeat(100);
        idx.index_document(DocumentId::ROOT, &text, 1);
        let results = idx.search("x");
        assert!(results.len() <= 50);
    }

    #[test]
    fn search_snippet_is_bounded() {
        let mut idx = SearchIndex::new();
        let text = format!("{}target{}", "a".repeat(500), "b".repeat(500));
        idx.index_document(DocumentId::ROOT, &text, 1);
        let results = idx.search("target");
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.len() <= MAX_SNIPPET_BYTES);
    }

    #[test]
    fn index_remove_document() {
        let mut idx = SearchIndex::new();
        idx.index_document(DocumentId::ROOT, "test", 1);
        assert_eq!(idx.len(), 1);
        idx.remove_document(DocumentId::ROOT);
        assert!(idx.is_empty());
    }

    #[test]
    fn backlink_graph_parses_markdown_links() {
        let mut graph = BacklinkGraph::new();
        graph.index_document(DocumentId::ROOT, "See [my doc](notes.md) for details");
        assert_eq!(graph.link_count(), 1);
        let backlinks = graph.backlinks_to("notes.md");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].label, "my doc");
    }

    #[test]
    fn backlink_graph_parses_wiki_links() {
        let mut graph = BacklinkGraph::new();
        graph.index_document(DocumentId::ROOT, "Related: [[daily-notes]]");
        assert_eq!(graph.link_count(), 1);
        let backlinks = graph.backlinks_to("daily-notes");
        assert_eq!(backlinks.len(), 1);
    }

    #[test]
    fn backlink_graph_ignores_non_md_links() {
        let mut graph = BacklinkGraph::new();
        graph.index_document(DocumentId::ROOT, "Visit [site](https://example.com)");
        assert_eq!(graph.link_count(), 0);
    }

    #[test]
    fn backlink_graph_links_from() {
        let mut graph = BacklinkGraph::new();
        graph.index_document(DocumentId::ROOT, "[a](a.md) [b](b.md)");
        let links = graph.links_from(DocumentId::ROOT);
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn backlink_graph_remove_document() {
        let mut graph = BacklinkGraph::new();
        graph.index_document(DocumentId::ROOT, "[a](a.md)");
        assert_eq!(graph.link_count(), 1);
        graph.remove_document(DocumentId::ROOT);
        assert_eq!(graph.link_count(), 0);
    }

    #[test]
    fn heading_extraction_handles_first_line() {
        let starts = extract_heading_starts("# Title\nbody\n## Sub");
        assert_eq!(starts, vec![0, 13]);
    }

    #[test]
    fn nearest_heading_finds_preceding() {
        let text = "# A\n\n# B\ntext";
        let starts = extract_heading_starts(text);
        let heading = nearest_heading(text, &starts, 9);
        assert_eq!(heading.as_deref(), Some("B"));
    }
}
