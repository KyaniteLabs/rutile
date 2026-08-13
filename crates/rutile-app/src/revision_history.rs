//! Local revision history contract (roadmap 11).
//!
//! Provides [`RevisionHistory`] for tracking document checkpoints with
//! timestamps and descriptions. See `docs/plan/revision-history-design.md`
//! for the resolved grilling questions.
//!
//! # Relationship to autosave
//!
//! Autosave captures crash-recovery snapshots. Revision history captures
//! user-visible checkpoints for compare/restore. The two are independent:
//! autosave is frequent and automatic; history is user-initiated (or
//! triggered by significant milestones like save).

use std::collections::VecDeque;

use rutile_types::Revision;

/// Maximum number of history entries retained (bounded).
pub const MAX_HISTORY_ENTRIES: usize = 100;

/// A single revision-history checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The document revision at this checkpoint.
    pub revision: Revision,
    /// Unix epoch milliseconds when the checkpoint was recorded.
    pub timestamp_ms: u64,
    /// Human-readable description (e.g. "Saved", "Before format", "Manual").
    pub description: String,
    /// Whether this checkpoint was user-initiated or automatic.
    pub source: HistorySource,
}

/// What triggered a history checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySource {
    /// User explicitly created a checkpoint.
    Manual,
    /// Recorded on save.
    OnSave,
    /// Recorded before a bulk operation (format, replace-all).
    BeforeBulk,
}

/// Bounded revision history for a single document (roadmap 11).
///
/// Tracks checkpoints with timestamps and descriptions. Entries are
/// most-recent-first. The history is bounded by [`MAX_HISTORY_ENTRIES`];
/// oldest entries are evicted when the cap is reached.
#[derive(Debug, Clone)]
pub struct RevisionHistory {
    entries: VecDeque<HistoryEntry>,
}

impl Default for RevisionHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl RevisionHistory {
    /// Creates an empty history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_HISTORY_ENTRIES),
        }
    }

    /// Records a checkpoint at the front of the history.
    /// Evicts the oldest entry if at capacity.
    pub fn record(&mut self, entry: HistoryEntry) {
        if self.entries.len() >= MAX_HISTORY_ENTRIES {
            self.entries.pop_back();
        }
        self.entries.push_front(entry);
    }

    /// Returns all entries, most-recent-first.
    ///
    /// Requires `&mut self` because `VecDeque::make_contiguous` rearranges
    /// the internal buffer to produce a single contiguous slice. After
    /// sustained `push_front` + `pop_back` the head wraps past index 0 and
    /// `as_slices().0` would return only the front segment.
    pub fn entries(&mut self) -> &[HistoryEntry] {
        self.entries.make_contiguous();
        self.entries.as_slices().0
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears all history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns the most recent entry, if any.
    #[must_use]
    pub fn latest(&self) -> Option<&HistoryEntry> {
        self.entries.front()
    }

    /// Finds the entry closest to a given revision (for restore operations).
    #[must_use]
    pub fn find_revision(&self, revision: Revision) -> Option<&HistoryEntry> {
        self.entries.iter().find(|e| e.revision == revision)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_history_is_empty() {
        let h = RevisionHistory::new();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn record_adds_to_front() {
        let mut h = RevisionHistory::new();
        h.record(HistoryEntry {
            revision: Revision::new(1),
            timestamp_ms: 100,
            description: "first".into(),
            source: HistorySource::Manual,
        });
        h.record(HistoryEntry {
            revision: Revision::new(2),
            timestamp_ms: 200,
            description: "second".into(),
            source: HistorySource::OnSave,
        });
        assert_eq!(h.len(), 2);
        assert_eq!(h.latest().unwrap().revision, Revision::new(2));
    }

    #[test]
    fn history_caps_at_max() {
        let mut h = RevisionHistory::new();
        for i in 0..(MAX_HISTORY_ENTRIES + 50) {
            h.record(HistoryEntry {
                revision: Revision::new(i as u64),
                timestamp_ms: i as u64 * 1000,
                description: format!("entry {i}"),
                source: HistorySource::Manual,
            });
        }
        assert_eq!(h.len(), MAX_HISTORY_ENTRIES);
        // Most recent should be the last recorded
        assert_eq!(
            h.latest().unwrap().revision,
            Revision::new((MAX_HISTORY_ENTRIES + 49) as u64)
        );
    }

    #[test]
    fn find_revision_locates_entry() {
        let mut h = RevisionHistory::new();
        h.record(HistoryEntry {
            revision: Revision::new(5),
            timestamp_ms: 500,
            description: "five".into(),
            source: HistorySource::BeforeBulk,
        });
        assert!(h.find_revision(Revision::new(5)).is_some());
        assert!(h.find_revision(Revision::new(99)).is_none());
    }

    #[test]
    fn clear_empties_history() {
        let mut h = RevisionHistory::new();
        h.record(HistoryEntry {
            revision: Revision::new(1),
            timestamp_ms: 100,
            description: "test".into(),
            source: HistorySource::Manual,
        });
        h.clear();
        assert!(h.is_empty());
    }
}
