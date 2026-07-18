//! Evidence schema validation — validates JSON instances against checked-in
//! rutile schema files using the `jsonschema` crate (Draft 2020-12).

use crate::tool_process;
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
    "preview-publication-authorization",
    "readiness-probe-bundle",
    "readiness-attestation",
];

/// Rutile evidence kinds that carry a `source: { commit, tree }` binding which
/// must match the current repository HEAD before the record can be trusted.
/// Used by [`validate_readiness_with_source`]; generic schema validation
/// ([`validate_kind`]) stays schema-only for every kind, including these.
const READINESS_SOURCE_BOUND_KINDS: &[&str] = &["readiness-probe-bundle", "readiness-attestation"];

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
    #[error("readiness source binding is only supported for kinds {expected}; got \"{kind}\"")]
    ReadinessKindNotSourceBound {
        kind: String,
        expected: &'static str,
    },
    #[error("readiness source binding cannot read repository HEAD at {repo}: {error}")]
    ReadinessSourceIo {
        repo: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("readiness source binding rejected: {0}")]
    ReadinessSourceInvalid(String),
    #[error("readiness source commit/tree do not match the current repository HEAD")]
    ReadinessSourceMismatch,
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

/// Validate an input JSON file against a readiness schema kind AND verify that
/// its embedded `source: { commit, tree }` matches the current repository HEAD.
///
/// This layers a source cross-check on top of generic schema validation
/// ([`validate_kind`]); `validate_kind` itself stays schema-only for every
/// kind, including the readiness kinds, so unrelated callers are unaffected.
/// Only `readiness-probe-bundle` and `readiness-attestation` are accepted
/// here — other kinds fail closed with [`ValidationError::ReadinessKindNotSourceBound`].
///
/// The current repository HEAD is derived via [`tool_process::git_isolated`],
/// mirroring the audited `release_preflight` source-binding path: the
/// `GIT_DIR`/`GIT_WORK_TREE`/global-config environment is stripped so caller
/// state cannot redirect the binding. This function never shells out directly.
///
/// Final signature verification of a readiness attestation is owned by
/// `readiness.rs`, not by this generic evidence lane.
pub fn validate_readiness_with_source(input: &Path, kind: &str) -> Result<(), ValidationError> {
    if !READINESS_SOURCE_BOUND_KINDS.contains(&kind) {
        return Err(ValidationError::ReadinessKindNotSourceBound {
            kind: kind.to_string(),
            expected: "readiness-probe-bundle, readiness-attestation",
        });
    }
    // Step 1: schema validation — fail closed on any schema error first so a
    // malformed record can never reach the source cross-check.
    validate_kind(input, kind)?;
    // Step 2: read the recorded source commit/tree. The schema guarantees the
    // `^[0-9a-f]{40}$` shape, but we re-check defensively so this binding stays
    // correct independent of future schema drift.
    let input_str = std::fs::read_to_string(input).map_err(|e| ValidationError::InputRead {
        path: input.to_owned(),
        error: e,
    })?;
    let value: serde_json::Value = serde_json::from_str(&input_str)?;
    let source = parse_readiness_source(&value)?;
    // Step 3: cross-check against the current repository HEAD.
    verify_current_source(&source)
}

/// Typed source binding extracted from a readiness record's `source` object.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadinessSource {
    commit: String,
    tree: String,
}

fn parse_readiness_source(value: &serde_json::Value) -> Result<ReadinessSource, ValidationError> {
    let Some(source) = value.get("source") else {
        return Err(ValidationError::ReadinessSourceInvalid(
            "missing /source object".into(),
        ));
    };
    let commit = source
        .get("commit")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::ReadinessSourceInvalid("missing /source/commit".into()))?;
    let tree = source
        .get("tree")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidationError::ReadinessSourceInvalid("missing /source/tree".into()))?;
    if !is_lowercase_sha40(commit) {
        return Err(ValidationError::ReadinessSourceInvalid(
            "/source/commit must be a 40-char lowercase hex SHA".into(),
        ));
    }
    if !is_lowercase_sha40(tree) {
        return Err(ValidationError::ReadinessSourceInvalid(
            "/source/tree must be a 40-char lowercase hex SHA".into(),
        ));
    }
    Ok(ReadinessSource {
        commit: commit.to_string(),
        tree: tree.to_string(),
    })
}

fn is_lowercase_sha40(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Pinned repository root derived at compile time from the `xtask` crate
/// location, mirroring `release_preflight::workspace_root`. Source binding must
/// never follow the runtime working directory or inherited Git environment.
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is a workspace member")
}

