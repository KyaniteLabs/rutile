use feathermark_core::{
    MAX_GENERATED_BODY_BYTES, MAX_RENDERED_PAGE_BYTES, RenderError, RenderLimits, SourceBlock,
    SourceBlockKind, render_markdown, render_markdown_with_limits, validate_source_blocks,
};
use std::time::{Duration, Instant};

#[test]
fn renders_a_revisioned_complete_page_with_only_fixed_assets() {
    let rendered = render_markdown("# Hello\n\nworld", 7).unwrap();

    assert_eq!(rendered.revision, 7);
    assert!(rendered.page.starts_with("<!doctype html>"));
    assert!(rendered.page.contains("data-feathermark-revision=\"7\""));
    assert!(
        rendered
            .page
            .contains("href=\"feathermark://preview/v1/assets/preview.css\"")
    );
    assert!(
        rendered
            .page
            .contains("src=\"feathermark://preview/v1/assets/bridge.js\"")
    );
    assert!(!rendered.page.contains("http://"));
    assert!(!rendered.page.contains("https://"));
    assert!(rendered.body.len() <= MAX_GENERATED_BODY_BYTES);
    assert!(rendered.page.len() <= MAX_RENDERED_PAGE_BYTES);
}

#[test]
fn escapes_raw_html_and_converts_images_to_alt_only() {
    let rendered = render_markdown(
        "<svg onload=alert(1)></svg> ![kitten <b>](https://tracker.invalid/a.png)",
        1,
    )
    .unwrap();

    assert!(rendered.body.contains("&lt;svg onload=alert(1)&gt;"));
    assert!(rendered.body.contains("[image: kitten &lt;b&gt;]"));
    assert!(!rendered.body.contains("<svg"));
    assert!(!rendered.body.contains("<img"));
    assert!(!rendered.body.contains("tracker.invalid"));
}

#[test]
fn links_use_the_shared_typed_policy_and_never_href() {
    let rendered = render_markdown(
        "[safe](HTTPS://Example.COM/a) [bad](javascript:alert(1))",
        2,
    )
    .unwrap();

    assert!(
        rendered
            .body
            .contains("data-feathermark-url=\"https://example.com/a\"")
    );
    assert!(rendered.body.contains("role=\"link\""));
    assert!(!rendered.body.contains("href="));
    assert!(!rendered.body.contains("javascript:"));
    assert!(rendered.body.contains(">bad<"));
}

#[test]
fn source_blocks_prefer_deep_visible_leaves_and_preserve_unicode_boundaries() {
    let source = "> outer\n>\n> paragraph éβ\n\n---\n";
    let rendered = render_markdown(source, 9).unwrap();

    assert_eq!(rendered.blocks.len(), 3);
    assert_eq!(rendered.blocks[0].kind, SourceBlockKind::Paragraph);
    assert_eq!(rendered.blocks[1].kind, SourceBlockKind::Paragraph);
    assert_eq!(rendered.blocks[2].kind, SourceBlockKind::ThematicBreak);
    for (ordinal, block) in rendered.blocks.iter().enumerate() {
        assert_eq!(block.ordinal as usize, ordinal);
        assert_eq!(block.dom_id, format!("sb-9-{ordinal}"));
        assert!(source.is_char_boundary(block.start));
        assert!(source.is_char_boundary(block.end));
    }
}

#[test]
fn giant_leaf_is_replaced_by_exact_nonempty_continuations() {
    let source = format!("{}\n", "é".repeat(20_000));
    let rendered = render_markdown(&source, 11).unwrap();
    assert!(rendered.blocks.len() >= 2);
    assert_eq!(rendered.blocks[0].kind, SourceBlockKind::Paragraph);
    assert!(
        rendered.blocks[1..]
            .iter()
            .all(|block| block.kind == SourceBlockKind::Continuation)
    );

    let mut cursor = rendered.blocks[0].start;
    for (index, block) in rendered.blocks.iter().enumerate() {
        assert_eq!(block.start, cursor);
        assert!(block.end > block.start);
        assert!(block.end - block.start <= 32 * 1024 + 4);
        assert_eq!(block.segment_index as usize, index);
        assert_eq!(block.segment_count as usize, rendered.blocks.len());
        cursor = block.end;
    }
    assert_eq!(cursor, source.len());
}

