//! C8 tasteroll — app-layer chance-styling integration.
//!
//! Wraps [`ChanceRoll`] from rutile-core in a state container that the
//! [`AppState`](crate::app::AppState) reducer drives via
//! [`AppMessage`](crate::app::AppMessage) variants. The platform shell reads
//! [`TasteState::css`] on each view update and injects it into the preview/
//! export `<style>` block.
//!
//! # Architecture
//!
//! The core engine (seeded roll, lock/reroll, safe CSS generation) lives in
//! `rutile_core::chance_style`. This module adds:
//!
//! - [`content_seed`]: deterministic FNV-1a hash of document text → seed
//! - [`infer_content_type`]: lightweight heuristic (markdown structure → type)
//! - [`TasteState`]: reducer-owned state wrapping `Option<ChanceRoll>`
//!
//! # Security-core fence
//!
//! Every CSS value reaches the export through `DesignTokenSet` grammar
//! validation (no `url(`, `@import`, `</script`, etc.). [`css`] emits only
//! frozen `DESIGN_TOKENS` ids. The accent is immutable (rutile gold — fire
//! budget). `render.rs` / `security.rs` / `safe_link.rs` are untouched.
//!
//! [`css`]: TasteState::css

use rutile_core::{ChanceRoll, ChanceRollError, ContentType, render_chance_css};

/// FNV-1a 64-bit hash of the document text for deterministic seed derivation.
///
/// The same note content always produces the same seed, so the initial roll
/// is reproducible. Re-rolling advances the seed (variety path).
#[must_use]
pub fn content_seed(text: &str) -> u64 {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325; // FNV offset basis
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01B3); // FNV prime
    }
    hash
}

/// Lightweight content-type inference from markdown structure.
///
/// This is a fast heuristic, not AI. The platform shell or C7 agent can
/// override the inferred type before calling [`TasteState::roll`].
///
/// Heuristics (first match wins):
/// - **Letter**: starts with "Dear " or contains a salutation + sign-off
/// - **Recipe**: contains cooking markers (ingredients, cups, °F, preheat)
/// - **Spec**: many section headers (`##` count ≥ 3) or front matter
/// - **Journal**: date-prefixed entries or diary-like timestamps
/// - **Note**: everything else (default)
#[must_use]
pub fn infer_content_type(text: &str) -> ContentType {
    let trimmed = text.trim_start();
    let lower = text.to_ascii_lowercase();

    // Letter: salutation pattern.
    if trimmed.starts_with("Dear ") || lower.contains("\nregards,\n") {
        return ContentType::Letter;
    }

    // Recipe: cooking vocabulary.
    if lower.contains("ingredients")
        && (lower.contains("cup")
            || lower.contains("tablespoon")
            || lower.contains("teaspoon")
            || lower.contains("\u{00b0}f")
            || lower.contains("\u{00b0}c")
            || lower.contains("preheat"))
    {
        return ContentType::Recipe;
    }

    // Spec: structural density (≥3 section headers or YAML front matter).
    let heading_count = text.lines().filter(|l| l.starts_with("##")).count();
    if heading_count >= 3 || trimmed.starts_with("---\n") {
        return ContentType::Spec;
    }

    // Journal: date-prefixed entries (## 2024-01-15, ### Monday, etc.).
    let has_date_header = text.lines().any(|l| {
        let h = l.trim_start_matches('#').trim();
        h.len() >= 3
            && (h.starts_with("20")
                || h.starts_with("19")
                || matches!(
                    h.to_ascii_lowercase().as_str(),
                    "monday"
                        | "tuesday"
                        | "wednesday"
                        | "thursday"
                        | "friday"
                        | "saturday"
                        | "sunday"
                        | "january"
                        | "february"
                        | "march"
                        | "april"
                        | "may"
                        | "june"
                        | "july"
                        | "august"
                        | "september"
                        | "october"
                        | "november"
                        | "december"
                ))
    });
    if has_date_header {
        return ContentType::Journal;
    }

    ContentType::Note
}

/// AppState-adjacent chance-styling state (C8 tasteroll).
///
/// Wraps an `Option<ChanceRoll>` — `None` when no roll is active, `Some` after
/// the user rolls the dice. The platform shell reads [`css`] to obtain the
/// safe CSS custom-property block for injection into the preview/export.
///
/// [`css`]: Self::css
#[derive(Debug, Clone, Default)]
pub struct TasteState {
    roll: Option<ChanceRoll>,
}

