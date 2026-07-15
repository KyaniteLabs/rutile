//! Evidence schema validation — validates JSON instances against checked-in
//! rutile schema files using the `jsonschema` crate (Draft 2020-12).

use std::path::{Path, PathBuf};
use thiserror::Error;

/// All rutile v1 schema kinds known to the evidence system.
pub const KNOWN_SCHEMA_KINDS: &[&str] = &[
    "production-provenance",
    "evidence-index",
    "artifact-inspection",
    "gate-result",
    "release-prerequisite-preflight",
    "w0b-stage0-blocked-receipt",
    "accessibility-attestation",
    "performance-evidence",
];

/// Resolve a schema kind string (e.g. "production-provenance") to the
/// checked-in schema file under `schemas/rutile.<kind>.v1.schema.json`.
pub fn schema_path(kind: &str) -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is a workspace member");
    let filename = format!("rutile.{kind}.v1.schema.json");
    let candidate = root.join("schemas").join(filename);
    candidate.is_file().then_some(candidate)
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("unknown schema kind \"{0}\"; known kinds: {1}")]
    UnknownSchemaKind(String, String),
    #[error("cannot read schema file {path}: {error}")]
    SchemaRead {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("cannot read input file {path}: {error}")]
    InputRead {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("schema file is not valid JSON: {0}")]
    SchemaParse(#[from] serde_json::Error),
    #[error("failed to compile schema: {0}")]
    SchemaCompile(String),
    #[error("validation failed:\n{0}")]
    ValidationFailed(String),
}

/// Validate an input JSON file against the named schema kind.
///
/// Returns `Ok(())` on success, `Err(ValidationError)` on any failure
/// (unknown schema, unreadable file, malformed JSON, or validation errors).
/// The caller should exit non-zero on `Err`.
pub fn validate_kind(input: &Path, kind: &str) -> Result<(), ValidationError> {
    let schema_file = schema_path(kind).ok_or_else(|| {
        ValidationError::UnknownSchemaKind(kind.to_string(), KNOWN_SCHEMA_KINDS.join(", "))
    })?;
    validate_file(input, &schema_file)
}

/// Validate an input JSON file against an explicit schema file path.
pub fn validate_file(input: &Path, schema_file: &Path) -> Result<(), ValidationError> {
    let schema_str =
        std::fs::read_to_string(schema_file).map_err(|e| ValidationError::SchemaRead {
            path: schema_file.to_owned(),
            error: e,
        })?;
    let input_str = std::fs::read_to_string(input).map_err(|e| ValidationError::InputRead {
        path: input.to_owned(),
        error: e,
    })?;

    let schema_value: serde_json::Value = serde_json::from_str(&schema_str)?;
    let input_value: serde_json::Value = serde_json::from_str(&input_str)?;

    let validator = jsonschema::validator_for(&schema_value)
        .map_err(|e| ValidationError::SchemaCompile(e.to_string()))?;

    let errors: Vec<String> = validator
        .iter_errors(&input_value)
        .map(|e| format!("  - {e}"))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::ValidationFailed(errors.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_path_resolves_known_kinds() {
        assert!(schema_path("production-provenance").is_some());
        assert!(schema_path("evidence-index").is_some());
        assert!(schema_path("artifact-inspection").is_some());
        assert!(schema_path("accessibility-attestation").is_some());
        assert!(schema_path("performance-evidence").is_some());
    }

    #[test]
    fn schema_path_returns_none_for_unknown_kind() {
        assert!(schema_path("nonexistent").is_none());
    }

    /// The checked-in Wave 4 accessibility sample must validate against the
    /// accessibility-attestation schema. This guards against schema regressions
    /// that would silently break the release evidence pipeline.
    #[test]
    fn accessibility_attestation_sample_validates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member");
        let sample =
            root.join("release/evidence/samples/rutile.accessibility-attestation.v1.sample.json");
        assert!(
            sample.is_file(),
            "accessibility-attestation sample must be checked in at {}",
            sample.display()
        );
        validate_kind(&sample, "accessibility-attestation").unwrap_or_else(|e| panic!("{e}"));
    }

    /// The checked-in Wave 4 performance sample must validate against the
    /// performance-evidence schema.
    #[test]
    fn performance_evidence_sample_validates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member");
        let sample =
            root.join("release/evidence/samples/rutile.performance-evidence.v1.sample.json");
        assert!(
            sample.is_file(),
            "performance-evidence sample must be checked in at {}",
            sample.display()
        );
        validate_kind(&sample, "performance-evidence").unwrap_or_else(|e| panic!("{e}"));
    }

    /// Host-local absolute paths and secret markers must be rejected by the
    /// accessibility-attestation schema (evidence_ref is repo-relative only).
    #[test]
    fn accessibility_attestation_rejects_host_path_in_evidence_ref() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member");
        let schema_file = root.join("schemas/rutile.accessibility-attestation.v1.schema.json");
        let value = serde_json::json!({
            "schema": "rutile.accessibility-attestation.v1",
            "version": 1,
            "source_commit": "47c23f57274be89b84d119f803612665677f0654",
            "platform": "macos",
            "tool": "voiceover",
            "rows": [{
                "action": "file/open",
                "passed": true,
                "evidence_ref": "/Users/leaker/evidence.wav"
            }],
            "summary": { "passed": 1, "total": 1, "failed": 0 }
        });
        validate_value_against_file(&value, &schema_file)
            .expect_err("host-local path in evidence_ref must be rejected");
    }

    /// budget_ms of zero is meaningless and must be rejected by the
    /// performance-evidence schema.
    #[test]
    fn performance_evidence_rejects_zero_budget() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member");
        let schema_file = root.join("schemas/rutile.performance-evidence.v1.schema.json");
        let value = serde_json::json!({
            "schema": "rutile.performance-evidence.v1",
            "version": 1,
            "source_commit": "47c23f57274be89b84d119f803612665677f0654",
            "platform": "linux",
            "budget_ref": "docs/decisions/0002-release-budgets.md",
            "measurements": [{
                "operation": "preview/render",
                "input_size": 1048576,
                "p50_ms": 3.2,
                "p99_ms": 7.8,
                "budget_ms": 0,
                "passed": true
            }],
            "summary": { "passed": 1, "total": 1, "over_budget": 0 }
        });
        validate_value_against_file(&value, &schema_file)
            .expect_err("budget_ms of zero must be rejected");
    }

    /// Helper: validate an in-memory JSON value against an explicit schema file.
    fn validate_value_against_file(
        value: &serde_json::Value,
        schema_file: &Path,
    ) -> Result<(), ValidationError> {
        let schema_str =
            std::fs::read_to_string(schema_file).map_err(|e| ValidationError::SchemaRead {
                path: schema_file.to_owned(),
                error: e,
            })?;
        let schema_value: serde_json::Value = serde_json::from_str(&schema_str)?;
        let validator = jsonschema::validator_for(&schema_value)
            .map_err(|e| ValidationError::SchemaCompile(e.to_string()))?;
        let errors: Vec<String> = validator
            .iter_errors(value)
            .map(|e| format!("  - {e}"))
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationError::ValidationFailed(errors.join("\n")))
        }
    }

    #[test]
    fn validate_rejects_unknown_schema_kind() {
        let result = validate_kind(Path::new("/dev/null"), "nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown schema kind"));
    }
}
