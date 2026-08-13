//! Revision-bound, offset-based two-way scroll synchronization.
//!
//! This module consumes measured views of render-owned source blocks through
//! [`ScrollAnchorView`]. It deliberately does not own or redefine the renderer's
//! permanent `SourceBlock` type.

use rutile_types::{InteractionId, Revision};

const MIN_LEASE_MS: u64 = 150;
const MIN_LEASE_FRAMES: u64 = 2;
const MAX_LEASE_MS: u64 = 500;
const PREVIEW_TOLERANCE_PX: f64 = 1.0;
const SAMPLE_COUNT: usize = 100;

/// The projection of a render-owned source block needed by scroll mapping.
pub trait ScrollAnchorView {
    fn revision(&self) -> Revision;
    fn start(&self) -> usize;
    fn end(&self) -> usize;
    fn ordinal(&self) -> u32;
    fn preview_top(&self) -> f64;
}

/// Viewport bounds captured after both panes have completed layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollGeometry {
    pub revision: Revision,
    pub document_len: usize,
    pub source_max_top: usize,
    pub preview_max_y: f64,
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    start: usize,
    end: usize,
    ordinal: u32,
    preview_top: f64,
}

/// Immutable, revision-specific binary-search indices for both directions.
#[derive(Clone, Debug)]
pub struct ScrollMap {
    geometry: ScrollGeometry,
    by_source: Vec<Anchor>,
    by_preview: Vec<Anchor>,
    ordinal_zero: Option<Anchor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScrollError {
    #[error("stale scroll revision: expected {expected}, got {actual}")]
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    #[error("invalid source range {start}..{end} for document length {document_len}")]
    InvalidRange {
        start: usize,
        end: usize,
        document_len: usize,
    },
    #[error("scroll position must be finite")]
    NonFinitePosition,
    #[error("scroll geometry is outside the document or viewport bounds")]
    InvalidGeometry,
    #[error("scroll position does not belong to the pane that emitted it")]
    MismatchedPosition,
    #[error("interaction id space is exhausted")]
    InteractionIdOverflow,
}

impl ScrollMap {
    pub fn new<A>(
        geometry: ScrollGeometry,
        anchors: impl IntoIterator<Item = A>,
    ) -> Result<Self, ScrollError>
    where
        A: ScrollAnchorView,
    {
        if !geometry.preview_max_y.is_finite()
            || geometry.preview_max_y < 0.0
            || geometry.source_max_top > geometry.document_len
        {
            return Err(ScrollError::InvalidGeometry);
        }

        let mut by_source = Vec::new();
        for anchor in anchors {
            if anchor.revision() != geometry.revision {
                return Err(ScrollError::StaleRevision {
                    expected: geometry.revision,
                    actual: anchor.revision(),
                });
            }
            let start = anchor.start();
            let end = anchor.end();
            if start > end || end > geometry.document_len {
                return Err(ScrollError::InvalidRange {
                    start,
                    end,
                    document_len: geometry.document_len,
                });
            }
            let preview_top = anchor.preview_top();
            if !preview_top.is_finite() {
                return Err(ScrollError::NonFinitePosition);
            }
            by_source.push(Anchor {
                start,
                end,
                ordinal: anchor.ordinal(),
                preview_top: preview_top.clamp(0.0, geometry.preview_max_y),
            });
        }

        let ordinal_zero = by_source
            .iter()
            .min_by_key(|anchor| anchor.ordinal)
            .copied();
        by_source.sort_by_key(|anchor| (anchor.start, anchor.end, anchor.ordinal));
        let mut by_preview = by_source.clone();
        by_preview.sort_by(|left, right| {
            left.preview_top
                .total_cmp(&right.preview_top)
                .then_with(|| left.ordinal.cmp(&right.ordinal))
                .then_with(|| left.start.cmp(&right.start))
                .then_with(|| left.end.cmp(&right.end))
        });

        Ok(Self {
            geometry,
            by_source,
            by_preview,
            ordinal_zero,
        })
    }

    pub fn revision(&self) -> Revision {
        self.geometry.revision
    }

    pub fn geometry(&self) -> ScrollGeometry {
        self.geometry
    }

