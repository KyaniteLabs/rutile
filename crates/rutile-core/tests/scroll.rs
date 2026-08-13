use rutile_core::{
    InteractionLease, Pane, ScrollAnchorView, ScrollClock, ScrollError, ScrollGeometry, ScrollMap,
    ScrollOutcome, ScrollPosition, ScrollSynchronizer, ScrollTarget, SuppressionReason,
    preview_samples, source_samples,
};
use rutile_types::{InteractionId, Revision};

#[derive(Clone, Copy, Debug)]
struct Block {
    revision: Revision,
    start: usize,
    end: usize,
    ordinal: u32,
    top: f64,
}

impl ScrollAnchorView for Block {
    fn revision(&self) -> Revision {
        self.revision
    }

    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.end
    }

    fn ordinal(&self) -> u32 {
        self.ordinal
    }

    fn preview_top(&self) -> f64 {
        self.top
    }
}

fn map(source_max_top: usize, preview_max_y: f64) -> ScrollMap {
    ScrollMap::new(
        ScrollGeometry {
            revision: Revision::new(7),
            document_len: 100,
            source_max_top,
            preview_max_y,
        },
        [
            Block {
                revision: Revision::new(7),
                start: 0,
                end: 10,
                ordinal: 0,
                top: 0.0,
            },
            Block {
                revision: Revision::new(7),
                start: 20,
                end: 40,
                ordinal: 1,
                top: 25.0,
            },
            Block {
                revision: Revision::new(7),
                start: 40,
                end: 70,
                ordinal: 2,
                top: 25.0,
            },
            Block {
                revision: Revision::new(7),
                start: 70,
                end: 100,
                ordinal: 3,
                top: 90.0,
            },
        ],
    )
    .unwrap()
}

#[test]
fn no_scroll_is_the_first_branch_in_both_directions() {
    for (source_max_top, preview_max_y) in [(0, 0.0), (0, 100.0), (100, 0.0)] {
        let map = ScrollMap::new(
            ScrollGeometry {
                revision: Revision::new(7),
                document_len: source_max_top,
                source_max_top,
                preview_max_y,
            },
            std::iter::empty::<Block>(),
        )
        .unwrap();

        assert_eq!(
            map.source_to_preview(Revision::new(7), usize::MAX).unwrap(),
            0.0
        );
        assert_eq!(
            map.preview_to_source(Revision::new(7), f64::NAN).unwrap(),
            0
        );
    }
}

#[test]
fn source_mapping_uses_containing_then_greatest_prior_block() {
    let map = map(80, 100.0);
    assert_eq!(map.source_to_preview(Revision::new(7), 5).unwrap(), 0.0);
    assert_eq!(map.source_to_preview(Revision::new(7), 15).unwrap(), 0.0);
    assert_eq!(map.source_to_preview(Revision::new(7), 20).unwrap(), 25.0);
    assert_eq!(map.source_to_preview(Revision::new(7), 69).unwrap(), 25.0);
}

#[test]
fn positions_before_the_first_anchor_fall_back_to_ordinal_zero() {
    let map = ScrollMap::new(
        ScrollGeometry {
            revision: Revision::new(3),
            document_len: 50,
            source_max_top: 49,
            preview_max_y: 50.0,
        },
        [Block {
            revision: Revision::new(3),
            start: 10,
            end: 20,
            ordinal: 0,
            top: 12.0,
        }],
    )
    .unwrap();

    assert_eq!(map.source_to_preview(Revision::new(3), 0).unwrap(), 12.0);
    assert_eq!(map.preview_to_source(Revision::new(3), 0.0).unwrap(), 10);
}

#[test]
fn source_bottom_maps_exactly_to_preview_bottom() {
    let map = map(80, 100.0);
    assert_eq!(map.source_to_preview(Revision::new(7), 80).unwrap(), 100.0);
    assert_eq!(
        map.source_to_preview(Revision::new(7), usize::MAX).unwrap(),
        100.0
    );
}

#[test]
fn preview_mapping_binary_searches_tops_and_breaks_ties_by_greater_ordinal() {
    let map = map(80, 100.0);
    assert_eq!(map.preview_to_source(Revision::new(7), 0.0).unwrap(), 0);
    assert_eq!(map.preview_to_source(Revision::new(7), 24.0).unwrap(), 40);
    assert_eq!(map.preview_to_source(Revision::new(7), 25.0).unwrap(), 40);
    assert_eq!(map.preview_to_source(Revision::new(7), 89.0).unwrap(), 70);
}

#[test]
fn preview_bottom_requests_document_eof() {
    let map = map(80, 100.0);
    assert_eq!(map.preview_to_source(Revision::new(7), 99.0).unwrap(), 100);
    assert_eq!(
        map.preview_to_source(Revision::new(7), 1000.0).unwrap(),
        100
    );
}

