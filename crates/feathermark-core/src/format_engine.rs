//! The Wave-1 format engine (SPEC §6, LD-3).
//!
//! Turns a [`FormatCommand`] plus the current [`Document`] and [`Selection`]
//! into a bounded, span-scoped [`EditPlan`] the existing document contract can
//! apply. The engine never rewrites the buffer: every command inserts or
//! removes small markers (`**`, `` ` ``, `> `, `- [ ] `, fences, list numbers)
//! around existing text. Selections that would need more than
//! [`MAX_PLAN_EDITS`](crate::MAX_PLAN_EDITS) edits or exceed
//! [`MAX_PLAN_TOTAL_BYTES`](crate::MAX_PLAN_TOTAL_BYTES) are rejected with a
//! typed [`EditPlanError`] rather than widened into a whole-buffer rewrite.
//!
//! All toggles are idempotent round-trips: applying the same toggle twice over
//! the resulting selection restores the original markdown.

use crate::{
    Document, Edit, EditPlan, EditPlanError, FormatCommand, ListMarker, MAX_LINK_URL_BYTES,
    OrderedDelimiter, Selection, SmartEnterAction,
};

/// The result of a [`FormatCommand::SmartEnter`]: the decided action plus the
/// plan that realises it. Shells use `action` to reason about semantics
/// (telemetry, tests, IME state) without re-parsing the buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartEnterOutcome {
    pub action: SmartEnterAction,
    pub plan: EditPlan,
}

/// Applies a formatting command to `document` at `selection`, producing a
/// bounded [`EditPlan`].
///
/// For [`FormatCommand::SmartEnter`] the decided [`SmartEnterAction`] is
/// discarded; call [`smart_enter`] when the action is needed.
pub fn apply_format(
    document: &Document,
    selection: Selection,
    command: FormatCommand,
) -> Result<EditPlan, EditPlanError> {
    let text = document.snapshot().to_string();
    let built = build(&text, selection, &command)?;
    EditPlan::new(document.revision(), built.edits, built.selection_after)
}

/// Smart-Enter (SPEC §6 smart typing): continue lists/quotes/checklists,
/// exit on an empty item (double-Enter), renumber ordered lists. Returns the
/// decided [`SmartEnterAction`] alongside the plan that realises it.
pub fn smart_enter(
    document: &Document,
    selection: Selection,
) -> Result<SmartEnterOutcome, EditPlanError> {
    let text = document.snapshot().to_string();
    let built = build(&text, selection, &FormatCommand::SmartEnter)?;
    let action = built
        .action
        .expect("smart-enter build always reports an action");
    let plan = EditPlan::new(document.revision(), built.edits, built.selection_after)?;
    Ok(SmartEnterOutcome { action, plan })
}

struct Built {
    edits: Vec<Edit>,
    selection_after: Selection,
    action: Option<SmartEnterAction>,
}

impl Built {
    fn plain(edits: Vec<Edit>, selection_after: Selection) -> Self {
        Self {
            edits,
            selection_after,
            action: None,
        }
    }
}

fn build(
    text: &str,
    selection: Selection,
    command: &FormatCommand,
) -> Result<Built, EditPlanError> {
    let built = match command {
        FormatCommand::ToggleBold => toggle_inline(text, selection, "**", false),
        FormatCommand::ToggleItalic => toggle_inline(text, selection, "*", true),
        FormatCommand::ToggleCodeSpan => toggle_inline(text, selection, "`", false),
        FormatCommand::ToggleCodeBlock => toggle_code_block(text, selection),
        FormatCommand::InsertLink { url } => return insert_link(selection, url.as_deref()),
        FormatCommand::CycleHeading => cycle_heading(text, selection),
        FormatCommand::ToggleQuote => toggle_lines(text, selection, LineToggle::Quote),
        FormatCommand::ToggleBulletList => toggle_lines(text, selection, LineToggle::Bullet),
        FormatCommand::ToggleOrderedList => toggle_lines(text, selection, LineToggle::Ordered),
        FormatCommand::ToggleChecklist => toggle_lines(text, selection, LineToggle::Checklist),
        FormatCommand::SmartEnter => smart_enter_build(text, selection),
    };
    Ok(built)
}

// --- small text helpers -----------------------------------------------------

fn ordered_sel(selection: Selection) -> (usize, usize) {
    let a = selection.anchor.min(selection.head);
    let b = selection.anchor.max(selection.head);
    (a, b)
}

