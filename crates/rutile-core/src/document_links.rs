//! Bounded link metadata from in-memory Markdown. No file or URL is opened.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

/// Maximum link records retained from one document.
pub const MAX_DOCUMENT_LINKS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentLink {
    /// Local destination, with any fragment removed; not filesystem-resolved.
    pub target: String,
    pub label: String,
    pub byte_offset: usize,
}

/// Extracts local Markdown and wiki links using the vendored Markdown parser.
/// Code, images, remote URLs, and raw HTML do not become document links.
#[must_use]
pub fn document_links(text: &str) -> Vec<DocumentLink> {
    if text.len() > crate::MAX_DOCUMENT_BYTES {
        return Vec::new();
    }
    let mut links = Vec::new();
    let mut pending: Option<DocumentLink> = None;
    for (event, range) in Parser::new_ext(text, Options::ENABLE_WIKILINKS).into_offset_iter() {
        match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                pending = local_target(&dest_url, matches!(link_type, LinkType::WikiLink { .. }))
                    .map(|target| DocumentLink {
                        target,
                        label: String::new(),
                        byte_offset: range.start,
                    });
            }
            Event::Text(value) | Event::Code(value) => {
                if let Some(link) = pending.as_mut() {
                    for ch in value.chars() {
                        if link.label.len() + ch.len_utf8() > 200 {
                            break;
                        }
                        link.label.push(ch);
                    }
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some(link) = pending.take() {
                    links.push(link);
                    if links.len() == MAX_DOCUMENT_LINKS {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    links
}

fn local_target(destination: &str, wiki: bool) -> Option<String> {
    let path = destination.split('#').next()?.trim();
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with("//")
        || path.contains([':', '\\', '?'])
        || path.chars().any(char::is_control)
    {
        return None;
    }
    let markdown = path.rsplit_once('.').is_some_and(|(_, suffix)| {
        suffix.eq_ignore_ascii_case("md") || suffix.eq_ignore_ascii_case("markdown")
    });
    (wiki || markdown).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_metadata_preserves_unicode_offsets_and_visible_labels() {
        let text = "🪶 [**Café**](notes/café.md#section) and [[plan|The plan]]";
        let links = document_links(text);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, "notes/café.md");
        assert_eq!(links[0].label, "Café");
        assert_eq!(&text[links[0].byte_offset..links[0].byte_offset + 1], "[");
        assert_eq!(links[1].target, "plan");
        assert_eq!(links[1].label, "The plan");
    }

    #[test]
    fn links_and_labels_have_explicit_resource_bounds() {
        let links = document_links(&"[x](a.md) ".repeat(MAX_DOCUMENT_LINKS + 1));
        assert_eq!(links.len(), MAX_DOCUMENT_LINKS);
        let links = document_links(&format!("[{}](a.md)", "🪶".repeat(100)));
        assert_eq!(links[0].label.len(), 200);
        assert!(document_links(&"x".repeat(crate::MAX_DOCUMENT_BYTES + 1)).is_empty());
    }

    #[test]
    fn local_metadata_never_admits_remote_destinations() {
        for target in [
            "https://host/a.md",
            "//host/a.md",
            "file:a.md",
            "a.md?query",
        ] {
            assert!(document_links(&format!("[x]({target}) [[{target}]]")).is_empty());
        }
        assert_eq!(document_links("[x](/notes/a.md)")[0].target, "/notes/a.md");
    }
}
