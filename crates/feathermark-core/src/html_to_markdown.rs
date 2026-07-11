//! Bounded, allowlist-driven clipboard-HTML → Markdown conversion.
//!
//! Smart paste turns whatever HTML a browser or word processor put on the
//! clipboard into clean Markdown *text* that then flows through the existing
//! [`crate::render`] pipeline. This module never emits pass-through HTML: the
//! only thing it produces is Markdown, and only from an explicit element
//! allowlist. Everything outside the allowlist is unwrapped to its text
//! content, and `<script>`/`<style>`/comments are dropped entirely.
//!
//! Design constraints (mirroring the render/security seams):
//!
//! * **Bounded input.** Anything larger than [`MAX_HTML_INPUT_BYTES`] is
//!   rejected with a typed error, like the render byte caps.
//! * **Bounded nesting.** A hand-rolled tokenizer feeds an *iterative* DOM
//!   builder (no recursion while parsing), and the element stack is capped at
//!   [`MAX_NESTING_DEPTH`] — the same defensive stance as
//!   [`crate::render::MAX_RENDER_NESTING_DEPTH`]. Deeply nested or unclosed
//!   input yields a typed error rather than a stack overflow.
//! * **Bounded output.** The serializer refuses to grow past
//!   [`MAX_OUTPUT_BYTES`] (blockquote/list prefixing can otherwise multiply a
//!   line by the nesting depth) and returns a typed error instead.
//! * **No panics on hostile input.** Pasted clipboard content is untrusted;
//!   every slice is taken on validated `char` boundaries and every arithmetic
//!   step saturates.
//!
//! A dedicated `html_to_markdown` fuzz target exercises these guarantees.

use feathermark_types::SafeLinkTarget;
use thiserror::Error;

/// Maximum clipboard-HTML input accepted, in bytes (2 MiB).
pub const MAX_HTML_INPUT_BYTES: usize = 2 * 1024 * 1024;

/// Maximum Markdown output produced, in bytes (4 MiB).
///
/// Prefixed constructs (nested blockquotes, list continuation indentation)
/// can expand a single source line by the nesting depth, so the serializer is
/// capped independently of the input.
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Maximum element nesting the converter will build and walk.
///
/// The DOM is built iteratively so parsing never overflows the stack, but the
/// serializer walks the tree recursively. This cap keeps that recursion far
/// below any stack-overflow threshold while remaining well beyond any real
/// clipboard payload. It also bounds blockquote/list prefix expansion.
pub const MAX_NESTING_DEPTH: usize = 256;

/// Errors returned by [`html_to_markdown`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum HtmlToMarkdownError {
    /// Input exceeded [`MAX_HTML_INPUT_BYTES`].
    #[error("clipboard HTML exceeds the {MAX_HTML_INPUT_BYTES} byte input cap")]
    InputTooLarge,
    /// Element nesting exceeded [`MAX_NESTING_DEPTH`].
    #[error("clipboard HTML nesting exceeds the depth cap")]
    NestingTooDeep,
    /// Generated Markdown exceeded [`MAX_OUTPUT_BYTES`].
    #[error("generated Markdown exceeds the {MAX_OUTPUT_BYTES} byte output cap")]
    OutputTooLarge,
}

/// Convert clipboard HTML into safe, minimal Markdown text.
///
/// The result contains no pass-through HTML: only the allowlisted elements
/// (`h1`–`h6`, `p`, `br`, `strong`/`b`, `em`/`i`, `code`, `pre`, `blockquote`,
/// `ul`/`ol`/`li`, `a[href]` with `http`/`https`/`mailto`, `hr`) map to
/// Markdown; everything else is unwrapped to its decoded text content.
///
/// # Errors
///
/// Returns [`HtmlToMarkdownError`] when the input, nesting, or output exceed
/// their bounds. It never panics on any input.
pub fn html_to_markdown(html: &str) -> Result<String, HtmlToMarkdownError> {
    if html.len() > MAX_HTML_INPUT_BYTES {
        return Err(HtmlToMarkdownError::InputTooLarge);
    }
    let tokens = tokenize(html);
    let forest = build_dom(&tokens)?;
    let mut serializer = Serializer::new();
    let body = serializer.render_forest(&forest, 0)?;
    Ok(normalize_document(&body))
}

