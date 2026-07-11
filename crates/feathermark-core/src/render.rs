//! Revisioned Markdown rendering and source-anchor construction.

use std::collections::HashMap;
use std::ops::Range;

use feathermark_types::{Revision, SafeLinkTarget};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};
use thiserror::Error;

use crate::security::{Attribute, Class, SafeNode, SourceAttributes, Tag as SafeTag};

pub const MAX_GENERATED_BODY_BYTES: usize = 80 * 1024 * 1024;
pub const MAX_RENDERED_PAGE_BYTES: usize = 96 * 1024 * 1024;
pub const MAX_SOURCE_BLOCK_BYTES: usize = 32 * 1024;

/// Maximum block/inline nesting the renderer will build into a `SafeNode` tree.
///
/// The tree is traversed recursively when serializing to HTML, when
/// partitioning inline segments, and when the tree is dropped. Hostile Markdown
/// (e.g. a document of `"> "` repeated) can request effectively unbounded
/// nesting; a document at the 20 MiB cap yields millions of levels, overflowing
/// any thread stack. With `panic = "abort"` a stack overflow aborts the whole
/// process, so untrusted input must never reach that depth. This cap bounds the
/// tree depth to a value far below any stack-overflow threshold while remaining
/// orders of magnitude beyond any realistic document.
pub const MAX_RENDER_NESTING_DEPTH: usize = 1024;

const PREVIEW_CSS: &str = "feathermark://preview/v1/assets/preview.css";
const PREVIEW_BRIDGE: &str = "feathermark://preview/v1/assets/bridge.js";
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderLimits {
    pub max_body_bytes: usize,
    pub max_page_bytes: usize,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: MAX_GENERATED_BODY_BYTES,
            max_page_bytes: MAX_RENDERED_PAGE_BYTES,
        }
    }
}

