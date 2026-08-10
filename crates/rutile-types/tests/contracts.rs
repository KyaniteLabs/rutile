use rutile_types::{InteractionId, Revision, SafeLinkTarget};

#[test]
fn revision_and_interaction_id_are_canonical_u64_types() {
    let revision: Revision = u64::MAX;
    let interaction: InteractionId = revision;
    assert_eq!(interaction, u64::MAX);
}

#[test]
fn canonicalizes_allowed_absolute_urls() {
    let cases = [
        ("HTTP://Example.COM/a", "http://example.com/a"),
        ("https://example.com/a?b=c#d", "https://example.com/a?b=c#d"),
        ("mailto:person@example.com", "mailto:person@example.com"),
        ("https&colon;//example.com/", "https://example.com/"),
    ];
    for (input, expected) in cases {
        let parsed = SafeLinkTarget::parse(input).expect(input);
        assert_eq!(parsed.as_canonical_str(), expected);
        assert_eq!(parsed.to_string(), expected);
    }
}

#[test]
fn parse_wire_requires_the_exact_canonical_serialization() {
    assert!(SafeLinkTarget::parse_wire("HTTP://example.com/").is_err());
    assert!(SafeLinkTarget::parse_wire("https&colon;//example.com/").is_err());
    assert_eq!(
        SafeLinkTarget::parse_wire("https://example.com/")
            .unwrap()
            .as_canonical_str(),
        "https://example.com/"
    );
}

#[test]
fn rejects_unsafe_or_ambiguous_targets() {
    let invalid = [
        " javascript:alert(1)",
        "javascript:alert(1)",
        "data:text/html,hello",
        "blob:https://example.com/id",
        "file:///etc/passwd",
        "%66ile:///etc/passwd",
        "%2566ile:///etc/passwd",
        "//example.com/path",
        r"\\server\share",
        "https://user@example.com/",
        "https://example.com/a\\b",
        "https://example.com/%zz",
        "https://example.com/\r\nheader:x",
        "https://example.com/\0",
        "https://example.com/ snow",
        "https://example.com/café",
        "<svg onload=alert(1)>",
        "<math href=javascript:alert(1)>",
        "url(https://example.com)",
        "@import https://example.com",
        "onclick=alert(1)",
    ];
    for input in invalid {
        assert!(SafeLinkTarget::parse(input).is_err(), "accepted {input:?}");
    }
}

#[test]
fn rejects_targets_longer_than_768_canonical_bytes() {
    let input = format!("https://example.com/{}", "a".repeat(750));
    assert!(input.len() > 768);
    assert!(SafeLinkTarget::parse(&input).is_err());
}

#[test]
fn parses_bytes_without_accepting_malformed_utf8() {
    assert!(SafeLinkTarget::parse_bytes(&[0xff, 0xfe]).is_err());
}

#[test]
fn rejects_percent_encoded_forbidden_bytes_and_alternative_spellings() {
    let invalid = [
        "https://example.com/%00",
        "https://example.com/%0d",
        "https://example.com/%0A",
        "https://example.com/%09",
        "https://example.com/%20",
        "https://example.com/%5c",
        "https://example.com/%5C",
        "https://example.com/%25%35%63",
        "https://example.com/%41",
        "https://example.com/%c3%a9",
    ];
    for input in invalid {
        assert!(
            SafeLinkTarget::parse(input).is_err(),
            "parse accepted {input}"
        );
        assert!(
            SafeLinkTarget::parse_wire(input).is_err(),
            "parse_wire accepted {input}"
        );
    }
}