#[test]
fn giant_inline_leaf_clones_formatting_and_link_shells_at_segment_cuts() {
    let label = "a".repeat(70_000);
    let source = format!("**[{label}](HTTPS://Example.COM/path)**\n");
    let rendered = render_markdown(&source, 12).unwrap();

    assert!(rendered.blocks.len() >= 3);
    assert!(rendered.body.matches("<strong>").count() >= 3);
    assert!(rendered.body.matches("<a role=\"link\"").count() >= 3);
    assert!(!rendered.body.contains("**"));
    assert!(!rendered.body.contains("]("));
    assert_eq!(
        rendered
            .body
            .matches("data-feathermark-url=\"https://example.com/path\"")
            .count(),
        rendered.blocks.len()
    );
}

#[test]
fn giant_inline_code_clones_code_shells_without_leaking_markdown_delimiters() {
    let source = format!("`{}`\n", "z".repeat(70_000));
    let rendered = render_markdown(&source, 15).unwrap();

    assert!(rendered.body.matches("<code>").count() >= 3);
    assert!(!rendered.body.contains('`'));
    assert_eq!(rendered.body.matches('z').count(), 70_000);
}

#[test]
fn table_rows_place_anchors_inside_aligned_header_and_data_cells() {
    let source = "| left | right |\n| :--- | ---: |\n| a | b |\n";
    let rendered = render_markdown(source, 13).unwrap();

    assert_eq!(
        rendered
            .blocks
            .iter()
            .filter(|block| block.kind == SourceBlockKind::TableRow)
            .count(),
        2
    );
    assert!(
        rendered
            .body
            .contains("<th class=\"align-left\"><span id=\"sb-13-0\"")
    );
    assert!(rendered.body.contains("<th class=\"align-right\">"));
    assert!(
        rendered
            .body
            .contains("<td class=\"align-left\"><span id=\"sb-13-1\"")
    );
    assert!(rendered.body.contains("<td class=\"align-right\">"));
    assert!(!rendered.body.contains("style="));
}

#[test]
fn malformed_source_ranges_are_rejected_not_clamped() {
    let source = "é";
    let malformed = vec![SourceBlock {
        revision: 1,
        start: 1,
        end: 2,
        ordinal: 0,
        depth: 0,
        kind: SourceBlockKind::Paragraph,
        dom_id: "sb-1-0".into(),
        segment_index: 0,
        segment_count: 1,
    }];
    assert_eq!(
        validate_source_blocks(source, 1, &malformed),
        Err(RenderError::InvalidSourceRange)
    );
}

#[test]
fn source_block_validation_rejects_overlap_oversize_and_broken_segment_groups() {
    let source = "a".repeat(70_000);
    let valid = render_markdown(&source, 14).unwrap().blocks;

    let mut overlap = valid.clone();
    overlap[1].start = overlap[0].start;
    assert_eq!(
        validate_source_blocks(&source, 14, &overlap),
        Err(RenderError::InvalidSourceRange)
    );

    let mut oversize = valid.clone();
    oversize[0].end = oversize[0].start + 32 * 1024 + 1;
    assert_eq!(
        validate_source_blocks(&source, 14, &oversize),
        Err(RenderError::InvalidSourceRange)
    );

    let mut broken_group = valid.clone();
    broken_group[1].segment_count += 1;
    assert_eq!(
        validate_source_blocks(&source, 14, &broken_group),
        Err(RenderError::InvalidSourceRange)
    );

    let mut continuation_first = valid;
    continuation_first[0].kind = SourceBlockKind::Continuation;
    assert_eq!(
        validate_source_blocks(&source, 14, &continuation_first),
        Err(RenderError::InvalidSourceRange)
    );
}

