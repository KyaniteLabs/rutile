//! Document counters: words, characters, and reading time (SPEC §9).
//!
//! Every function here is a pure, single-pass, allocation-free scan over a
//! borrowed `&str`, so counting a 20 MiB document costs one linear walk and no
//! heap traffic. Counts are computed in core and displayed by the native
//! status bars; the shells never re-implement them.
//!
//! ## Definitions (documented, deliberate)
//!
//! - **Words** are maximal runs of non-whitespace separated by Unicode
//!   whitespace, i.e. exactly [`str::split_whitespace`]. This matches what a
//!   status-bar "word count" is expected to report and needs no dictionary.
//! - **Characters** are counted as **Unicode scalar values** (Rust [`char`]),
//!   *not* grapheme clusters. Scalar counting is O(n), dependency-free, and
//!   stable; grapheme segmentation would pull in a Unicode-tables crate and is
//!   deliberately deferred. A "character" here is therefore one `char`, so a
//!   base letter plus a combining mark counts as two, and an astral emoji
//!   counts as one.
//! - **Reading time** derives from the word count at [`READING_WPM`] words per
//!   minute; see [`reading_time_seconds`].

/// Words-per-minute constant behind the reading-time estimate.
///
/// 200 WPM is the conventional midpoint for adult silent reading of prose and
/// is the value the status bar advertises.
pub const READING_WPM: usize = 200;

/// Word, character, and (derived) reading-time counts for a document, all from
/// a single scan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    /// Whitespace-separated words (see module docs).
    pub words: usize,
    /// Unicode scalar values (see module docs).
    pub chars: usize,
}

impl Counts {
    /// Estimated reading time in whole seconds at [`READING_WPM`], rounded to
    /// the nearest second. Zero words yields zero seconds.
    pub fn reading_time_seconds(self) -> u64 {
        reading_time_seconds_from_words(self.words)
    }
}

/// Computes word and character counts in one linear pass.
pub fn counts(text: &str) -> Counts {
    Counts {
        words: word_count(text),
        chars: char_count(text),
    }
}

/// Counts whitespace-separated words (Unicode whitespace via
/// [`str::split_whitespace`]). Empty and whitespace-only input yield `0`.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Counts Unicode scalar values ([`char`]s). See the module docs for why this
/// is scalars, not grapheme clusters.
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// Estimated reading time in whole seconds at [`READING_WPM`], rounded to the
/// nearest second, computed from `text`'s word count.
pub fn reading_time_seconds(text: &str) -> u64 {
    reading_time_seconds_from_words(word_count(text))
}

fn reading_time_seconds_from_words(words: usize) -> u64 {
    // seconds = round(words / WPM * 60), integer arithmetic, no panic:
    // READING_WPM is a nonzero constant.
    let words = words as u64;
    let wpm = READING_WPM as u64;
    (words.saturating_mul(60).saturating_add(wpm / 2)) / wpm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_counts_zero() {
        let counts = counts("");
        assert_eq!(counts, Counts { words: 0, chars: 0 });
        assert_eq!(counts.reading_time_seconds(), 0);
    }

    #[test]
    fn whitespace_only_has_no_words_but_counts_chars() {
        let text = "   \n\t \r\n";
        assert_eq!(word_count(text), 0);
        assert_eq!(char_count(text), text.chars().count());
        assert_eq!(reading_time_seconds(text), 0);
    }

    #[test]
    fn ascii_prose_counts_words_and_chars() {
        let text = "the quick brown fox";
        assert_eq!(word_count(text), 4);
        assert_eq!(char_count(text), 19);
    }

    #[test]
    fn unicode_words_and_scalar_chars() {
        // "café 世界 😀": three whitespace-separated words.
        // chars: c,a,f,é (precomposed) = 4; ' ' = 1; 世,界 = 2; ' ' = 1;
        // 😀 (one astral scalar) = 1 => 9 scalar values.
        let text = "caf\u{e9} \u{4e16}\u{754c} \u{1f600}";
        assert_eq!(word_count(text), 3);
        assert_eq!(char_count(text), 9);
    }

    #[test]
    fn combining_marks_count_per_scalar_not_grapheme() {
        // "e" + combining acute accent is two scalar values, one grapheme.
        let text = "e\u{301}";
        assert_eq!(char_count(text), 2);
    }

    #[test]
    fn reading_time_rounds_to_nearest_second() {
        // 100 words at 200 WPM = 0.5 min = 30 s.
        let text = "word ".repeat(100);
        assert_eq!(word_count(&text), 100);
        assert_eq!(reading_time_seconds(&text), 30);

        // 200 words = exactly one minute.
        let text = "word ".repeat(200);
        assert_eq!(reading_time_seconds(&text), 60);

        // 3 words: 3/200*60 = 0.9 s, rounds to 1 s.
        assert_eq!(super::reading_time_seconds_from_words(3), 1);
        // 1 word: 0.3 s rounds to 0.
        assert_eq!(super::reading_time_seconds_from_words(1), 0);
    }

    #[test]
    fn leading_and_trailing_whitespace_does_not_inflate_words() {
        assert_eq!(word_count("   hello   world   "), 2);
    }
}
