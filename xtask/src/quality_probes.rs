//! Headless quality-probe catalog (roadmap 14 / ralplan PR-E).
//!
//! This is **not** readiness attestation. The catalog is a new identity
//! (`QUALITY_PROBE_IDS`) disjoint from [`crate::readiness::PROBE_IDS`]. The
//! harness never signs, never writes `publication_authorized`, and never
//! emits VoiceOver `passed: true`. Without a physical GUI every probe is
//! `unattested` and the bundle's `attested` flag is `false`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::readiness::PROBE_IDS;

/// Schema identifier for the unsigned quality-probe bundle.
///
/// Distinct from `rutile.readiness-attestation.v1` and
/// `rutile.readiness-probe-bundle.v1`.
pub const QUALITY_BUNDLE_SCHEMA: &str = "rutile.quality-probe-bundle.v1";

/// Schema version for the quality-probe bundle.
pub const QUALITY_SCHEMA_VERSION: u64 = 1;

/// Expected probe count. Mirrors readiness' exact-14 contract so the catalogs
/// are the same size but never the same membership.
pub const EXPECTED_QUALITY_PROBE_COUNT: usize = 14;

/// The exactly-14 quality probe identifiers (domains 5–8), in canonical order.
///
/// Membership must stay disjoint from [`PROBE_IDS`]. Authoring source of
/// truth is also listed in `docs/evidence/quality-evidence-gate.md`.
pub const QUALITY_PROBE_IDS: [&str; EXPECTED_QUALITY_PROBE_COUNT] = [
    "quality-idle-rss-budget",
    "quality-idle-cpu-budget",
    "quality-startup-to-interactive",
    "quality-voiceover-window-role",
    "quality-voiceover-editor-text",
    "quality-voiceover-preview-web",
    "quality-voiceover-chrome-controls",
    "quality-keyboard-command-palette",
    "quality-keyboard-view-modes",
    "quality-keyboard-find-replace",
    "quality-keyboard-file-operations",
    "quality-lifecycle-open-save",
    "quality-lifecycle-close-restore",
    "quality-lifecycle-crash-recovery",
];

/// Typed catalog failure. Every variant is fail-closed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("quality probe catalog must contain exactly {expected} ids, got {got}")]
    WrongCount { expected: usize, got: usize },
    #[error("quality probe catalog contains a duplicate id: {0}")]
    Duplicate(String),
    #[error("quality probe catalog contains an empty id")]
    EmptyId,
    #[error("quality probe id {0} collides with readiness PROBE_IDS")]
    CollidesWithReadiness(String),
}

/// One quality probe row. `state` is never `"passed"` and there is no
/// `passed` field — VoiceOver cannot be claimed from a headless run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QualityProbe {
    pub id: String,
    pub state: String,
}

/// Unsigned quality-probe bundle. `attested` is always `false` when this
/// harness emits it (no GUI). The JSON shape deliberately omits
/// `publication_authorized`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QualityProbeBundle {
    pub schema: String,
    pub schema_version: u64,
    pub attested: bool,
    pub probes: Vec<QualityProbe>,
}

/// Validates a candidate catalog. Used for the compiled `QUALITY_PROBE_IDS`
/// and for unit tests that inject malformed lists.
pub fn validate_ids(ids: &[&str]) -> Result<(), CatalogError> {
    if ids.len() != EXPECTED_QUALITY_PROBE_COUNT {
        return Err(CatalogError::WrongCount {
            expected: EXPECTED_QUALITY_PROBE_COUNT,
            got: ids.len(),
        });
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.is_empty() {
            return Err(CatalogError::EmptyId);
        }
        if !seen.insert(*id) {
            return Err(CatalogError::Duplicate((*id).to_owned()));
        }
        if PROBE_IDS.contains(id) {
            return Err(CatalogError::CollidesWithReadiness((*id).to_owned()));
        }
    }
    Ok(())
}