#[test]
fn body_and_page_caps_return_typed_errors() {
    let source = "hello";
    let body_error = render_markdown_with_limits(
        source,
        1,
        RenderLimits {
            max_body_bytes: 3,
            max_page_bytes: usize::MAX,
        },
    )
    .unwrap_err();
    assert_eq!(body_error, RenderError::PreviewTooLarge);

    let page_error = render_markdown_with_limits(
        source,
        1,
        RenderLimits {
            max_body_bytes: usize::MAX,
            max_page_bytes: 16,
        },
    )
    .unwrap_err();
    assert_eq!(page_error, RenderError::PreviewTooLarge);
}

#[test]
fn caller_limits_cannot_raise_the_universal_hard_caps() {
    assert_eq!(
        RenderLimits {
            max_body_bytes: usize::MAX,
            max_page_bytes: usize::MAX,
        }
        .clamped(),
        RenderLimits::default()
    );
}

#[test]
fn one_and_five_mib_render_regression_gates_are_linear_enough_for_interactive_use() {
    for (bytes, gate) in [
        (1024 * 1024, Duration::from_secs(5)),
        (5 * 1024 * 1024, Duration::from_secs(20)),
    ] {
        let source = format!("{}\n", "a".repeat(bytes - 1));
        let started = Instant::now();
        let rendered = render_markdown(&source, bytes as u64).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(rendered.blocks.first().unwrap().start, 0);
        assert_eq!(rendered.blocks.last().unwrap().end, source.len());
        assert!(
            elapsed <= gate,
            "{bytes}-byte render took {elapsed:?}, gate {gate:?}"
        );
    }
}

#[test]
fn fuzz_regression_list_item_link_reference_definition_renders_without_panicking() {
    // 30-minute fuzz campaign artifact (2026-07-10): a tight list item whose
    // paragraph is only a link-reference definition panicked inside
    // pulldown-cmark (parse.rs:2199 unwrap on empty tight paragraph).
    let source = "-\t[`]:I\r\t\t";
    let rendered = render_markdown(source, 11).unwrap();
    validate_source_blocks(source, 11, &rendered.blocks).unwrap();
}

#[test]
fn fuzz_regression_link_reference_definition_source_blocks_are_valid() {
    // Fuzz campaign artifact (2026-07-10): build_source_blocks failed its own
    // validation (InvalidSourceRange) on this 11-byte input.
    let source = "[ =5(]:$#\n\t";
    let blocks = feathermark_core::build_source_blocks(source, 23).unwrap();
    validate_source_blocks(source, 23, &blocks).unwrap();
}

#[test]
fn hostile_deep_nesting_is_rejected_not_a_stack_overflow() {
    // QA round 1 (2026-07-10): a small hostile document of deeply nested
    // blockquotes ("> " repeated) builds a SafeNode tree whose depth mirrors the
    // nesting. The tree is traversed recursively (to_html, partition, Drop), so
    // unbounded nesting overflowed the thread stack and, under panic = "abort",
    // aborted the whole process -- a hostile-document DoS. A 20 MiB document of
    // "> " yields millions of levels and overflows any stack. Rendering must now
    // reject over-deep nesting with a bounded error instead of crashing.
    let source = "> ".repeat(50_000) + "x";
    assert_eq!(
        render_markdown(&source, 1),
        Err(RenderError::NestingTooDeep),
    );
}

#[test]
fn nesting_at_the_cap_still_renders() {
    // One below the cap must still render successfully (no false rejection of
    // merely-deep-but-bounded documents).
    let depth = feathermark_core::MAX_RENDER_NESTING_DEPTH - 1;
    let source = "> ".repeat(depth) + "x";
    let rendered = render_markdown(&source, 1).expect("depth below the cap renders");
    validate_source_blocks(&source, 1, &rendered.blocks).unwrap();
}