impl RenderLimits {
    pub fn clamped(self) -> Self {
        Self {
            max_body_bytes: self.max_body_bytes.min(MAX_GENERATED_BODY_BYTES),
            max_page_bytes: self.max_page_bytes.min(MAX_RENDERED_PAGE_BYTES),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedPage {
    pub revision: Revision,
    pub body: String,
    pub page: String,
    pub blocks: Vec<SourceBlock>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceBlockKind {
    Heading,
    Paragraph,
    CodeBlock,
    TableRow,
    FootnoteDefinition,
    ThematicBreak,
    LeafFallback,
    Continuation,
}

impl SourceBlockKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Heading => "heading",
            Self::Paragraph => "paragraph",
            Self::CodeBlock => "code-block",
            Self::TableRow => "table-row",
            Self::FootnoteDefinition => "footnote-definition",
            Self::ThematicBreak => "thematic-break",
            Self::LeafFallback => "leaf-fallback",
            Self::Continuation => "continuation",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBlock {
    pub revision: Revision,
    pub start: usize,
    pub end: usize,
    pub ordinal: u32,
    pub depth: u16,
    pub kind: SourceBlockKind,
    pub dom_id: String,
    pub segment_index: u16,
    pub segment_count: u16,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RenderError {
    #[error("generated preview exceeds its byte cap")]
    PreviewTooLarge,
    #[error("parser produced an invalid source range")]
    InvalidSourceRange,
    #[error("source block count exceeds representable limits")]
    TooManySourceBlocks,
    #[error("document nesting exceeds the render depth cap")]
    NestingTooDeep,
}

pub fn render_markdown(source: &str, revision: Revision) -> Result<RenderedPage, RenderError> {
    render_markdown_with_limits(source, revision, RenderLimits::default())
}

pub fn render_markdown_with_limits(
    source: &str,
    revision: Revision,
    limits: RenderLimits,
) -> Result<RenderedPage, RenderError> {
    let limits = limits.clamped();
    let blocks = build_source_blocks(source, revision)?;
    let block_index = BlockIndex::new(&blocks);
    let nodes = render_nodes(source, &block_index)?;
    let body = SafeNode::element(SafeTag::Main, vec![], nodes).to_html();
    if body.len() > limits.max_body_bytes {
        return Err(RenderError::PreviewTooLarge);
    }

    let page = complete_page(revision, &body);
    if page.len() > limits.max_page_bytes {
        return Err(RenderError::PreviewTooLarge);
    }
    Ok(RenderedPage {
        revision,
        body,
        page,
        blocks,
    })
}

pub fn build_source_blocks(
    source: &str,
    revision: Revision,
) -> Result<Vec<SourceBlock>, RenderError> {
    let mut candidates = Vec::new();
    let mut stack: Vec<BlockFrame> = Vec::new();
    let mut event_ordinal = 0_u32;

    for (event, range) in markdown_parser(source).into_offset_iter() {
        validate_range(source, &range)?;
        for frame in &mut stack {
            frame.include(&range);
        }

        match event {
            Event::Start(tag) => {
                if stack.len() >= MAX_RENDER_NESTING_DEPTH {
                    return Err(RenderError::NestingTooDeep);
                }
                let suppressed = stack.iter().any(|frame| {
                    matches!(
                        frame.anchor,
                        Some(
                            SourceBlockKind::CodeBlock
                                | SourceBlockKind::TableRow
                                | SourceBlockKind::FootnoteDefinition
                        )
                    )
                });
                let anchor = if suppressed { None } else { anchor_kind(&tag) };
                stack.push(BlockFrame::new(
                    range,
                    anchor,
                    is_container(&tag),
                    stack.len().saturating_add(1),
                    event_ordinal,
                )?);
            }
            Event::End(_) => {
                if let Some(frame) = stack.pop() {
                    let candidate = if let Some(kind) = frame.anchor {
                        Some(frame.candidate(kind))
                    } else if frame.container && frame.descendant_leaves == 0 {
                        Some(frame.candidate(SourceBlockKind::LeafFallback))
                    } else {
                        None
                    };
                    if let Some(candidate) = candidate {
                        for ancestor in &mut stack {
                            ancestor.descendant_leaves =
                                ancestor.descendant_leaves.saturating_add(1);
                        }
                        candidates.push(candidate);
                    }
                }
            }
            Event::Rule => candidates.push(Candidate {
                start: range.start,
                end: range.end,
                depth: u16::try_from(stack.len().saturating_add(1)).unwrap_or(u16::MAX),
                kind: SourceBlockKind::ThematicBreak,
                placement_kind: SourceBlockKind::ThematicBreak,
                event_ordinal,
            }),
            _ => {}
        }
        event_ordinal = event_ordinal.saturating_add(1);
    }

    if candidates.is_empty() {
        candidates.clear();
        candidates.push(Candidate {
            start: 0,
            end: source.len(),
            depth: 0,
            kind: SourceBlockKind::LeafFallback,
            placement_kind: SourceBlockKind::LeafFallback,
            event_ordinal: 0,
        });
    }

    let candidates = resolve_candidates(candidates);
    let mut segments = Vec::new();
    for candidate in candidates {
        segments.extend(split_candidate(source, candidate)?);
    }
    segments.sort_by_key(|segment| {
        (
            segment.start,
            segment.end,
            segment.event_ordinal,
            segment.segment_index,
        )
    });

    let mut blocks = Vec::with_capacity(segments.len());
    for (ordinal, segment) in segments.into_iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| RenderError::TooManySourceBlocks)?;
        blocks.push(SourceBlock {
            revision,
            start: segment.start,
            end: segment.end,
            ordinal,
            depth: segment.depth,
            kind: segment.kind,
            dom_id: format!("sb-{revision}-{ordinal}"),
            segment_index: segment.segment_index,
            segment_count: segment.segment_count,
        });
    }
    validate_source_blocks(source, revision, &blocks)?;
    Ok(blocks)
}

pub fn validate_source_blocks(
    source: &str,
    revision: Revision,
    blocks: &[SourceBlock],
) -> Result<(), RenderError> {
    if blocks.is_empty() {
        return Err(RenderError::InvalidSourceRange);
    }
    for (ordinal, block) in blocks.iter().enumerate() {
        if block.revision != revision
            || block.start > block.end
            || block.end > source.len()
            || !source.is_char_boundary(block.start)
            || !source.is_char_boundary(block.end)
            || block.ordinal as usize != ordinal
            || block.dom_id != format!("sb-{revision}-{ordinal}")
            || block.segment_count == 0
            || block.segment_index >= block.segment_count
            || (block.start == block.end && block.kind != SourceBlockKind::LeafFallback)
            || block.end.saturating_sub(block.start) > MAX_SOURCE_BLOCK_BYTES
        {
            return Err(RenderError::InvalidSourceRange);
        }
        if let Some(previous) = ordinal.checked_sub(1).and_then(|index| blocks.get(index)) {
            if previous.start > block.start || previous.end > block.start {
                return Err(RenderError::InvalidSourceRange);
            }
        }
    }

    let mut group_start = 0;
    while group_start < blocks.len() {
        let first = &blocks[group_start];
        if first.segment_index != 0 || first.kind == SourceBlockKind::Continuation {
            return Err(RenderError::InvalidSourceRange);
        }
        let count = first.segment_count as usize;
        if group_start + count > blocks.len() {
            return Err(RenderError::InvalidSourceRange);
        }
        let group = &blocks[group_start..group_start + count];
        for (index, block) in group.iter().enumerate() {
            if block.segment_index as usize != index
                || block.segment_count != first.segment_count
                || block.depth != first.depth
                || (index > 0 && block.kind != SourceBlockKind::Continuation)
                || (index > 0 && group[index - 1].end != block.start)
            {
                return Err(RenderError::InvalidSourceRange);
            }
        }
        group_start += count;
    }
    Ok(())
}

fn markdown_parser(source: &str) -> Parser<'_> {
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_TASKLISTS;
    Parser::new_ext(source, options)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct OwnerKey {
    start: usize,
    end: usize,
    kind: SourceBlockKind,
}

struct BlockIndex<'a> {
    blocks: &'a [SourceBlock],
    owners: HashMap<OwnerKey, Range<usize>>,
}

impl<'a> BlockIndex<'a> {
    fn new(blocks: &'a [SourceBlock]) -> Self {
        let mut owners = HashMap::with_capacity(blocks.len());
        let mut start = 0;
        while start < blocks.len() {
            let count = blocks[start].segment_count as usize;
            let end = (start + count).min(blocks.len());
            let owner_end = blocks[end - 1].end;
            owners.insert(
                OwnerKey {
                    start: blocks[start].start,
                    end: owner_end,
                    kind: blocks[start].kind,
                },
                start..end,
            );
            start = end;
        }
        Self { blocks, owners }
    }

    fn get(&self, owner: &Range<usize>, kind: SourceBlockKind) -> &'a [SourceBlock] {
        self.owners
            .get(&OwnerKey {
                start: owner.start,
                end: owner.end,
                kind,
            })
            .map_or(&[], |range| &self.blocks[range.clone()])
    }

    fn first(&self) -> Option<&'a SourceBlock> {
        self.blocks.first()
    }
}

