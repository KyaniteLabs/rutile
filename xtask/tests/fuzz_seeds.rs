use std::fs;

use feathermark_protocol::{PreviewEventV1, decode_preview_event};

#[test]
fn committed_preview_seed_set_reaches_all_four_success_variants() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview_event");
    for (name, expected) in [
        ("bridge_ready_rev0", "bridge_ready"),
        ("painted_rev1", "painted"),
        ("scroll_rev2", "scroll"),
        ("link_https_rev3", "link_activated"),
    ] {
        let bytes = fs::read(root.join(name)).unwrap();
        let prefix: [u8; 8] = bytes[..8].try_into().unwrap();
        let revision = u64::from_le_bytes(prefix);
        let actual = match decode_preview_event(&bytes[8..], revision).unwrap() {
            PreviewEventV1::BridgeReady { revision: event } => {
                assert_eq!(event, revision);
                "bridge_ready"
            }
            PreviewEventV1::Painted {
                revision: event, ..
            } => {
                assert_eq!(event, revision);
                "painted"
            }
            PreviewEventV1::Scroll {
                revision: event, ..
            } => {
                assert_eq!(event, revision);
                "scroll"
            }
            PreviewEventV1::LinkActivated {
                revision: event, ..
            } => {
                assert_eq!(event, revision);
                "link_activated"
            }
        };
        assert_eq!(actual, expected);
    }
}
