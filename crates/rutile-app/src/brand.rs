pub const PRODUCT_NAME: &str = "Rutile";
pub const ENDORSEMENT: &str = "A local-first writing studio by Kyanite.";
pub const STARTER_DOCUMENT: &str = "# Rutile\n\nStart writing…\n";
pub const SOURCE_EDITOR_LABEL: &str = "Rutile source editor";

#[must_use]
pub fn status_title(status: &str) -> String {
    format!("{PRODUCT_NAME} — {status}")
}

#[cfg(test)]
mod tests {
    use super::{ENDORSEMENT, PRODUCT_NAME, SOURCE_EDITOR_LABEL, STARTER_DOCUMENT, status_title};

    #[test]
    fn approved_brand_copy_is_exact() {
        assert_eq!(PRODUCT_NAME, "Rutile");
        assert_eq!(ENDORSEMENT, "A local-first writing studio by Kyanite.");
        assert_eq!(STARTER_DOCUMENT, "# Rutile\n\nStart writing…\n");
        assert_eq!(SOURCE_EDITOR_LABEL, "Rutile source editor");
    }

    #[test]
    fn status_title_prefixes_status_with_product_name() {
        assert_eq!(status_title("Modified"), "Rutile — Modified");
    }
}