fn verify_current_source(source: &ReadinessSource) -> Result<(), ValidationError> {
    let repo = workspace_root();
    let output = tool_process::git_isolated(
        repo,
        &["--no-replace-objects", "rev-parse", "HEAD", "HEAD^{tree}"],
        &[],
    )
    .map_err(|error| ValidationError::ReadinessSourceIo {
        repo: repo.to_path_buf(),
        error,
    })?;
    if !output.status.success() || output.stdout.len() > 256 {
        return Err(ValidationError::ReadinessSourceInvalid(
            "cannot derive current repository source (non-success exit or oversize output)".into(),
        ));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        ValidationError::ReadinessSourceInvalid("git source output is not UTF-8".into())
    })?;
    let mut lines = text.lines();
    let head_matches = lines.next() == Some(source.commit.as_str());
    let tree_matches = lines.next() == Some(source.tree.as_str());
    let no_extra_line = lines.next().is_none();
    if !(head_matches && tree_matches && no_extra_line) {
        return Err(ValidationError::ReadinessSourceMismatch);
    }
    Ok(())
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

    // ---- G001 readiness-probe-bundle / readiness-attestation tests ----
    //
    // These cover the four acceptance cases: both schemas resolve, valid
    // in-test samples validate (schema + source binding), host-local/secret
    // evidence refs fail schema validation, and a current-source mismatch
    // fails the readiness source cross-check. Signature verification itself
    // is owned by readiness.rs and is intentionally out of scope here.

    // Single source of truth: import the readiness contract constants from
    // the production readiness module rather than duplicating them. Any drift
    // between these values and the checked-in schemas is caught by the
    // code→schema round-trip tests in readiness.rs.
    use crate::readiness::{PROBE_IDS, READINESS_DISCLAIMER, READINESS_DOMAIN_STR};

    /// Mirror the release_preflight test sandbox: keep temp files under the
    /// workspace `target/tmp` so they cannot leak into the source tree.
    fn readiness_temp_root() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate lives in the workspace")
            .join("target")
            .join("tmp");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(&root).unwrap()
    }

    fn write_readiness_json(
        dir: &tempfile::TempDir,
        name: &str,
        value: &serde_json::Value,
    ) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        path
    }

    /// Resolve the current workspace HEAD/tree via the same isolated git path
    /// the production source-binding check uses, so the positive tests stay
    /// self-consistent with `validate_readiness_with_source`.
    fn current_workspace_source() -> (String, String) {
        let output = tool_process::git_isolated(
            workspace_root(),
            &["--no-replace-objects", "rev-parse", "HEAD", "HEAD^{tree}"],
            &[],
        )
        .expect("isolated git rev-parse must succeed in the workspace");
        assert!(
            output.status.success(),
            "git rev-parse HEAD HEAD^{{tree}} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut lines = std::str::from_utf8(&output.stdout)
            .expect("git source output is UTF-8")
            .lines();
        let commit = lines.next().expect("HEAD sha").to_string();
        let tree = lines.next().expect("tree sha").to_string();
        (commit, tree)
    }

    fn hex_fill(ch: char, len: usize) -> String {
        ch.to_string().repeat(len)
    }

    fn readiness_probes(observed_at: u64) -> Vec<serde_json::Value> {
        PROBE_IDS
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "state": "attested",
                    "observed_at_unix_ms": observed_at,
                    "evidence_ref": format!("release/evidence/readiness/{id}.json"),
                    "evidence_sha256": hex_fill('a', 64),
                })
            })
            .collect()
    }

    fn readiness_bundle_value(commit: &str, tree: &str) -> serde_json::Value {
        let observed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|v| v.as_millis() as u64)
            .unwrap_or(0);
        serde_json::json!({
            "schema": "rutile.readiness-probe-bundle.v1",
            "version": 1,
            "generated_at_unix_ms": observed_at,
            "source": { "commit": commit, "tree": tree },
            "runner_lock_ref": "release/evidence/readiness/runner-lock.json",
            "runner_lock_sha256": hex_fill('b', 64),
            "probes": readiness_probes(observed_at),
            "actionable_blockers": [],
        })
    }

    /// Build a valid attestation by layering verifier/authority/ready/disclaimer
    /// on top of the bundle shape and swapping the `schema` const.
    fn readiness_attestation_value(commit: &str, tree: &str) -> serde_json::Value {
        let observed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|v| v.as_millis() as u64)
            .unwrap_or(0);
        let expires_at = observed_at + 365 * 24 * 60 * 60 * 1000u64;
        let mut value = readiness_bundle_value(commit, tree);
        value["schema"] = serde_json::json!("rutile.readiness-attestation.v1");
        value["verifier"] = serde_json::json!({
            "identity": "independent-readiness-verifier",
            "key_fingerprint": hex_fill('c', 64),
            "signing_public_key_hex": hex_fill('d', 64),
            "independence_evidence_ref": "release/evidence/readiness/independence.json",
        });
        value["authority"] = serde_json::json!({
            "domain": READINESS_DOMAIN_STR,
            "canonical_message_sha256": hex_fill('e', 64),
            "signature_hex": hex_fill('f', 128),
            "signed_at_unix_ms": observed_at,
            "expires_at_unix_ms": expires_at,
        });
        value["ready"] = serde_json::json!(true);
        value["disclaimer"] = serde_json::json!(READINESS_DISCLAIMER);
        value
    }

    #[test]
    fn readiness_schema_kinds_resolve() {
        assert!(schema_path("readiness-probe-bundle").is_some());
        assert!(schema_path("readiness-attestation").is_some());
        assert!(
            KNOWN_SCHEMA_KINDS.contains(&"readiness-probe-bundle"),
            "readiness-probe-bundle must be registered"
        );
        assert!(
            KNOWN_SCHEMA_KINDS.contains(&"readiness-attestation"),
            "readiness-attestation must be registered"
        );
    }

    #[test]
    fn readiness_probe_bundle_sample_validates_schema_and_source() {
        let (commit, tree) = current_workspace_source();
        let bundle = readiness_bundle_value(&commit, &tree);
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "bundle.json", &bundle);
        validate_kind(&input, "readiness-probe-bundle")
            .unwrap_or_else(|e| panic!("bundle must schema-validate: {e}"));
        validate_readiness_with_source(&input, "readiness-probe-bundle")
            .unwrap_or_else(|e| panic!("bundle must bind to current source: {e}"));
    }

    #[test]
    fn readiness_attestation_sample_validates_schema_and_source() {
        let (commit, tree) = current_workspace_source();
        let attestation = readiness_attestation_value(&commit, &tree);
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "attestation.json", &attestation);
        validate_kind(&input, "readiness-attestation")
            .unwrap_or_else(|e| panic!("attestation must schema-validate: {e}"));
        validate_readiness_with_source(&input, "readiness-attestation")
            .unwrap_or_else(|e| panic!("attestation must bind to current source: {e}"));
    }

    #[test]
    fn readiness_bundle_rejects_host_local_evidence_ref() {
        let (commit, tree) = current_workspace_source();
        let mut bundle = readiness_bundle_value(&commit, &tree);
        bundle["probes"][0]["evidence_ref"] = serde_json::json!("/Users/leaker/evidence.bin");
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "bundle-host.json", &bundle);
        let err = validate_kind(&input, "readiness-probe-bundle")
            .expect_err("host-local evidence_ref must be rejected by the schema");
        assert!(
            matches!(err, ValidationError::ValidationFailed(_)),
            "expected schema ValidationFailed, got: {err}"
        );
    }

    #[test]
    fn readiness_bundle_rejects_secret_evidence_ref() {
        let (commit, tree) = current_workspace_source();
        let mut bundle = readiness_bundle_value(&commit, &tree);
        bundle["probes"][1]["evidence_ref"] = serde_json::json!("release/secrets/leaked.key");
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "bundle-secret.json", &bundle);
        let err = validate_kind(&input, "readiness-probe-bundle")
            .expect_err("secret-bearing evidence_ref must be rejected by the schema");
        assert!(
            matches!(err, ValidationError::ValidationFailed(_)),
            "expected schema ValidationFailed, got: {err}"
        );
    }

    #[test]
    fn readiness_attestation_rejects_host_local_independence_ref() {
        let (commit, tree) = current_workspace_source();
        let mut attestation = readiness_attestation_value(&commit, &tree);
        attestation["verifier"]["independence_evidence_ref"] =
            serde_json::json!("/Users/leaker/independence.bin");
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "attestation-host.json", &attestation);
        validate_kind(&input, "readiness-attestation")
            .expect_err("host-local independence_evidence_ref must be rejected by the schema");
    }

    #[test]
    fn readiness_source_check_rejects_mismatched_commit() {
        // Well-formed 40-char lowercase SHAs, but deliberately NOT the current
        // HEAD/tree: schema validation must still pass, and the source
        // cross-check must fail closed.
        let bundle = readiness_bundle_value(
            "0000000000000000000000000000000000000000",
            "1111111111111111111111111111111111111111",
        );
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "bundle-mismatch.json", &bundle);
        validate_kind(&input, "readiness-probe-bundle")
            .expect("schema must accept well-formed but mismatched source");
        let err = validate_readiness_with_source(&input, "readiness-probe-bundle")
            .expect_err("mismatched source must fail the readiness source check");
        assert!(
            matches!(err, ValidationError::ReadinessSourceMismatch),
            "expected ReadinessSourceMismatch, got: {err}"
        );
    }

    #[test]
    fn readiness_source_check_rejects_non_readiness_kind() {
        let dir = readiness_temp_root();
        let input = write_readiness_json(&dir, "not-readiness.json", &serde_json::json!({}));
        let err = validate_readiness_with_source(&input, "production-provenance")
            .expect_err("non-readiness kind must not be source-bound");
        assert!(
            matches!(err, ValidationError::ReadinessKindNotSourceBound { .. }),
            "expected ReadinessKindNotSourceBound, got: {err}"
        );
    }
}
