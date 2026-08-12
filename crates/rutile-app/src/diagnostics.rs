//! Bounded in-memory diagnostics buffer (roadmap 14).
//!
//! A fixed-capacity ring buffer of recent diagnostic events that powers the
//! "Copy Diagnostics" path: the user copies a redacted, privacy-safe text dump
//! to paste into a bug report. The buffer is **in-memory only** — it is never
//! persisted to disk and never transmitted anywhere. Copy is strictly
//! user-initiated.
//!
//! # Privacy
//!
//! [`copy_diagnostics`] redacts absolute paths to basenames and never includes
//! document text — only severity, category, a trimmed message, and a coarse
//! timestamp. A user can safely paste the dump anywhere.
//!
//! # Security-core fence
//!
//! The buffer stores only diagnostic metadata the app itself produces. It never
//! reads files, never interprets URLs, and never constructs HTML.

/// Maximum entries retained in the ring buffer.
pub const DIAGNOSTICS_CAPACITY: usize = 256;
/// Maximum bytes of a single message before it is truncated.
pub const MAX_MESSAGE_BYTES: usize = 512;

/// Severity of a diagnostic entry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Info,
    #[default]
    Warning,
    Error,
}

impl DiagnosticSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Coarse category for grouping diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiagnosticCategory {
    Render,
    Autosave,
    Session,
    Export,
    #[default]
    General,
}

impl DiagnosticCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Autosave => "autosave",
            Self::Session => "session",
            Self::Export => "export",
            Self::General => "general",
        }
    }
}

/// A single diagnostic entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticEntry {
    pub timestamp_ms: u64,
    pub severity: DiagnosticSeverity,
    pub category: DiagnosticCategory,
    pub message: String,
}

/// Bounded ring buffer of recent diagnostic events.
#[derive(Debug, Clone)]
pub struct DiagnosticsBuffer {
    entries: Vec<DiagnosticEntry>,
    capacity: usize,
}

impl Default for DiagnosticsBuffer {
    fn default() -> Self {
        Self::new(DIAGNOSTICS_CAPACITY)
    }
}

impl DiagnosticsBuffer {
    /// Creates an empty buffer bounded by `capacity` (clamped to
    /// `DIAGNOSTICS_CAPACITY` so a hostile caller can't balloon memory).
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity.min(DIAGNOSTICS_CAPACITY)),
            capacity: capacity.min(DIAGNOSTICS_CAPACITY),
        }
    }

    /// Appends an entry, truncating its message to [`MAX_MESSAGE_BYTES`] and
    /// evicting the oldest when at capacity.
    pub fn record(&mut self, mut entry: DiagnosticEntry) {
        truncate_in_place(&mut entry.message, MAX_MESSAGE_BYTES);
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    /// Convenience: records a trimmed entry.
    pub fn log(
        &mut self,
        timestamp_ms: u64,
        severity: DiagnosticSeverity,
        category: DiagnosticCategory,
        message: impl Into<String>,
    ) {
        self.record(DiagnosticEntry {
            timestamp_ms,
            severity,
            category,
            message: message.into(),
        });
    }

    /// Number of entries currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries in insertion order (oldest first).
    pub fn entries(&self) -> &[DiagnosticEntry] {
        &self.entries
    }

    /// Clears the buffer.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Produces a redacted, privacy-safe text dump for "Copy Diagnostics".
    ///
    /// Absolute paths are reduced to basenames; no document text is included.
    pub fn copy_diagnostics(&self) -> String {
        let mut out = String::from("# Rutile diagnostics (redacted)\n\n");
        for e in &self.entries {
            out.push_str(&format!(
                "[{}] {} {}: {}\n",
                e.timestamp_ms,
                e.severity.as_str(),
                e.category.as_str(),
                redact_paths(&e.message),
            ));
        }
        out
    }
}

/// Truncates `s` to at most `max_bytes` on a UTF-8 boundary, appending "…".
fn truncate_in_place(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push('…');
}

