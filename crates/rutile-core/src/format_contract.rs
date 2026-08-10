//! Typed contracts for the 0.2 auto-formatting layer (SPEC §6, LD-3).
//!
//! Wave 0 freezes the *shape* of the formatting seam: platform shells bind
//! keys/toolbar buttons to a [`FormatCommand`]; the (Wave 1) format engine
//! turns a command plus the current document and selection into an
//! [`EditPlan`] — a bounded set of span-scoped edits the existing
//! [`Document`](crate::Document)/`EditorAdapter` contracts already know how
//! to apply. No engine lives here; these are types and invariants only.
//!
//! The locked edit-complexity rule is enforced structurally: an [`EditPlan`]
//! can only be built through [`EditPlan::new`], which rejects any plan whose
//! edits could amount to a whole-buffer rewrite. Formatting always means
//! "insert or remove small markers around existing text", never "replace the
//! document with new text".

use std::ops::Range;

use rutile_types::Revision;
use thiserror::Error;

use crate::{Edit, EditTransaction, Selection, TransactionKind};

/// Maximum number of edits a single plan may carry.
///
/// Generous enough for per-line operations over a large selection (quote or
/// list toggles, ordered-list renumbering) while keeping every plan bounded.
/// Selections needing more edits must be rejected by the engine with a typed
/// error rather than widened into fewer, larger edits.
pub const MAX_PLAN_EDITS: usize = 4096;

/// Maximum bytes a single planned edit may *remove* (its replaced span).
///
/// Format commands remove markers (`**`, `` ` ``, `> `, `- [ ] `, fences),
/// never document prose; find/replace removes at most one bounded match.
pub const MAX_PLANNED_SPAN_BYTES: usize = 2 * 1024;

/// Maximum bytes a single planned edit may insert.
pub const MAX_PLANNED_REPLACEMENT_BYTES: usize = 2 * 1024;

/// Maximum bytes a whole plan may touch (replaced spans + replacements).
///
/// This is the structural "no whole-buffer rewrites" guarantee: it is far
/// below [`MAX_DOCUMENT_BYTES`](crate::MAX_DOCUMENT_BYTES), so a plan can
/// never re-emit the document, only decorate it.
pub const MAX_PLAN_TOTAL_BYTES: usize = 256 * 1024;

/// Maximum bytes for a URL carried by [`FormatCommand::InsertLink`].
pub const MAX_LINK_URL_BYTES: usize = 2048;

/// A user-visible formatting command (SPEC §6), platform-neutral.
///
/// Shells translate keystrokes, menu items, and toolbar buttons into exactly
/// these commands and route the resulting [`EditPlan`] through the existing
/// reducer. The enumeration is frozen for 0.2; see the
/// `format_command_enumeration_is_stable` contract test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatCommand {
    /// Toggle `**strong**` on the selection or word at the cursor.
    ToggleBold,
    /// Toggle `*emphasis*` on the selection or word at the cursor.
    ToggleItalic,
    /// Toggle an inline `` `code` `` span.
    ToggleCodeSpan,
    /// Toggle a fenced code block around the selected lines.
    ToggleCodeBlock,
    /// Wrap the selection as a link. `None` produces a placeholder URL slot
    /// with the cursor inside it; `Some` (e.g. a URL pasted over a selection)
    /// supplies the destination. URLs longer than [`MAX_LINK_URL_BYTES`]
    /// must be rejected by the engine.
    InsertLink { url: Option<String> },
    /// Cycle the current block through paragraph → H1 → … → H6 → paragraph.
    CycleHeading,
    /// Toggle `> ` quoting on the selected lines.
    ToggleQuote,
    /// Toggle `- ` bullets on the selected lines.
    ToggleBulletList,
    /// Toggle `1. ` numbering on the selected lines.
    ToggleOrderedList,
    /// Toggle `- [ ] ` checklist markers on the selected lines.
    ToggleChecklist,
    /// Smart Enter: continue lists/quotes/checklists, exit on an empty item,
    /// renumber ordered lists. The engine reports what it decided as a
    /// [`SmartEnterAction`].
    SmartEnter,
}

impl FormatCommand {
    /// Stable wire/telemetry names, in declaration order.
    pub const VARIANT_NAMES: [&'static str; 11] = [
        "toggle-bold",
        "toggle-italic",
        "toggle-code-span",
        "toggle-code-block",
        "insert-link",
        "cycle-heading",
        "toggle-quote",
        "toggle-bullet-list",
        "toggle-ordered-list",
        "toggle-checklist",
        "smart-enter",
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::ToggleBold => "toggle-bold",
            Self::ToggleItalic => "toggle-italic",
            Self::ToggleCodeSpan => "toggle-code-span",
            Self::ToggleCodeBlock => "toggle-code-block",
            Self::InsertLink { .. } => "insert-link",
            Self::CycleHeading => "cycle-heading",
            Self::ToggleQuote => "toggle-quote",
            Self::ToggleBulletList => "toggle-bullet-list",
            Self::ToggleOrderedList => "toggle-ordered-list",
            Self::ToggleChecklist => "toggle-checklist",
            Self::SmartEnter => "smart-enter",
        }
    }
}

/// CommonMark bullet markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListMarker {
    Dash,
    Asterisk,
    Plus,
}

impl ListMarker {
    pub fn as_char(self) -> char {
        match self {
            Self::Dash => '-',
            Self::Asterisk => '*',
            Self::Plus => '+',
        }
    }
}

/// CommonMark ordered-list delimiters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderedDelimiter {
    Dot,
    Paren,
}

impl OrderedDelimiter {
    pub fn as_char(self) -> char {
        match self {
            Self::Dot => '.',
            Self::Paren => ')',
        }
    }
}

