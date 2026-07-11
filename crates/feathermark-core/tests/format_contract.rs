use feathermark_core::{
    Document, Edit, EditPlan, EditPlanError, FormatCommand, ListMarker, MAX_PLAN_EDITS,
    MAX_PLAN_TOTAL_BYTES, MAX_PLANNED_REPLACEMENT_BYTES, MAX_PLANNED_SPAN_BYTES, OrderedDelimiter,
    Selection, SmartEnterAction, TransactionKind,
};

fn edit(start: usize, end: usize, replacement: &str) -> Edit {
    Edit {
        byte_range: start..end,
        replacement: replacement.into(),
    }
}

fn plan(edits: Vec<Edit>) -> Result<EditPlan, EditPlanError> {
    EditPlan::new(0, edits, Selection::collapsed(0))
}

/// Every 0.2 format command, pinned. Adding, removing, or renaming a variant
/// must fail this test so the contract freeze is explicit.
#[test]
fn format_command_enumeration_is_stable() {
    let all = [
        FormatCommand::ToggleBold,
        FormatCommand::ToggleItalic,
        FormatCommand::ToggleCodeSpan,
        FormatCommand::ToggleCodeBlock,
        FormatCommand::InsertLink { url: None },
        FormatCommand::CycleHeading,
        FormatCommand::ToggleQuote,
        FormatCommand::ToggleBulletList,
        FormatCommand::ToggleOrderedList,
        FormatCommand::ToggleChecklist,
        FormatCommand::SmartEnter,
    ];
    let names: Vec<&str> = all.iter().map(FormatCommand::name).collect();
    assert_eq!(names, FormatCommand::VARIANT_NAMES);
    assert_eq!(
        FormatCommand::VARIANT_NAMES,
        [
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
        ]
    );
}

#[test]
fn insert_link_name_is_stable_regardless_of_payload() {
    let with_url = FormatCommand::InsertLink {
        url: Some("https://example.com".into()),
    };
    assert_eq!(with_url.name(), "insert-link");
    assert_eq!(
        FormatCommand::InsertLink { url: None }.name(),
        "insert-link"
    );
}

/// Smart-Enter continuation semantics as typed operations, pinned.
#[test]
fn smart_enter_action_enumeration_is_stable() {
    let all = [
        SmartEnterAction::InsertNewline,
        SmartEnterAction::ContinueBullet {
            marker: ListMarker::Dash,
        },
        SmartEnterAction::ContinueOrdered {
            number: 2,
            delimiter: OrderedDelimiter::Dot,
        },
        SmartEnterAction::ContinueChecklist {
            marker: ListMarker::Asterisk,
        },
        SmartEnterAction::ContinueQuote { depth: 1 },
        SmartEnterAction::ExitEmptyItem,
    ];
    let names: Vec<&str> = all.iter().map(SmartEnterAction::name).collect();
    assert_eq!(names, SmartEnterAction::VARIANT_NAMES);
    assert_eq!(
        SmartEnterAction::VARIANT_NAMES,
        [
            "insert-newline",
            "continue-bullet",
            "continue-ordered",
            "continue-checklist",
            "continue-quote",
            "exit-empty-item",
        ]
    );
}

#[test]
fn list_marker_and_delimiter_enumerations_are_stable() {
    let markers = [ListMarker::Dash, ListMarker::Asterisk, ListMarker::Plus];
    assert_eq!(
        markers.map(ListMarker::as_char),
        ['-', '*', '+'],
        "CommonMark bullet markers are exactly -, *, +"
    );
    let delimiters = [OrderedDelimiter::Dot, OrderedDelimiter::Paren];
    assert_eq!(
        delimiters.map(OrderedDelimiter::as_char),
        ['.', ')'],
        "CommonMark ordered-list delimiters are exactly . and )"
    );
}

#[test]
fn edit_plan_accepts_span_scoped_marker_edits() {
    let built = EditPlan::new(
        7,
        vec![edit(4, 4, "**"), edit(10, 10, "**")],
        Selection {
            anchor: 6,
            head: 12,
        },
    )
    .unwrap();
    assert_eq!(built.base_revision(), 7);
    assert_eq!(built.edits().len(), 2);
    assert_eq!(
        built.selection_after(),
        Selection {
            anchor: 6,
            head: 12
        }
    );
}

#[test]
fn edit_plan_rejects_empty_edit_lists() {
    assert_eq!(plan(vec![]), Err(EditPlanError::Empty));
}

