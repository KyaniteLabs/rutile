use feathermark_app::app::{AppEffect, AppMessage, AppState, PreviewState};
use feathermark_protocol::PreviewEventV1;
use feathermark_types::SafeLinkTarget;

#[test]
fn editing_marks_dirty_and_coalesces_render_through_an_effect() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::DocumentEdited { revision: 3 });

    assert!(state.dirty());
    assert_eq!(state.revision(), 3);
    assert_eq!(effects, vec![AppEffect::ScheduleRender { revision: 3 }]);
}

#[test]
fn stale_render_and_paint_acknowledgements_have_no_state_effect() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 5 });

    assert_eq!(
        state.reduce(AppMessage::RenderAccepted {
            revision: 4,
            page_bytes: 20,
        }),
        vec![AppEffect::IgnoredStale { revision: 4 }]
    );
    assert_eq!(state.preview(), &PreviewState::Waiting { revision: 5 });

    assert_eq!(
        state.reduce(AppMessage::PreviewEvent(PreviewEventV1::Painted {
            revision: 4,
            frame_seq: 2,
        })),
        vec![AppEffect::IgnoredStale { revision: 4 }]
    );
}

#[test]
fn current_render_navigates_and_two_frame_paint_marks_ready() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 6 });
    assert_eq!(
        state.reduce(AppMessage::RenderAccepted {
            revision: 6,
            page_bytes: 100,
        }),
        vec![AppEffect::NavigatePreview {
            revision: 6,
            page_bytes: 100,
        }]
    );
    state.reduce(AppMessage::PreviewEvent(PreviewEventV1::Painted {
        revision: 6,
        frame_seq: 2,
    }));
    assert_eq!(state.preview(), &PreviewState::Ready { revision: 6 });
}

#[test]
fn typed_link_activation_crosses_the_reducer_without_a_string_url() {
    let target = SafeLinkTarget::parse("https://example.com/").unwrap();
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::PreviewEvent(PreviewEventV1::LinkActivated {
        revision: 0,
        target: target.clone(),
    }));

    assert_eq!(effects, vec![AppEffect::PresentLink(target)]);
}

#[test]
fn stale_preview_scroll_never_moves_the_source() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 3 });
    assert_eq!(
        state.reduce(AppMessage::PreviewEvent(PreviewEventV1::Scroll {
            revision: 2,
            source_start: 9,
            interaction_id: 7,
            user: true,
        })),
        vec![AppEffect::IgnoredStale { revision: 2 }]
    );
}

#[test]
fn stale_editor_acknowledgement_cannot_roll_back_the_revision() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 3 });

    assert_eq!(
        state.reduce(AppMessage::DocumentEdited { revision: 2 }),
        vec![AppEffect::IgnoredStale { revision: 2 }]
    );
    assert_eq!(state.revision(), 3);
}