fn render_nodes(source: &str, blocks: &BlockIndex<'_>) -> Result<Vec<SafeNode>, RenderError> {
    let mut roots = Vec::new();
    let mut stack: Vec<NodeFrame> = Vec::new();

    for (event, range) in markdown_parser(source).into_offset_iter() {
        validate_range(source, &range)?;
        for frame in &mut stack {
            frame.include(&range);
        }
        match event {
            Event::Start(tag) => {
                if stack.len() >= MAX_RENDER_NESTING_DEPTH {
                    return Err(RenderError::NestingTooDeep);
                }
                let in_table_head = stack
                    .iter()
                    .any(|frame| matches!(frame.kind, NodeFrameKind::TableHead));
                let cell_alignment = if matches!(tag, Tag::TableCell) {
                    stack
                        .iter_mut()
                        .rev()
                        .find_map(NodeFrame::next_table_alignment)
                } else {
                    None
                };
                stack.push(NodeFrame::new(tag, range, in_table_head, cell_alignment));
            }
            Event::End(_end) => {
                if let Some(frame) = stack.pop() {
                    let node = frame.finish(blocks);
                    append_node(node, &mut stack, &mut roots);
                }
            }
            Event::Text(text) => append_text(&text, range, &mut stack, &mut roots),
            Event::Code(text) => {
                let text = text.into_string();
                let text_range = source[range.clone()]
                    .find(&text)
                    .map(|offset| {
                        let start = range.start + offset;
                        start..start + text.len()
                    })
                    .unwrap_or_else(|| range.clone());
                append_node(
                    SafeNode::element(
                        SafeTag::Code,
                        vec![],
                        vec![SafeNode::text(text).with_source_range(text_range)],
                    ),
                    &mut stack,
                    &mut roots,
                );
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                append_text(&html, range, &mut stack, &mut roots)
            }
            Event::SoftBreak => append_text("\n", range, &mut stack, &mut roots),
            Event::HardBreak => append_node(
                SafeNode::element(SafeTag::Br, vec![], vec![]).with_source_range(range),
                &mut stack,
                &mut roots,
            ),
            Event::Rule => {
                let block = blocks.get(&range, SourceBlockKind::ThematicBreak).first();
                let node = block.map_or_else(
                    || SafeNode::element(SafeTag::Hr, vec![], vec![]),
                    |block| SafeNode::thematic_break(source_attributes(block)),
                );
                append_node(node.with_source_range(range), &mut stack, &mut roots);
            }
            Event::FootnoteReference(name) => append_node(
                SafeNode::element(
                    SafeTag::Sup,
                    vec![],
                    vec![SafeNode::text(name.into_string()).with_source_range(range.clone())],
                )
                .with_source_range(range),
                &mut stack,
                &mut roots,
            ),
            Event::TaskListMarker(checked) => append_node(
                SafeNode::element(SafeTag::Span, vec![Attribute::TaskChecked(checked)], vec![])
                    .with_source_range(range),
                &mut stack,
                &mut roots,
            ),
            _ => {}
        }
    }

    if source.is_empty() {
        if let Some(block) = blocks.first() {
            roots.push(SafeNode::source_fallback(source_attributes(block)));
        }
    }
    Ok(roots)
}