fn insertion(at: usize, text: &str) -> Edit {
    Edit {
        byte_range: at..at,
        replacement: text.to_owned(),
    }
}

fn removal(start: usize, end: usize) -> Edit {
    Edit {
        byte_range: start..end,
        replacement: String::new(),
    }
}

/// True when `text[start..start + needle.len()] == needle`, safely (no panic on
/// non-boundary or out-of-range slices).
fn eq_at(text: &str, start: usize, needle: &str) -> bool {
    text.get(start..start + needle.len()) == Some(needle)
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Expands a collapsed cursor to the surrounding word, or returns the position
/// unchanged when the cursor is not inside a word.
fn word_at(text: &str, pos: usize) -> (usize, usize) {
    let mut start = pos;
    for (index, ch) in text[..pos].char_indices().rev() {
        if is_word_char(ch) {
            start = index;
        } else {
            break;
        }
    }
    let mut end = pos;
    for (index, ch) in text[pos..].char_indices() {
        if is_word_char(ch) {
            end = pos + index + ch.len_utf8();
        } else {
            break;
        }
    }
    (start, end)
}

/// Byte offset of the start of the line containing `off`.
fn line_start(text: &str, off: usize) -> usize {
    text[..off].rfind('\n').map(|index| index + 1).unwrap_or(0)
}

/// Byte offset of the end of the line containing `off` (before its newline).
fn line_end(text: &str, off: usize) -> usize {
    text[off..]
        .find('\n')
        .map(|index| off + index)
        .unwrap_or(text.len())
}

/// The `(start, end)` bounds of every line touched by `[a, b]`.
fn lines_in_range(text: &str, a: usize, b: usize) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut pos = line_start(text, a);
    loop {
        let start = pos;
        let end = line_end(text, start);
        lines.push((start, end));
        if end >= text.len() {
            break;
        }
        let next = end + 1; // byte after '\n'
        if next > b {
            break;
        }
        pos = next;
    }
    lines
}

/// Maps an original offset through a sorted, non-overlapping edit list.
/// Insertions at the offset push it forward (cursor lands after the marker).
fn shift(edits: &[Edit], off: usize) -> usize {
    let mut delta: isize = 0;
    for edit in edits {
        let start = edit.byte_range.start;
        let end = edit.byte_range.end;
        if end <= off {
            delta += edit.replacement.len() as isize - (end - start) as isize;
        } else if start <= off {
            // `off` falls inside a replaced span: land at the replacement end.
            return (start as isize + delta + edit.replacement.len() as isize) as usize;
        } else {
            break;
        }
    }
    (off as isize + delta) as usize
}

// --- inline toggles (bold / italic / code span) -----------------------------

/// A single-character marker (`*`) must not match one arm of a `**` pair.
fn marker_before(text: &str, start: usize, marker: &str, single_star: bool) -> bool {
    if start < marker.len() || !eq_at(text, start - marker.len(), marker) {
        return false;
    }
    if single_star {
        let prev = start - marker.len();
        if prev >= 1 && eq_at(text, prev - 1, "*") {
            return false;
        }
    }
    true
}

fn marker_after(text: &str, end: usize, marker: &str, single_star: bool) -> bool {
    if !eq_at(text, end, marker) {
        return false;
    }
    if single_star {
        let after = end + marker.len();
        if after < text.len() && eq_at(text, after, "*") {
            return false;
        }
    }
    true
}

fn toggle_inline(text: &str, selection: Selection, marker: &str, single_star: bool) -> Built {
    let (mut start, mut end) = ordered_sel(selection);
    if start == end {
        let (word_start, word_end) = word_at(text, start);
        start = word_start;
        end = word_end;
    }
    let width = marker.len();

    // Unwrap markers sitting just outside the span.
    if marker_before(text, start, marker, single_star)
        && marker_after(text, end, marker, single_star)
    {
        let edits = vec![removal(start - width, start), removal(end, end + width)];
        return Built::plain(
            edits,
            Selection {
                anchor: start - width,
                head: end - width,
            },
        );
    }

    // Unwrap markers sitting just inside the span (whole `**bold**` selected).
    if end >= start + 2 * width
        && eq_at(text, start, marker)
        && text.get(end - width..end) == Some(marker)
    {
        let edits = vec![removal(start, start + width), removal(end - width, end)];
        return Built::plain(
            edits,
            Selection {
                anchor: start,
                head: end - 2 * width,
            },
        );
    }

    // Empty target (cursor not inside a word): one edit, cursor between markers.
    if start == end {
        let edits = vec![insertion(start, &format!("{marker}{marker}"))];
        return Built::plain(edits, Selection::collapsed(start + width));
    }

    // Wrap the span.
    let edits = vec![insertion(start, marker), insertion(end, marker)];
    Built::plain(
        edits,
        Selection {
            anchor: start + width,
            head: end + width,
        },
    )
}

