//! Command palette interaction contract (roadmap 06).
//!
//! The palette is the user-facing surface over the [`ActionRegistry`] (locked
//! by roadmap 03). The registry *describes* commands; this module drives the
//! query → rank → select → dispatch interaction. The reducer owns a
//! [`CommandPalette`] and advances it through [`AppMessage`](crate::app::AppMessage)
//! variants; the platform shell renders the [`PaletteCandidate`] rows and
//! resolves keyboard navigation.
//!
//! # Design
//!
//! See `docs/plan/command-palette-design.md`. Key decisions: one source of
//! truth (the registry), prefix > word-prefix > substring ranking, live
//! availability via `CommandDescriptor::message`, dispatch never bypasses the
//! reducer.
//!
//! # Security-core fence
//!
//! No code here constructs raw HTML/URLs or bypasses the security core. The
//! palette only produces [`AppMessage`](crate::app::AppMessage) values that the
//! existing reducer already validates.

use crate::actions::{ActionRegistry, CommandCategory, CommandDescriptor, CommandId, Shortcut};

/// Maximum number of candidate rows the palette surfaces for one query.
pub const MAX_PALETTE_RESULTS: usize = 50;

/// How well a command matches the query (lower ordinal = better).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchRank {
    /// The query equals the command id exactly.
    ExactId,
    /// The title starts with the query.
    TitlePrefix,
    /// A whitespace-delimited word in the title starts with the query.
    WordPrefix,
    /// The query is a substring of the title or id (the catch-all tier).
    Substring,
}

/// A ranked, owned palette row. Availability is resolved live by the shell
/// (via `CommandDescriptor::message(&AppState)`), so it is not stored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteCandidate {
    pub id: CommandId,
    pub title: &'static str,
    pub category: CommandCategory,
    pub shortcut: Option<Shortcut>,
    pub rank: MatchRank,
}

/// The palette interaction state.
///
/// Owned by [`AppState`](crate::app::AppState) and driven through reducer
/// messages. The shell reads [`candidates`](CommandPalette::candidates) and
/// [`selected_index`](CommandPalette::selected_index) to render.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPalette {
    open: bool,
    query: String,
    candidates: Vec<PaletteCandidate>,
    selected: usize,
}

impl CommandPalette {
    /// Whether the palette is currently visible.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the palette (clearing any previous query).
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
    }

    /// Closes the palette and resets its query/selection.
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
    }

    /// The current query string.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The ranked candidate rows for the current query.
    #[must_use]
    pub fn candidates(&self) -> &[PaletteCandidate] {
        &self.candidates
    }

    /// The index of the selected row, or `None` when there are no candidates.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        (!self.candidates.is_empty()).then_some(self.selected)
    }

    /// The selected candidate, or `None` when empty.
    #[must_use]
    pub fn selected_candidate(&self) -> Option<&PaletteCandidate> {
        self.candidates.get(self.selected)
    }

    /// Recomputes the ranked candidate list for `query` and resets selection to
    /// the first row. An empty query surfaces every command in registry order.
    pub fn set_query(&mut self, query: &str, registry: &ActionRegistry) {
        self.query.clear();
        self.query.push_str(query);

        let q = query.trim().to_ascii_lowercase();
        let mut ranked: Vec<PaletteCandidate> = registry
            .iter()
            .filter_map(|desc| {
                rank_command(desc, &q).map(|rank| PaletteCandidate {
                    id: desc.id,
                    title: desc.title,
                    category: desc.category,
                    shortcut: desc.shortcut,
                    rank,
                })
            })
            .collect();

        // Stable sort by rank: the best tier wins, ties keep registry order
        // (static catalog before dynamic, then declaration order).
        ranked.sort_by_key(|c| c.rank);
        ranked.truncate(MAX_PALETTE_RESULTS);

        self.candidates = ranked;
        self.selected = 0;
    }

    /// Moves the selection down by `delta`, clamped to the candidate range.
    /// Returns whether the selection changed.
    pub fn move_selection(&mut self, delta: i32) -> bool {
        if self.candidates.is_empty() {
            return false;
        }
        let last = self.candidates.len() - 1;
        let target = (self.selected as i32)
            .saturating_add(delta)
            .clamp(0, last as i32) as usize;
        if target == self.selected {
            false
        } else {
            self.selected = target;
            true
        }
    }
}

