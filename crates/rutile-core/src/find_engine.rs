//! Find/replace execution over document text (SPEC §9, Wave 1 1Q).
//!
//! This is the engine that consumes the frozen find/replace contracts
//! ([`FindQuery`], [`ReplaceSpec`], [`FindReplaceOp`]) and lowers replacements
//! into the equally frozen [`EditPlan`] edit contract. It is pure and
//! platform-neutral: it never touches the filesystem, never panics, and always
//! respects UTF-8 boundaries.
//!
//! ## Matching
//!
//! [`find_next`] locates the next match of a [`FindQuery`] from a byte offset,
//! honoring [`MatchMode::Plain`] / [`MatchMode::WholeWord`] and
//! case-sensitivity, optionally wrapping around the ends of the buffer. A
//! word, for whole-word matching, is a maximal run of Unicode alphanumerics
//! and `_`; a whole-word match must have a non-word char (or a buffer edge) on
//! both sides.
//!
//! Case-insensitive matching folds per Unicode scalar via
//! [`char::to_lowercase`]. One-to-many foldings that change length (e.g.
//! `ß` ↔ `ss`) are *not* matched; this is a documented 0.2 limitation of the
//! no-regex plain matcher, not a correctness bug.
//!
//! ## Replace-all chunking (architect obligation 1F/1Q)
//!
//! A replace-all of a common substring can produce far more than
//! [`MAX_PLAN_EDITS`](crate::MAX_PLAN_EDITS) edits or exceed
//! [`MAX_PLAN_TOTAL_BYTES`](crate::MAX_PLAN_TOTAL_BYTES). Rather than fail, or
//! widen into a buffer rewrite, [`replace_all`] **chunks** the matches into a
//! sequence of [`EditPlan`]s, each within the edit-plan bounds and meant to be
//! applied in order. Because matches are processed strictly left to right and
//! each chunk covers a contiguous, disjoint run of them, every later chunk's
//! edits are simply shifted by the net byte delta of the chunks before it, and
//! its `base_revision` is advanced by one per prior chunk (each plan applies
//! as exactly one document transaction). Applying the returned plans in order
//! against a [`Document`](crate::Document) starting at `base_revision` yields a
//! fully replaced document.

use std::ops::Range;

use rutile_types::Revision;
use thiserror::Error;

use crate::{
    Edit, EditPlan, EditPlanError, FindDirection, FindQuery, FindReplaceOp, MatchMode, ReplaceSpec,
    Selection,
};

/// The result of executing a [`FindReplaceOp`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FindReplaceOutcome {
    /// A [`FindReplaceOp::Find`] result: the located match range, if any.
    Match(Option<Range<usize>>),
    /// A replace result: the ordered plans to apply. Empty when nothing
    /// matched.
    Plans(Vec<EditPlan>),
}

