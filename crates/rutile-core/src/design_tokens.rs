//! Frozen public export-design token API (node T).
//!
//! C7 (AI-steered export) and C8 (tasteroll) emit ONLY validated
//! [`DesignTokenValue`]s through this module. The value grammar rejects
//! CSS-injection substrings *by construction*, so no rolled or steered value can
//! smuggle script, imports, or external references into the self-contained HTML
//! export. `render.rs` / `security.rs` / `safe_link.rs` are untouched (the
//! security-core fence); this module is an additive, validated write surface that
//! downstream code reaches the export template through.
//!
//! API stability: [`DESIGN_TOKENS`] is the frozen surface. Adding a token id is a
//! minor (backwards-compatible) bump; removing or renaming one is breaking. The
//! value grammar is immutable: it may grow stricter, never looser.

use std::collections::HashMap;

/// The frozen set of export-design token ids (CSS custom-property names already
/// emitted by the 0.2 export template). C7/C8 may only set values for these ids.
pub const DESIGN_TOKENS: &[&str] = &[
    "measure",
    "accent",
    "bg",
    "ink",
    "muted",
    "surface",
    "border",
    "radius",
    "edge",
    "step--1",
    "step-0",
    "step-1",
    "step-2",
    "step-3",
    "step-4",
    "font-body",
    "font-mono",
    "line-height",
    "heading-weight",
    "density-scale",
    "dur-fast",
    "ease-out",
    "marker",
    "rule-needle",
];

/// Substrings that MUST NOT appear in any token value — any one of them would let
/// a rolled/steered value break self-containment or execute script/style import in
/// the exported note. Matched case-insensitively against the whole value.
const FORBIDDEN_SUBSTRINGS: &[&str] = &[
    "url(",
    "@import",
    "expression(",
    "javascript:",
    "data:",
    "vbscript:",
    "behavior:",
    "-moz-binding",
    "</",
    "<script",
    "/>",
];

/// Maximum byte length of a single token value. CSS custom-property values are
/// short (colors, lengths, numbers, font stacks); this bounds any abuse and keeps
/// the exported `<style>` block predictable.
pub const MAX_TOKEN_VALUE_BYTES: usize = 128;

/// A token value that has passed the value grammar. Construct via
/// [`DesignTokenValue::new`]; the inner string is reachable only after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignTokenValue {
    value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenValueError {
    #[error("token value is empty")]
    Empty,
    #[error("token value exceeds the {0}-byte cap")]
    TooLong(usize),
    #[error("token value contains a control character")]
    ControlChar,
    #[error("token value contains a forbidden substring: {0}")]
    Forbidden(&'static str),
}

impl DesignTokenValue {
    /// Validate `value` against the value grammar.
    ///
    /// Rules: non-empty; ≤ [`MAX_TOKEN_VALUE_BYTES`] bytes; no control
    /// characters; no [`FORBIDDEN_SUBSTRINGS`] (case-insensitive). A value that
    /// would break self-containment or execute script never becomes a
    /// `DesignTokenValue`.
    pub fn new(value: &str) -> Result<Self, TokenValueError> {
        if value.is_empty() {
            return Err(TokenValueError::Empty);
        }
        if value.len() > MAX_TOKEN_VALUE_BYTES {
            return Err(TokenValueError::TooLong(MAX_TOKEN_VALUE_BYTES));
        }
        if value.chars().any(char::is_control) {
            return Err(TokenValueError::ControlChar);
        }
        let lower = value.to_ascii_lowercase();
        for &ban in FORBIDDEN_SUBSTRINGS {
            if lower.contains(ban) {
                return Err(TokenValueError::Forbidden(ban));
            }
        }
        Ok(Self {
            value: value.to_owned(),
        })
    }

    /// The validated value, safe to write into the export `<style>` block.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DesignTokenSetError {
    #[error("unknown design token id: {0}")]
    UnknownToken(String),
    #[error("token {id} rejected by the value grammar: {source}")]
    InvalidValue {
        id: &'static str,
        #[source]
        source: TokenValueError,
    },
}

/// A validated set of export-design token values. C7/C8 build one of these and
/// the export template consumes only ids in [`DESIGN_TOKENS`] with grammar-passing
/// values; an unknown id or an injection-carrying value is rejected at insertion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesignTokenSet {
    values: HashMap<&'static str, DesignTokenValue>,
}