fn append_text(
    text: &str,
    range: Range<usize>,
    stack: &mut [NodeFrame],
    roots: &mut Vec<SafeNode>,
) {
    if let Some(image) = stack.iter_mut().rev().find(|frame| frame.is_image()) {
        image.alt.push_str(text);
    } else {
        append_node(SafeNode::text(text).with_source_range(range), stack, roots);
    }
}

fn append_node(node: SafeNode, stack: &mut [NodeFrame], roots: &mut Vec<SafeNode>) {
    if let Some(frame) = stack.last_mut() {
        if !frame.is_image() {
            frame.children.push(node);
        }
    } else {
        roots.push(node);
    }
}

#[derive(Debug)]
struct NodeFrame {
    kind: NodeFrameKind,
    start: usize,
    end: usize,
    children: Vec<SafeNode>,
    alt: String,
}

#[derive(Debug)]
enum NodeFrameKind {
    Paragraph,
    Heading(HeadingLevel),
    Blockquote,
    CodeBlock(Option<String>),
    List(Option<u64>),
    Item,
    FootnoteDefinition,
    Table {
        alignments: Vec<Alignment>,
        next_cell: usize,
    },
    TableHead,
    TableRow,
    TableCell(bool, Alignment),
    Emphasis,
    Strong,
    Strikethrough,
    Link(Option<SafeLinkTarget>),
    Image,
    Transparent,
}

impl NodeFrame {
    fn new(
        tag: Tag<'_>,
        range: Range<usize>,
        in_table_head: bool,
        cell_alignment: Option<Alignment>,
    ) -> Self {
        let kind = match tag {
            Tag::Paragraph => NodeFrameKind::Paragraph,
            Tag::Heading { level, .. } => NodeFrameKind::Heading(level),
            Tag::BlockQuote(_) => NodeFrameKind::Blockquote,
            Tag::CodeBlock(kind) => NodeFrameKind::CodeBlock(match kind {
                CodeBlockKind::Indented => None,
                CodeBlockKind::Fenced(info) => info
                    .split_ascii_whitespace()
                    .next()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            }),
            Tag::List(start) => NodeFrameKind::List(start),
            Tag::Item => NodeFrameKind::Item,
            Tag::FootnoteDefinition(_) => NodeFrameKind::FootnoteDefinition,
            Tag::Table(alignments) => NodeFrameKind::Table {
                alignments,
                next_cell: 0,
            },
            Tag::TableHead => NodeFrameKind::TableHead,
            Tag::TableRow => NodeFrameKind::TableRow,
            Tag::TableCell => {
                NodeFrameKind::TableCell(in_table_head, cell_alignment.unwrap_or(Alignment::None))
            }
            Tag::Emphasis => NodeFrameKind::Emphasis,
            Tag::Strong => NodeFrameKind::Strong,
            Tag::Strikethrough => NodeFrameKind::Strikethrough,
            Tag::Link { dest_url, .. } => {
                NodeFrameKind::Link(SafeLinkTarget::parse(dest_url.as_ref()).ok())
            }
            Tag::Image { .. } => NodeFrameKind::Image,
            _ => NodeFrameKind::Transparent,
        };
        Self {
            kind,
            start: range.start,
            end: range.end,
            children: vec![],
            alt: String::new(),
        }
    }