// ---------------------------------------------------------------------------
// Element allowlist
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Element {
    Heading(u8),
    Paragraph,
    LineBreak,
    Strong,
    Emphasis,
    Code,
    Preformatted,
    Blockquote,
    UnorderedList,
    OrderedList,
    ListItem,
    Anchor,
    ThematicBreak,
    /// Any element outside the allowlist: unwrapped to its children.
    Unknown,
}

impl Element {
    fn from_name(name: &str) -> Self {
        match name {
            "h1" => Self::Heading(1),
            "h2" => Self::Heading(2),
            "h3" => Self::Heading(3),
            "h4" => Self::Heading(4),
            "h5" => Self::Heading(5),
            "h6" => Self::Heading(6),
            "p" => Self::Paragraph,
            "br" => Self::LineBreak,
            "strong" | "b" => Self::Strong,
            "em" | "i" => Self::Emphasis,
            "code" => Self::Code,
            "pre" => Self::Preformatted,
            "blockquote" => Self::Blockquote,
            "ul" => Self::UnorderedList,
            "ol" => Self::OrderedList,
            "li" => Self::ListItem,
            "a" => Self::Anchor,
            "hr" => Self::ThematicBreak,
            _ => Self::Unknown,
        }
    }

    /// Void elements never have children and never open a scope.
    fn is_void(self) -> bool {
        matches!(self, Self::LineBreak | Self::ThematicBreak)
    }

