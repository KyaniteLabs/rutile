#![no_main]

use std::collections::HashSet;

use rutile_core::{
    MAX_GENERATED_BODY_BYTES, MAX_RENDERED_PAGE_BYTES, render_markdown, validate_source_blocks,
};
use rutile_types::SafeLinkTarget;
use libfuzzer_sys::fuzz_target;

const CSS: &str = "rutile://preview/v1/assets/preview.css";
const BRIDGE: &str = "rutile://preview/v1/assets/bridge.js";
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";

fuzz_target!(|input: &[u8]| {
    let Ok(source) = std::str::from_utf8(input) else {
        return;
    };
    let Ok(rendered) = render_markdown(source, 17) else {
        return;
    };
    assert!(rendered.body.len() <= MAX_GENERATED_BODY_BYTES);
    assert!(rendered.page.len() <= MAX_RENDERED_PAGE_BYTES);
    validate_source_blocks(source, 17, &rendered.blocks).unwrap();
    inspect_html(&rendered.body, false);
    inspect_html(&rendered.page, true);
});

fn inspect_html(html: &str, complete_page: bool) {
    let mut cursor = 0;
    let mut stack: Vec<&str> = Vec::new();
    let mut css = 0;
    let mut bridge = 0;
    let mut csp = 0;
    while let Some(relative) = html[cursor..].find('<') {
        let start = cursor + relative;
        let end = html[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap();
        let token = &html[start + 1..end];
        cursor = end + 1;
        if token.eq_ignore_ascii_case("!doctype html") {
            assert!(complete_page);
            continue;
        }
        if let Some(name) = token.strip_prefix('/') {
            assert!(!name.is_empty() && name.bytes().all(valid_name_byte));
            assert_eq!(stack.pop(), Some(name));
            continue;
        }

        let name_end = token
            .find(|character: char| character.is_ascii_whitespace())
            .unwrap_or(token.len());
        let name = &token[..name_end];
        assert!(allowed_tag(name, complete_page), "unexpected tag {name}");
        let attributes = parse_attributes(&token[name_end..]);
        validate_attributes(
            name,
            &attributes,
            complete_page,
            &mut css,
            &mut bridge,
            &mut csp,
        );
        if !matches!(name, "hr" | "br" | "meta" | "link") {
            stack.push(name);
        }
    }
    assert!(stack.is_empty());
    if complete_page {
        assert_eq!((css, bridge, csp), (1, 1, 1));
    } else {
        assert_eq!((css, bridge, csp), (0, 0, 0));
    }
}

fn parse_attributes(input: &str) -> Vec<(&str, Option<&str>)> {
    let bytes = input.as_bytes();
    let mut cursor = 0;
    let mut attributes = Vec::new();
    let mut names = HashSet::new();
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        let name_start = cursor;
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'='
        {
            assert!(valid_name_byte(bytes[cursor]));
            cursor += 1;
        }
        let name = &input[name_start..cursor];
        assert!(!name.is_empty() && names.insert(name));
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value = if cursor < bytes.len() && bytes[cursor] == b'=' {
            cursor += 1;
            assert_eq!(bytes.get(cursor), Some(&b'"'));
            cursor += 1;
            let value_start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'"' {
                cursor += 1;
            }
            assert!(cursor < bytes.len());
            let value = &input[value_start..cursor];
            cursor += 1;
            Some(value)
        } else {
            None
        };
        attributes.push((name, value));
    }
    attributes
}

