#![no_main]

use feathermark_protocol::{MAX_DOCUMENT_BYTES, PreviewEventV1, decode_preview_event};
use feathermark_types::SafeLinkTarget;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(prefix) = data.get(..8).and_then(|bytes| bytes.try_into().ok()) else {
        return;
    };
    let loaded_revision = u64::from_le_bytes(prefix);
    if let Ok(event) = decode_preview_event(&data[8..], loaded_revision) {
        let revision = match &event {
            PreviewEventV1::BridgeReady { revision }
            | PreviewEventV1::Painted { revision, .. }
            | PreviewEventV1::Scroll { revision, .. }
            | PreviewEventV1::LinkActivated { revision, .. } => *revision,
        };
        assert_eq!(revision, loaded_revision);
        if let PreviewEventV1::Scroll { source_start, .. } = event {
            assert!(source_start <= MAX_DOCUMENT_BYTES);
        } else if let PreviewEventV1::LinkActivated { target, .. } = event {
            assert_eq!(
                SafeLinkTarget::parse_wire(target.as_canonical_str()).unwrap(),
                target
            );
        }
    }
});