    /// Whether this element intrinsically starts a Markdown block.
    fn is_block(self) -> bool {
        matches!(
            self,
            Self::Heading(_)
                | Self::Paragraph
                | Self::Preformatted
                | Self::Blockquote
                | Self::UnorderedList
                | Self::OrderedList
                | Self::ListItem
                | Self::ThematicBreak
        )
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Token {
    /// Raw text with HTML entities still encoded.
    Text(String),
    Start {
        name: String,
        href: Option<String>,
        self_closing: bool,
    },
    End {
        name: String,
    },
}

/// Split HTML into a flat token stream. Comments, doctypes, processing
/// instructions, and the contents of `<script>`/`<style>` are discarded. A `<`
/// that does not begin a recognizable tag is treated as literal text.
fn tokenize(html: &str) -> Vec<Token> {
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let mut text_start = 0;

    let flush_text = |tokens: &mut Vec<Token>, from: usize, to: usize| {
        if to > from {
            tokens.push(Token::Text(html[from..to].to_string()));
        }
    };

    while cursor < len {
        if bytes[cursor] != b'<' {
            cursor += 1;
            continue;
        }

        // Markup declarations and processing instructions: `<!...`, `<?...`.
        if let Some(&next) = bytes.get(cursor + 1) {
            if next == b'!' {
                flush_text(&mut tokens, text_start, cursor);
                cursor = skip_declaration(bytes, cursor);
                text_start = cursor;
                continue;
            }
            if next == b'?' {
                flush_text(&mut tokens, text_start, cursor);
                cursor = skip_to_gt(bytes, cursor + 2);
                text_start = cursor;
                continue;
            }
        }

        let is_end = bytes.get(cursor + 1) == Some(&b'/');
        let name_start = if is_end { cursor + 2 } else { cursor + 1 };
        // A tag name must begin with an ASCII letter; otherwise `<` is literal.
        if !matches!(bytes.get(name_start), Some(b) if b.is_ascii_alphabetic()) {
            cursor += 1;
            continue;
        }

        flush_text(&mut tokens, text_start, cursor);

        let mut index = name_start;
        while index < len && is_name_byte(bytes[index]) {
            index += 1;
        }
        let name = html[name_start..index].to_ascii_lowercase();
        let (tag_end, self_closing) = scan_tag_end(bytes, index);

        if is_end {
            tokens.push(Token::End { name });
            cursor = tag_end;
            text_start = cursor;
            continue;
        }

        // Raw-text elements: drop the element and its entire contents.
        if name == "script" || name == "style" {
            cursor = skip_raw_text(html, bytes, tag_end, &name);
            text_start = cursor;
            continue;
        }

        let href = if name == "a" {
            extract_href(&html[index..tag_end.min(len)])
        } else {
            None
        };
        tokens.push(Token::Start {
            name,
            href,
            self_closing,
        });
        cursor = tag_end;
        text_start = cursor;
    }

    flush_text(&mut tokens, text_start, len);
    tokens
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')
}

/// Skip a `<!-- comment -->`, `<![CDATA[...]]>`, or `<!doctype ...>`.
/// `cursor` points at the opening `<`.
fn skip_declaration(bytes: &[u8], cursor: usize) -> usize {
    if bytes[cursor + 1..].starts_with(b"!--") {
        let mut index = cursor + 4;
        while index + 3 <= bytes.len() {
            if &bytes[index..index + 3] == b"-->" {
                return index + 3;
            }
            index += 1;
        }
        return bytes.len();
    }
    skip_to_gt(bytes, cursor + 2)
}

fn skip_to_gt(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'>' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

/// Find the end of a start/end tag beginning at `index` (just past the name),
/// honoring quoted attribute values so a `>` inside a value is not the end.
/// Returns the byte offset just past `>` and whether the tag self-closed.
fn scan_tag_end(bytes: &[u8], mut index: usize) -> (usize, bool) {
    let len = bytes.len();
    let mut quote: Option<u8> = None;
    let mut last_solidus = false;
    while index < len {
        let byte = bytes[index];
        match quote {
            Some(q) => {
                if byte == q {
                    quote = None;
                }
                last_solidus = false;
            }
            None => match byte {
                b'"' | b'\'' => {
                    quote = Some(byte);
                    last_solidus = false;
                }
                b'>' => return (index + 1, last_solidus),
                b'/' => last_solidus = true,
                b if b.is_ascii_whitespace() => {}
                _ => last_solidus = false,
            },
        }
        index += 1;
    }
    (len, false)
}

/// Skip the contents of a raw-text element up to and including its close tag.
/// `content_start` points just past the opening tag's `>`.
///
/// The close tag is matched case-insensitively **in place**: pre-lowercasing the
/// whole remaining input per raw-text tag is O(n) *per tag*, so a 2 MiB paste of
/// repeated `<style></style>` pairs would be O(n²) — a denial-of-service on a
/// single paste. Scanning byte-by-byte for the next `</name` stops at the first
/// match, keeping adjacent pairs O(total). `name` is already lowercase.
fn skip_raw_text(_html: &str, bytes: &[u8], content_start: usize, name: &str) -> usize {
    let needle = format!("</{name}");
    let needle = needle.as_bytes();
    let mut index = content_start.min(bytes.len());
    while index + needle.len() <= bytes.len() {
        if bytes[index] == b'<' && bytes[index..index + needle.len()].eq_ignore_ascii_case(needle) {
            return skip_to_gt(bytes, index + needle.len());
        }
        index += 1;
    }
    bytes.len()
}

/// Pull the first `href` attribute value out of a start tag's attribute span.
fn extract_href(attr_span: &str) -> Option<String> {
    let lower = attr_span.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find("href") {
        let name_start = search_from + relative;
        let before_ok = name_start == 0
            || lower.as_bytes()[name_start - 1].is_ascii_whitespace()
            || lower.as_bytes()[name_start - 1] == b'/';
        let after = name_start + 4;
        // Skip whitespace to find `=`.
        let mut index = after;
        let raw = attr_span.as_bytes();
        while index < raw.len() && raw[index].is_ascii_whitespace() {
            index += 1;
        }
        if before_ok && index < raw.len() && raw[index] == b'=' {
            index += 1;
            while index < raw.len() && raw[index].is_ascii_whitespace() {
                index += 1;
            }
            return Some(read_attr_value(attr_span, index));
        }
        search_from = name_start + 4;
    }
    None
}

fn read_attr_value(attr_span: &str, start: usize) -> String {
    let raw = attr_span.as_bytes();
    if start >= raw.len() {
        return String::new();
    }
    let quote = raw[start];
    if quote == b'"' || quote == b'\'' {
        let value_start = start + 1;
        let mut index = value_start;
        while index < raw.len() && raw[index] != quote {
            index += 1;
        }
        attr_span[value_start..index].to_string()
    } else {
        let mut index = start;
        while index < raw.len() && !raw[index].is_ascii_whitespace() && raw[index] != b'>' {
            index += 1;
        }
        attr_span[start..index].to_string()
    }
}

// ---------------------------------------------------------------------------
// Iterative DOM builder
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Node {
    Text(String),
    Element(ElementNode),
}

#[derive(Debug)]
struct ElementNode {
    element: Element,
    href: Option<String>,
    children: Vec<Node>,
}

struct OpenElement {
    element: Element,
    name: String,
    href: Option<String>,
    children: Vec<Node>,
}

impl OpenElement {
    fn into_node(self) -> Node {
        Node::Element(ElementNode {
            element: self.element,
            href: self.href,
            children: self.children,
        })
    }
}

/// Build a bounded DOM forest from the token stream. Unclosed tags are
/// auto-closed at end of input; stray close tags are ignored; nesting beyond
/// [`MAX_NESTING_DEPTH`] is a typed error.
fn build_dom(tokens: &[Token]) -> Result<Vec<Node>, HtmlToMarkdownError> {
    let mut roots: Vec<Node> = Vec::new();
    let mut stack: Vec<OpenElement> = Vec::new();

    for token in tokens {
        match token {
            Token::Text(text) => push_child(&mut stack, &mut roots, Node::Text(text.clone())),
            Token::Start {
                name,
                href,
                self_closing,
            } => {
                let element = Element::from_name(name);
                if element.is_void() || *self_closing {
                    push_child(
                        &mut stack,
                        &mut roots,
                        Node::Element(ElementNode {
                            element,
                            href: href.clone(),
                            children: Vec::new(),
                        }),
                    );
                    continue;
                }
                if stack.len() >= MAX_NESTING_DEPTH {
                    return Err(HtmlToMarkdownError::NestingTooDeep);
                }
                stack.push(OpenElement {
                    element,
                    name: name.clone(),
                    href: href.clone(),
                    children: Vec::new(),
                });
            }
            Token::End { name } => {
                if let Some(position) = stack.iter().rposition(|open| &open.name == name) {
                    while stack.len() > position {
                        close_top(&mut stack, &mut roots);
                    }
                }
            }
        }
    }

    while !stack.is_empty() {
        close_top(&mut stack, &mut roots);
    }
    Ok(roots)
}

fn push_child(stack: &mut [OpenElement], roots: &mut Vec<Node>, node: Node) {
    match stack.last_mut() {
        Some(open) => open.children.push(node),
        None => roots.push(node),
    }
}

fn close_top(stack: &mut Vec<OpenElement>, roots: &mut Vec<Node>) {
    let Some(open) = stack.pop() else {
        return;
    };
    let node = open.into_node();
    push_child(stack, roots, node);
}

// ---------------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------------

struct Serializer;

impl Serializer {
    fn new() -> Self {
        Self
    }

    fn guard(&self, output: &str) -> Result<(), HtmlToMarkdownError> {
        if output.len() > MAX_OUTPUT_BYTES {
            Err(HtmlToMarkdownError::OutputTooLarge)
        } else {
            Ok(())
        }
    }

    /// Render a list of sibling nodes as block-level Markdown, gathering runs
    /// of inline content into paragraphs.
    fn render_forest(
        &mut self,
        nodes: &[Node],
        depth: usize,
    ) -> Result<String, HtmlToMarkdownError> {
        let mut blocks: Vec<String> = Vec::new();
        let mut inline = String::new();

        for node in nodes {
            if is_block_node(node) {
                self.flush_inline(&mut inline, &mut blocks)?;
                let block = self.render_block(node, depth)?;
                if !block.trim().is_empty() {
                    blocks.push(block);
                }
            } else {
                self.render_inline(node, depth, &mut inline)?;
                self.guard(&inline)?;
            }
        }
        self.flush_inline(&mut inline, &mut blocks)?;

        let joined = blocks.join("\n\n");
        self.guard(&joined)?;
        Ok(joined)
    }

    fn flush_inline(
        &self,
        inline: &mut String,
        blocks: &mut Vec<String>,
    ) -> Result<(), HtmlToMarkdownError> {
        let trimmed = inline.trim();
        if !trimmed.is_empty() {
            blocks.push(escape_block_leading(trimmed));
        }
        inline.clear();
        Ok(())
    }

    fn render_block(&mut self, node: &Node, depth: usize) -> Result<String, HtmlToMarkdownError> {
        let Node::Element(element) = node else {
            // Non-elements are handled inline; treat defensively as inline.
            let mut inline = String::new();
            self.render_inline(node, depth, &mut inline)?;
            return Ok(escape_block_leading(inline.trim()));
        };
        let next = depth.saturating_add(1);
        let rendered = match element.element {
            Element::Heading(level) => {
                let text = self.render_inline_children(element, next)?;
                let text = collapse_spaces(text.trim());
                if text.is_empty() {
                    String::new()
                } else {
                    format!("{} {text}", "#".repeat(level as usize))
                }
            }
            Element::Paragraph => {
                let text = self.render_inline_children(element, next)?;
                escape_block_leading(text.trim())
            }
            Element::ThematicBreak => "---".to_string(),
            Element::Preformatted => render_code_block(&collect_code_text(element)),
            Element::Blockquote => {
                let inner = self.render_forest(&element.children, next)?;
                prefix_lines(&inner, "> ", ">")
            }
            Element::UnorderedList => self.render_list(element, next, None)?,
            Element::OrderedList => self.render_list(element, next, Some(1))?,
            Element::ListItem => self.render_forest(&element.children, next)?,
            Element::Unknown => self.render_forest(&element.children, next)?,
            // Inline elements should not reach here, but degrade gracefully.
            _ => {
                let mut inline = String::new();
                self.render_inline(node, depth, &mut inline)?;
                escape_block_leading(inline.trim())
            }
        };
        self.guard(&rendered)?;
        Ok(rendered)
    }

    fn render_list(
        &mut self,
        list: &ElementNode,
        depth: usize,
        ordered_start: Option<u64>,
    ) -> Result<String, HtmlToMarkdownError> {
        let mut lines: Vec<String> = Vec::new();
        let mut ordinal = ordered_start.unwrap_or(1);
        // Track the joined output length incrementally: re-joining the whole
        // accumulated list on every item to size-check it is O(n²) and turns a
        // ~2 MiB paste of many list items into a multi-minute UI hang (the same
        // hostile-clipboard DoS class as the round-1 `skip_raw_text` fix). Sum
        // the running length instead and join exactly once at the end.
        let mut joined_len = 0usize;
        for child in &list.children {
            let Node::Element(item) = child else {
                continue;
            };
            if item.element != Element::ListItem {
                continue;
            }
            let content = self.render_forest(&item.children, depth.saturating_add(1))?;
            let content = content.trim_end();
            let marker = match ordered_start {
                Some(_) => {
                    let marker = format!("{ordinal}. ");
                    ordinal = ordinal.saturating_add(1);
                    marker
                }
                None => "- ".to_string(),
            };
            let indent = " ".repeat(marker.len());
            let item_text = indent_continuation(content, &marker, &indent);
            // Account for the "\n" separator between items (not before the first).
            let separator = usize::from(!lines.is_empty());
            joined_len = joined_len
                .saturating_add(separator)
                .saturating_add(item_text.len());
            if joined_len > MAX_OUTPUT_BYTES {
                return Err(HtmlToMarkdownError::OutputTooLarge);
            }
            lines.push(item_text);
        }
        Ok(lines.join("\n"))
    }

    fn render_inline_children(
        &mut self,
        element: &ElementNode,
        depth: usize,
    ) -> Result<String, HtmlToMarkdownError> {
        let mut output = String::new();
        for child in &element.children {
            self.render_inline(child, depth, &mut output)?;
        }
        self.guard(&output)?;
        Ok(output)
    }

    fn render_inline(
        &mut self,
        node: &Node,
        depth: usize,
        output: &mut String,
    ) -> Result<(), HtmlToMarkdownError> {
        match node {
            Node::Text(raw) => {
                output.push_str(&escape_inline_text(raw));
            }
            Node::Element(element) => {
                let next = depth.saturating_add(1);
                match element.element {
                    Element::Strong => {
                        let inner = self.render_inline_children(element, next)?;
                        wrap_emphasis(output, &inner, "**");
                    }
                    Element::Emphasis => {
                        let inner = self.render_inline_children(element, next)?;
                        wrap_emphasis(output, &inner, "*");
                    }
                    Element::Code => {
                        let text = collect_code_text(element);
                        output.push_str(&render_code_span(&text));
                    }
                    Element::LineBreak => output.push_str("  \n"),
                    Element::Anchor => {
                        let inner = self.render_inline_children(element, next)?;
                        push_link(output, element.href.as_deref(), &inner);
                    }
                    // Block elements appearing in an inline run, and any
                    // unknown/transparent wrapper, are unwrapped.
                    _ => {
                        for child in &element.children {
                            self.render_inline(child, next, output)?;
                        }
                    }
                }
            }
        }
        self.guard(output)?;
        Ok(())
    }
}

fn is_block_node(node: &Node) -> bool {
    match node {
        Node::Text(_) => false,
        Node::Element(element) => {
            element.element.is_block()
                || (element.element == Element::Unknown
                    && element.children.iter().any(is_block_node))
        }
    }
}

// ---------------------------------------------------------------------------
// Text / emphasis / link helpers
// ---------------------------------------------------------------------------

/// Decode entities, collapse whitespace, then re-encode so pasted prose stays
/// literal and inert. Angle brackets and ampersands become HTML entities (so
/// the output can never contain a raw `<tag` sequence), and the Markdown
/// inline-significant punctuation is backslash-escaped.
fn escape_inline_text(raw: &str) -> String {
    let decoded = html_escape::decode_html_entities(raw);
    let collapsed = collapse_whitespace(decoded.as_ref());
    let mut output = String::with_capacity(collapsed.len());
    for character in collapsed.chars() {
        match character {
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                output.push('\\');
                output.push(character);
            }
            other => output.push(other),
        }
    }
    output
}

/// Collapse runs of ASCII whitespace (and non-breaking spaces) to single
/// spaces. Leading/trailing single spaces are preserved so inline boundaries
/// survive; block trimming removes them later.
fn collapse_whitespace(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_ascii_whitespace() || character == '\u{a0}' {
            pending_space = true;
            continue;
        }
        if pending_space {
            output.push(' ');
            pending_space = false;
        }
        output.push(character);
    }
    if pending_space {
        output.push(' ');
    }
    output
}

/// Collapse internal runs of spaces to one, trimming ends.
fn collapse_spaces(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut pending = false;
    for character in value.chars() {
        if character == ' ' {
            pending = true;
            continue;
        }
        if pending && !output.is_empty() {
            output.push(' ');
        }
        pending = false;
        output.push(character);
    }
    output
}

/// Wrap inner content in an emphasis delimiter, moving leading/trailing
/// whitespace outside the markers (CommonMark forbids `** text **`).
fn wrap_emphasis(output: &mut String, inner: &str, delimiter: &str) {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        if !inner.is_empty() {
            output.push(' ');
        }
        return;
    }
    if inner.starts_with(char::is_whitespace) {
        output.push(' ');
    }
    output.push_str(delimiter);
    output.push_str(trimmed);
    output.push_str(delimiter);
    if inner.ends_with(char::is_whitespace) {
        output.push(' ');
    }
}