fn validate_attributes(
    tag: &str,
    attributes: &[(&str, Option<&str>)],
    complete_page: bool,
    css: &mut usize,
    bridge: &mut usize,
    csp: &mut usize,
) {
    for (name, value) in attributes {
        match (tag, *name, *value) {
            ("html", "data-rutile-revision", Some("17")) if complete_page => {}
            ("meta", "charset", Some("utf-8")) if complete_page => {}
            ("meta", "http-equiv", Some("Content-Security-Policy")) if complete_page => {}
            ("meta", "content", Some(value)) if complete_page && value == CSP => *csp += 1,
            ("link", "rel", Some("stylesheet")) if complete_page => {}
            ("link", "href", Some(value)) if complete_page && value == CSS => *css += 1,
            ("script", "defer", None) if complete_page => {}
            ("script", "src", Some(value)) if complete_page && value == BRIDGE => *bridge += 1,
            ("ol", "start", Some(value)) => assert!(positive_decimal(value)),
            ("code", "class", Some(value)) => assert!(valid_language_class(value)),
            ("th" | "td", "class", Some("align-left" | "align-center" | "align-right")) => {}
            ("span", "class", Some(value)) => assert!(matches!(
                value,
                "source-segment" | "source-segment code-segment" | "image-alt" | "task-checkbox"
            )),
            ("div", "class", Some("source-fallback")) => {}
            ("span" | "div" | "hr", "id", Some(value)) => assert!(valid_source_id(value)),
            ("span" | "div" | "hr", "data-source-start" | "data-source-end", Some(value)) => {
                assert!(decimal(value))
            }
            ("span" | "div" | "hr", "data-source-revision", Some("17")) => {}
            ("span" | "div" | "hr", "data-source-kind", Some(value)) => assert!(matches!(
                value,
                "heading"
                    | "paragraph"
                    | "code-block"
                    | "table-row"
                    | "footnote-definition"
                    | "thematic-break"
                    | "leaf-fallback"
                    | "continuation"
            )),
            ("span", "role", Some("checkbox")) => {}
            ("span", "aria-checked", Some("true" | "false")) => {}
            ("a", "role", Some("link")) => {}
            ("a", "tabindex", Some("0")) => {}
            ("a", "data-rutile-url", Some(value)) => {
                let decoded = html_escape::decode_html_entities(value);
                SafeLinkTarget::parse_wire(decoded.as_ref()).unwrap();
            }
            _ => panic!("unexpected attribute {name:?}={value:?} on {tag}"),
        }
    }

    match tag {
        "html" => require_exact_names(attributes, &["data-rutile-revision"]),
        "meta" if attributes.iter().any(|(name, _)| *name == "charset") => {
            require_exact_names(attributes, &["charset"])
        }
        "meta" => require_exact_names(attributes, &["content", "http-equiv"]),
        "link" => require_exact_names(attributes, &["href", "rel"]),
        "script" => require_exact_names(attributes, &["defer", "src"]),
        "a" => require_exact_names(attributes, &["data-rutile-url", "role", "tabindex"]),
        _ => {}
    }
}

fn require_exact_names(attributes: &[(&str, Option<&str>)], expected: &[&str]) {
    let actual: HashSet<_> = attributes.iter().map(|(name, _)| *name).collect();
    let expected: HashSet<_> = expected.iter().copied().collect();
    assert_eq!(actual, expected);
}

fn allowed_tag(tag: &str, complete_page: bool) -> bool {
    matches!(
        tag,
        "main"
            | "section"
            | "div"
            | "p"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "code"
            | "em"
            | "strong"
            | "del"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "th"
            | "td"
            | "hr"
            | "br"
            | "sup"
            | "a"
            | "span"
    ) || (complete_page && matches!(tag, "html" | "head" | "meta" | "link" | "script" | "body"))
}

fn valid_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-')
}

fn decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn positive_decimal(value: &str) -> bool {
    decimal(value) && value.parse::<u64>().is_ok_and(|value| value > 0)
}

fn valid_source_id(value: &str) -> bool {
    let mut parts = value.split('-');
    matches!(parts.next(), Some("sb"))
        && matches!(parts.next(), Some("17"))
        && parts.next().is_some_and(decimal)
        && parts.next().is_none()
}

fn valid_language_class(value: &str) -> bool {
    let Some(language) = value.strip_prefix("language-") else {
        return false;
    };
    !language.is_empty()
        && language.len() <= 32
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
