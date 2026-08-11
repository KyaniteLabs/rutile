//! Leaf types shared across Rutile production crates.

pub mod safe_link;

pub use safe_link::{SafeLinkError, SafeLinkTarget};

pub type Revision = u64;
pub type InteractionId = u64;
/// Stable, opaque identifier for an open document within the editor shell.
///
/// Distinct from [`Revision`] (a document's edit generation) and
/// [`InteractionId`] (a user-interaction sequence): a `DocumentId` names *which*
/// document, not its state or an interaction with it. The newtype wrapper keeps
/// it from being silently passed where a `Revision` or `InteractionId` is
/// expected. The single-document baseline uses [`DocumentId::ROOT`] exclusively;
/// multi-document/tabs (roadmap 08) mint fresh ids from a shell-owned counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentId(u64);

impl DocumentId {
    /// The shell's initial sole document (the single-document baseline).
    pub const ROOT: DocumentId = DocumentId(0);

    /// Wrap a shell-minted counter value. Callers must never hard-code ids.
    #[must_use]
    pub const fn new(value: u64) -> DocumentId {
        DocumentId(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::DocumentId;

    #[test]
    fn root_is_stable_zero_and_documents_are_distinct() {
        assert_eq!(DocumentId::ROOT, DocumentId::new(0));
        assert_ne!(DocumentId::ROOT, DocumentId::new(1));
        assert_eq!(DocumentId::new(7).get(), 7);
    }

    #[test]
    fn document_id_does_not_equate_to_revision_or_interaction_aliases() {
        // DocumentId is a newtype, not a u64 alias, so it cannot be silently
        // mixed with Revision/InteractionId at a type-check level.
        fn _accepts_document(id: DocumentId) -> DocumentId {
            id
        }
        let _ = _accepts_document(DocumentId::ROOT);
        // These would fail to compile if uncommented (type mismatch):
        //   let _: DocumentId = 0u64;          // Revision/InteractionId are u64
        //   _accepts_document(0u64);
    }
}