// --- fenced code block ------------------------------------------------------

const FENCE_OPEN: &str = "```\n";
const FENCE_CLOSE: &str = "\n```";

fn toggle_code_block(text: &str, selection: Selection) -> Built {
    let (sel_start, sel_end) = ordered_sel(selection);
    let first = line_start(text, sel_start);
    let last = line_end(text, sel_end);

    // Unwrap: fences exactly around the selected block.
    let has_open = first >= FENCE_OPEN.len() && eq_at(text, first - FENCE_OPEN.len(), FENCE_OPEN);
    let has_close = eq_at(text, last, FENCE_CLOSE);
    if has_open && has_close {
        let edits = vec![
            removal(first - FENCE_OPEN.len(), first),
            removal(last, last + FENCE_CLOSE.len()),
        ];
        let shift_len = FENCE_OPEN.len();
        return Built::plain(
            edits,
            Selection {
                anchor: sel_start - shift_len,
                head: sel_end - shift_len,
            },
        );
    }

    // Wrap the selected lines.
    let edits = vec![insertion(first, FENCE_OPEN), insertion(last, FENCE_CLOSE)];
    let shift_len = FENCE_OPEN.len();
    Built::plain(
        edits,
        Selection {
            anchor: sel_start + shift_len,
            head: sel_end + shift_len,
        },
    )
}

// --- link -------------------------------------------------------------------

fn insert_link(selection: Selection, url: Option<&str>) -> Result<Built, EditPlanError> {
    // URL rejection is an explicit, typed error rather than a byte-budget
    // accident: `MAX_LINK_URL_BYTES` is the contract's stated limit.
    if let Some(url) = url {
        if url.len() > MAX_LINK_URL_BYTES {
            return Err(EditPlanError::ReplacementTooLarge {
                index: 0,
                len: url.len(),
                max: MAX_LINK_URL_BYTES,
            });
        }
    }
    let (start, end) = ordered_sel(selection);
    let placeholder = "url";
    let destination = url.unwrap_or(placeholder);
    let closing = format!("]({destination})");

    let edits = vec![insertion(start, "["), insertion(end, &closing)];
    // Final layout: `[` <text> `](` <destination> `)`
    let dest_start = end + 1 /* [ */ + 2 /* ]( */;
    let selection_after = if url.is_some() {
        Selection::collapsed(dest_start + destination.len() + 1)
    } else {
        // Select the placeholder so the user can type over it.
        Selection {
            anchor: dest_start,
            head: dest_start + placeholder.len(),
        }
    };
    Ok(Built::plain(edits, selection_after))
}

// --- heading cycling --------------------------------------------------------

/// Returns the current ATX heading level (0 = paragraph) and its marker byte
/// length (`"### "` is level 3, 4 bytes) at the start of `line`.
fn heading_level(text: &str, line_start_off: usize) -> (u8, usize) {
    let line = &text[line_start_off..line_end(text, line_start_off)];
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        (hashes as u8, hashes + 1)
    } else {
        (0, 0)
    }
}

fn cycle_heading(text: &str, selection: Selection) -> Built {
    let (sel_start, sel_end) = ordered_sel(selection);
    let start = line_start(text, sel_start);
    let (level, old_len) = heading_level(text, start);
    let next = if level >= 6 { 0 } else { level + 1 };
    let new_marker = if next == 0 {
        String::new()
    } else {
        format!("{} ", "#".repeat(next as usize))
    };
    let new_len = new_marker.len();
    let edit = Edit {
        byte_range: start..start + old_len,
        replacement: new_marker,
    };
    let edits = vec![edit];
    let map = |off: usize| -> usize {
        if off >= start + old_len {
            (off as isize + new_len as isize - old_len as isize) as usize
        } else if off >= start {
            start + new_len
        } else {
            off
        }
    };
    Built::plain(
        edits,
        Selection {
            anchor: map(sel_start),
            head: map(sel_end),
        },
    )
}

