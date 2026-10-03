//! Editable find-bar state belongs to a document, independent of its shell.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindField {
    Query,
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindBarView {
    pub query: String,
    pub replacement: String,
    pub replace_enabled: bool,
    pub focus: FindField,
    pub status: String,
}
