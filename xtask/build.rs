#[path = "src/runner/config_manifest.rs"]
mod config_manifest;

use std::env;
use std::fs;
use std::path::Path;

use config_manifest::{ManifestState, parse_manifest_state, render_production_config};

fn main() {
    const TRUST: &str = "runner-trust-roots-v1.json";
    const DISPATCH: &str = "runner-dispatch-v1.toml";
    println!("cargo:rerun-if-changed={TRUST}");
    println!("cargo:rerun-if-changed={DISPATCH}");

    let trust = read_optional(TRUST);
    let dispatch = read_optional(DISPATCH);
    let state = parse_manifest_state(trust.as_deref(), dispatch.as_deref())
        .unwrap_or_else(|error| panic!("invalid production runner provisioning: {error}"));
    let generated = match state {
        ManifestState::Unprovisioned => {
            "pub(crate) const PRODUCTION_RUNNER_CONFIG: ProductionRunnerConfig = ProductionRunnerConfig::Unprovisioned;\n".into()
        }
        ManifestState::Provisioned(manifests) => render_production_config(&manifests),
    };
    let out = env::var_os("OUT_DIR").expect("Cargo always sets OUT_DIR");
    fs::write(
        Path::new(&out).join("production_runner_config.rs"),
        generated,
    )
    .expect("write production runner configuration");
}

fn read_optional(path: &str) -> Option<Vec<u8>> {
    Path::new(path)
        .exists()
        .then(|| fs::read(path).unwrap_or_else(|error| panic!("read {path}: {error}")))
}
