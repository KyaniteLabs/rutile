use rutile_core::{SafeNode, SourceAttributes, SourceBlockKind};
use rutile_types::Revision;
use rutile_types::SafeLinkTarget;

#[test]
fn typed_nodes_escape_text_and_expose_no_generic_tag_or_attribute_constructor() {
    let node = SafeNode::paragraph(vec![SafeNode::text("<script>&\"")]);
    assert_eq!(node.to_html(), "<p>&lt;script&gt;&amp;&quot;</p>");
}

#[test]
fn typed_links_can_only_contain_the_shared_safe_link_target() {
    let target = SafeLinkTarget::parse("HTTPS://Example.COM/path").unwrap();
    let node = SafeNode::link(target, vec![SafeNode::text("go")]);
    let html = node.to_html();
    assert_eq!(
        html,
        "<a role=\"link\" tabindex=\"0\" data-rutile-url=\"https://example.com/path\">go</a>"
    );
    assert!(!html.contains("href"));
}

#[test]
fn source_attributes_are_generated_and_kind_is_a_fixed_enum() {
    let attrs = SourceAttributes::new(Revision::new(4), 2, 8, 0, SourceBlockKind::Paragraph);
    let node = SafeNode::source_segment(attrs, vec![SafeNode::text("x")], false);
    assert_eq!(
        node.to_html(),
        "<span id=\"sb-4-0\" class=\"source-segment\" data-source-start=\"2\" data-source-end=\"8\" data-source-revision=\"4\" data-source-kind=\"paragraph\">x</span>"
    );
}

#[test]
fn image_alternatives_are_text_only() {
    assert_eq!(
        SafeNode::image_alt("x < y").to_html(),
        "<span class=\"image-alt\">[image: x &lt; y]</span>"
    );
}