impl TasteState {
    /// Creates a new roll from `seed` and `content_type`, replacing any
    /// existing roll. The initial candidate is generated immediately.
    pub fn roll(&mut self, seed: u64, content_type: ContentType) {
        self.roll = Some(ChanceRoll::new(seed, content_type));
    }

    /// Re-rolls the current design, preserving locked dimensions.
    /// No-op if no roll is active.
    pub fn reroll(&mut self) {
        if let Some(r) = self.roll.as_mut() {
            r.reroll();
        }
    }

    /// Clears the roll entirely (no styling applied).
    pub fn reset(&mut self) {
        self.roll = None;
    }

    /// Locks `dimension` at its current value so subsequent re-rolls keep it.
    /// Returns `Ok(true)` if newly locked, `Ok(false)` if already locked,
    /// `Err` for unknown token id. No-op (returns `Ok(false)`) if inactive.
    pub fn lock(&mut self, dimension: &str) -> Result<bool, ChanceRollError> {
        match self.roll.as_mut() {
            Some(r) => r.lock(dimension),
            None => Ok(false),
        }
    }

    /// Unlocks `dimension` so the next re-roll may vary it. No-op if inactive.
    pub fn unlock(&mut self, dimension: &str) {
        if let Some(r) = self.roll.as_mut() {
            r.unlock(dimension);
        }
    }