#[test]
fn empty_duplicate_gap_and_continuation_shaped_blocks_are_deterministic() {
    let map = ScrollMap::new(
        ScrollGeometry {
            revision: Revision::new(9),
            document_len: 50,
            source_max_top: 49,
            preview_max_y: 50.0,
        },
        [
            Block {
                revision: Revision::new(9),
                start: 0,
                end: 0,
                ordinal: 0,
                top: 0.0,
            },
            Block {
                revision: Revision::new(9),
                start: 0,
                end: 10,
                ordinal: 1,
                top: 0.0,
            },
            Block {
                revision: Revision::new(9),
                start: 20,
                end: 35,
                ordinal: 2,
                top: 20.0,
            },
            // A continuation is deliberately indistinguishable to this mapping layer.
            Block {
                revision: Revision::new(9),
                start: 35,
                end: 50,
                ordinal: 3,
                top: 35.0,
            },
        ],
    )
    .unwrap();

    assert_eq!(map.source_to_preview(Revision::new(9), 0).unwrap(), 0.0);
    assert_eq!(map.source_to_preview(Revision::new(9), 15).unwrap(), 0.0);
    assert_eq!(map.source_to_preview(Revision::new(9), 35).unwrap(), 35.0);
    assert_eq!(map.preview_to_source(Revision::new(9), 0.0).unwrap(), 0);
}

#[test]
fn stale_revisions_are_rejected_without_mapping() {
    let map = map(80, 100.0);
    assert_eq!(
        map.source_to_preview(Revision::new(8), 0),
        Err(ScrollError::StaleRevision {
            expected: Revision::new(7),
            actual: Revision::new(8)
        })
    );
    assert_eq!(
        map.preview_to_source(Revision::new(6), 0.0),
        Err(ScrollError::StaleRevision {
            expected: Revision::new(7),
            actual: Revision::new(6)
        })
    );
}

#[test]
fn construction_rejects_stale_invalid_and_nonfinite_anchors() {
    let geometry = ScrollGeometry {
        revision: Revision::new(7),
        document_len: 10,
        source_max_top: 9,
        preview_max_y: 10.0,
    };
    let stale = Block {
        revision: Revision::new(6),
        start: 0,
        end: 1,
        ordinal: 0,
        top: 0.0,
    };
    assert!(matches!(
        ScrollMap::new(geometry, [stale]),
        Err(ScrollError::StaleRevision { .. })
    ));

    let invalid = Block {
        revision: Revision::new(7),
        start: 8,
        end: 11,
        ordinal: 0,
        top: 0.0,
    };
    assert!(matches!(
        ScrollMap::new(geometry, [invalid]),
        Err(ScrollError::InvalidRange { .. })
    ));

    let nan = Block {
        revision: Revision::new(7),
        start: 0,
        end: 1,
        ordinal: 0,
        top: f64::NAN,
    };
    assert!(matches!(
        ScrollMap::new(geometry, [nan]),
        Err(ScrollError::NonFinitePosition)
    ));
}