/// Emit a Markdown link if the href is an allowed `http`/`https`/`mailto`
/// target; otherwise drop the link and keep its text.
fn push_link(output: &mut String, href: Option<&str>, inner: &str) {
    let target = href
        .map(|value| html_escape::decode_html_entities(value).into_owned())
        .and_then(|value| SafeLinkTarget::parse(value.trim()).ok());
    match target {
        Some(target) => {
            let url = target.as_canonical_str();
            let text = if inner.trim().is_empty() {
                escape_inline_text(url)
            } else {
                inner.trim().to_string()
            };
            output.push('[');
            output.push_str(&text);
            output.push_str("](");
            output.push_str(url);
            output.push(')');
        }
        None => output.push_str(inner.trim()),
    }
}

/// Recursively collect decoded text for code contexts, preserving whitespace
/// and turning `<br>` into newlines.
fn collect_code_text(element: &ElementNode) -> String {
    let mut output = String::new();
    collect_code_text_into(&element.children, &mut output);
    output
}

fn collect_code_text_into(nodes: &[Node], output: &mut String) {
    for node in nodes {
        match node {
            Node::Text(raw) => {
                output.push_str(html_escape::decode_html_entities(raw).as_ref());
            }
            Node::Element(element) => {
                if element.element == Element::LineBreak {
                    output.push('\n');
                } else {
                    collect_code_text_into(&element.children, output);
                }
            }
        }
    }
}