/// Ranks a command against a trimmed, lowercased query `q`.
/// Returns `None` when the command does not match.
fn rank_command(desc: &CommandDescriptor, q: &str) -> Option<MatchRank> {
    if q.is_empty() {
        // Empty query: every command matches, preserving registry order via the
        // shared lowest tier + stable sort.
        return Some(MatchRank::Substring);
    }
    let id = desc.id.0.to_ascii_lowercase();
    let title = desc.title.to_ascii_lowercase();

    if id == q {
        return Some(MatchRank::ExactId);
    }
    if title.starts_with(q) {
        return Some(MatchRank::TitlePrefix);
    }
    if word_prefix_match(&title, q) {
        return Some(MatchRank::WordPrefix);
    }
    if title.contains(q) || id.contains(q) {
        return Some(MatchRank::Substring);
    }
    None
}

/// True when any whitespace-delimited word in `title` starts with `q`.
fn word_prefix_match(title: &str, q: &str) -> bool {
    title
        .split(|c: char| c.is_whitespace())
        .any(|word| !word.is_empty() && word.starts_with(q))
}

/// Convenience: a localized-agnostic display string for a shortcut.
/// macOS-style glyphs ("⇧⌘P"). The platform shell is the canonical resolver.
impl Shortcut {
    /// Returns a display string like `"⇧⌘P"`, or `None` when there is no key.
    #[must_use]
    pub fn display(&self) -> Option<String> {
        if self.key.is_empty() {
            return None;
        }
        let mut s = String::new();
        if self.modifiers.ctrl {
            s.push('^');
        }
        if self.modifiers.alt {
            s.push('⌥');
        }
        if self.modifiers.shift {
            s.push('⇧');
        }
        if self.modifiers.cmd {
            s.push('⌘');
        }
        // Capitalize the first ASCII letter for readability ("s" -> "S",
        // "f1" -> "F1"); leave multi-word keys ("space") untouched.
        let key = match self.key.chars().next() {
            Some(c) if c.is_ascii_lowercase() => {
                let mut owned = self.key.to_string();
                owned.make_ascii_uppercase();
                owned
            }
            _ => self.key.to_string(),
        };
        s.push_str(&key);
        Some(s)
    }
}

#[cfg(test)]
mod command_palette_tests {
    use super::*;
    use crate::actions::{CommandCategory, CommandDescriptor, Shortcut};

    fn desc(id: &'static str, title: &'static str) -> CommandDescriptor {
        CommandDescriptor {
            id: CommandId(id),
            title,
            category: CommandCategory::File,
            shortcut: None,
            message: |_| None,
        }
    }

    fn registry(cmds: &[CommandDescriptor]) -> ActionRegistry {
        // Bypass the static-catalog API for test-only dynamic registries.
        let mut reg = ActionRegistry::from_static(&[]);
        for c in cmds {
            reg.register(*c).unwrap();
        }
        reg
    }

    // -- Ranking tiers -------------------------------------------------------

    #[test]
    fn empty_query_surfaces_all_in_order() {
        let reg = registry(&[desc("file.new", "New"), desc("file.save", "Save")]);
        let mut p = CommandPalette::default();
        p.open();
        p.set_query("", &reg);
        assert_eq!(p.candidates().len(), 2);
        assert_eq!(p.candidates()[0].id.0, "file.new");
        assert_eq!(p.candidates()[1].id.0, "file.save");
    }

    #[test]
    fn exact_id_match_ranks_above_title_prefix() {
        let reg = registry(&[
            desc("file.save", "Save"),        // title prefix for "save"
            desc("save", "Persist Document"), // exact id for "save"
        ]);
        let mut p = CommandPalette::default();
        p.set_query("save", &reg);
        assert_eq!(p.candidates()[0].id.0, "save");
        assert_eq!(p.candidates()[0].rank, MatchRank::ExactId);
        assert_eq!(p.candidates()[1].id.0, "file.save");
        assert_eq!(p.candidates()[1].rank, MatchRank::TitlePrefix);
    }

    #[test]
    fn word_prefix_match_detected() {
        let reg = registry(&[desc("file.save-as", "Save As…")]);
        let mut p = CommandPalette::default();
        p.set_query("as", &reg);
        assert_eq!(p.candidates().len(), 1);
        assert_eq!(p.candidates()[0].rank, MatchRank::WordPrefix);
    }