#[test]
#[allow(clippy::reversed_empty_ranges)] // the reversed range is the thing under test
fn edit_plan_rejects_reversed_ranges() {
    assert_eq!(
        plan(vec![Edit {
            byte_range: 5..2,
            replacement: String::new(),
        }]),
        Err(EditPlanError::ReversedRange {
            index: 0,
            start: 5,
            end: 2,
        })
    );
}

#[test]
fn edit_plan_rejects_unsorted_or_overlapping_edits() {
    assert_eq!(
        plan(vec![edit(10, 12, ""), edit(4, 6, "")]),
        Err(EditPlanError::OverlappingEdits {
            index: 1,
            start: 4,
            prior_end: 12,
        })
    );
    assert_eq!(
        plan(vec![edit(0, 8, ""), edit(4, 6, "")]),
        Err(EditPlanError::OverlappingEdits {
            index: 1,
            start: 4,
            prior_end: 8,
        })
    );
}

#[test]
fn edit_plan_rejects_whole_buffer_spans() {
    let span = MAX_PLANNED_SPAN_BYTES + 1;
    assert_eq!(
        plan(vec![edit(0, span, "")]),
        Err(EditPlanError::SpanTooLarge {
            index: 0,
            len: span,
            max: MAX_PLANNED_SPAN_BYTES,
        })
    );
}

#[test]
fn edit_plan_rejects_oversized_replacements() {
    let replacement = "x".repeat(MAX_PLANNED_REPLACEMENT_BYTES + 1);
    assert_eq!(
        plan(vec![edit(0, 0, &replacement)]),
        Err(EditPlanError::ReplacementTooLarge {
            index: 0,
            len: replacement.len(),
            max: MAX_PLANNED_REPLACEMENT_BYTES,
        })
    );
}

#[test]
fn edit_plan_rejects_too_many_edits() {
    let edits: Vec<Edit> = (0..MAX_PLAN_EDITS + 1)
        .map(|index| edit(index * 2, index * 2, "x"))
        .collect();
    assert_eq!(
        plan(edits),
        Err(EditPlanError::TooManyEdits {
            count: MAX_PLAN_EDITS + 1,
            max: MAX_PLAN_EDITS,
        })
    );
}

#[test]
fn edit_plan_rejects_plans_over_the_total_byte_budget() {
    // Each edit is individually valid, but together they exceed the plan
    // budget, so a large-scale rewrite assembled from small edits is still
    // unrepresentable.
    let replacement = "x".repeat(MAX_PLANNED_REPLACEMENT_BYTES);
    let per_edit = MAX_PLANNED_REPLACEMENT_BYTES;
    let needed = MAX_PLAN_TOTAL_BYTES / per_edit + 1;
    assert!(
        needed <= MAX_PLAN_EDITS,
        "test must trip the byte budget first"
    );
    let edits: Vec<Edit> = (0..needed)
        .map(|index| edit(index * 2, index * 2, &replacement))
        .collect();
    let result = plan(edits);
    assert!(
        matches!(result, Err(EditPlanError::PlanTooLarge { max, .. }) if max == MAX_PLAN_TOTAL_BYTES),
        "expected PlanTooLarge, got {result:?}"
    );
}

// The "no whole-buffer rewrites" guarantee, enforced at compile time: a plan's
// total and per-edit byte budgets sit far below the document cap, so a plan can
// never re-emit a 20 MiB document.
const _: () = assert!(MAX_PLAN_TOTAL_BYTES < feathermark_core::MAX_DOCUMENT_BYTES);
const _: () = assert!(MAX_PLANNED_SPAN_BYTES < feathermark_core::MAX_DOCUMENT_BYTES);

#[test]
fn edit_plan_converts_into_an_applicable_transaction() {
    let mut document = Document::new("bold me").unwrap();
    let built = EditPlan::new(
        document.revision(),
        vec![edit(0, 0, "**"), edit(7, 7, "**")],
        Selection { anchor: 2, head: 9 },
    )
    .unwrap();
    let transaction = built.into_transaction(41);
    assert_eq!(transaction.id, 41);
    assert_eq!(transaction.kind, TransactionKind::Programmatic);
    document.apply(transaction).unwrap();
    assert_eq!(document.snapshot().to_string(), "**bold me**");
}

#[test]
fn edit_plan_transactions_respect_document_bounds_checks() {
    let mut document = Document::new("ab").unwrap();
    let built = EditPlan::new(
        document.revision(),
        vec![edit(100, 101, "")],
        Selection::collapsed(0),
    )
    .unwrap();
    // The plan is internally consistent; the document still enforces its own
    // bounds when the transaction is applied.
    assert!(document.apply(built.into_transaction(1)).is_err());
}