/// Replaces absolute path-like substrings (`/foo/bar/baz`) with their basename.
fn redact_paths(message: &str) -> String {
    // Match runs of /segment that look like paths (at least one slash + a leaf).
    let mut out = String::with_capacity(message.len());
    let bytes = message.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            // Scan forward to the end of a /-separated run of non-space chars.
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let segment = &message[start..j];
            // Keep only the basename (last path component).
            if let Some(pos) = segment.rfind('/') {
                out.push_str(&segment[pos + 1..]);
            } else {
                out.push_str(segment);
            }
            i = j;
        } else {
            // Copy through the next non-space run verbatim.
            let start = i;
            while i < bytes.len() && bytes[i] != b'/' {
                // Stop at whitespace so the next /-run is detected.
                if bytes[i].is_ascii_whitespace() {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push_str(&message[start..i]);
        }
    }
    out
}

#[cfg(test)]
mod diagnostics_tests {
    use super::*;

    #[test]
    fn empty_buffer_is_empty() {
        let buf = DiagnosticsBuffer::default();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert_eq!(
            buf.copy_diagnostics(),
            "# Rutile diagnostics (redacted)\n\n"
        );
    }

    #[test]
    fn record_appends_in_order() {
        let mut buf = DiagnosticsBuffer::default();
        buf.log(
            10,
            DiagnosticSeverity::Info,
            DiagnosticCategory::Render,
            "painted",
        );
        buf.log(
            20,
            DiagnosticSeverity::Error,
            DiagnosticCategory::Autosave,
            "flush failed",
        );
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.entries()[0].message, "painted");
        assert_eq!(buf.entries()[1].message, "flush failed");
    }

    #[test]
    fn ring_buffer_evicts_oldest_at_capacity() {
        let mut buf = DiagnosticsBuffer::new(3);
        for i in 0..5 {
            buf.log(
                i,
                DiagnosticSeverity::Info,
                DiagnosticCategory::General,
                format!("e{i}"),
            );
        }
        assert_eq!(buf.len(), 3);
        // Oldest two evicted; entries are e2, e3, e4.
        assert_eq!(buf.entries()[0].message, "e2");
        assert_eq!(buf.entries()[2].message, "e4");
    }

    #[test]
    fn capacity_clamps_to_max() {
        let buf = DiagnosticsBuffer::new(99_999);
        // Clamped to DIAGNOSTICS_CAPACITY, so a huge capacity can't balloon memory.
        assert_eq!(buf.capacity, DIAGNOSTICS_CAPACITY);
    }

    #[test]
    fn long_message_is_truncated() {
        let mut buf = DiagnosticsBuffer::default();
        let huge = "x".repeat(MAX_MESSAGE_BYTES + 100);
        buf.log(
            0,
            DiagnosticSeverity::Error,
            DiagnosticCategory::General,
            huge,
        );
        let msg = &buf.entries()[0].message;
        assert!(msg.len() <= MAX_MESSAGE_BYTES + 4); // + ellipsis slack
        assert!(msg.ends_with('…'));
    }

    #[test]
    fn copy_diagnostics_redacts_absolute_paths() {
        let mut buf = DiagnosticsBuffer::default();
        buf.log(
            0,
            DiagnosticSeverity::Error,
            DiagnosticCategory::Autosave,
            "failed to write /Users/simon/secret/notes.md",
        );
        let dump = buf.copy_diagnostics();
        assert!(!dump.contains("/Users/"));
        assert!(!dump.contains("secret/"));
        // Basename preserved so the report is still useful.
        assert!(dump.contains("notes.md"));
    }

    #[test]
    fn copy_diagnostics_includes_severity_and_category() {
        let mut buf = DiagnosticsBuffer::default();
        buf.log(
            0,
            DiagnosticSeverity::Warning,
            DiagnosticCategory::Render,
            "over budget",
        );
        let dump = buf.copy_diagnostics();
        assert!(dump.contains("WARN"));
        assert!(dump.contains("render"));
        assert!(dump.contains("over budget"));
    }

    #[test]
    fn clear_empties() {
        let mut buf = DiagnosticsBuffer::default();
        buf.log(
            0,
            DiagnosticSeverity::Info,
            DiagnosticCategory::General,
            "x",
        );
        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn redact_paths_preserves_non_path_text() {
        assert_eq!(redact_paths("no paths here"), "no paths here");
        assert_eq!(
            redact_paths("see report.txt for details"),
            "see report.txt for details"
        );
    }

    #[test]
    fn redact_paths_reduces_nested_paths_to_basename() {
        assert_eq!(redact_paths("/a/b/c.md"), "c.md");
        assert_eq!(
            redact_paths("err at /home/u/docs/draft.md done"),
            "err at draft.md done"
        );
    }
}