// --- per-line toggles (quote / bullet / ordered / checklist) ----------------

#[derive(Clone, Copy)]
enum LineToggle {
    Quote,
    Bullet,
    Ordered,
    Checklist,
}

/// Length of an existing marker of this kind at the line start, or `None` if
/// the line does not carry one.
fn existing_marker_len(text: &str, line: (usize, usize), toggle: LineToggle) -> Option<usize> {
    let (start, end) = line;
    let content = &text[start..end];
    match toggle {
        LineToggle::Quote => {
            if content.starts_with("> ") {
                Some(2)
            } else if content.starts_with('>') {
                Some(1)
            } else {
                None
            }
        }
        LineToggle::Bullet => {
            if is_bullet(content) {
                Some(2)
            } else {
                None
            }
        }
        LineToggle::Checklist => checklist_len(content),
        LineToggle::Ordered => ordered_len(content),
    }
}

fn is_bullet(content: &str) -> bool {
    matches!(content.get(0..2), Some("- ") | Some("* ") | Some("+ "))
}

fn checklist_len(content: &str) -> Option<usize> {
    for marker in ['-', '*', '+'] {
        for state in ["[ ] ", "[x] ", "[X] "] {
            let prefix = format!("{marker} {state}");
            if content.starts_with(&prefix) {
                return Some(prefix.len());
            }
        }
    }
    None
}

/// Length of a leading `N. ` / `N) ` ordered marker, if present.
fn ordered_len(content: &str) -> Option<usize> {
    let digits = content.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let delim = content.as_bytes().get(digits);
    let space = content.as_bytes().get(digits + 1);
    if matches!(delim, Some(b'.') | Some(b')')) && space == Some(&b' ') {
        Some(digits + 2)
    } else {
        None
    }
}

/// A line participates in a toggle unless it is a blank line under a list
/// command (blank lines separate lists and never take a list marker).
fn participates(text: &str, line: (usize, usize), toggle: LineToggle) -> bool {
    let (start, end) = line;
    match toggle {
        LineToggle::Quote => true,
        _ => !text[start..end].trim().is_empty(),
    }
}

fn toggle_lines(text: &str, selection: Selection, toggle: LineToggle) -> Built {
    let (sel_start, sel_end) = ordered_sel(selection);
    let lines: Vec<(usize, usize)> = lines_in_range(text, sel_start, sel_end)
        .into_iter()
        .filter(|&line| participates(text, line, toggle))
        .collect();

    if lines.is_empty() {
        // Nothing to toggle: an empty plan is rejected by EditPlan::new.
        return Built::plain(Vec::new(), selection);
    }

    let all_marked = lines
        .iter()
        .all(|&line| existing_marker_len(text, line, toggle).is_some());

    let mut edits = Vec::new();
    if all_marked {
        for &(start, _) in &lines {
            let len = existing_marker_len(text, (start, line_end(text, start)), toggle)
                .expect("all lines marked");
            edits.push(removal(start, start + len));
        }
    } else {
        match toggle {
            LineToggle::Ordered => {
                // Normalise every participating line to a sequential number.
                for (index, &(start, end)) in lines.iter().enumerate() {
                    let existing = ordered_len(&text[start..end]).unwrap_or(0);
                    edits.push(Edit {
                        byte_range: start..start + existing,
                        replacement: format!("{}. ", index + 1),
                    });
                }
            }
            _ => {
                let marker = match toggle {
                    LineToggle::Quote => "> ",
                    LineToggle::Bullet => "- ",
                    LineToggle::Checklist => "- [ ] ",
                    LineToggle::Ordered => unreachable!(),
                };
                for &(start, end) in &lines {
                    if existing_marker_len(text, (start, end), toggle).is_none() {
                        edits.push(insertion(start, marker));
                    }
                }
            }
        }
    }

    if edits.is_empty() {
        return Built::plain(Vec::new(), selection);
    }

    let selection_after = Selection {
        anchor: shift(&edits, selection.anchor),
        head: shift(&edits, selection.head),
    };
    Built::plain(edits, selection_after)
}

// --- smart Enter ------------------------------------------------------------

#[derive(Clone, Copy)]
enum Prefix {
    None,
    Quote {
        depth: u8,
        marker_len: usize,
    },
    Bullet {
        marker: char,
        marker_len: usize,
    },
    Checklist {
        marker: char,
        marker_len: usize,
    },
    Ordered {
        number: u64,
        delim: char,
        marker_len: usize,
    },
}