#[test]
fn deterministic_sample_formulas_include_exact_endpoints() {
    let source = source_samples(1000);
    let preview = preview_samples(333.75);
    assert_eq!(source.len(), 100);
    assert_eq!(source[0], 0);
    assert_eq!(source[99], 999);
    assert!(source.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(preview[0], 0.0);
    assert_eq!(preview[99], 333.0);
    assert!(preview.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(source_samples(0), vec![0; 100]);
    assert_eq!(preview_samples(0.0), vec![0.0; 100]);
    assert_eq!(source_samples(usize::MAX)[99], usize::MAX - 1);
    assert_eq!(map(80, 100.0).geometry().document_len, 100);
}

#[test]
fn user_gestures_own_fresh_interactions_and_target_the_other_pane() {
    let map = map(80, 100.0);
    let mut sync = ScrollSynchronizer::new(Revision::new(7), InteractionId::new(41));
    let first = sync
        .handle_user(
            &map,
            Pane::Source,
            ScrollPosition::SourceByte(20),
            ScrollClock {
                monotonic_ms: 1_000,
                preview_frame: 10,
            },
        )
        .unwrap();
    let second = sync
        .handle_user(
            &map,
            Pane::Preview,
            ScrollPosition::PreviewY(24.0),
            ScrollClock {
                monotonic_ms: 1_001,
                preview_frame: 10,
            },
        )
        .unwrap();

    let ScrollOutcome::Command(first) = first else {
        panic!("command")
    };
    let ScrollOutcome::Command(second) = second else {
        panic!("command")
    };
    assert_eq!(first.interaction_id, InteractionId::new(41));
    assert_eq!(first.target_pane, Pane::Preview);
    assert_eq!(first.target, ScrollTarget::PreviewY(25.0));
    assert_eq!(second.interaction_id, InteractionId::new(42));
    assert_eq!(second.target_pane, Pane::Source);
    assert_eq!(second.target, ScrollTarget::SourceByte(40));
    assert_eq!(sync.lease().unwrap().owner, Pane::Preview);
}

#[test]
fn stale_or_mismatched_user_events_do_not_consume_an_interaction_id() {
    let map = map(80, 100.0);
    let mut sync = ScrollSynchronizer::new(Revision::new(7), InteractionId::new(5));
    assert!(matches!(
        sync.handle_user(
            &map,
            Pane::Source,
            ScrollPosition::SourceByte(0),
            ScrollClock {
                monotonic_ms: 0,
                preview_frame: 0
            },
        ),
        Ok(ScrollOutcome::Command(_))
    ));
    let err = sync.handle_user_revision(
        &map,
        Revision::new(6),
        Pane::Source,
        ScrollPosition::SourceByte(0),
        ScrollClock {
            monotonic_ms: 1,
            preview_frame: 0,
        },
    );
    assert!(matches!(err, Err(ScrollError::StaleRevision { .. })));
    assert_eq!(sync.next_interaction_id(), InteractionId::new(6));
}

#[test]
fn lease_requires_150ms_and_two_frames_but_never_exceeds_500ms() {
    let lease = InteractionLease {
        interaction_id: InteractionId::new(1),
        owner: Pane::Source,
        target: Pane::Preview,
        started: ScrollClock {
            monotonic_ms: 1_000,
            preview_frame: 10,
        },
    };
    assert!(lease.is_active(ScrollClock {
        monotonic_ms: 1_149,
        preview_frame: 12
    }));
    assert!(lease.is_active(ScrollClock {
        monotonic_ms: 1_200,
        preview_frame: 11
    }));
    assert!(!lease.is_active(ScrollClock {
        monotonic_ms: 1_150,
        preview_frame: 12
    }));
    assert!(!lease.is_active(ScrollClock {
        monotonic_ms: 1_500,
        preview_frame: 10
    }));
}

#[test]
fn programmatic_echoes_and_reversals_never_emit_commands() {
    let map = map(80, 100.0);
    let mut sync = ScrollSynchronizer::new(Revision::new(7), InteractionId::new(100));
    let ScrollOutcome::Command(command) = sync
        .handle_user(
            &map,
            Pane::Source,
            ScrollPosition::SourceByte(20),
            ScrollClock {
                monotonic_ms: 0,
                preview_frame: 0,
            },
        )
        .unwrap()
    else {
        panic!("command")
    };

    assert_eq!(
        sync.handle_programmatic(
            Revision::new(7),
            Pane::Preview,
            command.interaction_id,
            ScrollClock {
                monotonic_ms: 100,
                preview_frame: 2
            },
        )
        .unwrap(),
        ScrollOutcome::Suppressed(SuppressionReason::ProgrammaticEcho)
    );
    assert_eq!(
        sync.handle_programmatic(
            Revision::new(7),
            Pane::Source,
            command.interaction_id,
            ScrollClock {
                monotonic_ms: 110,
                preview_frame: 2
            },
        )
        .unwrap(),
        ScrollOutcome::Suppressed(SuppressionReason::DirectionReversal)
    );
    assert_eq!(
        sync.handle_programmatic(
            Revision::new(7),
            Pane::Preview,
            command.interaction_id,
            ScrollClock {
                monotonic_ms: 600,
                preview_frame: 20
            },
        )
        .unwrap(),
        ScrollOutcome::Suppressed(SuppressionReason::ExpiredInteraction)
    );
}

#[test]
fn one_hundred_alternating_gestures_have_unique_ids_and_zero_ping_pong() {
    let map = map(80, 100.0);
    let mut sync = ScrollSynchronizer::new(Revision::new(7), InteractionId::new(1));
    for i in 0..100_u64 {
        let pane = if i % 2 == 0 {
            Pane::Source
        } else {
            Pane::Preview
        };
        let position = if pane == Pane::Source {
            ScrollPosition::SourceByte((i as usize) % 80)
        } else {
            ScrollPosition::PreviewY((i % 99) as f64)
        };
        let ScrollOutcome::Command(command) = sync
            .handle_user(
                &map,
                pane,
                position,
                ScrollClock {
                    monotonic_ms: i * 10,
                    preview_frame: i,
                },
            )
            .unwrap()
        else {
            panic!("command")
        };
        assert_eq!(command.interaction_id, InteractionId::new(i + 1));
        assert!(matches!(
            sync.handle_programmatic(
                Revision::new(7),
                command.target_pane,
                command.interaction_id,
                ScrollClock {
                    monotonic_ms: i * 10 + 1,
                    preview_frame: i + 1
                },
            )
            .unwrap(),
            ScrollOutcome::Suppressed(_)
        ));
    }
}
