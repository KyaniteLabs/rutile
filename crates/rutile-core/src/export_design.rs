//! C7 AI-steered export — deterministic candidate generation + steering clamp
//! (node C7 core).
//!
//! The default candidate path is DETERMINISTIC: given a content type and a seed
//! it produces a [`DesignTokenSet`] whose values stay inside the design rails
//! (DESIGN-SYSTEM.md "C7 AI-steering surface"). The accent hue, fire budget,
//! contrast floors, and corner radius are IMMUTABLE — chance/steering may only
//! vary measure, density, line-height, heading-weight, and the body stack.
//!
//! All output flows through [`crate::design_tokens::DesignTokenSet`], so every
//! value is grammar-validated (no script/imports/external refs can reach the
//! export). The AFM (Apple Foundation Models) natural-language→steering mapping
//! is a feature-flagged stretch behind this deterministic core; it is NOT in the
//! default path. `render.rs` / `security.rs` / `safe_link.rs` are untouched.
//!
//! The PRNG is an inline SplitMix64 so this node adds no new dependency; C8
//! (tasteroll) will re-use it for seeded rolls or swap to the `chance` crate when
//! that dependency is added.

use crate::design_tokens::DesignTokenSet;

/// The immutable rutile-gold accent (DESIGN-SYSTEM.md: anchor `#C9921E`). Chance
/// and steering NEVER vary it — the fire budget allows exactly one accent.
pub const ACCENT_RUTILE_GOLD: &str = "#C9921E";

/// Continuous design rails (DESIGN-SYSTEM.md "C7 AI-steering surface").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignRail {
    /// `--measure` in CSS character units.
    pub measure_ch: (u8, u8),
    /// `--density-scale` multiplier.
    pub density: (f64, f64),
    /// `--line-height`.
    pub line_height: (f64, f64),
    /// `--heading-weight`.
    pub heading_weight: (u16, u16),
}

impl DesignRail {
    /// The locked Rutile design rails. Adding a rail or tightening a bound is a
    /// minor-semver change; loosening past these floors is forbidden by the
    /// design system (fire budget / WCAG AA).
    pub const DEFAULT: DesignRail = DesignRail {
        measure_ch: (58, 75),
        density: (0.85, 1.15),
        line_height: (1.5, 1.8),
        heading_weight: (600, 800),
    };

    fn clamp_measure(self, v: f64) -> u8 {
        let (lo, hi) = (f64::from(self.measure_ch.0), f64::from(self.measure_ch.1));
        v.round().clamp(lo, hi) as u8
    }
    fn clamp_density(self, v: f64) -> f64 {
        v.clamp(self.density.0, self.density.1)
    }
    fn clamp_line_height(self, v: f64) -> f64 {
        v.clamp(self.line_height.0, self.line_height.1)
    }
    fn clamp_heading_weight(self, v: f64) -> u16 {
        v.round().clamp(
            f64::from(self.heading_weight.0),
            f64::from(self.heading_weight.1),
        ) as u16
    }
}

/// Coarse note content type. Biases the candidate's starting point; every value
/// still stays inside [`DesignRail::DEFAULT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Journal,
    Spec,
    Letter,
    Recipe,
    Note,
}

impl ContentType {
    /// Default body stack for this content type (letters/journals lean serif).
    const fn default_body_stack(self) -> &'static str {
        match self {
            ContentType::Letter | ContentType::Journal => "'Iowan Old Style', Georgia, serif",
            ContentType::Spec | ContentType::Recipe | ContentType::Note => {
                "-apple-system, 'Segoe UI', sans-serif"
            }
        }
    }
}

/// A typed, rail-clamped steering adjustment. The AFM stretch maps natural
/// language ("denser", "wider measure") onto one of these; the deterministic
/// core applies + clamps it directly.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SteeringAdjustment {
    /// Add to `--measure` (clamped to the rail).
    pub measure_delta_ch: f64,
    /// Multiply `--density-scale` by this (clamped to the rail).
    pub density_factor: f64,
    /// Add to `--line-height` (clamped to the rail).
    pub line_height_delta: f64,
    /// Add to `--heading-weight` (clamped to the rail).
    pub heading_weight_delta: f64,
}