fn indent_len(line: &str) -> usize {
    line.bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count()
}

fn parse_prefix(line: &str) -> Prefix {
    let indent = indent_len(line);
    let rest = &line[indent..];

    // Quote nesting: repeated "> ".
    if rest.starts_with("> ") {
        let mut depth = 0u8;
        let mut pos = indent;
        while line[pos..].starts_with("> ") {
            depth = depth.saturating_add(1);
            pos += 2;
        }
        return Prefix::Quote {
            depth,
            marker_len: pos,
        };
    }

    // Checklist before bullet (a checklist item starts with a bullet marker).
    if let Some(len) = checklist_len(rest) {
        let marker = rest.as_bytes()[0] as char;
        return Prefix::Checklist {
            marker,
            marker_len: indent + len,
        };
    }

    if is_bullet(rest) {
        let marker = rest.as_bytes()[0] as char;
        return Prefix::Bullet {
            marker,
            marker_len: indent + 2,
        };
    }

    if let Some(len) = ordered_len(rest) {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let number: u64 = rest[..digits].parse().unwrap_or(0);
        let delim = rest.as_bytes()[digits] as char;
        return Prefix::Ordered {
            number,
            delim,
            marker_len: indent + len,
        };
    }

    Prefix::None
}

fn list_marker(marker: char) -> ListMarker {
    match marker {
        '*' => ListMarker::Asterisk,
        '+' => ListMarker::Plus,
        _ => ListMarker::Dash,
    }
}

fn ordered_delimiter(delim: char) -> OrderedDelimiter {
    match delim {
        ')' => OrderedDelimiter::Paren,
        _ => OrderedDelimiter::Dot,
    }
}