/// Render an inline code span, choosing a backtick fence long enough to hold
/// the content and padding when it would otherwise touch a backtick.
fn render_code_span(text: &str) -> String {
    let flattened = text.replace('\n', " ");
    let trimmed = flattened.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let fence = "`".repeat(longest_backtick_run(trimmed) + 1);
    let needs_padding = trimmed.starts_with('`') || trimmed.ends_with('`');
    if needs_padding {
        format!("{fence} {trimmed} {fence}")
    } else {
        format!("{fence}{trimmed}{fence}")
    }
}

/// Render a fenced code block with a fence longer than any backtick run in the
/// content.
fn render_code_block(text: &str) -> String {
    let content = text.trim_matches('\n');
    let fence = "`".repeat(longest_backtick_run(content).max(2) + 1);
    if content.is_empty() {
        format!("{fence}\n{fence}")
    } else {
        format!("{fence}\n{content}\n{fence}")
    }
}

fn longest_backtick_run(value: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for character in value.chars() {
        if character == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Prefix every line of `content`; blank lines get `empty_prefix`.
fn prefix_lines(content: &str, prefix: &str, empty_prefix: &str) -> String {
    let mut output = String::new();
    for (index, line) in content.split('\n').enumerate() {
        if index > 0 {
            output.push('\n');
        }
        if line.is_empty() {
            output.push_str(empty_prefix);
        } else {
            output.push_str(prefix);
            output.push_str(line);
        }
    }
    output
}

/// Apply a list marker to the first line and indent continuation lines,
/// producing a tight list item (interior blank lines are dropped).
fn indent_continuation(content: &str, marker: &str, indent: &str) -> String {
    let mut output = String::new();
    let mut first = true;
    for line in content.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        if first {
            output.push_str(marker);
            output.push_str(line);
            first = false;
        } else {
            output.push('\n');
            output.push_str(indent);
            output.push_str(line);
        }
    }
    output
}

/// Backslash-escape a leading Markdown block marker so pasted prose does not
/// become a heading/list/quote/etc.
fn escape_block_leading(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let bytes = text.as_bytes();
    let first = bytes[0];
    let needs_escape = match first {
        b'#' | b'>' | b'=' | b'+' | b'|' | b'~' => true,
        b'-' | b'*' => bytes.get(1).is_none_or(|b| b.is_ascii_whitespace()),
        b'0'..=b'9' => leading_ordered_marker(bytes),
        _ => false,
    };
    if needs_escape {
        let mut output = String::with_capacity(text.len() + 1);
        output.push('\\');
        output.push_str(text);
        output
    } else {
        text.to_string()
    }
}

/// Detect a `123.` / `123)` ordered-list marker at the start of a line.
fn leading_ordered_marker(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    matches!(bytes.get(index), Some(b'.') | Some(b')'))
        && bytes.get(index + 1).is_none_or(|b| b.is_ascii_whitespace())
}

/// Trim leading/trailing blank lines and collapse 3+ blank lines to one.
fn normalize_document(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut blank_run = 0;
    for line in body.split('\n') {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                output.push('\n');
            }
        } else {
            blank_run = 0;
            output.push_str(line);
            output.push('\n');
        }
    }
    output.trim_matches('\n').to_string()
}
