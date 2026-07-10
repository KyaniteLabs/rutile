use std::sync::Arc;

use feathermark_core::{RenderError, RenderedPage, render_markdown};
use feathermark_types::Revision;

pub const DEBOUNCE_MS: u64 = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRequest {
    pub revision: Revision,
    pub source: Arc<str>,
}

impl RenderRequest {
    pub fn new(revision: Revision, source: Arc<str>) -> Self {
        Self { revision, source }
    }
}

#[derive(Debug)]
pub struct RenderPermit {
    id: u64,
    revision: Revision,
    source: Arc<str>,
}

impl RenderPermit {
    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn execute(self) -> CompletedRender {
        self.execute_with(|source, revision| match render_markdown(source, revision) {
            Ok(page) => RenderJobResult::Rendered(page),
            Err(error) => RenderJobResult::Failed { revision, error },
        })
    }

    pub fn execute_with(
        self,
        renderer: impl FnOnce(&str, Revision) -> RenderJobResult,
    ) -> CompletedRender {
        let result = renderer(&self.source, self.revision);
        CompletedRender {
            id: self.id,
            issued_revision: self.revision,
            result,
        }
    }
}

#[derive(Debug)]
pub struct CompletedRender {
    id: u64,
    issued_revision: Revision,
    result: RenderJobResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderJobResult {
    Rendered(RenderedPage),
    Failed {
        revision: Revision,
        error: RenderError,
    },
}

impl RenderJobResult {
    fn revision(&self) -> Revision {
        match self {
            Self::Rendered(page) => page.revision,
            Self::Failed { revision, .. } => *revision,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Completion {
    Accepted(RenderedPage),
    PreviewTooLarge {
        revision: Revision,
    },
    Failed {
        revision: Revision,
        error: RenderError,
    },
    DiscardedStale {
        revision: Revision,
    },
    UnknownJob {
        id: u64,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SchedulerStats {
    pub skipped_revisions: u64,
    pub stale_results: u64,
    pub retained_stale_pages: usize,
}

#[derive(Clone, Debug)]
struct Pending {
    request: RenderRequest,
    last_submitted_ms: u64,
}

#[derive(Clone, Copy, Debug)]
struct Running {
    id: u64,
    revision: Revision,
}

#[derive(Debug, Default)]
pub struct RenderScheduler {
    running: Option<Running>,
    pending: Option<Pending>,
    next_job_id: u64,
    stats: SchedulerStats,
}

impl RenderScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(&mut self, request: RenderRequest, now_ms: u64) {
        if self
            .pending
            .replace(Pending {
                request,
                last_submitted_ms: now_ms,
            })
            .is_some()
        {
            self.stats.skipped_revisions = self.stats.skipped_revisions.saturating_add(1);
        }
    }

    pub fn start_ready(&mut self, now_ms: u64) -> Option<RenderPermit> {
        if self.running.is_some() {
            return None;
        }
        let pending = self.pending.as_ref()?;
        if now_ms.saturating_sub(pending.last_submitted_ms) < DEBOUNCE_MS {
            return None;
        }

        let pending = self.pending.take().expect("pending was just inspected");
        self.next_job_id = self.next_job_id.saturating_add(1);
        let permit = RenderPermit {
            id: self.next_job_id,
            revision: pending.request.revision,
            source: pending.request.source,
        };
        self.running = Some(Running {
            id: permit.id,
            revision: permit.revision,
        });
        Some(permit)
    }

    pub fn finish(&mut self, completed: CompletedRender, current_revision: Revision) -> Completion {
        let Some(running) = self.running.as_ref() else {
            return Completion::UnknownJob { id: completed.id };
        };
        if running.id != completed.id || running.revision != completed.issued_revision {
            return Completion::UnknownJob { id: completed.id };
        }

        let running_revision = running.revision;
        self.running = None;
        let result_revision = completed.result.revision();
        if result_revision != running_revision
            || completed.issued_revision != running_revision
            || result_revision != current_revision
        {
            self.stats.stale_results = self.stats.stale_results.saturating_add(1);
            self.stats.retained_stale_pages = 0;
            return Completion::DiscardedStale {
                revision: result_revision,
            };
        }

        match completed.result {
            RenderJobResult::Rendered(page) => Completion::Accepted(page),
            RenderJobResult::Failed {
                revision,
                error: RenderError::PreviewTooLarge,
            } => Completion::PreviewTooLarge { revision },
            RenderJobResult::Failed { revision, error } => Completion::Failed { revision, error },
        }
    }

    pub fn running_revision(&self) -> Option<Revision> {
        self.running.as_ref().map(|running| running.revision)
    }

    pub fn pending_revision(&self) -> Option<Revision> {
        self.pending
            .as_ref()
            .map(|pending| pending.request.revision)
    }

    pub fn pending_depth(&self) -> usize {
        usize::from(self.pending.is_some())
    }

    pub fn stats(&self) -> SchedulerStats {
        self.stats
    }
}