    fn include(&mut self, range: &Range<usize>) {
        self.start = self.start.min(range.start);
        self.end = self.end.max(range.end);
    }

    fn is_image(&self) -> bool {
        matches!(self.kind, NodeFrameKind::Image)
    }

    fn next_table_alignment(&mut self) -> Option<Alignment> {
        let NodeFrameKind::Table {
            alignments,
            next_cell,
        } = &mut self.kind
        else {
            return None;
        };
        if alignments.is_empty() {
            return Some(Alignment::None);
        }
        let alignment = alignments[*next_cell % alignments.len()];
        *next_cell = next_cell.saturating_add(1);
        Some(alignment)
    }

    fn finish(self, blocks: &BlockIndex<'_>) -> SafeNode {
        let range = self.start..self.end;
        let node = match self.kind {
            NodeFrameKind::Paragraph => SafeNode::paragraph(wrap_inline_segments(
                &range,
                self.children,
                blocks,
                SourceBlockKind::Paragraph,
                false,
            )),
            NodeFrameKind::Heading(level) => SafeNode::element(
                heading_tag(level),
                vec![],
                wrap_inline_segments(
                    &range,
                    self.children,
                    blocks,
                    SourceBlockKind::Heading,
                    false,
                ),
            ),
            NodeFrameKind::Blockquote => {
                SafeNode::element(SafeTag::Blockquote, vec![], self.children)
            }
            NodeFrameKind::CodeBlock(language) => {
                let mut attributes = vec![];
                if let Some(language) = language {
                    attributes.push(Attribute::CodeLanguage(language));
                }
                let children = wrap_inline_segments(
                    &range,
                    self.children,
                    blocks,
                    SourceBlockKind::CodeBlock,
                    true,
                );
                SafeNode::element(
                    SafeTag::Pre,
                    vec![],
                    vec![SafeNode::element(SafeTag::Code, attributes, children)],
                )
            }
            NodeFrameKind::List(start) => SafeNode::element(
                if start.is_some() {
                    SafeTag::Ol
                } else {
                    SafeTag::Ul
                },
                start.map(Attribute::OrderedStart).into_iter().collect(),
                self.children,
            ),
            NodeFrameKind::Item => SafeNode::element(SafeTag::Li, vec![], self.children),
            NodeFrameKind::FootnoteDefinition => SafeNode::element(
                SafeTag::Section,
                vec![],
                wrap_inline_segments(
                    &range,
                    self.children,
                    blocks,
                    SourceBlockKind::FootnoteDefinition,
                    false,
                ),
            ),
            NodeFrameKind::Table { .. } => SafeNode::element(SafeTag::Table, vec![], self.children),
            NodeFrameKind::TableHead => SafeNode::element(
                SafeTag::Thead,
                vec![],
                vec![render_table_row(&range, self.children, blocks)],
            ),
            NodeFrameKind::TableRow => render_table_row(&range, self.children, blocks),
            NodeFrameKind::TableCell(heading, alignment) => SafeNode::element(
                if heading { SafeTag::Th } else { SafeTag::Td },
                alignment_attribute(alignment).into_iter().collect(),
                self.children,
            ),
            NodeFrameKind::Emphasis => SafeNode::element(SafeTag::Em, vec![], self.children),
            NodeFrameKind::Strong => SafeNode::element(SafeTag::Strong, vec![], self.children),
            NodeFrameKind::Strikethrough => SafeNode::element(SafeTag::Del, vec![], self.children),
            NodeFrameKind::Link(Some(target)) => SafeNode::link(target, self.children),
            NodeFrameKind::Link(None) | NodeFrameKind::Transparent => {
                SafeNode::element(SafeTag::Span, vec![], self.children)
            }
            NodeFrameKind::Image => SafeNode::image_alt(&self.alt),
        };
        node.with_source_range(range)
    }
}