    #[test]
    fn substring_is_lowest_tier() {
        let reg = registry(&[desc("file.new", "New Document")]);
        let mut p = CommandPalette::default();
        // "umen" is inside "Document" but not a prefix of any word.
        p.set_query("umen", &reg);
        assert_eq!(p.candidates().len(), 1);
        assert_eq!(p.candidates()[0].rank, MatchRank::Substring);
    }

    #[test]
    fn id_substring_also_matches() {
        let reg = registry(&[desc("format.toggle-bold", "Bold")]);
        let mut p = CommandPalette::default();
        p.set_query("format", &reg);
        assert_eq!(p.candidates().len(), 1);
    }

    #[test]
    fn no_match_returns_empty() {
        let reg = registry(&[desc("file.save", "Save")]);
        let mut p = CommandPalette::default();
        p.set_query("zzz", &reg);
        assert!(p.candidates().is_empty());
        assert!(p.selected_index().is_none());
    }

    // -- Case insensitivity --------------------------------------------------

    #[test]
    fn query_is_case_insensitive() {
        let reg = registry(&[desc("file.save", "Save")]);
        let mut p = CommandPalette::default();
        p.set_query("SAVE", &reg);
        assert_eq!(p.candidates().len(), 1);
        assert_eq!(p.candidates()[0].rank, MatchRank::TitlePrefix);
    }

    // -- Selection ------------------------------------------------------------

    #[test]
    fn selection_starts_at_first_row() {
        let reg = registry(&[desc("a", "A"), desc("b", "B"), desc("c", "C")]);
        let mut p = CommandPalette::default();
        p.set_query("", &reg);
        assert_eq!(p.selected_index(), Some(0));
        assert_eq!(p.selected_candidate().unwrap().id.0, "a");
    }

    #[test]
    fn move_selection_clamps_to_bounds() {
        let reg = registry(&[desc("a", "A"), desc("b", "B")]);
        let mut p = CommandPalette::default();
        p.set_query("", &reg);
        assert!(p.move_selection(1));
        assert_eq!(p.selected_index(), Some(1));
        assert!(!p.move_selection(1)); // clamped at last
        assert_eq!(p.selected_index(), Some(1));
        assert!(p.move_selection(-5)); // clamped at 0
        assert_eq!(p.selected_index(), Some(0));
    }

    #[test]
    fn move_selection_noop_when_empty() {
        let mut p = CommandPalette::default();
        assert!(!p.move_selection(1));
        assert!(p.selected_index().is_none());
    }

    #[test]
    fn set_query_resets_selection() {
        let reg = registry(&[desc("a", "A"), desc("b", "B")]);
        let mut p = CommandPalette::default();
        p.set_query("", &reg);
        p.move_selection(1);
        assert_eq!(p.selected_index(), Some(1));
        p.set_query("a", &reg);
        assert_eq!(p.selected_index(), Some(0));
    }

    // -- open/close lifecycle ------------------------------------------------

    #[test]
    fn open_close_resets_state() {
        let reg = registry(&[desc("a", "A")]);
        let mut p = CommandPalette::default();
        p.open();
        assert!(p.is_open());
        p.set_query("a", &reg);
        assert!(!p.candidates().is_empty());
        p.close();
        assert!(!p.is_open());
        assert!(p.query().is_empty());
        assert!(p.candidates().is_empty());
    }

    // -- Result cap -----------------------------------------------------------

    #[test]
    fn results_capped_at_max() {
        let mut cmds = Vec::new();
        for i in 0..(MAX_PALETTE_RESULTS + 20) {
            cmds.push(desc(Box::leak(format!("c.{i}").into_boxed_str()), "cmd"));
        }
        let reg = registry(&cmds);
        let mut p = CommandPalette::default();
        p.set_query("", &reg);
        assert_eq!(p.candidates().len(), MAX_PALETTE_RESULTS);
    }

    // -- Shortcut display -----------------------------------------------------

    #[test]
    fn shortcut_display_macos_glyphs() {
        assert_eq!(Shortcut::cmd("s").display().unwrap(), "⌘S");
        assert_eq!(Shortcut::cmd_shift("p").display().unwrap(), "⇧⌘P");
        assert_eq!(Shortcut::cmd("f1").display().unwrap(), "⌘F1");
    }
}