/// Builds the unsigned unattested bundle. Fails only if the compiled catalog
/// is malformed.
pub fn emit_unattested() -> Result<QualityProbeBundle, CatalogError> {
    validate_ids(&QUALITY_PROBE_IDS)?;
    Ok(QualityProbeBundle {
        schema: QUALITY_BUNDLE_SCHEMA.to_owned(),
        schema_version: QUALITY_SCHEMA_VERSION,
        attested: false,
        probes: QUALITY_PROBE_IDS
            .iter()
            .map(|id| QualityProbe {
                id: (*id).to_owned(),
                state: "unattested".to_owned(),
            })
            .collect(),
    })
}

/// Writes the unsigned bundle as pretty JSON. Never signs.
pub fn write_unattested(path: &Path) -> Result<QualityProbeBundle, Box<dyn std::error::Error>> {
    let bundle = emit_unattested()?;
    let json = serde_json::to_string_pretty(&bundle)?;
    fs::write(path, json)?;
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readiness::PROBE_IDS;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_is_fourteen_unique_and_disjoint_from_readiness() {
        assert_eq!(QUALITY_PROBE_IDS.len(), 14);
        validate_ids(&QUALITY_PROBE_IDS).expect("compiled catalog must be well-formed");
        let quality: BTreeSet<_> = QUALITY_PROBE_IDS.iter().copied().collect();
        let readiness: BTreeSet<_> = PROBE_IDS.iter().copied().collect();
        let overlap: Vec<_> = quality.intersection(&readiness).copied().collect();
        assert!(
            overlap.is_empty(),
            "QUALITY_PROBE_IDS ∩ PROBE_IDS must be empty, got {overlap:?}"
        );
        assert_eq!(quality.len(), 14);
    }

    #[test]
    fn emit_is_unsigned_unattested_without_publication_or_voiceover_pass() {
        let bundle = emit_unattested().expect("compiled catalog");
        assert!(!bundle.attested);
        assert_eq!(bundle.schema, QUALITY_BUNDLE_SCHEMA);
        assert_ne!(
            bundle.schema,
            crate::readiness::READINESS_ATTESTATION_SCHEMA
        );
        assert_ne!(bundle.schema, crate::readiness::READINESS_BUNDLE_SCHEMA);
        assert_eq!(bundle.probes.len(), 14);
        for (probe, expected) in bundle.probes.iter().zip(QUALITY_PROBE_IDS) {
            assert_eq!(probe.id, expected);
            assert_eq!(probe.state, "unattested");
            assert_ne!(probe.state, "passed");
        }
        let value = serde_json::to_value(&bundle).expect("serialize");
        assert_eq!(value["attested"], false);
        assert!(value.get("publication_authorized").is_none());
        let dumped = serde_json::to_string(&bundle).expect("json");
        assert!(
            !dumped.contains("passed\":true") && !dumped.contains("\"passed\": true"),
            "VoiceOver passed:true must never appear: {dumped}"
        );
        assert!(!dumped.contains("publication_authorized"));
    }

    #[test]
    fn malformed_catalogs_are_rejected() {
        assert!(matches!(
            validate_ids(&[]),
            Err(CatalogError::WrongCount {
                expected: 14,
                got: 0
            })
        ));
        let mut too_few = QUALITY_PROBE_IDS.to_vec();
        too_few.pop();
        assert!(matches!(
            validate_ids(&too_few),
            Err(CatalogError::WrongCount { got: 13, .. })
        ));
        let mut dup = QUALITY_PROBE_IDS;
        dup[1] = dup[0];
        assert!(matches!(
            validate_ids(&dup),
            Err(CatalogError::Duplicate(_))
        ));
        let mut empty = QUALITY_PROBE_IDS;
        empty[3] = "";
        assert_eq!(validate_ids(&empty), Err(CatalogError::EmptyId));
        let mut collide = QUALITY_PROBE_IDS;
        collide[0] = PROBE_IDS[0];
        assert!(matches!(
            validate_ids(&collide),
            Err(CatalogError::CollidesWithReadiness(_))
        ));
    }
}
