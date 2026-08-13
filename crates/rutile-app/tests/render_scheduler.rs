use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rutile_app::render_scheduler::{
    Completion, DEBOUNCE_MS, RenderJobResult, RenderRequest, RenderScheduler,
};
use rutile_core::{RenderError, RenderedPage};
use rutile_types::Revision;

fn request(revision: u64) -> RenderRequest {
    RenderRequest::new(
        Revision::new(revision),
        Arc::from(format!("revision {revision}")),
    )
}

fn page(revision: u64) -> RenderedPage {
    RenderedPage {
        revision: Revision::new(revision),
        body: String::new(),
        page: format!("<html>{revision}</html>"),
        blocks: vec![],
    }
}

#[test]
fn debounce_restarts_after_the_latest_edit() {
    let mut scheduler = RenderScheduler::new();
    scheduler.submit(request(1), 0);
    scheduler.submit(request(2), 49);

    assert!(scheduler.start_ready(98).is_none());
    assert_eq!(
        scheduler.start_ready(49 + DEBOUNCE_MS).unwrap().revision(),
        Revision::new(2)
    );
}

#[test]
fn running_job_has_one_replaceable_pending_slot() {
    let mut scheduler = RenderScheduler::new();
    scheduler.submit(request(1), 0);
    let first = scheduler.start_ready(DEBOUNCE_MS).unwrap();
    scheduler.submit(request(2), 51);
    scheduler.submit(request(3), 52);

    assert_eq!(scheduler.running_revision(), Some(Revision::new(1)));
    assert_eq!(scheduler.pending_revision(), Some(Revision::new(3)));
    assert_eq!(scheduler.pending_depth(), 1);
    assert_eq!(scheduler.stats().skipped_revisions, 1);

    let completed = first.execute_with(|_, _| RenderJobResult::Rendered(page(1)));
    assert_eq!(
        scheduler.finish(completed, Revision::new(3)),
        Completion::DiscardedStale {
            revision: Revision::new(1)
        }
    );
    assert_eq!(scheduler.stats().retained_stale_pages, 0);
}

#[test]
fn size_error_is_typed_and_never_becomes_a_page() {
    let mut scheduler = RenderScheduler::new();
    scheduler.submit(request(7), 0);
    let job = scheduler.start_ready(DEBOUNCE_MS).unwrap();

    let completed = job.execute_with(|_, _| RenderJobResult::Failed {
        revision: Revision::new(7),
        error: RenderError::PreviewTooLarge,
    });
    assert_eq!(
        scheduler.finish(completed, Revision::new(7)),
        Completion::PreviewTooLarge {
            revision: Revision::new(7)
        }
    );
}

#[test]
fn a_result_cannot_impersonate_a_different_running_job_revision() {
    let mut scheduler = RenderScheduler::new();
    scheduler.submit(request(4), 0);
    let job = scheduler.start_ready(DEBOUNCE_MS).unwrap();

    let completed = job.execute_with(|_, _| RenderJobResult::Rendered(page(5)));
    assert_eq!(
        scheduler.finish(completed, Revision::new(5)),
        Completion::DiscardedStale {
            revision: Revision::new(5)
        }
    );
}

#[test]
fn one_thousand_edits_keep_only_the_latest_pending_revision() {
    let mut scheduler = RenderScheduler::new();
    scheduler.submit(request(0), 0);
    let running = scheduler.start_ready(DEBOUNCE_MS).unwrap();

    for revision in 1..=1_000 {
        scheduler.submit(request(revision), DEBOUNCE_MS + revision);
        assert!(scheduler.pending_depth() <= 1);
    }

    assert_eq!(scheduler.pending_revision(), Some(Revision::new(1_000)));
    assert_eq!(scheduler.stats().skipped_revisions, 999);
    assert!(matches!(
        scheduler.finish(
            running.execute_with(|_, _| RenderJobResult::Rendered(page(0))),
            Revision::new(1_000)
        ),
        Completion::DiscardedStale { .. }
    ));

    let newest = scheduler
        .start_ready(DEBOUNCE_MS + 1_000 + DEBOUNCE_MS)
        .unwrap();
    assert_eq!(newest.revision(), Revision::new(1_000));
    assert!(matches!(
        scheduler.finish(
            newest.execute_with(|_, _| RenderJobResult::Rendered(page(1_000))),
            Revision::new(1_000),
        ),
        Completion::Accepted(_)
    ));
    assert_eq!(scheduler.pending_depth(), 0);
    assert_eq!(scheduler.stats().retained_stale_pages, 0);
}

#[test]
fn only_one_unique_permit_can_invoke_the_renderer() {
    static INVOCATIONS: AtomicUsize = AtomicUsize::new(0);
    INVOCATIONS.store(0, Ordering::SeqCst);

    let mut scheduler = RenderScheduler::new();
    scheduler.submit(request(12), 0);
    let permit = scheduler.start_ready(DEBOUNCE_MS).unwrap();
    assert!(scheduler.start_ready(DEBOUNCE_MS + 1).is_none());

    let completed = permit.execute_with(|source, revision| {
        INVOCATIONS.fetch_add(1, Ordering::SeqCst);
        assert_eq!(source, "revision 12");
        RenderJobResult::Rendered(page(revision.get()))
    });
    assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 1);
    assert!(matches!(
        scheduler.finish(completed, Revision::new(12)),
        Completion::Accepted(_)
    ));
    assert_eq!(INVOCATIONS.load(Ordering::SeqCst), 1);
}
