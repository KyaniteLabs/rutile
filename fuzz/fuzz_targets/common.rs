const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 96 * 1024 * 1024;
const BLOCK_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy)]
struct Block {
    revision: u64,
    start: usize,
    end: usize,
    ordinal: u32,
}

pub fn assert_shared_contracts(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    if source.len() > MAX_SOURCE_BYTES {
        return;
    }
    let revision = 17;
    let blocks = source_blocks(source, revision);
    assert!(!blocks.is_empty());
    for (ordinal, block) in blocks.iter().enumerate() {
        assert_eq!(block.revision, revision);
        assert_eq!(block.ordinal as usize, ordinal);
        assert!(block.start <= block.end && block.end <= source.len());
        assert!(source.is_char_boundary(block.start));
        assert!(source.is_char_boundary(block.end));
    }
    assert!(blocks.windows(2).all(|pair| pair[0].end <= pair[1].start));

    let page = bounded_render(source);
    assert!(page.len() <= MAX_PAGE_BYTES);
    assert!(std::str::from_utf8(page.as_bytes()).is_ok());
    assert_generated_node_allowlist(&page);
}

fn source_blocks(source: &str, revision: u64) -> Vec<Block> {
    if source.is_empty() {
        return vec![Block {
            revision,
            start: 0,
            end: 0,
            ordinal: 0,
        }];
    }
    let mut blocks = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let target = (start + BLOCK_BYTES).min(source.len());
        let mut end = target;
        while end > start && !source.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = source[start..]
                .char_indices()
                .nth(1)
                .map(|(offset, _)| start + offset)
                .unwrap_or(source.len());
        }
        blocks.push(Block {
            revision,
            start,
            end,
            ordinal: blocks.len() as u32,
        });
        start = end;
    }
    blocks
}

fn bounded_render(source: &str) -> String {
    let mut page = String::from("<main><p>");
    for character in source.chars() {
        let escaped = match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => {
                page.push(character);
                continue;
            }
        };
        if page.len() + escaped.len() + 11 > MAX_PAGE_BYTES {
            break;
        }
        page.push_str(escaped);
    }
    page.push_str("</p></main>");
    page
}

fn assert_generated_node_allowlist(page: &str) {
    assert!(page.starts_with("<main><p>"));
    assert!(page.ends_with("</p></main>"));
    let interior = &page[9..page.len() - 11];
    assert!(!interior.contains('<'));
    assert!(!interior.contains("href="));
    assert!(!interior.contains("src="));
    assert!(!interior.contains("style="));
    assert!(!interior.contains("onload="));
}
