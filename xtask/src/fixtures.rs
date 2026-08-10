use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureSpec {
    pub name: &'static str,
    pub file_name: &'static str,
    pub bytes: usize,
}

pub const FIXTURE_SPECS: &[FixtureSpec] = &[
    FixtureSpec {
        name: "small",
        file_name: "small.md",
        bytes: 128,
    },
    FixtureSpec {
        name: "unicode",
        file_name: "unicode.md",
        bytes: 1_024,
    },
    FixtureSpec {
        name: "one-mib",
        file_name: "one-mib.md",
        bytes: 1_048_576,
    },
    FixtureSpec {
        name: "five-mib",
        file_name: "five-mib.md",
        bytes: 5_242_880,
    },
    FixtureSpec {
        name: "hostile",
        file_name: "hostile.md",
        bytes: 4_096,
    },
    FixtureSpec {
        name: "nested",
        file_name: "nested.md",
        bytes: 8_192,
    },
    FixtureSpec {
        name: "giant-block",
        file_name: "giant-block.md",
        bytes: 65_536,
    },
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    pub schema: String,
    pub fixtures: Vec<FixtureManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifestEntry {
    pub name: String,
    pub file: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("fixture I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("fixture manifest JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fixture {0} differs from its deterministic definition")]
    Drift(String),
    #[error("fixture directory contains an unmanifested entry: {0}")]
    Unmanifested(String),
}

pub fn generate_fixtures(directory: &Path) -> Result<FixtureManifest, FixtureError> {
    if directory.exists() {
        reject_unmanifested_entries(directory)?;
    }
    fs::create_dir_all(directory)?;
    let manifest = expected_manifest();
    for spec in FIXTURE_SPECS {
        fs::write(directory.join(spec.file_name), fixture_bytes(spec))?;
    }
    let mut encoded = serde_json::to_vec_pretty(&manifest)?;
    encoded.push(b'\n');
    fs::write(directory.join("manifest-v1.json"), encoded)?;
    Ok(manifest)
}

pub fn verify_fixtures(directory: &Path) -> Result<FixtureManifest, FixtureError> {
    reject_unmanifested_entries(directory)?;
    let expected = expected_manifest();
    let recorded: FixtureManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest-v1.json"))?)?;
    if recorded != expected {
        return Err(FixtureError::Drift("manifest-v1.json".into()));
    }
    for spec in FIXTURE_SPECS {
        let actual = fs::read(directory.join(spec.file_name))?;
        if actual != fixture_bytes(spec) {
            return Err(FixtureError::Drift(spec.name.into()));
        }
    }
    Ok(recorded)
}

fn reject_unmanifested_entries(directory: &Path) -> Result<(), FixtureError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let allowed =
            name == "manifest-v1.json" || FIXTURE_SPECS.iter().any(|spec| name == spec.file_name);
        if !allowed || !entry.file_type()?.is_file() {
            return Err(FixtureError::Unmanifested(
                name.to_string_lossy().into_owned(),
            ));
        }
    }
    Ok(())
}

fn expected_manifest() -> FixtureManifest {
    FixtureManifest {
        schema: "rutile.fixtures.v1".into(),
        fixtures: FIXTURE_SPECS
            .iter()
            .map(|spec| {
                let bytes = fixture_bytes(spec);
                FixtureManifestEntry {
                    name: spec.name.into(),
                    file: spec.file_name.into(),
                    bytes: bytes.len(),
                    sha256: hex_sha256(&bytes),
                }
            })
            .collect(),
    }
}

fn fixture_bytes(spec: &FixtureSpec) -> Vec<u8> {
    if spec.name == "giant-block" {
        let prefix = b"```text\n";
        let suffix = b"\n```\n";
        let mut output = Vec::with_capacity(spec.bytes);
        output.extend_from_slice(prefix);
        output.resize(spec.bytes - suffix.len(), b'g');
        output.extend_from_slice(suffix);
        return output;
    }
    let seed: &[u8] = match spec.name {
        "small" => b"# Rutile\n\nSmall deterministic fixture.\n\n- alpha\n- beta\n",
        "unicode" => "# Unicode\n\n日本語の入力🙂\nCafe\u{301} and café.\nمرحبا\n\n".as_bytes(),
        "one-mib" | "five-mib" => {
            b"## Deterministic paragraph\n\nThe quick brown fox edits Markdown safely. 0123456789\n\n"
        }
        "hostile" => b"# Hostile input\n\n<script>alert(1)</script>\n<img src=x onerror=alert(1)>\n[j](javascript:alert(1))\n![x](file:///etc/passwd)\n<style>@import url(https://example.invalid)</style>\n",
        "nested" => b"> 1. outer\n>    - inner\n>      ```rust\n>      let nested = true;\n>      ```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[^n]: nested footnote\n",
        _ => unreachable!("closed fixture set"),
    };
    let mut output = Vec::with_capacity(spec.bytes);
    while output.len() + seed.len() <= spec.bytes {
        output.extend_from_slice(seed);
    }
    output.resize(spec.bytes, b'x');
    if output.last().is_some_and(u8::is_ascii_whitespace) {
        *output.last_mut().expect("fixture is nonempty") = b'x';
    }
    output
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
