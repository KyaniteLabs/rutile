//! Leaf types shared across FeatherMark production crates.

pub mod safe_link;

pub use safe_link::{SafeLinkError, SafeLinkTarget};

pub type Revision = u64;
pub type InteractionId = u64;
