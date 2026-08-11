//! C8 tasteroll — chance-styled notes: seeded roll + lock/reroll (node C8 core).
//!
//! Builds on C7 ([`crate::export_design::generate_candidate`]) and T
//! ([`crate::design_tokens::DesignTokenSet`]). A [`ChanceRoll`] generates a
//! candidate design from a seed (default: the note's content hash, so the same
//! note rolls the same design), lets the user lock dimensions, and re-rolls the
//! unlocked ones. [`render_chance_css`] turns a validated token set into the
//! `:root { --token: value; }` CSS block that the export template consumes.
//!
//! Safety: every value reaches the export through [`DesignTokenSet`], so it is
//! grammar-validated (no `url(`/`@import`/`</script`/…). [`render_chance_css`]
//! emits only frozen [`DESIGN_TOKENS`] ids. The accent is IMMUTABLE (fire
//! budget). `render.rs` / `security.rs` / `safe_link.rs` are untouched; the
//! export-validator/`inspect()` gate the final self-contained HTML downstream.
//!
//! The export-template *injection point* (where `render_chance_css`'s output is
//! spliced into the HTML) is owned by the export integrator and is intentionally
//! NOT in this module — it stays additive and does not modify the frozen render
//! path.

use crate::design_tokens::{DESIGN_TOKENS, DesignTokenSet, DesignTokenValue};
use crate::export_design::{ACCENT_RUTILE_GOLD, ContentType, generate_candidate};
use std::collections::HashMap;

/// Seed advanced on each re-roll to vary unlocked dimensions (the "timestamp"
/// variety path; the initial seed is the note content hash by default).
const REROLL_SEED_DELTA: u64 = 0x9E3779B97F4A7C15;

/// A seeded, lockable chance-styling roll. Deterministic for a given
/// `(seed, content_type)` and lock set.
#[derive(Debug, Clone)]
pub struct ChanceRoll {
    seed: u64,
    content_type: ContentType,
    locked: HashMap<&'static str, DesignTokenValue>,
    last: DesignTokenSet,
}

#[derive(Debug, thiserror::Error)]
pub enum ChanceRollError {
    #[error("unknown design token id: {0}")]
    UnknownToken(String),
}

impl ChanceRoll {
    /// Start a roll from `seed` for `content_type`. The initial candidate is
    /// generated immediately and available via [`ChanceRoll::current`].
    #[must_use]
    pub fn new(seed: u64, content_type: ContentType) -> Self {
        let last = generate_candidate(content_type, seed);
        Self {
            seed,
            content_type,
            locked: HashMap::new(),
            last,
        }
    }

    /// The current (most recently rolled) candidate.
    #[must_use]
    pub fn current(&self) -> &DesignTokenSet {
        &self.last
    }

    /// Lock `id` at its current value so subsequent re-rolls keep it. Returns
    /// `Ok(true)` if newly locked, `Ok(false)` if already locked, or an error
    /// for an unknown token id. The accent cannot be unlocked-ably varied (it is
    /// always rutile gold), but locking it is a harmless no-op-ish record.
    pub fn lock(&mut self, id: &str) -> Result<bool, ChanceRollError> {
        let static_id = DESIGN_TOKENS
            .iter()
            .copied()
            .find(|token| *token == id)
            .ok_or_else(|| ChanceRollError::UnknownToken(id.to_owned()))?;
        if self.locked.contains_key(static_id) {
            return Ok(false);
        }
        let value = self
            .last
            .get(static_id)
            .cloned()
            .unwrap_or_else(|| DesignTokenValue::new(ACCENT_RUTILE_GOLD).unwrap());
        self.locked.insert(static_id, value);
        Ok(true)
    }

    /// Unlock `id` so the next re-roll may vary it again.
    pub fn unlock(&mut self, id: &str) {
        if let Some(static_id) = DESIGN_TOKENS.iter().copied().find(|t| *t == id) {
            self.locked.remove(static_id);
        }
    }

