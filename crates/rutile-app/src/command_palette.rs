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

/// One projected palette row for a native panel (availability is live).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRowView {
    pub title: &'static str,
    pub shortcut: Option<String>,
    pub available: bool,
    pub selected: bool,
}

/// Snapshot the shell renders into an `NSPanel`. `None` means the panel is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalettePanelSnapshot {
    pub query: String,
    pub rows: Vec<PaletteRowView>,
}

impl PalettePanelSnapshot {
    /// Plain-text body for the nonactivating display panel.
    #[must_use]
    pub fn display_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Filter: ");
        if self.query.is_empty() {
            out.push_str("(all commands)");
        } else {
            out.push_str(&self.query);
        }
        out.push('\n');
        for row in &self.rows {
            let marker = if row.selected { '>' } else { ' ' };
            let avail = if row.available { ' ' } else { '·' };
            let shortcut = row.shortcut.as_deref().unwrap_or("");
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("\n{marker}{avail}{title:<28} {shortcut}", title = row.title),
            );
        }
        out
    }
}

/// Lifecycle of the command palette.
///
/// ```text
///            open()                 set_query() / move_selection()
///   Closed ─────────▶ Open{query,candidates,selected} ◀──────┐
///     ▲                 │  │                                 │
///     │    close()      │  └─────────────────────────────────┘
///     └─────────────────┘       open() on Open = reset to empty query
/// ```
///
/// Query, candidates, and selection only exist while the palette is open, so
/// "closed but holding candidates" (which previously let
/// [`AppMessage::PaletteSubmit`](crate::app::AppMessage::PaletteSubmit)
/// dispatch a command from an invisible palette) is unrepresentable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PalettePhase {
    #[default]
    Closed,
    Open {
        query: String,
        candidates: Vec<PaletteCandidate>,
        selected: usize,
    },
}

/// Error returned when a palette transition is attempted from the wrong phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PaletteError {
    #[error("the command palette is closed")]
    Closed,
}

/// The palette interaction state.
///
/// Owned by [`AppState`](crate::app::AppState) and driven through reducer
/// messages. The shell reads [`candidates`](CommandPalette::candidates) and
/// [`selected_index`](CommandPalette::selected_index) to render. Every write
/// goes through the named transitions on this type; see [`PalettePhase`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPalette {
    phase: PalettePhase,
}

impl CommandPalette {
    /// The current lifecycle phase.
    #[must_use]
    pub const fn phase(&self) -> &PalettePhase {
        &self.phase
    }

