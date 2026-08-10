//! Typed, deny-by-default HTML construction.
//!
//! Tag names and attribute names are deliberately private. Rendering code can
//! select only the variants below, so untrusted Markdown never becomes markup.

use std::ops::Range;

use rutile_types::{Revision, SafeLinkTarget};

use crate::render::SourceBlockKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceAttributes {
    revision: Revision,
    start: usize,
    end: usize,
    ordinal: u32,
    kind: SourceBlockKind,
}

impl SourceAttributes {
    pub fn new(
        revision: Revision,
        start: usize,
        end: usize,
        ordinal: u32,
        kind: SourceBlockKind,
    ) -> Self {
        Self {
            revision,
            start,
            end,
            ordinal,
            kind,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafeNode {
    node: Node,
    source: Option<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Node {
    Text(String),
    Element {
        tag: Tag,
        attributes: Vec<Attribute>,
        children: Vec<SafeNode>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tag {
    Main,
    Section,
    Div,
    P,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    Blockquote,
    Pre,
    Code,
    Em,
    Strong,
    Del,
    Ul,
    Ol,
    Li,
    Table,
    Thead,
    Tr,
    Th,
    Td,
    Hr,
    Br,
    Sup,
    A,
    Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Attribute {
    Source(SourceAttributes),
    SourceWrapper(SourceAttributes, SourceWrapperClass),
    Class(Class),
    OrderedStart(u64),
    CodeLanguage(String),
    TaskChecked(bool),
    Link(SafeLinkTarget),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Class {
    ImageAlt,
    AlignLeft,
    AlignCenter,
    AlignRight,
    TaskCheckbox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceWrapperClass {
    Segment,
    CodeSegment,
    Fallback,
}

impl SafeNode {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            node: Node::Text(value.into()),
            source: None,
        }
    }

    pub fn paragraph(children: Vec<Self>) -> Self {
        Self::element(Tag::P, vec![], children)
    }

    pub fn link(target: SafeLinkTarget, children: Vec<Self>) -> Self {
        Self::element(Tag::A, vec![Attribute::Link(target)], children)
    }

    pub fn image_alt(alt: &str) -> Self {
        Self::element(
            Tag::Span,
            vec![Attribute::Class(Class::ImageAlt)],
            vec![Self::text(format!("[image: {alt}]"))],
        )
    }

    pub fn source_segment(source: SourceAttributes, children: Vec<Self>, code: bool) -> Self {
        Self::element(
            Tag::Span,
            vec![Attribute::SourceWrapper(
                source,
                if code {
                    SourceWrapperClass::CodeSegment
                } else {
                    SourceWrapperClass::Segment
                },
            )],
            children,
        )
    }

    pub fn to_html(&self) -> String {
        let mut output = String::new();
        self.write_html(&mut output);
        output
    }

    pub(crate) fn element(tag: Tag, attributes: Vec<Attribute>, children: Vec<Self>) -> Self {
        Self {
            node: Node::Element {
                tag,
                attributes,
                children,
            },
            source: None,
        }
    }

    pub(crate) fn with_source_range(mut self, source: Range<usize>) -> Self {
        self.source = Some(source);
        self
    }

    pub(crate) fn source_range(&self) -> Option<&Range<usize>> {
        self.source.as_ref()
    }

    /// Clone only the typed formatting shells and text that overlap `target`.
    /// No Markdown source bytes are serialized by this operation.
    pub(crate) fn partition(&self, target: &Range<usize>) -> Option<Self> {
        if let Some(source) = &self.source {
            if !overlaps(source, target) {
                return None;
            }
        }
        let node = match &self.node {
            Node::Text(value) => Node::Text(match &self.source {
                Some(source) => partition_text(value, source, target),
                None => value.clone(),
            }),
            Node::Element {
                tag,
                attributes,
                children,
            } => {
                let partitioned: Vec<_> = children
                    .iter()
                    .filter_map(|child| child.partition(target))
                    .collect();
                if !children.is_empty() && partitioned.is_empty() {
                    return None;
                }
                Node::Element {
                    tag: *tag,
                    attributes: attributes.clone(),
                    children: partitioned,
                }
            }
        };
        Some(Self {
            node,
            source: self.source.clone(),
        })
    }

    pub(crate) fn replace_children_with_source_segments(
        &mut self,
        blocks: &[crate::render::SourceBlock],
    ) {
        let Node::Element { children, .. } = &mut self.node else {
            return;
        };
        let original = std::mem::take(children);
        *children = blocks
            .iter()
            .map(|block| {
                let range = block.start..block.end;
                let partition = original
                    .iter()
                    .filter_map(|child| child.partition(&range))
                    .collect();
                Self::source_segment(
                    SourceAttributes::new(
                        block.revision,
                        block.start,
                        block.end,
                        block.ordinal,
                        block.kind,
                    ),
                    partition,
                    false,
                )
            })
            .collect();
    }

    pub(crate) fn source_fallback(source: SourceAttributes) -> Self {
        Self::element(
            Tag::Div,
            vec![Attribute::SourceWrapper(
                source,
                SourceWrapperClass::Fallback,
            )],
            vec![],
        )
    }

    pub(crate) fn thematic_break(source: SourceAttributes) -> Self {
        Self::element(Tag::Hr, vec![Attribute::Source(source)], vec![])
    }

    fn write_html(&self, output: &mut String) {
        match &self.node {
            Node::Text(value) => escape_text(value, output),
            Node::Element {
                tag,
                attributes,
                children,
            } => {
                output.push('<');
                output.push_str(tag.as_str());
                for attribute in attributes {
                    attribute.write_html(output);
                }
                output.push('>');
                if !tag.is_void() {
                    for child in children {
                        child.write_html(output);
                    }
                    output.push_str("</");
                    output.push_str(tag.as_str());
                    output.push('>');
                }
            }
        }
    }
}

impl Tag {
    fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Section => "section",
            Self::Div => "div",
            Self::P => "p",
            Self::H1 => "h1",
            Self::H2 => "h2",
            Self::H3 => "h3",
            Self::H4 => "h4",
            Self::H5 => "h5",
            Self::H6 => "h6",
            Self::Blockquote => "blockquote",
            Self::Pre => "pre",
            Self::Code => "code",
            Self::Em => "em",
            Self::Strong => "strong",
            Self::Del => "del",
            Self::Ul => "ul",
            Self::Ol => "ol",
            Self::Li => "li",
            Self::Table => "table",
            Self::Thead => "thead",
            Self::Tr => "tr",
            Self::Th => "th",
            Self::Td => "td",
            Self::Hr => "hr",
            Self::Br => "br",
            Self::Sup => "sup",
            Self::A => "a",
            Self::Span => "span",
        }
    }

    fn is_void(self) -> bool {
        matches!(self, Self::Hr | Self::Br)
    }
}

impl Attribute {
    fn write_html(&self, output: &mut String) {
        match self {
            Self::Source(source) => {
                write_source_id(output, source);
                write_source_data(output, source);
            }
            Self::SourceWrapper(source, class) => {
                write_source_id(output, source);
                write_quoted(output, "class", class.as_str());
                write_source_data(output, source);
            }
            Self::Class(class) => write_quoted(output, "class", class.as_str()),
            Self::OrderedStart(start) => {
                if *start > 0 {
                    write_quoted(output, "start", &start.to_string());
                }
            }
            Self::CodeLanguage(language) => {
                if valid_language(language) {
                    write_quoted(output, "class", &format!("language-{language}"));
                }
            }
            Self::TaskChecked(checked) => {
                write_quoted(output, "class", Class::TaskCheckbox.as_str());
                write_quoted(output, "role", "checkbox");
                write_quoted(
                    output,
                    "aria-checked",
                    if *checked { "true" } else { "false" },
                );
            }
            Self::Link(target) => {
                write_quoted(output, "role", "link");
                write_quoted(output, "tabindex", "0");
                write_quoted(output, "data-rutile-url", target.as_canonical_str());
            }
        }
    }
}

fn write_source_id(output: &mut String, source: &SourceAttributes) {
    write_quoted(
        output,
        "id",
        &format!("sb-{}-{}", source.revision, source.ordinal),
    );
}

fn write_source_data(output: &mut String, source: &SourceAttributes) {
    write_quoted(output, "data-source-start", &source.start.to_string());
    write_quoted(output, "data-source-end", &source.end.to_string());
    write_quoted(output, "data-source-revision", &source.revision.to_string());
    write_quoted(output, "data-source-kind", source.kind.as_str());
}

impl Class {
    fn as_str(self) -> &'static str {
        match self {
            Self::ImageAlt => "image-alt",
            Self::AlignLeft => "align-left",
            Self::AlignCenter => "align-center",
            Self::AlignRight => "align-right",
            Self::TaskCheckbox => "task-checkbox",
        }
    }
}

impl SourceWrapperClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Segment => "source-segment",
            Self::CodeSegment => "source-segment code-segment",
            Self::Fallback => "source-fallback",
        }
    }
}

fn valid_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 32
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn write_quoted(output: &mut String, name: &str, value: &str) {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    escape_attribute(value, output);
    output.push('"');
}

fn escape_text(value: &str, output: &mut String) {
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

fn escape_attribute(value: &str, output: &mut String) {
    escape_text(value, output);
}

fn overlaps(source: &Range<usize>, target: &Range<usize>) -> bool {
    if source.start == source.end {
        source.start >= target.start && source.start <= target.end
    } else {
        source.start < target.end && target.start < source.end
    }
}

fn partition_text(value: &str, source: &Range<usize>, target: &Range<usize>) -> String {
    let start = source.start.max(target.start);
    let end = source.end.min(target.end);
    if start <= source.start && end >= source.end {
        return value.to_owned();
    }
    let source_len = source.end.saturating_sub(source.start);
    if source_len == 0 || value.is_empty() {
        return String::new();
    }

    let value_len = value.len();
    let mut value_start = (start.saturating_sub(source.start) * value_len) / source_len;
    let mut value_end = (end.saturating_sub(source.start) * value_len).div_ceil(source_len);
    value_start = value_start.min(value_len);
    value_end = value_end.min(value_len).max(value_start);
    while value_start < value_len && !value.is_char_boundary(value_start) {
        value_start += 1;
    }
    while value_end > value_start && !value.is_char_boundary(value_end) {
        value_end -= 1;
    }
    value[value_start..value_end].to_owned()
}