fn wrap_inline_segments(
    owner: &Range<usize>,
    children: Vec<SafeNode>,
    blocks: &BlockIndex<'_>,
    placement: SourceBlockKind,
    code: bool,
) -> Vec<SafeNode> {
    let matching = blocks.get(owner, placement);
    match matching {
        [] => children,
        [block] => vec![SafeNode::source_segment(
            source_attributes(block),
            children,
            code,
        )],
        many => many
            .iter()
            .map(|block| {
                let range = block.start..block.end;
                SafeNode::source_segment(
                    source_attributes(block),
                    children
                        .iter()
                        .filter_map(|child| child.partition(&range))
                        .collect(),
                    code,
                )
            })
            .collect(),
    }
}

fn render_table_row(
    owner: &Range<usize>,
    mut cells: Vec<SafeNode>,
    blocks: &BlockIndex<'_>,
) -> SafeNode {
    let matching = blocks.get(owner, SourceBlockKind::TableRow);
    if !matching.is_empty() && !cells.is_empty() {
        let mut assignments: Vec<Vec<SourceBlock>> = vec![vec![]; cells.len()];
        for block in matching {
            let cell_index = cells
                .iter()
                .position(|cell| {
                    cell.source_range()
                        .is_some_and(|range| block.start >= range.start && block.start < range.end)
                })
                .unwrap_or(0);
            assignments[cell_index].push(block.clone());
        }
        for (cell, assigned) in cells.iter_mut().zip(assignments) {
            if !assigned.is_empty() {
                cell.replace_children_with_source_segments(&assigned);
            }
        }
    }
    SafeNode::element(SafeTag::Tr, vec![], cells)
}

fn alignment_attribute(alignment: Alignment) -> Option<Attribute> {
    match alignment {
        Alignment::None => None,
        Alignment::Left => Some(Attribute::Class(Class::AlignLeft)),
        Alignment::Center => Some(Attribute::Class(Class::AlignCenter)),
        Alignment::Right => Some(Attribute::Class(Class::AlignRight)),
    }
}

fn source_attributes(block: &SourceBlock) -> SourceAttributes {
    SourceAttributes::new(
        block.revision,
        block.start,
        block.end,
        block.ordinal,
        block.kind,
    )
}

fn heading_tag(level: HeadingLevel) -> SafeTag {
    match level {
        HeadingLevel::H1 => SafeTag::H1,
        HeadingLevel::H2 => SafeTag::H2,
        HeadingLevel::H3 => SafeTag::H3,
        HeadingLevel::H4 => SafeTag::H4,
        HeadingLevel::H5 => SafeTag::H5,
        HeadingLevel::H6 => SafeTag::H6,
    }
}

fn complete_page(revision: Revision, body: &str) -> String {
    format!(
        "<!doctype html><html data-feathermark-revision=\"{revision}\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\"><link rel=\"stylesheet\" href=\"{PREVIEW_CSS}\"><script defer src=\"{PREVIEW_BRIDGE}\"></script></head><body>{body}</body></html>"
    )
}

#[derive(Clone, Debug)]
struct BlockFrame {
    start: usize,
    end: usize,
    anchor: Option<SourceBlockKind>,
    container: bool,
    depth: u16,
    event_ordinal: u32,
    descendant_leaves: u32,
}

impl BlockFrame {
    fn new(
        range: Range<usize>,
        anchor: Option<SourceBlockKind>,
        container: bool,
        depth: usize,
        event_ordinal: u32,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            start: range.start,
            end: range.end,
            anchor,
            container,
            depth: u16::try_from(depth).map_err(|_| RenderError::TooManySourceBlocks)?,
            event_ordinal,
            descendant_leaves: 0,
        })
    }

    fn include(&mut self, range: &Range<usize>) {
        self.start = self.start.min(range.start);
        self.end = self.end.max(range.end);
    }

    fn candidate(self, kind: SourceBlockKind) -> Candidate {
        Candidate {
            start: self.start,
            end: self.end,
            depth: self.depth,
            kind,
            placement_kind: kind,
            event_ordinal: self.event_ordinal,
        }
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    start: usize,
    end: usize,
    depth: u16,
    kind: SourceBlockKind,
    placement_kind: SourceBlockKind,
    event_ordinal: u32,
}

#[derive(Clone, Debug)]
struct Segment {
    start: usize,
    end: usize,
    depth: u16,
    kind: SourceBlockKind,
    #[allow(dead_code)]
    placement_kind: SourceBlockKind,
    event_ordinal: u32,
    segment_index: u16,
    segment_count: u16,
}