/// Inline SplitMix64 — deterministic, fixed-step PRNG. Same seed ⇒ same sequence.
struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B9_7F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF584_76D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform f64 in `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        let t = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + t * (hi - lo)
    }
}

/// Deterministically generate a candidate design for `content_type` from `seed`.
///
/// Varies only the rail-allowed tokens (measure, density, line-height,
/// heading-weight, body stack); the accent is always [`ACCENT_RUTILE_GOLD`] and
/// the fire budget / radius / contrast are never touched. Same `(content_type,
/// seed)` always yields the identical [`DesignTokenSet`].
pub fn generate_candidate(content_type: ContentType, seed: u64) -> DesignTokenSet {
    generate_with_rail(content_type, seed, DesignRail::DEFAULT)
}

fn generate_with_rail(content_type: ContentType, seed: u64, rail: DesignRail) -> DesignTokenSet {
    let mut rng = SplitMix64::new(seed);
    let mut set = DesignTokenSet::new();
    // Vary only within rails.
    set.set(
        "measure",
        &format!("{}ch", rail.clamp_measure(rng.range(58.0, 75.0))),
    )
    .expect("measure is a frozen token and the value is grammar-clean");
    set.set(
        "density-scale",
        &format!("{:.3}", rail.clamp_density(rng.range(0.85, 1.15))),
    )
    .expect("density-scale is a frozen token and the value is grammar-clean");
    set.set(
        "line-height",
        &format!("{:.2}", rail.clamp_line_height(rng.range(1.5, 1.8))),
    )
    .expect("line-height is a frozen token and the value is grammar-clean");
    set.set(
        "heading-weight",
        &format!("{}", rail.clamp_heading_weight(rng.range(600.0, 800.0))),
    )
    .expect("heading-weight is a frozen token and the value is grammar-clean");
    set.set("font-body", content_type.default_body_stack())
        .expect("font-body is a frozen token and the value is grammar-clean");
    // Accent is IMMUTABLE: always rutile gold, never varied by chance/steering.
    set.set("accent", ACCENT_RUTILE_GOLD)
        .expect("accent is a frozen token and #C9921E is grammar-clean");
    set
}

/// Apply a typed steering adjustment to a candidate, clamping every value back
/// inside the rails. The accent and any token not named in the adjustment are
/// preserved unchanged.
pub fn apply_steering(
    candidate: &DesignTokenSet,
    adjustment: SteeringAdjustment,
) -> DesignTokenSet {
    apply_steering_with_rail(candidate, adjustment, DesignRail::DEFAULT)
}

fn apply_steering_with_rail(
    candidate: &DesignTokenSet,
    adjustment: SteeringAdjustment,
    rail: DesignRail,
) -> DesignTokenSet {
    let mut out = candidate.clone();
    if adjustment.measure_delta_ch != 0.0 {
        let base = parse_number(candidate.get("measure")).unwrap_or(66.0);
        let v = rail.clamp_measure(base + adjustment.measure_delta_ch);
        out.set("measure", &format!("{v}ch"))
            .expect("measure clamp keeps the value grammar-clean");
    }
    if adjustment.density_factor != 0.0 && (adjustment.density_factor - 1.0).abs() > f64::EPSILON {
        let base = parse_number(candidate.get("density-scale")).unwrap_or(1.0);
        let v = rail.clamp_density(base * adjustment.density_factor);
        out.set("density-scale", &format!("{v:.3}"))
            .expect("density clamp keeps the value grammar-clean");
    }
    if adjustment.line_height_delta != 0.0 {
        let base = parse_number(candidate.get("line-height")).unwrap_or(1.6);
        let v = rail.clamp_line_height(base + adjustment.line_height_delta);
        out.set("line-height", &format!("{v:.2}"))
            .expect("line-height clamp keeps the value grammar-clean");
    }
    if adjustment.heading_weight_delta != 0.0 {
        let base = parse_number(candidate.get("heading-weight")).unwrap_or(700.0);
        let v = rail.clamp_heading_weight(base + adjustment.heading_weight_delta);
        out.set("heading-weight", &format!("{v}"))
            .expect("heading-weight clamp keeps the value grammar-clean");
    }
    // Accent is immutable: never adjusted here even if a caller tried to encode it.
    out.set("accent", ACCENT_RUTILE_GOLD)
        .expect("accent stays rutile gold");
    out
}

/// Parse the leading number off a token value like `"66ch"` / `"1.150"` / `"700"`.
fn parse_number(value: Option<&crate::design_tokens::DesignTokenValue>) -> Option<f64> {
    let s = value?.as_str();
    let num: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-'))
        .collect();
    num.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_for_same_seed_and_type() {
        let a = generate_candidate(ContentType::Spec, 42);
        let b = generate_candidate(ContentType::Spec, 42);
        assert_eq!(a, b, "same seed + type must yield identical candidates");
        // Different seed → (very likely) different candidate.
        let c = generate_candidate(ContentType::Spec, 43);
        assert_ne!(a, c, "different seed should vary the candidate");
    }

    #[test]
    fn every_candidate_stays_inside_the_rails() {
        for &seed in &[0, 1, 7, 42, 999, u64::MAX] {
            for content in [
                ContentType::Journal,
                ContentType::Spec,
                ContentType::Letter,
                ContentType::Recipe,
                ContentType::Note,
            ] {
                let set = generate_candidate(content, seed);
                let measure = parse_number(set.get("measure")).unwrap();
                let density = parse_number(set.get("density-scale")).unwrap();
                let lh = parse_number(set.get("line-height")).unwrap();
                let hw = parse_number(set.get("heading-weight")).unwrap();
                assert!(
                    (58.0..=75.0).contains(&measure),
                    "measure {measure} out of rail"
                );
                assert!(
                    (0.85..=1.15).contains(&density),
                    "density {density} out of rail"
                );
                assert!((1.5..=1.8).contains(&lh), "line-height {lh} out of rail");
                assert!(
                    (600.0..=800.0).contains(&hw),
                    "heading-weight {hw} out of rail"
                );
            }
        }
    }

    #[test]
    fn accent_is_immutable_rutile_gold_in_every_candidate() {
        for &seed in &[0, 1, 42, 1000] {
            let set = generate_candidate(ContentType::Note, seed);
            assert_eq!(set.get("accent").unwrap().as_str(), ACCENT_RUTILE_GOLD);
        }
    }

    #[test]
    fn steering_clamps_back_inside_the_rails() {
        let base = generate_candidate(ContentType::Spec, 1);
        // Push measure wildly past the rail; the clamp must hold.
        let pushed = apply_steering(
            &base,
            SteeringAdjustment {
                measure_delta_ch: 1000.0,
                density_factor: 5.0,
                line_height_delta: 100.0,
                heading_weight_delta: 5000.0,
            },
        );
        assert!((58.0..=75.0).contains(&parse_number(pushed.get("measure")).unwrap()));
        assert!((0.85..=1.15).contains(&parse_number(pushed.get("density-scale")).unwrap()));
        assert!((1.5..=1.8).contains(&parse_number(pushed.get("line-height")).unwrap()));
        assert!((600.0..=800.0).contains(&parse_number(pushed.get("heading-weight")).unwrap()));
        // Accent untouched by steering.
        assert_eq!(pushed.get("accent").unwrap().as_str(), ACCENT_RUTILE_GOLD);
    }

    #[test]
    fn steering_preserves_untouched_tokens() {
        let base = generate_candidate(ContentType::Letter, 7);
        let steered = apply_steering(
            &base,
            SteeringAdjustment {
                measure_delta_ch: 2.0,
                ..Default::default()
            },
        );
        // font-body preserved (not in the adjustment).
        assert_eq!(
            steered.get("font-body").unwrap().as_str(),
            base.get("font-body").unwrap().as_str()
        );
    }
}