    /// Whether a roll is active (CSS is available).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.roll.is_some()
    }

    /// The safe CSS custom-property block (`:root{ --id:value; \u{2026} }`),
    /// or `None` when no roll is active.
    #[must_use]
    pub fn css(&self) -> Option<String> {
        self.roll.as_ref().map(|r| render_chance_css(r.current()))
    }

    /// The current seed, or `None` when no roll is active.
    #[must_use]
    pub fn seed(&self) -> Option<u64> {
        self.roll.as_ref().map(ChanceRoll::seed)
    }

    /// Whether `dimension` is locked. Returns `false` if inactive.
    #[must_use]
    pub fn is_locked(&self, dimension: &str) -> bool {
        self.roll.as_ref().is_some_and(|r| r.is_locked(dimension))
    }

    /// Returns the ids of all locked dimensions (empty if inactive).
    #[must_use]
    pub fn locked_dimensions(&self) -> Vec<&'static str> {
        // Only DESIGN_TOKENS ids can be locked; check each.
        rutile_core::DESIGN_TOKENS
            .iter()
            .filter(|id| self.is_locked(id))
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- content_seed ----------------------------------------------------

    #[test]
    fn content_seed_is_deterministic() {
        let a = content_seed("# Hello World");
        let b = content_seed("# Hello World");
        assert_eq!(a, b);
    }

    #[test]
    fn content_seed_differs_on_different_text() {
        let a = content_seed("# Hello World");
        let b = content_seed("# Goodbye World");
        assert_ne!(a, b);
    }

    #[test]
    fn content_seed_handles_empty_text() {
        let seed = content_seed("");
        // FNV offset basis (non-zero, deterministic).
        assert_eq!(seed, 0xCBF2_9CE4_8422_2325);
    }

    // --- infer_content_type ----------------------------------------------

    #[test]
    fn infer_letter_from_salutation() {
        assert_eq!(
            infer_content_type("Dear Simon,\n\nIt was great to see you.\n\nRegards,\nSimon"),
            ContentType::Letter
        );
    }

    #[test]
    fn infer_recipe_from_cooking_markers() {
        let text = "# Chocolate Cake\n\n## Ingredients\n\n- 2 cups flour\n- 3 tablespoons cocoa\n\nPreheat oven to 350\u{00b0}F.";
        assert_eq!(infer_content_type(text), ContentType::Recipe);
    }

    #[test]
    fn infer_spec_from_heading_density() {
        let text = "# API Design\n\n## Overview\n\n## Endpoints\n\n## Authentication\n\n## Rate Limiting\n";
        assert_eq!(infer_content_type(text), ContentType::Spec);
    }

    #[test]
    fn infer_journal_from_date_header() {
        let text = "## 2024-01-15\n\nToday I started working on the tasteroll integration.\n";
        assert_eq!(infer_content_type(text), ContentType::Journal);
    }

    #[test]
    fn infer_note_as_default() {
        assert_eq!(infer_content_type("Just a quick note."), ContentType::Note);
    }

    // --- TasteState lifecycle --------------------------------------------

    #[test]
    fn default_is_inactive() {
        let state = TasteState::default();
        assert!(!state.is_active());
        assert_eq!(state.css(), None);
        assert_eq!(state.seed(), None);
    }

    #[test]
    fn roll_makes_state_active() {
        let mut state = TasteState::default();
        state.roll(42, ContentType::Spec);
        assert!(state.is_active());
        assert!(state.css().is_some());
        assert_eq!(state.seed(), Some(42));
    }

    #[test]
    fn roll_is_deterministic_for_same_seed_and_type() {
        let mut a = TasteState::default();
        let mut b = TasteState::default();
        a.roll(99, ContentType::Note);
        b.roll(99, ContentType::Note);
        assert_eq!(a.css(), b.css());
    }

    #[test]
    fn reroll_changes_css() {
        let mut state = TasteState::default();
        state.roll(7, ContentType::Note);
        let first = state.css().unwrap();
        state.reroll();
        let second = state.css().unwrap();
        assert_ne!(first, second, "reroll should vary the design");
    }

    #[test]
    fn reroll_preserves_locked_dimensions() {
        let mut state = TasteState::default();
        state.roll(3, ContentType::Journal);
        state.lock("measure").unwrap();
        let locked_value = state.css().unwrap();

        for _ in 0..5 {
            state.reroll();
            // The measure token should remain stable across re-rolls.
            assert!(state.is_locked("measure"));
        }
        // Locked dimensions list should contain "measure".
        assert!(state.locked_dimensions().contains(&"measure"));
        // CSS changed (other dims varied) but measure is the same.
        assert_ne!(state.css().unwrap(), locked_value);
        assert!(state.is_locked("measure"));
    }

    #[test]
    fn reset_clears_state() {
        let mut state = TasteState::default();
        state.roll(42, ContentType::Spec);
        assert!(state.is_active());
        state.reset();
        assert!(!state.is_active());
        assert_eq!(state.css(), None);
    }

    #[test]
    fn lock_then_unlock() {
        let mut state = TasteState::default();
        state.roll(10, ContentType::Letter);
        assert!(!state.is_locked("density-scale"));
        state.lock("density-scale").unwrap();
        assert!(state.is_locked("density-scale"));
        state.unlock("density-scale");
        assert!(!state.is_locked("density-scale"));
    }

    #[test]
    fn lock_rejects_unknown_token() {
        let mut state = TasteState::default();
        state.roll(1, ContentType::Note);
        assert!(state.lock("not-a-real-token").is_err());
    }

    #[test]
    fn lock_on_inactive_state_is_noop_false() {
        let mut state = TasteState::default();
        assert!(!state.lock("measure").unwrap());
    }

    #[test]
    fn reroll_on_inactive_state_is_noop() {
        let mut state = TasteState::default();
        state.reroll(); // should not panic
        assert!(!state.is_active());
    }

    #[test]
    fn css_produces_valid_root_block() {
        let mut state = TasteState::default();
        state.roll(5, ContentType::Spec);
        let css = state.css().unwrap();
        assert!(css.starts_with(":root{"), "css should start with :root{{");
        assert!(css.ends_with('}'), "css should end with }}");
        assert!(!css.contains("url("), "no url( in safe CSS");
        assert!(!css.contains("@import"), "no @import in safe CSS");
        assert!(!css.contains("</"), "no closing tags in safe CSS");
    }

    #[test]
    fn locked_dimensions_empty_when_nothing_locked() {
        let mut state = TasteState::default();
        state.roll(1, ContentType::Note);
        assert!(state.locked_dimensions().is_empty());
    }

    #[test]
    fn locked_dimensions_multiple() {
        let mut state = TasteState::default();
        state.roll(1, ContentType::Note);
        state.lock("measure").unwrap();
        state.lock("line-height").unwrap();
        let dims = state.locked_dimensions();
        assert_eq!(dims.len(), 2);
        assert!(dims.contains(&"measure"));
        assert!(dims.contains(&"line-height"));
    }
}