fn anchor_kind(tag: &Tag<'_>) -> Option<SourceBlockKind> {
    match tag {
        Tag::Heading { .. } => Some(SourceBlockKind::Heading),
        Tag::Paragraph => Some(SourceBlockKind::Paragraph),
        Tag::CodeBlock(_) => Some(SourceBlockKind::CodeBlock),
        Tag::TableHead => Some(SourceBlockKind::TableRow),
        Tag::TableRow => Some(SourceBlockKind::TableRow),
        Tag::FootnoteDefinition(_) => Some(SourceBlockKind::FootnoteDefinition),
        _ => None,
    }
}

fn is_container(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::BlockQuote(_)
            | Tag::List(_)
            | Tag::Item
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::FootnoteDefinition(_)
    )
}

fn resolve_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.sort_by_key(|candidate| {
        (
            std::cmp::Reverse(candidate.depth),
            candidate.end.saturating_sub(candidate.start),
            candidate.event_ordinal,
        )
    });
    let mut retained: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if retained
            .iter()
            .all(|other| !ranges_overlap(&candidate, other))
        {
            retained.push(candidate);
        }
    }
    retained.sort_by_key(|candidate| (candidate.start, candidate.end, candidate.event_ordinal));
    retained
}

fn ranges_overlap(left: &Candidate, right: &Candidate) -> bool {
    if left.start == left.end || right.start == right.end {
        left.start == right.start && left.end == right.end
    } else {
        left.start < right.end && right.start < left.end
    }
}

fn split_candidate(source: &str, candidate: Candidate) -> Result<Vec<Segment>, RenderError> {
    validate_range(source, &(candidate.start..candidate.end))?;
    if candidate.end.saturating_sub(candidate.start) <= MAX_SOURCE_BLOCK_BYTES
        || candidate.start == candidate.end
    {
        return Ok(vec![Segment {
            start: candidate.start,
            end: candidate.end,
            depth: candidate.depth,
            // Zero-width anchors exist (e.g. the empty paragraph pulldown-cmark
            // emits for a whitespace-only line after a link-reference
            // definition); only LeafFallback may be zero-width.
            kind: if candidate.start == candidate.end {
                SourceBlockKind::LeafFallback
            } else {
                candidate.kind
            },
            placement_kind: candidate.placement_kind,
            event_ordinal: candidate.event_ordinal,
            segment_index: 0,
            segment_count: 1,
        }]);
    }

    let mut cuts = vec![candidate.start];
    let mut previous = candidate.start;
    while previous < candidate.end {
        let target = previous
            .saturating_add(MAX_SOURCE_BLOCK_BYTES)
            .min(candidate.end);
        let cut = if target == candidate.end {
            target
        } else {
            choose_cut(source, previous, target, candidate.end)
        };
        if cut <= previous {
            return Err(RenderError::InvalidSourceRange);
        }
        cuts.push(cut);
        previous = cut;
    }
    let segment_count =
        u16::try_from(cuts.len() - 1).map_err(|_| RenderError::TooManySourceBlocks)?;
    let mut result = Vec::with_capacity(segment_count as usize);
    for index in 0..segment_count as usize {
        result.push(Segment {
            start: cuts[index],
            end: cuts[index + 1],
            depth: candidate.depth,
            kind: if index == 0 {
                candidate.kind
            } else {
                SourceBlockKind::Continuation
            },
            placement_kind: candidate.placement_kind,
            event_ordinal: candidate.event_ordinal,
            segment_index: index as u16,
            segment_count,
        });
    }
    Ok(result)
}

fn choose_cut(source: &str, previous: usize, target: usize, end: usize) -> usize {
    if let Some(relative) = source[previous..target].rfind('\n') {
        let cut = previous + relative + 1;
        if cut > previous {
            return cut;
        }
    }
    let mut cut = target;
    while cut > previous && !source.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut > previous {
        return cut;
    }
    source[previous..end]
        .char_indices()
        .nth(1)
        .map_or(end, |(offset, _)| previous + offset)
}

fn validate_range(source: &str, range: &Range<usize>) -> Result<(), RenderError> {
    if range.start > range.end
        || range.end > source.len()
        || !source.is_char_boundary(range.start)
        || !source.is_char_boundary(range.end)
    {
        Err(RenderError::InvalidSourceRange)
    } else {
        Ok(())
    }
}
