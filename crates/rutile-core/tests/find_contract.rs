use rutile_core::{
    Document, EditPlan, FindDirection, FindError, FindQuery, FindReplaceOp, MAX_FIND_PATTERN_BYTES,
    MAX_FIND_REPLACEMENT_BYTES, MAX_PLANNED_REPLACEMENT_BYTES, MatchMode, ReplaceSpec, Selection,
};

fn query(pattern: &str) -> FindQuery {
    FindQuery::new(pattern.into(), MatchMode::Plain, true).unwrap()
}

/// The only match modes representable in 0.2. Regex is deliberately absent;
/// adding a variant must fail this pinned enumeration.
#[test]
fn match_mode_enumeration_is_stable_and_regex_free() {
    let all = [MatchMode::Plain, MatchMode::WholeWord];
    let names: Vec<&str> = all.iter().map(MatchMode::name).collect();
    assert_eq!(names, MatchMode::VARIANT_NAMES);
    assert_eq!(MatchMode::VARIANT_NAMES, ["plain", "whole-word"]);
}

#[test]
fn find_direction_enumeration_is_stable() {
    let all = [FindDirection::Forward, FindDirection::Backward];
    let names: Vec<&str> = all.iter().map(FindDirection::name).collect();
    assert_eq!(names, FindDirection::VARIANT_NAMES);
    assert_eq!(FindDirection::VARIANT_NAMES, ["forward", "backward"]);
}

#[test]
fn find_replace_op_enumeration_is_stable() {
    let all = [
        FindReplaceOp::Find {
            query: query("needle"),
            from_byte: 0,
            direction: FindDirection::Forward,
            wrap: true,
        },
        FindReplaceOp::ReplaceCurrent {
            spec: ReplaceSpec::new(query("needle"), "thread".into()).unwrap(),
            current: 4..10,
        },
        FindReplaceOp::ReplaceAll {
            spec: ReplaceSpec::new(query("needle"), "thread".into()).unwrap(),
        },
    ];
    let names: Vec<&str> = all.iter().map(FindReplaceOp::name).collect();
    assert_eq!(names, FindReplaceOp::VARIANT_NAMES);
    assert_eq!(
        FindReplaceOp::VARIANT_NAMES,
        ["find", "replace-current", "replace-all"]
    );
}

#[test]
fn find_query_accepts_bounded_patterns() {
    let built = FindQuery::new("Needle".into(), MatchMode::WholeWord, false).unwrap();
    assert_eq!(built.pattern(), "Needle");
    assert_eq!(built.mode(), MatchMode::WholeWord);
    assert!(!built.case_sensitive());
}

#[test]
fn find_query_rejects_empty_patterns() {
    assert!(matches!(
        FindQuery::new(String::new(), MatchMode::Plain, true),
        Err(FindError::EmptyPattern)
    ));
}

#[test]
fn find_query_rejects_oversized_patterns() {
    let pattern = "x".repeat(MAX_FIND_PATTERN_BYTES + 1);
    assert!(matches!(
        FindQuery::new(pattern, MatchMode::Plain, true),
        Err(FindError::PatternTooLarge {
            len,
            max: MAX_FIND_PATTERN_BYTES,
        }) if len == MAX_FIND_PATTERN_BYTES + 1
    ));
}

#[test]
fn replace_spec_rejects_oversized_replacements() {
    let replacement = "x".repeat(MAX_FIND_REPLACEMENT_BYTES + 1);
    assert!(matches!(
        ReplaceSpec::new(query("needle"), replacement),
        Err(FindError::ReplacementTooLarge {
            len,
            max: MAX_FIND_REPLACEMENT_BYTES,
        }) if len == MAX_FIND_REPLACEMENT_BYTES + 1
    ));
}

#[test]
fn replace_spec_exposes_its_parts() {
    let spec = ReplaceSpec::new(query("needle"), "thread".into()).unwrap();
    assert_eq!(spec.query().pattern(), "needle");
    assert_eq!(spec.replacement(), "thread");
}

// Every replacement a ReplaceSpec can carry must fit in an EditPlan edit, so
// replace-all is always expressible as span-scoped edits. Enforced at compile
// time against the format-contract budget.
const _: () = assert!(MAX_FIND_REPLACEMENT_BYTES <= MAX_PLANNED_REPLACEMENT_BYTES);
const _: () = assert!(MAX_FIND_PATTERN_BYTES <= MAX_FIND_REPLACEMENT_BYTES);

#[test]
fn a_replacement_round_trips_through_the_edit_plan_contract() {
    let mut document = Document::new("one needle, two needles").unwrap();
    let spec = ReplaceSpec::new(query("needle"), "thread".into()).unwrap();
    let plan = EditPlan::new(
        document.revision(),
        vec![
            rutile_core::Edit {
                byte_range: 4..10,
                replacement: spec.replacement().into(),
            },
            rutile_core::Edit {
                byte_range: 16..22,
                replacement: spec.replacement().into(),
            },
        ],
        Selection::collapsed(4),
    )
    .unwrap();
    document.apply(plan.into_transaction(7)).unwrap();
    assert_eq!(document.snapshot().to_string(), "one thread, two threads");
}
