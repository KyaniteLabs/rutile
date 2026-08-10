//! Typed find/replace operation contracts (SPEC §9).
//!
//! 0.2 supports plain-substring and whole-word matching only. Regex matching
//! is deliberately *not representable*: [`MatchMode`] has no regex variant,
//! and its enumeration is pinned by contract test. The (Wave 1) engine
//! consumes these types and expresses every replacement as span-scoped edits
//! through [`EditPlan`](crate::EditPlan), so replace-all inherits the same
//! bounded-edit guarantees as the format layer.

use std::ops::Range;

use thiserror::Error;

/// Maximum bytes for a search pattern.
pub const MAX_FIND_PATTERN_BYTES: usize = 1024;
/// Maximum bytes for a replacement string. Kept within the per-edit
/// replacement cap so any replacement fits in a planned edit.
pub const MAX_FIND_REPLACEMENT_BYTES: usize = 2 * 1024;

/// How a pattern matches. Frozen for 0.2: no regex variant exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchMode {
    /// Literal substring match.
    Plain,
    /// Literal match bounded by non-word characters on both sides.
    WholeWord,
}

impl MatchMode {
    /// Stable names, in declaration order.
    pub const VARIANT_NAMES: [&'static str; 2] = ["plain", "whole-word"];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::WholeWord => "whole-word",
        }
    }
}

/// Search direction from the caller's position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindDirection {
    Forward,
    Backward,
}

impl FindDirection {
    /// Stable names, in declaration order.
    pub const VARIANT_NAMES: [&'static str; 2] = ["forward", "backward"];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }
}

/// Errors from constructing find/replace values.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum FindError {
    #[error("search pattern is empty")]
    EmptyPattern,
    #[error("search pattern is {len} bytes; the maximum is {max}")]
    PatternTooLarge { len: usize, max: usize },
    #[error("replacement is {len} bytes; the maximum is {max}")]
    ReplacementTooLarge { len: usize, max: usize },
}

/// A validated search query. Fields are private so an unbounded or empty
/// pattern is unrepresentable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindQuery {
    pattern: String,
    mode: MatchMode,
    case_sensitive: bool,
}

impl FindQuery {
    pub fn new(pattern: String, mode: MatchMode, case_sensitive: bool) -> Result<Self, FindError> {
        if pattern.is_empty() {
            return Err(FindError::EmptyPattern);
        }
        if pattern.len() > MAX_FIND_PATTERN_BYTES {
            return Err(FindError::PatternTooLarge {
                len: pattern.len(),
                max: MAX_FIND_PATTERN_BYTES,
            });
        }
        Ok(Self {
            pattern,
            mode,
            case_sensitive,
        })
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn mode(&self) -> MatchMode {
        self.mode
    }

    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }
}

/// A validated query-plus-replacement pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceSpec {
    query: FindQuery,
    replacement: String,
}

impl ReplaceSpec {
    pub fn new(query: FindQuery, replacement: String) -> Result<Self, FindError> {
        if replacement.len() > MAX_FIND_REPLACEMENT_BYTES {
            return Err(FindError::ReplacementTooLarge {
                len: replacement.len(),
                max: MAX_FIND_REPLACEMENT_BYTES,
            });
        }
        Ok(Self { query, replacement })
    }

    pub fn query(&self) -> &FindQuery {
        &self.query
    }

    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

/// The find/replace operations a shell may request (frozen for 0.2).
///
/// The engine answers `Find` with an optional match range and the replace
/// operations with an [`EditPlan`](crate::EditPlan), keeping every mutation
/// inside the bounded edit contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FindReplaceOp {
    /// Locate the next match from `from_byte`, optionally wrapping.
    Find {
        query: FindQuery,
        from_byte: usize,
        direction: FindDirection,
        wrap: bool,
    },
    /// Replace the match the UI currently highlights.
    ReplaceCurrent {
        spec: ReplaceSpec,
        current: Range<usize>,
    },
    /// Replace every match in the document. Plans exceeding the edit-plan
    /// bounds must be rejected by the engine with a typed error, never
    /// widened into a buffer rewrite.
    ReplaceAll { spec: ReplaceSpec },
}

impl FindReplaceOp {
    /// Stable names, in declaration order.
    pub const VARIANT_NAMES: [&'static str; 3] = ["find", "replace-current", "replace-all"];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Find { .. } => "find",
            Self::ReplaceCurrent { .. } => "replace-current",
            Self::ReplaceAll { .. } => "replace-all",
        }
    }
}