/// What the engine decided a [`FormatCommand::SmartEnter`] means at the
/// current cursor position (SPEC §6 smart typing), as a typed operation.
///
/// The corresponding buffer changes travel in the [`EditPlan`]; this value
/// lets shells and tests reason about the semantics without re-parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartEnterAction {
    /// No continuation context: a plain newline.
    InsertNewline,
    /// Continue a bullet list with the same marker.
    ContinueBullet { marker: ListMarker },
    /// Continue an ordered list; `number` is the next item's number after
    /// renumbering.
    ContinueOrdered {
        number: u64,
        delimiter: OrderedDelimiter,
    },
    /// Continue a checklist with an unchecked item.
    ContinueChecklist { marker: ListMarker },
    /// Continue a quote at the given nesting depth.
    ContinueQuote { depth: u8 },
    /// Enter on an empty item (double-Enter): remove the empty marker and
    /// exit the block.
    ExitEmptyItem,
}

impl SmartEnterAction {
    /// Stable names, in declaration order.
    pub const VARIANT_NAMES: [&'static str; 6] = [
        "insert-newline",
        "continue-bullet",
        "continue-ordered",
        "continue-checklist",
        "continue-quote",
        "exit-empty-item",
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::InsertNewline => "insert-newline",
            Self::ContinueBullet { .. } => "continue-bullet",
            Self::ContinueOrdered { .. } => "continue-ordered",
            Self::ContinueChecklist { .. } => "continue-checklist",
            Self::ContinueQuote { .. } => "continue-quote",
            Self::ExitEmptyItem => "exit-empty-item",
        }
    }
}

/// Why an [`EditPlan`] could not be constructed.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum EditPlanError {
    #[error("plan has no edits")]
    Empty,
    #[error("plan has {count} edits; the maximum is {max}")]
    TooManyEdits { count: usize, max: usize },
    #[error("edit {index} range {start}..{end} is reversed")]
    ReversedRange {
        index: usize,
        start: usize,
        end: usize,
    },
    #[error("edit {index} starts at {start} before the prior edit ends at {prior_end}")]
    OverlappingEdits {
        index: usize,
        start: usize,
        prior_end: usize,
    },
    #[error("edit {index} replaces {len} bytes; the per-edit span cap is {max}")]
    SpanTooLarge {
        index: usize,
        len: usize,
        max: usize,
    },
    #[error("edit {index} inserts {len} bytes; the per-edit replacement cap is {max}")]
    ReplacementTooLarge {
        index: usize,
        len: usize,
        max: usize,
    },
    #[error("plan touches {bytes} bytes; the plan budget is {max}")]
    PlanTooLarge { bytes: usize, max: usize },
}

/// A validated, bounded set of span-scoped edits plus the selection the
/// caller should end up with.
///
/// Construction is only possible through [`EditPlan::new`]; the fields stay
/// private so no plan can bypass the edit-complexity invariants. Bounds
/// against the live buffer (offset validity, UTF-8 boundaries) remain the
/// document's job when the transaction is applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditPlan {
    base_revision: Revision,
    edits: Vec<Edit>,
    selection_after: Selection,
}

impl EditPlan {
    /// Validates and builds a plan.
    ///
    /// Edits must be non-empty, sorted, non-overlapping, and within the
    /// per-edit and whole-plan byte budgets.
    pub fn new(
        base_revision: Revision,
        edits: Vec<Edit>,
        selection_after: Selection,
    ) -> Result<Self, EditPlanError> {
        if edits.is_empty() {
            return Err(EditPlanError::Empty);
        }
        if edits.len() > MAX_PLAN_EDITS {
            return Err(EditPlanError::TooManyEdits {
                count: edits.len(),
                max: MAX_PLAN_EDITS,
            });
        }
        let mut prior_end = 0usize;
        let mut total_bytes = 0usize;
        for (index, edit) in edits.iter().enumerate() {
            let Range { start, end } = edit.byte_range;
            if start > end {
                return Err(EditPlanError::ReversedRange { index, start, end });
            }
            if index > 0 && start < prior_end {
                return Err(EditPlanError::OverlappingEdits {
                    index,
                    start,
                    prior_end,
                });
            }
            let span = end - start;
            if span > MAX_PLANNED_SPAN_BYTES {
                return Err(EditPlanError::SpanTooLarge {
                    index,
                    len: span,
                    max: MAX_PLANNED_SPAN_BYTES,
                });
            }
            if edit.replacement.len() > MAX_PLANNED_REPLACEMENT_BYTES {
                return Err(EditPlanError::ReplacementTooLarge {
                    index,
                    len: edit.replacement.len(),
                    max: MAX_PLANNED_REPLACEMENT_BYTES,
                });
            }
            total_bytes = total_bytes
                .saturating_add(span)
                .saturating_add(edit.replacement.len());
            if total_bytes > MAX_PLAN_TOTAL_BYTES {
                return Err(EditPlanError::PlanTooLarge {
                    bytes: total_bytes,
                    max: MAX_PLAN_TOTAL_BYTES,
                });
            }
            prior_end = end;
        }
        Ok(Self {
            base_revision,
            edits,
            selection_after,
        })
    }

    pub fn base_revision(&self) -> Revision {
        self.base_revision
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn selection_after(&self) -> Selection {
        self.selection_after
    }

    /// Lowers the plan into the existing document contract.
    pub fn into_transaction(self, id: u64) -> EditTransaction {
        EditTransaction {
            base_revision: self.base_revision,
            id,
            kind: TransactionKind::Programmatic,
            edits: self.edits,
        }
    }
}