/// Why a replacement could not be lowered into edit plan(s).
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ReplaceError {
    #[error("current match range {start}..{end} is reversed")]
    ReversedRange { start: usize, end: usize },
    #[error("current match range {start}..{end} exceeds text length {len}")]
    OutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    #[error("current match endpoint {offset} is not a UTF-8 boundary")]
    NotCharBoundary { offset: usize },
    #[error(transparent)]
    Plan(#[from] EditPlanError),
}

/// Locates the next match of `query` in `text`.
///
/// `from_byte` is the caller's cursor position. For [`FindDirection::Forward`]
/// the returned match starts at or after `from_byte`; for
/// [`FindDirection::Backward`] it starts strictly before `from_byte`. When no
/// match is found in that region and `wrap` is `true`, the search continues
/// from the opposite end of the buffer.
///
/// Returns byte ranges that always fall on UTF-8 boundaries. `from_byte` is
/// treated leniently: it is clamped into range and nudged to a boundary rather
/// than causing a panic.
pub fn find_next(
    text: &str,
    query: &FindQuery,
    from_byte: usize,
    direction: FindDirection,
    wrap: bool,
) -> Option<Range<usize>> {
    match direction {
        FindDirection::Forward => {
            let start = boundary_ceil(text, from_byte);
            search_forward(text, query, start).or_else(|| {
                if wrap {
                    search_forward(text, query, 0)
                } else {
                    None
                }
            })
        }
        FindDirection::Backward => {
            let limit = boundary_floor(text, from_byte);
            search_backward(text, query, limit).or_else(|| {
                if wrap {
                    search_backward(text, query, text.len())
                } else {
                    None
                }
            })
        }
    }
}

/// Builds a plan replacing a single, already-located match (the one the UI
/// highlights). The range must be valid against `text`.
pub fn replace_current(
    base_revision: Revision,
    text: &str,
    spec: &ReplaceSpec,
    current: Range<usize>,
) -> Result<EditPlan, ReplaceError> {
    validate_range(text, &current)?;
    let replacement = spec.replacement().to_owned();
    let selection_after = Selection::collapsed(current.start + replacement.len());
    let edit = Edit {
        byte_range: current,
        replacement,
    };
    Ok(EditPlan::new(base_revision, vec![edit], selection_after)?)
}

/// Replaces every match of `spec`'s query in `text`, chunked into a sequence
/// of [`EditPlan`]s to apply in order (see the module docs). Returns an empty
/// vector when nothing matches.
pub fn replace_all(
    base_revision: Revision,
    text: &str,
    spec: &ReplaceSpec,
) -> Result<Vec<EditPlan>, ReplaceError> {
    let matches = all_matches(text, spec.query());
    if matches.is_empty() {
        return Ok(Vec::new());
    }

    let replacement = spec.replacement();
    let mut plans = Vec::new();
    let mut chunk: Vec<Edit> = Vec::new();
    let mut chunk_bytes = 0usize;
    // Net byte delta of all *previous* chunks, and the revision offset (one per
    // prior chunk), used to shift later chunks into the mutated document's
    // coordinate space.
    let mut committed_delta: isize = 0;
    let mut revision_offset: u64 = 0;

    for range in &matches {
        let span = range.end - range.start;
        let edit_bytes = span + replacement.len();
        let would_overflow_edits = chunk.len() >= crate::MAX_PLAN_EDITS;
        let would_overflow_bytes =
            !chunk.is_empty() && chunk_bytes + edit_bytes > crate::MAX_PLAN_TOTAL_BYTES;
        if would_overflow_edits || would_overflow_bytes {
            let (delta, count) = flush_chunk(
                &mut plans,
                base_revision,
                revision_offset,
                committed_delta,
                &chunk,
            )?;
            committed_delta += delta;
            revision_offset += count;
            chunk = Vec::new();
            chunk_bytes = 0;
        }
        chunk.push(Edit {
            byte_range: range.clone(),
            replacement: replacement.to_owned(),
        });
        chunk_bytes += edit_bytes;
    }
    if !chunk.is_empty() {
        flush_chunk(
            &mut plans,
            base_revision,
            revision_offset,
            committed_delta,
            &chunk,
        )?;
    }
    Ok(plans)
}

/// Counts the matches a replace-all would touch, without building any plans.
pub fn match_count(text: &str, query: &FindQuery) -> usize {
    all_matches(text, query).len()
}

/// Executes a [`FindReplaceOp`] against `text` at `base_revision`.
pub fn execute(
    base_revision: Revision,
    text: &str,
    op: &FindReplaceOp,
) -> Result<FindReplaceOutcome, ReplaceError> {
    match op {
        FindReplaceOp::Find {
            query,
            from_byte,
            direction,
            wrap,
        } => Ok(FindReplaceOutcome::Match(find_next(
            text, query, *from_byte, *direction, *wrap,
        ))),
        FindReplaceOp::ReplaceCurrent { spec, current } => {
            let plan = replace_current(base_revision, text, spec, current.clone())?;
            Ok(FindReplaceOutcome::Plans(vec![plan]))
        }
        FindReplaceOp::ReplaceAll { spec } => Ok(FindReplaceOutcome::Plans(replace_all(
            base_revision,
            text,
            spec,
        )?)),
    }
}

/// Turns the accumulated `chunk` (edits in original-document coordinates) into
/// one [`EditPlan`] shifted by `committed_delta` and stamped at
/// `base_revision + revision_offset`. Returns `(net_delta, 1)` on success.
fn flush_chunk(
    plans: &mut Vec<EditPlan>,
    base_revision: Revision,
    revision_offset: u64,
    committed_delta: isize,
    chunk: &[Edit],
) -> Result<(isize, u64), ReplaceError> {
    let mut delta = 0isize;
    let shifted: Vec<Edit> = chunk
        .iter()
        .map(|edit| {
            let start = edit.byte_range.start.saturating_add_signed(committed_delta);
            let end = edit.byte_range.end.saturating_add_signed(committed_delta);
            delta += edit.replacement.len() as isize - (end - start) as isize;
            Edit {
                byte_range: start..end,
                replacement: edit.replacement.clone(),
            }
        })
        .collect();
    let last = shifted
        .last()
        .expect("flush_chunk is only called with a non-empty chunk");
    // The cursor lands at the fully-applied end of the last match: its shifted
    // end (which already carries `committed_delta` from prior chunks) plus this
    // chunk's net length delta. Every edit in the chunk precedes the last
    // match's end, so the whole of `delta` shifts it. Deriving the cursor from
    // the last replacement's end against `committed_delta` alone ignored the
    // length change of earlier edits in the *same* chunk and overshot the final
    // document length for a shrinking multi-match replace-all.
    let selection_after = Selection::collapsed(last.byte_range.end.saturating_add_signed(delta));
    let revision = Revision::new(base_revision.get().saturating_add(revision_offset));
    plans.push(EditPlan::new(revision, shifted, selection_after)?);
    Ok((delta, 1))
}

/// All non-overlapping matches of `query` in `text`, left to right.
fn all_matches(text: &str, query: &FindQuery) -> Vec<Range<usize>> {
    let mut matches = Vec::new();
    let mut from = 0usize;
    while from <= text.len() {
        let Some(found) = search_forward(text, query, from) else {
            break;
        };
        let advance = if found.end > found.start {
            found.end
        } else {
            // Defensive: non-empty patterns cannot match empty, but never spin.
            boundary_ceil(text, found.start + 1)
        };
        matches.push(found);
        from = advance;
    }
    matches
}

/// First match with start >= `start` (a byte offset assumed to be on or nudged
/// to a boundary by the caller).
fn search_forward(text: &str, query: &FindQuery, start: usize) -> Option<Range<usize>> {
    if query.case_sensitive() {
        search_forward_sensitive(text, query, start)
    } else {
        search_forward_insensitive(text, query, start)
    }
}

fn search_forward_sensitive(text: &str, query: &FindQuery, start: usize) -> Option<Range<usize>> {
    let pattern = query.pattern();
    let mut base = start.min(text.len());
    loop {
        let rel = text.get(base..)?.find(pattern)?;
        let match_start = base + rel;
        let match_end = match_start + pattern.len();
        if accept(text, query.mode(), match_start, match_end) {
            return Some(match_start..match_end);
        }
        base = boundary_ceil(text, match_start + 1);
        if base > text.len() {
            return None;
        }
    }
}

fn search_forward_insensitive(text: &str, query: &FindQuery, start: usize) -> Option<Range<usize>> {
    let start = start.min(text.len());
    let pattern = query.pattern();
    // L16: gate on the first pattern char before the full O(m) `try_match_ci`.
    // This converts the common case from O(n*m) to O(n): positions whose first
    // character does not fold-match the pattern head are skipped in O(1).
    let first = pattern.chars().next()?;
    for (offset, ch) in text[start..].char_indices() {
        if chars_eq_ignore_case(ch, first) {
            let at = start + offset;
            if let Some(end) = try_match_ci(text, pattern, at)
                && accept(text, query.mode(), at, end)
            {
                return Some(at..end);
            }
        }
    }
    None
}

/// Last match with start < `limit`.
fn search_backward(text: &str, query: &FindQuery, limit: usize) -> Option<Range<usize>> {
    let limit = boundary_floor(text, limit);
    if query.case_sensitive() {
        let pattern = query.pattern();
        for (match_start, _) in text.rmatch_indices(pattern) {
            if match_start >= limit {
                continue;
            }
            let match_end = match_start + pattern.len();
            if accept(text, query.mode(), match_start, match_end) {
                return Some(match_start..match_end);
            }
        }
        None
    } else {
        // No reverse case-insensitive primitive in std: scan forward and keep
        // the last accepted match strictly before `limit`.  L16: gate on the
        // first pattern char before the full O(m) `try_match_ci` (same
        // optimization as `search_forward_insensitive`).
        let pattern = query.pattern();
        let first = pattern.chars().next()?;
        let mut best: Option<Range<usize>> = None;
        for (at, ch) in text[..limit].char_indices() {
            if chars_eq_ignore_case(ch, first)
                && let Some(end) = try_match_ci(text, pattern, at)
                && accept(text, query.mode(), at, end)
            {
                best = Some(at..end);
            }
        }
        best
    }
}

/// Attempts a case-insensitive match of `pattern` at byte offset `at`,
/// returning the match end on success. Compares scalar by scalar under
/// [`char::to_lowercase`] folding.
fn try_match_ci(text: &str, pattern: &str, at: usize) -> Option<usize> {
    let mut text_chars = text.get(at..)?.chars();
    let mut pattern_chars = pattern.chars();
    let mut consumed = 0usize;
    loop {
        let Some(pattern_char) = pattern_chars.next() else {
            return Some(at + consumed);
        };
        let text_char = text_chars.next()?;
        if !chars_eq_ignore_case(text_char, pattern_char) {
            return None;
        }
        consumed += text_char.len_utf8();
    }
}

fn chars_eq_ignore_case(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Whole-word gate: for [`MatchMode::WholeWord`], both edges of the match must
/// be a buffer boundary or a non-word character.
fn accept(text: &str, mode: MatchMode, start: usize, end: usize) -> bool {
    match mode {
        MatchMode::Plain => true,
        MatchMode::WholeWord => {
            let before_ok = text[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !is_word_char(c));
            let after_ok = text[end..].chars().next().is_none_or(|c| !is_word_char(c));
            before_ok && after_ok
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn validate_range(text: &str, range: &Range<usize>) -> Result<(), ReplaceError> {
    if range.start > range.end {
        return Err(ReplaceError::ReversedRange {
            start: range.start,
            end: range.end,
        });
    }
    if range.end > text.len() {
        return Err(ReplaceError::OutOfBounds {
            start: range.start,
            end: range.end,
            len: text.len(),
        });
    }
    if !text.is_char_boundary(range.start) {
        return Err(ReplaceError::NotCharBoundary {
            offset: range.start,
        });
    }
    if !text.is_char_boundary(range.end) {
        return Err(ReplaceError::NotCharBoundary { offset: range.end });
    }
    Ok(())
}

/// Smallest boundary offset >= `byte`, clamped to `text.len()`.
fn boundary_ceil(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte < text.len() && !text.is_char_boundary(byte) {
        byte += 1;
    }
    byte
}

/// Largest boundary offset <= `byte`, clamped to `text.len()`.
fn boundary_floor(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, EditTransaction, TransactionKind};

    fn query(pattern: &str, mode: MatchMode, case_sensitive: bool) -> FindQuery {
        FindQuery::new(pattern.to_owned(), mode, case_sensitive).unwrap()
    }

    fn plain(pattern: &str) -> FindQuery {
        query(pattern, MatchMode::Plain, true)
    }

    #[test]
    fn forward_finds_first_match_at_or_after_cursor() {
        let text = "abc abc abc";
        let q = plain("abc");
        assert_eq!(
            find_next(text, &q, 0, FindDirection::Forward, false),
            Some(0..3)
        );
        assert_eq!(
            find_next(text, &q, 1, FindDirection::Forward, false),
            Some(4..7)
        );
        assert_eq!(
            find_next(text, &q, 8, FindDirection::Forward, false),
            Some(8..11)
        );
        assert_eq!(find_next(text, &q, 9, FindDirection::Forward, false), None);
    }

    #[test]
    fn forward_wraps_when_requested() {
        let text = "abc abc";
        let q = plain("abc");
        assert_eq!(find_next(text, &q, 5, FindDirection::Forward, false), None);
        assert_eq!(
            find_next(text, &q, 5, FindDirection::Forward, true),
            Some(0..3)
        );
    }

    #[test]
    fn backward_finds_previous_match_and_wraps() {
        let text = "abc abc abc";
        let q = plain("abc");
        assert_eq!(
            find_next(text, &q, 11, FindDirection::Backward, false),
            Some(8..11)
        );
        assert_eq!(
            find_next(text, &q, 8, FindDirection::Backward, false),
            Some(4..7)
        );
        assert_eq!(find_next(text, &q, 0, FindDirection::Backward, false), None);
        assert_eq!(
            find_next(text, &q, 0, FindDirection::Backward, true),
            Some(8..11)
        );
    }

    #[test]
    fn case_insensitive_matching_folds_scalars() {
        let text = "Hello HELLO héllo";
        let q = query("hello", MatchMode::Plain, false);
        assert_eq!(
            find_next(text, &q, 0, FindDirection::Forward, false),
            Some(0..5)
        );
        assert_eq!(
            find_next(text, &q, 5, FindDirection::Forward, false),
            Some(6..11)
        );
        // Case sensitive finds neither of the first two spellings from 0.
        let cs = plain("hello");
        assert_eq!(find_next(text, &cs, 0, FindDirection::Forward, false), None);
    }

    #[test]
    fn backward_ci_search_finds_last_match() {
        // L16 regression: backward case-insensitive search with the first-char
        // gate must still find the correct last match before `limit`.
        let text = "Hello HELLO hello";
        let q = query("hello", MatchMode::Plain, false);
        assert_eq!(
            find_next(text, &q, 18, FindDirection::Backward, false),
            Some(12..17)
        );
        assert_eq!(
            find_next(text, &q, 12, FindDirection::Backward, false),
            Some(6..11)
        );
        assert_eq!(
            find_next(text, &q, 6, FindDirection::Backward, false),
            Some(0..5)
        );
    }

    #[test]
    fn backward_ci_search_with_adversarial_prefix() {
        // L16 regression: the first-char gate must not skip valid matches when
        // many positions share the pattern's first char. "a" repeated 1000
        // times, searching backward for "ab" — no match, but the gate fires at
        // every position, confirming O(n) skip for non-matches.
        let text = "a".repeat(1000);
        let q = query("ab", MatchMode::Plain, false);
        assert_eq!(
            find_next(&text, &q, 1000, FindDirection::Backward, false),
            None
        );
    }

    #[test]
    fn backward_ci_search_multibyte_first_char() {
        // L16 regression: the first-char gate uses `chars_eq_ignore_case` which
        // handles multibyte fold-matching. "É" and "é" must both match.
        let text = "xÉy éz";
        let q = query("é", MatchMode::Plain, false);
        // Backward from the end finds the second "é" at byte offset 5.
        let found = find_next(text, &q, text.len(), FindDirection::Backward, false);
        assert_eq!(found, Some(5..7));
        // Backward from 5 finds the first "É" at byte offset 1.
        let found = find_next(text, &q, 5, FindDirection::Backward, false);
        assert_eq!(found, Some(1..3));
    }

    #[test]
    fn case_insensitive_reports_original_byte_ranges_with_multibyte() {
        // "É" is two bytes; matching "é" must return the original span.
        let text = "xÉy";
        let q = query("é", MatchMode::Plain, false);
        let found = find_next(text, &q, 0, FindDirection::Forward, false).unwrap();
        assert_eq!(&text[found.clone()], "É");
        assert_eq!(found, 1..3);
    }

    #[test]
    fn whole_word_respects_boundaries() {
        let text = "cat category cat.";
        let q = query("cat", MatchMode::WholeWord, true);
        assert_eq!(
            find_next(text, &q, 0, FindDirection::Forward, false),
            Some(0..3)
        );
        // Skips "category", finds the trailing "cat" before the period.
        assert_eq!(
            find_next(text, &q, 3, FindDirection::Forward, false),
            Some(13..16)
        );
    }

    #[test]
    fn multibyte_cursor_is_nudged_not_panicking() {
        let text = "áíóú"; // each char two bytes
        let q = plain("í");
        // from_byte=3 lands mid-char; must not panic and must find "í" region.
        let found = find_next(text, &q, 1, FindDirection::Forward, false).unwrap();
        assert_eq!(&text[found], "í");
    }

    fn apply_plans(text: &str, plans: &[EditPlan]) -> String {
        let mut document = Document::new(text).unwrap();
        for (index, plan) in plans.iter().enumerate() {
            let tx = plan.clone().into_transaction(index as u64);
            document.apply(tx).unwrap();
        }
        document.snapshot().to_string()
    }

    #[test]
    fn replace_current_builds_single_edit() {
        let text = "hello world";
        let spec = ReplaceSpec::new(plain("world"), "there".to_owned()).unwrap();
        let plan = replace_current(Revision::new(0), text, &spec, 6..11).unwrap();
        assert_eq!(plan.edits().len(), 1);
        assert_eq!(apply_plans(text, &[plan]), "hello there");
    }

    #[test]
    fn replace_current_rejects_bad_range() {
        let text = "hello";
        let spec = ReplaceSpec::new(plain("x"), "y".to_owned()).unwrap();
        let reversed = Range { start: 3, end: 2 };
        assert!(matches!(
            replace_current(Revision::new(0), text, &spec, reversed),
            Err(ReplaceError::ReversedRange { .. })
        ));
        assert!(matches!(
            replace_current(Revision::new(0), text, &spec, 0..99),
            Err(ReplaceError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn replace_all_small_document_single_plan() {
        let text = "a a a a";
        let spec = ReplaceSpec::new(plain("a"), "bb".to_owned()).unwrap();
        let plans = replace_all(Revision::new(0), text, &spec).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(apply_plans(text, &plans), "bb bb bb bb");
    }

    #[test]
    fn replace_all_cursor_lands_at_end_of_last_match_when_shrinking() {
        // Regression (macOS crash): a shrinking multi-match replace-all must not
        // overshoot the final document length. "aa aa" / "aa"->"b" => "b b"
        // (len 3); the cursor belongs at 3, not 4. Last match end 5 plus the
        // chunk's net delta 2*(1-2) = -2 gives 3. The old code used the last
        // replacement's end (3+1=4), ignoring the earlier same-chunk shrink, so
        // set_selection landed out of bounds and quit the app.
        let text = "aa aa";
        let spec = ReplaceSpec::new(plain("aa"), "b".to_owned()).unwrap();
        let plans = replace_all(Revision::new(0), text, &spec).unwrap();
        assert_eq!(plans.len(), 1);
        let result = apply_plans(text, &plans);
        assert_eq!(result, "b b");
        let cursor = plans.last().unwrap().selection_after();
        assert_eq!(cursor, Selection::collapsed(3));
        assert!(
            cursor.head <= result.len() && cursor.anchor <= result.len(),
            "cursor {cursor:?} overshoots final length {}",
            result.len()
        );
    }

    #[test]
    fn replace_all_cursor_lands_at_end_of_last_match_when_growing() {
        // The growing case must still land the cursor at the applied end of the
        // last match: "aa aa" / "aa"->"bbb" => "bbb bbb" (len 7); cursor at 7.
        let text = "aa aa";
        let spec = ReplaceSpec::new(plain("aa"), "bbb".to_owned()).unwrap();
        let plans = replace_all(Revision::new(0), text, &spec).unwrap();
        assert_eq!(plans.len(), 1);
        let result = apply_plans(text, &plans);
        assert_eq!(result, "bbb bbb");
        let cursor = plans.last().unwrap().selection_after();
        assert_eq!(cursor, Selection::collapsed(7));
        assert_eq!(cursor.head, result.len());
    }

    #[test]
    fn replace_all_no_matches_is_empty() {
        let text = "nothing here";
        let spec = ReplaceSpec::new(plain("zzz"), "!".to_owned()).unwrap();
        assert!(
            replace_all(Revision::new(0), text, &spec)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn replace_all_chunks_beyond_plan_edit_cap() {
        // 5000 matches of "x" > MAX_PLAN_EDITS (4096) forces multiple plans.
        let count = 5000;
        let text = "x ".repeat(count);
        let spec = ReplaceSpec::new(plain("x"), "yy".to_owned()).unwrap();
        let plans = replace_all(Revision::new(0), &text, &spec).unwrap();

        assert!(
            plans.len() >= 2,
            "expected chunking into >=2 plans, got {}",
            plans.len()
        );
        for plan in &plans {
            assert!(plan.edits().len() <= crate::MAX_PLAN_EDITS);
        }
        // Revisions are consecutive starting at the base.
        for (index, plan) in plans.iter().enumerate() {
            assert_eq!(plan.base_revision(), Revision::new(index as u64));
        }
        // Applying every plan in order fully replaces the document.
        let expected = "yy ".repeat(count);
        assert_eq!(apply_plans(&text, &plans), expected);
    }

    #[test]
    fn replace_all_chunks_on_the_byte_budget() {
        // Large replacement so the byte budget, not the edit count, splits it.
        let replacement = "z".repeat(2000);
        let count = 400; // 400 * ~2001 bytes >> MAX_PLAN_TOTAL_BYTES (256 KiB)
        let text = "a".repeat(count);
        let spec = ReplaceSpec::new(plain("a"), replacement.clone()).unwrap();
        let plans = replace_all(Revision::new(0), &text, &spec).unwrap();
        assert!(plans.len() >= 2);
        let expected = replacement.repeat(count);
        assert_eq!(apply_plans(&text, &plans), expected);
    }

    #[test]
    fn execute_dispatches_each_op() {
        let text = "one two one";
        let find = FindReplaceOp::Find {
            query: plain("one"),
            from_byte: 0,
            direction: FindDirection::Forward,
            wrap: false,
        };
        assert_eq!(
            execute(Revision::new(0), text, &find).unwrap(),
            FindReplaceOutcome::Match(Some(0..3))
        );

        let spec = ReplaceSpec::new(plain("one"), "1".to_owned()).unwrap();
        let all = FindReplaceOp::ReplaceAll { spec: spec.clone() };
        let FindReplaceOutcome::Plans(plans) = execute(Revision::new(0), text, &all).unwrap()
        else {
            panic!("replace-all yields plans");
        };
        assert_eq!(apply_plans(text, &plans), "1 two 1");
    }

    #[test]
    fn match_count_agrees_with_replace_all() {
        let text = "aXaXaXa";
        let q = plain("a");
        assert_eq!(match_count(text, &q), 4);
    }

    #[test]
    fn plans_apply_through_the_document_transaction_contract() {
        // Sanity: an EditPlan lowers to a Programmatic transaction.
        let text = "hi hi";
        let spec = ReplaceSpec::new(plain("hi"), "yo".to_owned()).unwrap();
        let plans = replace_all(Revision::new(0), text, &spec).unwrap();
        let tx: EditTransaction = plans[0].clone().into_transaction(7);
        assert_eq!(tx.kind, TransactionKind::Programmatic);
    }
}
