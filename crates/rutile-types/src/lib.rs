//! Leaf types shared across Rutile production crates.

// S+-tier lint policy: enforce pedantic + nursery, allow the opinionated set.
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

pub mod safe_link;

pub use safe_link::{SafeLinkError, SafeLinkTarget};

/// A document's edit generation (M8 newtype).
///
/// Distinct from [`InteractionId`] and [`DocumentId`]; the newtype wrapper
/// prevents transposed-argument bugs that a bare `u64` alias allows.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Revision(u64);

impl Revision {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A user-interaction sequence id. Distinct from [`Revision`] (a document's
/// edit generation) and [`DocumentId`] (which document).
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct InteractionId(u64);

impl InteractionId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for InteractionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Stable, opaque identifier for an open document within the editor shell.
///
/// Distinct from [`Revision`] (a document's edit generation) and
/// [`InteractionId`] (a user-interaction sequence): a `DocumentId` names *which*
/// document, not its state or an interaction with it. The newtype wrapper keeps
/// it from being silently passed where a `Revision` or `InteractionId` is
/// expected. The single-document baseline uses [`DocumentId::ROOT`] exclusively;
/// multi-document/tabs (roadmap 08) mint fresh ids from a shell-owned counter
/// seeded at 1 so generated ids never collide with [`DocumentId::ROOT`] (L7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentId(u64);

impl DocumentId {
    /// The shell's initial sole document (the single-document baseline).
    pub const ROOT: Self = Self(0);

    /// Wrap a shell-minted counter value. Callers must never hard-code ids.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{DocumentId, InteractionId, Revision};

    #[test]
    fn root_is_stable_zero_and_documents_are_distinct() {
        assert_eq!(DocumentId::ROOT, DocumentId::new(0));
        assert_ne!(DocumentId::ROOT, DocumentId::new(1));
        assert_eq!(DocumentId::new(7).get(), 7);
    }

    #[test]
    fn all_identifier_newtypes_are_type_distinct() {
        // L7/M8: Revision, InteractionId, and DocumentId are all distinct
        // newtypes. None can be silently passed where another is expected.
        fn accepts_revision(_: Revision) {}
        fn accepts_interaction(_: InteractionId) {}
        fn accepts_document(id: DocumentId) -> DocumentId {
            id
        }
        accepts_revision(Revision::new(0));
        accepts_interaction(InteractionId::new(0));
        let _ = accepts_document(DocumentId::ROOT);
        // These would fail to compile if uncommented (type mismatch):
        //   accepts_revision(InteractionId::new(0));
        //   accepts_interaction(DocumentId::ROOT);
        //   accepts_document(Revision::new(0));
    }
}