    /// Whether the palette is currently visible.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.phase, PalettePhase::Open { .. })
    }

    /// Transition `Closed | Open → Open` with an empty query and no rows.
    pub fn open(&mut self) {
        self.phase = PalettePhase::Open {
            query: String::new(),
            candidates: Vec::new(),
            selected: 0,
        };
    }

    /// Transition `Open | Closed → Closed`, dropping query and selection.
    pub fn close(&mut self) {
        self.phase = PalettePhase::Closed;
    }

    /// The current query string (empty while closed).
    #[must_use]
    pub fn query(&self) -> &str {
        match &self.phase {
            PalettePhase::Open { query, .. } => query,
            PalettePhase::Closed => "",
        }
    }

    /// The ranked candidate rows for the current query (empty while closed).
    #[must_use]
    pub fn candidates(&self) -> &[PaletteCandidate] {
        match &self.phase {
            PalettePhase::Open { candidates, .. } => candidates,
            PalettePhase::Closed => &[],
        }
    }

    /// The index of the selected row, or `None` when closed or empty.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        match &self.phase {
            PalettePhase::Open {
                candidates,
                selected,
                ..
            } => (!candidates.is_empty()).then_some(*selected),
            PalettePhase::Closed => None,
        }
    }

    /// The selected candidate, or `None` when closed or empty.
    #[must_use]
    pub fn selected_candidate(&self) -> Option<&PaletteCandidate> {
        match &self.phase {
            PalettePhase::Open {
                candidates,
                selected,
                ..
            } => candidates.get(*selected),
            PalettePhase::Closed => None,
        }
    }

    /// Transition `Open → Open`: recomputes the ranked candidate list for
    /// `query` and resets selection to the first row. An empty query surfaces
    /// every command in registry order.
    ///
    /// # Errors
    ///
    /// [`PaletteError::Closed`] when the palette is not open; state is untouched.
    pub fn set_query(
        &mut self,
        query: &str,
        registry: &ActionRegistry,
    ) -> Result<(), PaletteError> {
        let PalettePhase::Open {
            query: stored,
            candidates,
            selected,
        } = &mut self.phase
        else {
            return Err(PaletteError::Closed);
        };
        stored.clear();
        stored.push_str(query);

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

        *candidates = ranked;
        *selected = 0;
        Ok(())
    }

    /// Transition `Open → Open`: moves the selection by `delta`, clamped to the
    /// candidate range. Returns whether the selection changed (always `false`
    /// while closed or empty).
    pub fn move_selection(&mut self, delta: i32) -> bool {
        let PalettePhase::Open {
            candidates,
            selected,
            ..
        } = &mut self.phase
        else {
            return false;
        };
        if candidates.is_empty() {
            return false;
        }
        let last = candidates.len() - 1;
        let target = (*selected as i32)
            .saturating_add(delta)
            .clamp(0, last as i32) as usize;
        if target == *selected {
            false
        } else {
            *selected = target;
            true
        }
    }

    /// Transition `Open → Closed`, yielding the candidate that was selected.
    ///
    /// # Errors
    ///
    /// [`PaletteError::Closed`] when the palette is not open: a submit from a
    /// closed palette must never dispatch a command.
    pub fn submit(&mut self) -> Result<Option<PaletteCandidate>, PaletteError> {
        if !self.is_open() {
            return Err(PaletteError::Closed);
        }
        let chosen = self.selected_candidate().copied();
        self.close();
        Ok(chosen)
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

    fn opened() -> CommandPalette {
        let mut p = CommandPalette::default();
        p.open();
        p
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
        let mut p = opened();
        p.open();
        p.set_query("", &reg).unwrap();
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
        let mut p = opened();
        p.set_query("save", &reg).unwrap();
        assert_eq!(p.candidates()[0].id.0, "save");
        assert_eq!(p.candidates()[0].rank, MatchRank::ExactId);
        assert_eq!(p.candidates()[1].id.0, "file.save");
        assert_eq!(p.candidates()[1].rank, MatchRank::TitlePrefix);
    }

    #[test]
    fn word_prefix_match_detected() {
        let reg = registry(&[desc("file.save-as", "Save As…")]);
        let mut p = opened();
        p.set_query("as", &reg).unwrap();
        assert_eq!(p.candidates().len(), 1);
        assert_eq!(p.candidates()[0].rank, MatchRank::WordPrefix);
    }

    #[test]
    fn substring_is_lowest_tier() {
        let reg = registry(&[desc("file.new", "New Document")]);
        let mut p = opened();
        // "umen" is inside "Document" but not a prefix of any word.
        p.set_query("umen", &reg).unwrap();
        assert_eq!(p.candidates().len(), 1);
        assert_eq!(p.candidates()[0].rank, MatchRank::Substring);
    }

    #[test]
    fn id_substring_also_matches() {
        let reg = registry(&[desc("format.toggle-bold", "Bold")]);
        let mut p = opened();
        p.set_query("format", &reg).unwrap();
        assert_eq!(p.candidates().len(), 1);
    }

    #[test]
    fn no_match_returns_empty() {
        let reg = registry(&[desc("file.save", "Save")]);
        let mut p = opened();
        p.set_query("zzz", &reg).unwrap();
        assert!(p.candidates().is_empty());
        assert!(p.selected_index().is_none());
    }

    // -- Case insensitivity --------------------------------------------------

    #[test]
    fn query_is_case_insensitive() {
        let reg = registry(&[desc("file.save", "Save")]);
        let mut p = opened();
        p.set_query("SAVE", &reg).unwrap();
        assert_eq!(p.candidates().len(), 1);
        assert_eq!(p.candidates()[0].rank, MatchRank::TitlePrefix);
    }

    // -- Selection ------------------------------------------------------------

    #[test]
    fn selection_starts_at_first_row() {
        let reg = registry(&[desc("a", "A"), desc("b", "B"), desc("c", "C")]);
        let mut p = opened();
        p.set_query("", &reg).unwrap();
        assert_eq!(p.selected_index(), Some(0));
        assert_eq!(p.selected_candidate().unwrap().id.0, "a");
    }

    #[test]
    fn move_selection_clamps_to_bounds() {
        let reg = registry(&[desc("a", "A"), desc("b", "B")]);
        let mut p = opened();
        p.set_query("", &reg).unwrap();
        assert!(p.move_selection(1));
        assert_eq!(p.selected_index(), Some(1));
        assert!(!p.move_selection(1)); // clamped at last
        assert_eq!(p.selected_index(), Some(1));
        assert!(p.move_selection(-5)); // clamped at 0
        assert_eq!(p.selected_index(), Some(0));
    }

    #[test]
    fn move_selection_noop_when_empty() {
        let mut p = opened();
        assert!(!p.move_selection(1));
        assert!(p.selected_index().is_none());
    }

    #[test]
    fn set_query_resets_selection() {
        let reg = registry(&[desc("a", "A"), desc("b", "B")]);
        let mut p = opened();
        p.set_query("", &reg).unwrap();
        p.move_selection(1);
        assert_eq!(p.selected_index(), Some(1));
        p.set_query("a", &reg).unwrap();
        assert_eq!(p.selected_index(), Some(0));
    }

    // -- open/close lifecycle ------------------------------------------------

    #[test]
    fn open_close_resets_state() {
        let reg = registry(&[desc("a", "A")]);
        let mut p = CommandPalette::default();
        p.open();
        assert!(p.is_open());
        p.set_query("a", &reg).unwrap();
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
        let mut p = opened();
        p.set_query("", &reg).unwrap();
        assert_eq!(p.candidates().len(), MAX_PALETTE_RESULTS);
    }

    // -- Shortcut display -----------------------------------------------------

    #[test]
    fn shortcut_display_macos_glyphs() {
        assert_eq!(Shortcut::cmd("s").display().unwrap(), "⌘S");
        assert_eq!(Shortcut::cmd_shift("p").display().unwrap(), "⇧⌘P");
        assert_eq!(Shortcut::cmd("f1").display().unwrap(), "⌘F1");
    }

    // -- Phase machine ---------------------------------------------------------

    /// Exhaustive over [`PalettePhase`]: adding a phase without classifying it
    /// here fails to compile.
    fn phase_is_open(phase: &PalettePhase) -> bool {
        match phase {
            PalettePhase::Closed => false,
            PalettePhase::Open { .. } => true,
        }
    }

    #[test]
    fn default_phase_is_closed_and_projects_empty() {
        let p = CommandPalette::default();
        assert_eq!(p.phase(), &PalettePhase::Closed);
        assert!(!phase_is_open(p.phase()));
        assert!(p.query().is_empty());
        assert!(p.candidates().is_empty());
        assert_eq!(p.selected_index(), None);
        assert_eq!(p.selected_candidate(), None);
    }

    #[test]
    fn closed_palette_refuses_query_selection_and_submit() {
        let reg = registry(&[desc("a", "A"), desc("b", "B")]);
        let mut p = CommandPalette::default();
        assert_eq!(p.set_query("", &reg), Err(PaletteError::Closed));
        assert!(!p.move_selection(1));
        assert_eq!(p.submit(), Err(PaletteError::Closed));
        assert_eq!(
            p,
            CommandPalette::default(),
            "refused moves leave state untouched"
        );
    }

    #[test]
    fn submit_returns_selection_and_closes() {
        let reg = registry(&[desc("a", "A"), desc("b", "B")]);
        let mut p = opened();
        p.set_query("", &reg).unwrap();
        p.move_selection(1);
        let chosen = p.submit().unwrap().expect("row selected");
        assert_eq!(chosen.id.0, "b");
        assert_eq!(p.phase(), &PalettePhase::Closed);
        // Submit on an open-but-empty palette closes with no command.
        let mut p = opened();
        assert_eq!(p.submit(), Ok(None));
        assert!(!p.is_open());
    }

    #[test]
    fn reopen_resets_query() {
        let reg = registry(&[desc("a", "A")]);
        let mut p = opened();
        p.set_query("a", &reg).unwrap();
        p.open();
        assert!(p.query().is_empty());
        assert!(p.candidates().is_empty());
    }

    /// Stateful model test: drive random transition sequences (deterministic
    /// LCG, no extra dependency) and check the invariants after every step.
    #[test]
    fn random_transition_sequences_preserve_invariants() {
        let reg = registry(&[
            desc("file.new", "New"),
            desc("file.save", "Save"),
            desc("file.save-as", "Save As"),
            desc("view.split", "Split View"),
        ]);
        let queries = ["", "s", "save", "zzz", "view"];
        let mut seed: u64 = 0x5eed_cafe;
        for _run in 0..200 {
            let mut p = CommandPalette::default();
            let mut model_open = false;
            for _step in 0..40 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let pick = (seed >> 33) as usize;
                match pick % 6 {
                    0 => {
                        p.open();
                        model_open = true;
                    }
                    1 => {
                        p.close();
                        model_open = false;
                    }
                    2 => {
                        let r = p.set_query(queries[pick % queries.len()], &reg);
                        assert_eq!(r.is_ok(), model_open);
                    }
                    3 => {
                        let changed = p.move_selection(1);
                        assert!(model_open || !changed);
                    }
                    4 => {
                        let changed = p.move_selection(-1);
                        assert!(model_open || !changed);
                    }
                    _ => {
                        let r = p.submit();
                        assert_eq!(r.is_ok(), model_open);
                        model_open = false;
                    }
                }
                // Invariants.
                assert_eq!(p.is_open(), model_open);
                assert_eq!(phase_is_open(p.phase()), model_open);
                if !model_open {
                    assert!(p.query().is_empty());
                    assert!(p.candidates().is_empty());
                    assert_eq!(p.selected_index(), None);
                }
                if let Some(i) = p.selected_index() {
                    assert!(i < p.candidates().len());
                }
                assert!(p.candidates().len() <= MAX_PALETTE_RESULTS);
            }
        }
    }
}
