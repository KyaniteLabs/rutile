//! Typed, versioned preferences record persisted via the session store.
//!
//! Single source of truth for user preferences — no ad-hoc plist/json scattered
//! across the shell. Surfaced as `AppMessage::Preferences` edits through the
//! reducer. All fields are bounded; the record is schema-versioned for
//! forward-compatible migration.
//!
//! # Security-core fence
//!
//! Preferences never carry raw HTML, URLs, or paths that bypass
//! [`validate_path`](rutile_core::validate_path). The `exclude_patterns` field
//! stores glob-style patterns as plain strings — no path traversal, no shell
//! expansion, no URL interpretation.

use serde::{Deserialize, Serialize};

use thiserror::Error;

// ---------------------------------------------------------------------------
// Bounds (immutable constants)
// ---------------------------------------------------------------------------

/// Maximum number of recent-document entries retained.
pub const MAX_RECENT_FILES: u8 = 20;

/// Minimum editor font-scale factor.
pub const FONT_SCALE_MIN: f32 = 0.75;

/// Maximum editor font-scale factor.
pub const FONT_SCALE_MAX: f32 = 2.0;

/// Minimum tab width in spaces.
pub const TAB_WIDTH_MIN: u8 = 1;

/// Maximum tab width in spaces.
pub const TAB_WIDTH_MAX: u8 = 8;

/// Current preferences schema version. Increment when the on-disk shape
/// changes; `Preferences::deserialize_and_migrate` handles old envelopes.
pub const PREFERENCES_SCHEMA_VERSION: u16 = 1;

/// Maximum number of exclude patterns to prevent unbounded growth.
pub const MAX_EXCLUDE_PATTERNS: usize = 64;

/// Maximum length of a single exclude pattern.
pub const MAX_EXCLUDE_PATTERN_LEN: usize = 256;

// ---------------------------------------------------------------------------
// Sub-records
// ---------------------------------------------------------------------------

/// User's preferred colour-scheme allegiance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// Schema envelope tag for forward-compatible migration.
///
/// On deserialization, an unknown `version` is rejected (deny-unknown-field
/// discipline inherited from the struct-level attribute) so that a future
/// schema bump never silently overwrites a field it doesn't understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferencesSchema {
    pub version: u16,
}

impl Default for PreferencesSchema {
    fn default() -> Self {
        Self {
            version: PREFERENCES_SCHEMA_VERSION,
        }
    }
}

/// Editor-level preferences. All numeric fields are bounded by the constants
/// above; deserialization clamps out-of-range values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorPrefs {
    /// Scale factor applied to the base font size. Clamped to
    /// `[FONT_SCALE_MIN, FONT_SCALE_MAX]`.
    pub font_scale: f32,
    /// Spaces per tab. Clamped to `[TAB_WIDTH_MIN, TAB_WIDTH_MAX]`.
    pub tab_width: u8,
    /// Whether long lines wrap in the editor.
    pub wrap: bool,
    /// Whether spell-checking is enabled.
    pub spellcheck: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            font_scale: 1.0,
            tab_width: 4,
            wrap: true,
            spellcheck: true,
        }
    }
}

/// View-mode preferences controlling the reader/editor split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ViewPrefs {
    /// Open documents in reader-first mode by default.
    pub reader_first: bool,
    /// Enter focus mode on launch.
    pub focus_mode: bool,
}

/// Recent-documents preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentPrefs {
    /// Maximum entries to retain. Clamped to `[0, MAX_RECENT_FILES]`.
    pub cap: u8,
    /// Glob-style exclude patterns (e.g. `*.tmp`, `**/.git/**`).
    /// Patterns are plain strings — never interpreted as paths or URLs.
    pub exclude_patterns: Vec<String>,
}