    pub fn no_scroll(&self) -> bool {
        self.geometry.source_max_top == 0 || self.geometry.preview_max_y == 0.0
    }

    /// Maps the source viewport's top byte to a preview scroll position.
    pub fn source_to_preview(
        &self,
        revision: Revision,
        source_byte: usize,
    ) -> Result<f64, ScrollError> {
        self.require_revision(revision)?;

        // Normative first mapping branch. In particular this precedes EOF and
        // anchor lookup, so an empty map cannot accidentally synthesize motion.
        if self.no_scroll() {
            return Ok(0.0);
        }
        if source_byte >= self.geometry.source_max_top {
            return Ok(self.geometry.preview_max_y);
        }

        Ok(self
            .source_anchor(source_byte)
            .map_or(0.0, |anchor| anchor.preview_top))
    }

    /// Maps a preview viewport position to a requested source byte.
    pub fn preview_to_source(
        &self,
        revision: Revision,
        preview_y: f64,
    ) -> Result<usize, ScrollError> {
        self.require_revision(revision)?;

        // Normative first mapping branch. NaN is harmless when neither pane can
        // scroll because the only valid command in that state is exact zero.
        if self.no_scroll() {
            return Ok(0);
        }
        if !preview_y.is_finite() {
            return Err(ScrollError::NonFinitePosition);
        }

        let y = preview_y.clamp(0.0, self.geometry.preview_max_y);
        if y >= self.geometry.preview_max_y - PREVIEW_TOLERANCE_PX {
            return Ok(self.geometry.document_len);
        }

        let upper = y + PREVIEW_TOLERANCE_PX;
        let index = self
            .by_preview
            .partition_point(|anchor| anchor.preview_top <= upper);
        Ok(index
            .checked_sub(1)
            .and_then(|index| self.by_preview.get(index))
            .or(self.ordinal_zero.as_ref())
            .map_or(0, |anchor| anchor.start))
    }

    fn source_anchor(&self, source_byte: usize) -> Option<&Anchor> {
        let index = self
            .by_source
            .partition_point(|anchor| anchor.start <= source_byte);
        index
            .checked_sub(1)
            .and_then(|index| self.by_source.get(index))
            .or(self.ordinal_zero.as_ref())
    }