impl DesignTokenSet {
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Validate and insert a value for a known token id. Unknown ids are rejected
    /// so downstream code cannot invent new CSS custom properties.
    pub fn set(&mut self, id: &str, value: &str) -> Result<(), DesignTokenSetError> {
        let static_id = DESIGN_TOKENS
            .iter()
            .copied()
            .find(|token| *token == id)
            .ok_or_else(|| DesignTokenSetError::UnknownToken(id.to_owned()))?;
        let validated =
            DesignTokenValue::new(value).map_err(|source| DesignTokenSetError::InvalidValue {
                id: static_id,
                source,
            })?;
        self.values.insert(static_id, validated);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&DesignTokenValue> {
        DESIGN_TOKENS
            .iter()
            .copied()
            .find(|token| *token == id)
            .and_then(|token| self.values.get(token))
    }

    /// Iterate `(id, validated value)` pairs. Deterministic order is NOT guaranteed
    /// (callers that need a stable export string sort by id).
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &DesignTokenValue)> {
        self.values.iter().map(|(id, value)| (*id, value))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Number of frozen design tokens (convenience for tests/docs).
    #[must_use]
    pub fn token_surface_len() -> usize {
        DESIGN_TOKENS.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_css_values() {
        let v = DesignTokenValue::new("1.5").unwrap();
        assert_eq!(v.as_str(), "1.5");
        assert!(DesignTokenValue::new("#b8860b").is_ok()); // rutile-gold accent
        assert!(DesignTokenValue::new("'Iowan Old Style', Georgia, serif").is_ok());
        assert!(DesignTokenValue::new("66ch").is_ok());
    }

    #[test]
    fn rejects_empty_oversized_and_control_chars() {
        assert!(matches!(
            DesignTokenValue::new(""),
            Err(TokenValueError::Empty)
        ));
        let huge = "x".repeat(MAX_TOKEN_VALUE_BYTES + 1);
        assert!(matches!(
            DesignTokenValue::new(&huge),
            Err(TokenValueError::TooLong(_))
        ));
        assert!(matches!(
            DesignTokenValue::new("1.5\x07"),
            Err(TokenValueError::ControlChar)
        ));
    }

    #[test]
    fn rejects_every_forbidden_substring_case_insensitively() {
        for payload in [
            "url(foo)",
            "URL(foo)",
            "@import 'x'",
            "expression(alert(1))",
            "javascript:alert(1)",
            "data:text/html,x",
            "</style>",
            "<script>",
            "/>",
            "behavior:url(#x)",
            "-moz-binding: url(x)",
        ] {
            let err = DesignTokenValue::new(payload).unwrap_err();
            assert!(
                matches!(err, TokenValueError::Forbidden(_)),
                "payload {payload:?} should be forbidden, got {err:?}"
            );
        }
    }

    #[test]
    fn set_rejects_unknown_token_ids() {
        let mut set = DesignTokenSet::new();
        assert!(set.set("accent", "#b8860b").is_ok());
        assert!(matches!(
            set.set("evil-custom-prop", "1"),
            Err(DesignTokenSetError::UnknownToken(_))
        ));
    }

    #[test]
    fn set_rejects_injection_values_for_known_tokens() {
        let mut set = DesignTokenSet::new();
        assert!(matches!(
            set.set("accent", "url(x)"),
            Err(DesignTokenSetError::InvalidValue { .. })
        ));
        assert_eq!(set.len(), 0); // rejected value was not stored
    }

    #[test]
    fn token_surface_is_frozen_and_nonempty() {
        assert!(DesignTokenSet::token_surface_len() >= 16);
        // ids are unique
        let mut sorted = DESIGN_TOKENS.to_vec();
        sorted.sort_unstable();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(sorted, deduped);
    }
}