fn smart_enter_build(text: &str, selection: Selection) -> Built {
    let (sel_start, sel_end) = ordered_sel(selection);
    let cursor = sel_start;
    let ls = line_start(text, cursor);
    let le = line_end(text, cursor);
    let line = &text[ls..le];
    let prefix = parse_prefix(line);

    let marker_len = match prefix {
        Prefix::None => 0,
        Prefix::Quote { marker_len, .. }
        | Prefix::Bullet { marker_len, .. }
        | Prefix::Checklist { marker_len, .. }
        | Prefix::Ordered { marker_len, .. } => marker_len,
    };
    let content_empty = line
        .get(marker_len..)
        .map(|rest| rest.trim().is_empty())
        .unwrap_or(true);
    let collapsed = sel_start == sel_end;

    // Double-Enter on an empty item: remove the marker, exit the block.
    if collapsed && marker_len > 0 && content_empty {
        let edits = vec![removal(ls, le)];
        return Built {
            edits,
            selection_after: Selection::collapsed(ls),
            action: Some(SmartEnterAction::ExitEmptyItem),
        };
    }

    let indent = &line[..indent_len(line)];
    let (inserted, action, renumber_from) = match prefix {
        Prefix::None => ("\n".to_owned(), SmartEnterAction::InsertNewline, None),
        Prefix::Quote { depth, .. } => (
            format!("\n{}", "> ".repeat(depth as usize)),
            SmartEnterAction::ContinueQuote { depth },
            None,
        ),
        Prefix::Bullet { marker, .. } => (
            format!("\n{indent}{marker} "),
            SmartEnterAction::ContinueBullet {
                marker: list_marker(marker),
            },
            None,
        ),
        Prefix::Checklist { marker, .. } => (
            format!("\n{indent}{marker} [ ] "),
            SmartEnterAction::ContinueChecklist {
                marker: list_marker(marker),
            },
            None,
        ),
        Prefix::Ordered { number, delim, .. } => {
            let next = number.saturating_add(1);
            (
                format!("\n{indent}{next}{delim} "),
                SmartEnterAction::ContinueOrdered {
                    number: next,
                    delimiter: ordered_delimiter(delim),
                },
                Some((next, indent.to_owned())),
            )
        }
    };

    let mut edits = vec![Edit {
        byte_range: sel_start..sel_end,
        replacement: inserted.clone(),
    }];

    // Renumber the following contiguous ordered items so the list stays in
    // sequence after the insertion.
    if let Some((mut expected, indent)) = renumber_from {
        let mut pos = le;
        while pos < text.len() {
            let next_start = pos + 1; // skip '\n'
            if next_start > text.len() {
                break;
            }
            let next_end = line_end(text, next_start);
            let next_line = &text[next_start..next_end];
            let n_indent = indent_len(next_line);
            match ordered_len(&next_line[n_indent..]) {
                Some(_) if next_line.get(..n_indent) == Some(indent.as_str()) => {
                    let expected_next = expected.saturating_add(1);
                    let digits = next_line[n_indent..]
                        .bytes()
                        .take_while(u8::is_ascii_digit)
                        .count();
                    let num_start = next_start + n_indent;
                    edits.push(Edit {
                        byte_range: num_start..num_start + digits,
                        replacement: expected_next.to_string(),
                    });
                    expected = expected_next;
                    pos = next_end;
                }
                _ => break,
            }
        }
    }

    let selection_after = Selection::collapsed(sel_start + inserted.len());
    Built {
        edits,
        selection_after,
        action: Some(action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::new(text).unwrap()
    }

    /// Applies a plan and returns the resulting document text.
    fn apply(text: &str, selection: Selection, command: FormatCommand) -> (String, Selection) {
        let mut document = doc(text);
        let plan = apply_format(&document, selection, command).unwrap();
        let selection_after = plan.selection_after();
        document.apply(plan.into_transaction(1)).unwrap();
        (document.snapshot().to_string(), selection_after)
    }

    #[test]
    fn bold_wraps_selection() {
        let (out, sel) = apply(
            "bold me",
            Selection { anchor: 0, head: 4 },
            FormatCommand::ToggleBold,
        );
        assert_eq!(out, "**bold** me");
        assert_eq!(sel, Selection { anchor: 2, head: 6 });
    }

    #[test]
    fn bold_round_trip_is_idempotent() {
        let start = "bold me";
        let sel = Selection { anchor: 0, head: 4 };
        let (once, sel1) = apply(start, sel, FormatCommand::ToggleBold);
        assert_eq!(once, "**bold** me");
        let (twice, sel2) = apply(&once, sel1, FormatCommand::ToggleBold);
        assert_eq!(twice, start);
        assert_eq!(sel2, sel);
    }

    #[test]
    fn italic_does_not_unwrap_bold() {
        // Selecting "bold" inside **bold** and toggling italic must wrap, not
        // strip a star off the surrounding bold pair.
        let text = "**bold**";
        let (out, _) = apply(
            text,
            Selection { anchor: 2, head: 6 },
            FormatCommand::ToggleItalic,
        );
        assert_eq!(out, "***bold***");
    }

    #[test]
    fn italic_round_trip_is_idempotent() {
        let (once, sel1) = apply(
            "word",
            Selection { anchor: 0, head: 4 },
            FormatCommand::ToggleItalic,
        );
        assert_eq!(once, "*word*");
        let (twice, _) = apply(&once, sel1, FormatCommand::ToggleItalic);
        assert_eq!(twice, "word");
    }

    #[test]
    fn code_span_word_at_cursor() {
        let (out, sel) = apply(
            "let x",
            Selection::collapsed(1),
            FormatCommand::ToggleCodeSpan,
        );
        assert_eq!(out, "`let` x");
        assert_eq!(sel, Selection { anchor: 1, head: 4 });
    }

    #[test]
    fn code_span_empty_cursor_places_between_markers() {
        // Cursor at position 2 sits between the two spaces: not in any word.
        let (out, sel) = apply(
            "a  b",
            Selection::collapsed(2),
            FormatCommand::ToggleCodeSpan,
        );
        assert_eq!(out, "a `` b");
        assert_eq!(sel, Selection::collapsed(3));
        // Re-toggle removes the empty markers.
        let (back, _) = apply(&out, sel, FormatCommand::ToggleCodeSpan);
        assert_eq!(back, "a  b");
    }

    #[test]
    fn unwrap_when_selection_includes_markers() {
        let (out, sel) = apply(
            "**bold**",
            Selection { anchor: 0, head: 8 },
            FormatCommand::ToggleBold,
        );
        assert_eq!(out, "bold");
        assert_eq!(sel, Selection { anchor: 0, head: 4 });
    }

    #[test]
    fn unicode_selection_bold() {
        let text = "café résumé";
        // "café" is 5 bytes (é = 2 bytes).
        let (out, sel) = apply(
            text,
            Selection { anchor: 0, head: 5 },
            FormatCommand::ToggleBold,
        );
        assert_eq!(out, "**café** résumé");
        assert_eq!(sel, Selection { anchor: 2, head: 7 });
        let (back, _) = apply(&out, sel, FormatCommand::ToggleBold);
        assert_eq!(back, text);
    }

    #[test]
    fn code_block_wraps_and_unwraps() {
        let text = "line one\nline two";
        let sel = Selection {
            anchor: 0,
            head: text.len(),
        };
        let (out, sel1) = apply(text, sel, FormatCommand::ToggleCodeBlock);
        assert_eq!(out, "```\nline one\nline two\n```");
        let (back, _) = apply(&out, sel1, FormatCommand::ToggleCodeBlock);
        assert_eq!(back, text);
    }

    #[test]
    fn insert_link_with_url() {
        let (out, sel) = apply(
            "click",
            Selection { anchor: 0, head: 5 },
            FormatCommand::InsertLink {
                url: Some("https://x.io".into()),
            },
        );
        assert_eq!(out, "[click](https://x.io)");
        assert_eq!(sel, Selection::collapsed(out.len()));
    }

    #[test]
    fn insert_link_placeholder_selects_url_slot() {
        let (out, sel) = apply(
            "click",
            Selection { anchor: 0, head: 5 },
            FormatCommand::InsertLink { url: None },
        );
        assert_eq!(out, "[click](url)");
        assert_eq!(&out[sel.anchor..sel.head], "url");
    }

    #[test]
    fn insert_link_rejects_oversized_url() {
        let document = doc("click");
        let url = "x".repeat(MAX_LINK_URL_BYTES + 1);
        let result = apply_format(
            &document,
            Selection { anchor: 0, head: 5 },
            FormatCommand::InsertLink { url: Some(url) },
        );
        assert!(matches!(
            result,
            Err(EditPlanError::ReplacementTooLarge { max, .. }) if max == MAX_LINK_URL_BYTES
        ));
    }

    #[test]
    fn cycle_heading_advances_and_wraps() {
        let mut cur = "Title".to_owned();
        let expected = [
            "# Title",
            "## Title",
            "### Title",
            "#### Title",
            "##### Title",
            "###### Title",
            "Title",
        ];
        for want in expected {
            let (out, _) = apply(&cur, Selection::collapsed(0), FormatCommand::CycleHeading);
            assert_eq!(out, want, "cycling from {cur:?}");
            cur = out;
        }
    }

    #[test]
    fn quote_toggle_round_trip() {
        let text = "a\nb\nc";
        let sel = Selection {
            anchor: 0,
            head: text.len(),
        };
        let (out, sel1) = apply(text, sel, FormatCommand::ToggleQuote);
        assert_eq!(out, "> a\n> b\n> c");
        let (back, _) = apply(&out, sel1, FormatCommand::ToggleQuote);
        assert_eq!(back, text);
    }

    #[test]
    fn bullet_toggle_round_trip() {
        let text = "a\nb";
        let sel = Selection {
            anchor: 0,
            head: text.len(),
        };
        let (out, sel1) = apply(text, sel, FormatCommand::ToggleBulletList);
        assert_eq!(out, "- a\n- b");
        let (back, _) = apply(&out, sel1, FormatCommand::ToggleBulletList);
        assert_eq!(back, text);
    }

    #[test]
    fn ordered_toggle_numbers_sequentially() {
        let text = "a\nb\nc";
        let sel = Selection {
            anchor: 0,
            head: text.len(),
        };
        let (out, sel1) = apply(text, sel, FormatCommand::ToggleOrderedList);
        assert_eq!(out, "1. a\n2. b\n3. c");
        let (back, _) = apply(&out, sel1, FormatCommand::ToggleOrderedList);
        assert_eq!(back, text);
    }

    #[test]
    fn checklist_toggle_round_trip() {
        let text = "task";
        let sel = Selection { anchor: 0, head: 4 };
        let (out, sel1) = apply(text, sel, FormatCommand::ToggleChecklist);
        assert_eq!(out, "- [ ] task");
        let (back, _) = apply(&out, sel1, FormatCommand::ToggleChecklist);
        assert_eq!(back, text);
    }

    #[test]
    fn smart_enter_plain_newline() {
        let document = doc("plain");
        let outcome = smart_enter(&document, Selection::collapsed(5)).unwrap();
        assert_eq!(outcome.action, SmartEnterAction::InsertNewline);
    }

    #[test]
    fn smart_enter_continues_bullet() {
        let text = "- item";
        let document = doc(text);
        let outcome = smart_enter(&document, Selection::collapsed(text.len())).unwrap();
        assert_eq!(
            outcome.action,
            SmartEnterAction::ContinueBullet {
                marker: ListMarker::Dash
            }
        );
        let mut d = doc(text);
        let after = outcome.plan.selection_after();
        d.apply(outcome.plan.into_transaction(1)).unwrap();
        assert_eq!(d.snapshot().to_string(), "- item\n- ");
        assert_eq!(after, Selection::collapsed(text.len() + 3));
    }

    #[test]
    fn smart_enter_continues_checklist() {
        let text = "- [x] done";
        let document = doc(text);
        let outcome = smart_enter(&document, Selection::collapsed(text.len())).unwrap();
        assert_eq!(
            outcome.action,
            SmartEnterAction::ContinueChecklist {
                marker: ListMarker::Dash
            }
        );
        let mut d = doc(text);
        d.apply(outcome.plan.into_transaction(1)).unwrap();
        assert_eq!(d.snapshot().to_string(), "- [x] done\n- [ ] ");
    }

    #[test]
    fn smart_enter_continues_quote() {
        let text = "> quote";
        let document = doc(text);
        let outcome = smart_enter(&document, Selection::collapsed(text.len())).unwrap();
        assert_eq!(outcome.action, SmartEnterAction::ContinueQuote { depth: 1 });
        let mut d = doc(text);
        d.apply(outcome.plan.into_transaction(1)).unwrap();
        assert_eq!(d.snapshot().to_string(), "> quote\n> ");
    }

    #[test]
    fn smart_enter_continues_and_renumbers_ordered() {
        let text = "1. one\n2. two";
        let document = doc(text);
        // Cursor at end of first item.
        let outcome = smart_enter(&document, Selection::collapsed(6)).unwrap();
        assert_eq!(
            outcome.action,
            SmartEnterAction::ContinueOrdered {
                number: 2,
                delimiter: OrderedDelimiter::Dot
            }
        );
        let mut d = doc(text);
        d.apply(outcome.plan.into_transaction(1)).unwrap();
        assert_eq!(d.snapshot().to_string(), "1. one\n2. \n3. two");
    }

    #[test]
    fn smart_enter_exits_empty_item() {
        let text = "- item\n- ";
        let document = doc(text);
        let outcome = smart_enter(&document, Selection::collapsed(text.len())).unwrap();
        assert_eq!(outcome.action, SmartEnterAction::ExitEmptyItem);
        let mut d = doc(text);
        d.apply(outcome.plan.into_transaction(1)).unwrap();
        assert_eq!(d.snapshot().to_string(), "- item\n");
    }

    #[test]
    fn large_selection_over_edit_cap_is_rejected_cleanly() {
        // One line per edit; more lines than MAX_PLAN_EDITS must yield a typed
        // error, never a panic or a whole-buffer rewrite.
        let text = "x\n".repeat(crate::MAX_PLAN_EDITS + 10);
        let document = doc(&text);
        let sel = Selection {
            anchor: 0,
            head: text.len(),
        };
        let result = apply_format(&document, sel, FormatCommand::ToggleBulletList);
        assert!(matches!(
            result,
            Err(EditPlanError::TooManyEdits { max, .. }) if max == crate::MAX_PLAN_EDITS
        ));
    }

    // Randomised (via index) idempotency sweep over inline toggles.
    #[test]
    fn inline_toggles_are_idempotent_over_a_corpus() {
        let corpus = [
            "hello",
            "café",
            "a b c",
            "under_score",
            "日本語text",
            "x",
            "  spaced  ",
        ];
        let commands = [
            FormatCommand::ToggleBold,
            FormatCommand::ToggleItalic,
            FormatCommand::ToggleCodeSpan,
        ];
        for (ci, text) in corpus.iter().enumerate() {
            for (cj, command) in commands.iter().enumerate() {
                // Deterministic pseudo-random selection derived from indices.
                let len = text.len();
                let raw = (ci * 7 + cj * 3) % (len + 1);
                // Snap to a char boundary.
                let mut pos = raw.min(len);
                while pos < len && !text.is_char_boundary(pos) {
                    pos += 1;
                }
                let sel = Selection::collapsed(pos);
                let (once, sel1) = apply(text, sel, command.clone());
                let (twice, _) = apply(&once, sel1, command.clone());
                assert_eq!(
                    &twice, text,
                    "round trip failed for {text:?} with {command:?} at {pos}"
                );
            }
        }
    }
}