    fn require_revision(&self, actual: Revision) -> Result<(), ScrollError> {
        if actual == self.geometry.revision {
            Ok(())
        } else {
            Err(ScrollError::StaleRevision {
                expected: self.geometry.revision,
                actual,
            })
        }
    }
}

/// The exact 100 source samples from the acceptance oracle.
pub fn source_samples(document_len: usize) -> Vec<usize> {
    if document_len == 0 {
        return vec![0; SAMPLE_COUNT];
    }
    let last = document_len - 1;
    (0..SAMPLE_COUNT)
        .map(|index| ((index as u128 * last as u128) / (SAMPLE_COUNT - 1) as u128) as usize)
        .collect()
}

/// The exact 100 preview samples from the acceptance oracle.
pub fn preview_samples(preview_max_y: f64) -> Vec<f64> {
    if !preview_max_y.is_finite() || preview_max_y <= 0.0 {
        return vec![0.0; SAMPLE_COUNT];
    }
    (0..SAMPLE_COUNT)
        .map(|index| ((index as f64 * preview_max_y) / (SAMPLE_COUNT - 1) as f64).floor())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Source,
    Preview,
}

impl Pane {
    fn other(self) -> Self {
        match self {
            Self::Source => Self::Preview,
            Self::Preview => Self::Source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollPosition {
    SourceByte(usize),
    PreviewY(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollTarget {
    SourceByte(usize),
    PreviewY(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollClock {
    pub monotonic_ms: u64,
    pub preview_frame: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InteractionLease {
    pub interaction_id: InteractionId,
    pub owner: Pane,
    pub target: Pane,
    pub started: ScrollClock,
}

impl InteractionLease {
    pub fn is_active(self, now: ScrollClock) -> bool {
        let elapsed_ms = now.monotonic_ms.saturating_sub(self.started.monotonic_ms);
        let elapsed_frames = now.preview_frame.saturating_sub(self.started.preview_frame);
        elapsed_ms < MAX_LEASE_MS
            && (elapsed_ms < MIN_LEASE_MS || elapsed_frames < MIN_LEASE_FRAMES)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollCommand {
    pub revision: Revision,
    pub interaction_id: InteractionId,
    pub target_pane: Pane,
    pub target: ScrollTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressionReason {
    ProgrammaticEcho,
    DirectionReversal,
    ExpiredInteraction,
    UnknownInteraction,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollOutcome {
    Command(ScrollCommand),
    Suppressed(SuppressionReason),
}

/// Owns the current user interaction and prevents programmatic ping-pong.
#[derive(Clone, Debug)]
pub struct ScrollSynchronizer {
    revision: Revision,
    next_interaction_id: InteractionId,
    lease: Option<InteractionLease>,
}

impl ScrollSynchronizer {
    pub fn new(revision: Revision, first_interaction_id: InteractionId) -> Self {
        Self {
            revision,
            next_interaction_id: first_interaction_id,
            lease: None,
        }
    }

    pub fn next_interaction_id(&self) -> InteractionId {
        self.next_interaction_id
    }

    pub fn lease(&self) -> Option<InteractionLease> {
        self.lease
    }

    pub fn handle_user(
        &mut self,
        map: &ScrollMap,
        pane: Pane,
        position: ScrollPosition,
        clock: ScrollClock,
    ) -> Result<ScrollOutcome, ScrollError> {
        self.handle_user_revision(map, self.revision, pane, position, clock)
    }

    pub fn handle_user_revision(
        &mut self,
        map: &ScrollMap,
        revision: Revision,
        pane: Pane,
        position: ScrollPosition,
        clock: ScrollClock,
    ) -> Result<ScrollOutcome, ScrollError> {
        self.require_revision(revision)?;
        if map.revision() != self.revision {
            return Err(ScrollError::StaleRevision {
                expected: self.revision,
                actual: map.revision(),
            });
        }

        let target = match (pane, position) {
            (Pane::Source, ScrollPosition::SourceByte(byte)) => {
                ScrollTarget::PreviewY(map.source_to_preview(revision, byte)?)
            }
            (Pane::Preview, ScrollPosition::PreviewY(y)) => {
                ScrollTarget::SourceByte(map.preview_to_source(revision, y)?)
            }
            _ => return Err(ScrollError::MismatchedPosition),
        };

        let interaction_id = self.next_interaction_id;
        self.next_interaction_id = InteractionId::new(
            interaction_id
                .get()
                .checked_add(1)
                .ok_or(ScrollError::InteractionIdOverflow)?,
        );
        let target_pane = pane.other();
        self.lease = Some(InteractionLease {
            interaction_id,
            owner: pane,
            target: target_pane,
            started: clock,
        });

        Ok(ScrollOutcome::Command(ScrollCommand {
            revision,
            interaction_id,
            target_pane,
            target,
        }))
    }

    pub fn handle_programmatic(
        &self,
        revision: Revision,
        pane: Pane,
        interaction_id: InteractionId,
        clock: ScrollClock,
    ) -> Result<ScrollOutcome, ScrollError> {
        self.require_revision(revision)?;
        let Some(lease) = self.lease else {
            return Ok(ScrollOutcome::Suppressed(
                SuppressionReason::UnknownInteraction,
            ));
        };
        if lease.interaction_id != interaction_id {
            return Ok(ScrollOutcome::Suppressed(
                SuppressionReason::UnknownInteraction,
            ));
        }
        if !lease.is_active(clock) {
            return Ok(ScrollOutcome::Suppressed(
                SuppressionReason::ExpiredInteraction,
            ));
        }
        if pane == lease.target {
            Ok(ScrollOutcome::Suppressed(
                SuppressionReason::ProgrammaticEcho,
            ))
        } else {
            Ok(ScrollOutcome::Suppressed(
                SuppressionReason::DirectionReversal,
            ))
        }
    }

    fn require_revision(&self, actual: Revision) -> Result<(), ScrollError> {
        if actual == self.revision {
            Ok(())
        } else {
            Err(ScrollError::StaleRevision {
                expected: self.revision,
                actual,
            })
        }
    }
}