    /// Re-roll: advance the seed and regenerate, then overlay every locked
    /// dimension's frozen value. Locked dimensions never change across re-rolls.
    pub fn reroll(&mut self) -> &DesignTokenSet {
        self.seed = self.seed.wrapping_add(REROLL_SEED_DELTA);
        let mut fresh = generate_candidate(self.content_type, self.seed);
        for (id, value) in &self.locked {
            // Locked values are already grammar-validated; re-inserting can only
            // fail on an unknown id, which is impossible here (locked ids come
            // from DESIGN_TOKENS).
            let _ = fresh.set(id, value.as_str());
        }
        self.last = fresh;
        &self.last
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn is_locked(&self, id: &str) -> bool {
        DESIGN_TOKENS
            .iter()
            .copied()
            .find(|t| *t == id)
            .is_some_and(|t| self.locked.contains_key(t))
    }
}

/// Render a validated token set as a `:root { --id: value; … }` CSS block.
///
/// Only frozen [`DESIGN_TOKENS`] ids appear, in sorted order for determinism,
/// and every value is grammar-validated — so the result is safe to splice into
/// the self-contained export `<style>` (no script/import/external-ref vectors).
/// The accent is whatever the token set carries (rutile gold unless a caller
/// violated the immutability contract, which the validator still gates).
#[must_use]
pub fn render_chance_css(set: &DesignTokenSet) -> String {
    let mut entries: Vec<(&'static str, &DesignTokenValue)> = set.iter().collect();
    entries.sort_by_key(|(id, _)| *id);
    let mut css = String::from(":root{");
    for (id, value) in entries {
        css.push_str("--");
        css.push_str(id);
        css.push(':');
        css.push_str(value.as_str());
        css.push(';');
    }
    css.push('}');
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_roll_is_deterministic_for_same_seed_and_type() {
        let a = ChanceRoll::new(42, ContentType::Spec);
        let b = ChanceRoll::new(42, ContentType::Spec);
        assert_eq!(a.current(), b.current());
    }

    #[test]
    fn reroll_changes_unlocked_dimensions_but_keeps_locked_ones() {
        let mut roll = ChanceRoll::new(7, ContentType::Note);
        let first_measure = roll.current().get("measure").cloned();
        // Lock measure at its first-rolled value.
        assert!(roll.lock("measure").unwrap());
        assert!(roll.is_locked("measure"));
        let locked_measure = roll.current().get("measure").cloned();

        // Re-roll several times; the locked measure must never change.
        for _ in 0..5 {
            roll.reroll();
            assert_eq!(
                roll.current().get("measure").cloned(),
                locked_measure,
                "locked measure must persist across re-rolls"
            );
        }
        // Sanity: the locked value is the first-rolled one.
        assert_eq!(first_measure, locked_measure);
    }

    #[test]
    fn unlock_lets_a_dimension_vary_again() {
        let mut roll = ChanceRoll::new(11, ContentType::Letter);
        roll.lock("density-scale").unwrap();
        let locked = roll.current().get("density-scale").cloned();
        roll.reroll();
        assert_eq!(roll.current().get("density-scale").cloned(), locked);
        roll.unlock("density-scale");
        roll.reroll();
        // After unlock + reroll, density may differ (deterministic, but no longer
        // pinned). It must still be inside the rail.
        let d = roll
            .current()
            .get("density-scale")
            .map(|v| v.as_str().to_owned())
            .unwrap_or_default();
        let num: f64 = d.parse().unwrap_or(1.0);
        assert!((0.85..=1.15).contains(&num));
    }

    #[test]
    fn accent_is_rutile_gold_across_rerolls() {
        let mut roll = ChanceRoll::new(99, ContentType::Journal);
        for _ in 0..4 {
            assert_eq!(
                roll.current().get("accent").unwrap().as_str(),
                ACCENT_RUTILE_GOLD
            );
            roll.reroll();
        }
    }

    #[test]
    fn lock_rejects_unknown_token_ids() {
        let mut roll = ChanceRoll::new(1, ContentType::Note);
        assert!(matches!(
            roll.lock("not-a-real-token"),
            Err(ChanceRollError::UnknownToken(_))
        ));
    }

    #[test]
    fn render_chance_css_is_sorted_frozen_ids_only_and_safe() {
        let set = generate_candidate(ContentType::Spec, 5);
        let css = render_chance_css(&set);
        assert!(css.starts_with(":root{") && css.ends_with('}'));
        // Frozen-ids-only is guaranteed by DesignTokenSet (rejects unknown ids).
        // No injection vector can appear (values are grammar-validated).
        assert!(!css.contains("url("));
        assert!(!css.contains("</"));
        assert!(!css.contains("@import"));
        // Deterministic order: re-render yields identical bytes.
        assert_eq!(render_chance_css(&set), css);
    }
}