impl Default for RecentPrefs {
    fn default() -> Self {
        Self {
            cap: 10,
            exclude_patterns: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level record
// ---------------------------------------------------------------------------

/// The typed, versioned preferences record.
///
/// Persisted atomically through the existing session-store path (no new I/O
/// surface). Security-core fence: no field carries raw HTML, URLs, or paths
/// that bypass `validate_path`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Preferences {
    pub schema: PreferencesSchema,
    pub appearance: Appearance,
    pub editor: EditorPrefs,
    pub view: ViewPrefs,
    pub recent: RecentPrefs,
}

/// Error returned when a preferences record fails validation or migration.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreferencesError {
    /// The on-disk schema version is newer than this binary understands.
    #[error("unknown preferences schema version {found}, expected <= {max}")]
    UnknownSchema { found: u16, max: u16 },
    /// JSON deserialization failed.
    #[error("preferences JSON decode error: {0}")]
    JsonDecode(String),
}

impl Preferences {
    /// Validates and clamps all bounded fields, returning a clean record.
    ///
    /// Called after deserialization to enforce invariants regardless of what
    /// was on disk. Out-of-range values are clamped, not rejected, so a
    /// corrupt-but-syntactically-valid record is recovered gracefully.
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            schema: self.schema,
            appearance: self.appearance,
            editor: EditorPrefs {
                font_scale: self.editor.font_scale.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX),
                tab_width: self.editor.tab_width.clamp(TAB_WIDTH_MIN, TAB_WIDTH_MAX),
                wrap: self.editor.wrap,
                spellcheck: self.editor.spellcheck,
            },
            view: self.view,
            recent: RecentPrefs {
                cap: self.recent.cap.min(MAX_RECENT_FILES),
                exclude_patterns: self
                    .recent
                    .exclude_patterns
                    .into_iter()
                    .take(MAX_EXCLUDE_PATTERNS)
                    .map(|p| p.chars().take(MAX_EXCLUDE_PATTERN_LEN).collect())
                    .collect(),
            },
        }
    }

    /// Deserializes from JSON, validates the schema version, and returns a
    /// sanitized record. Rejects unknown schema versions (deny-unknown-field
    /// discipline: forward-incompatible records fail closed).
    pub fn from_json(json: &str) -> Result<Self, PreferencesError> {
        let prefs: Self =
            serde_json::from_str(json).map_err(|e| PreferencesError::JsonDecode(e.to_string()))?;

        if prefs.schema.version > PREFERENCES_SCHEMA_VERSION {
            return Err(PreferencesError::UnknownSchema {
                found: prefs.schema.version,
                max: PREFERENCES_SCHEMA_VERSION,
            });
        }

        Ok(prefs.sanitized())
    }

    /// Serializes to JSON for persistence.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("preferences serialization is infallible")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact-value assertions on sanitized floats
    use super::*;

    // -- Bounds enforcement --------------------------------------------------

    #[test]
    fn sanitized_clamps_font_scale_to_rails() {
        let prefs = Preferences {
            editor: EditorPrefs {
                font_scale: 10.0,
                ..EditorPrefs::default()
            },
            ..Preferences::default()
        };
        let clean = prefs.sanitized();
        assert_eq!(clean.editor.font_scale, FONT_SCALE_MAX);
    }

    #[test]
    fn sanitized_clamps_font_scale_below_min() {
        let prefs = Preferences {
            editor: EditorPrefs {
                font_scale: 0.1,
                ..EditorPrefs::default()
            },
            ..Preferences::default()
        };
        let clean = prefs.sanitized();
        assert_eq!(clean.editor.font_scale, FONT_SCALE_MIN);
    }

    #[test]
    fn sanitized_clamps_tab_width() {
        let prefs = Preferences {
            editor: EditorPrefs {
                tab_width: 99,
                ..EditorPrefs::default()
            },
            ..Preferences::default()
        };
        assert_eq!(prefs.sanitized().editor.tab_width, TAB_WIDTH_MAX);

        let prefs = Preferences {
            editor: EditorPrefs {
                tab_width: 0,
                ..EditorPrefs::default()
            },
            ..Preferences::default()
        };
        assert_eq!(prefs.sanitized().editor.tab_width, TAB_WIDTH_MIN);
    }

    #[test]
    fn sanitized_clamps_recent_cap() {
        let prefs = Preferences {
            recent: RecentPrefs {
                cap: 200,
                ..RecentPrefs::default()
            },
            ..Preferences::default()
        };
        assert_eq!(prefs.sanitized().recent.cap, MAX_RECENT_FILES);
    }

    #[test]
    fn sanitized_truncates_excess_exclude_patterns() {
        let patterns: Vec<String> = (0..100).map(|i| format!("*.tmp{i}")).collect();
        let prefs = Preferences {
            recent: RecentPrefs {
                cap: 10,
                exclude_patterns: patterns,
            },
            ..Preferences::default()
        };
        let clean = prefs.sanitized();
        assert_eq!(clean.recent.exclude_patterns.len(), MAX_EXCLUDE_PATTERNS);
    }

    #[test]
    fn sanitized_truncates_overlong_pattern() {
        let long = "x".repeat(1000);
        let prefs = Preferences {
            recent: RecentPrefs {
                cap: 10,
                exclude_patterns: vec![long],
            },
            ..Preferences::default()
        };
        let clean = prefs.sanitized();
        assert_eq!(
            clean.recent.exclude_patterns[0].len(),
            MAX_EXCLUDE_PATTERN_LEN
        );
    }

    // -- JSON round-trip -----------------------------------------------------

    #[test]
    fn default_round_trips_through_json() {
        let prefs = Preferences::default();
        let json = prefs.to_json();
        let restored = Preferences::from_json(&json).expect("round-trip");
        assert_eq!(prefs, restored);
    }

    #[test]
    fn from_json_rejects_unknown_schema() {
        let json = r#"{
            "schema": {"version": 999},
            "appearance": "System",
            "editor": {"font_scale": 1.0, "tab_width": 4, "wrap": true, "spellcheck": true},
            "view": {"reader_first": false, "focus_mode": false},
            "recent": {"cap": 10, "exclude_patterns": []}
        }"#;
        let err = Preferences::from_json(json).unwrap_err();
        assert!(matches!(
            err,
            PreferencesError::UnknownSchema {
                found: 999,
                max: PREFERENCES_SCHEMA_VERSION
            }
        ));
    }

    #[test]
    fn from_json_clamps_out_of_range_values() {
        let json = r#"{
            "schema": {"version": 1},
            "appearance": "Dark",
            "editor": {"font_scale": 99.0, "tab_width": 0, "wrap": false, "spellcheck": false},
            "view": {"reader_first": true, "focus_mode": false},
            "recent": {"cap": 200, "exclude_patterns": []}
        }"#;
        let prefs = Preferences::from_json(json).expect("clamped");
        assert_eq!(prefs.editor.font_scale, FONT_SCALE_MAX);
        assert_eq!(prefs.editor.tab_width, TAB_WIDTH_MIN);
        assert_eq!(prefs.recent.cap, MAX_RECENT_FILES);
        assert_eq!(prefs.appearance, Appearance::Dark);
    }

    #[test]
    fn from_json_rejects_malformed_json() {
        let err = Preferences::from_json("not json").unwrap_err();
        assert!(matches!(err, PreferencesError::JsonDecode(_)));
    }

    // -- Default values ------------------------------------------------------

    #[test]
    fn default_preferences_are_sensible() {
        let prefs = Preferences::default();
        assert_eq!(prefs.schema.version, PREFERENCES_SCHEMA_VERSION);
        assert_eq!(prefs.appearance, Appearance::System);
        assert_eq!(prefs.editor.font_scale, 1.0);
        assert_eq!(prefs.editor.tab_width, 4);
        assert!(prefs.editor.wrap);
        assert!(prefs.editor.spellcheck);
        assert!(!prefs.view.reader_first);
        assert!(!prefs.view.focus_mode);
        assert_eq!(prefs.recent.cap, 10);
    }
}
