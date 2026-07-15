#[path = "src/runner/config_manifest.rs"]
mod config_manifest;

use std::env;
use std::ffi::OsStr;
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

    embed_release_assets(&out);
}

/// Release assets consumed by `local_package.rs` (document-type plist,
/// freedesktop desktop/appdata/mime entries). In a full workspace build these
/// are read from `release/assets/`; if absent (e.g. the scaffold isolation
/// test, which copies only the xtask crate) empty placeholders are written so
/// the crate still compiles. A `cargo:warning` makes the fallback visible.
fn embed_release_assets(out_dir: &OsStr) {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let assets_root = Path::new(&manifest_dir).join("../release/assets");
    let dest = Path::new(out_dir).join("release-assets");
    fs::create_dir_all(&dest).expect("create release-assets dir in OUT_DIR");
    // (asset_filename, subdirectory under release/assets/)
    for (name, subdir) in &[
        ("document-types.plist", "macos"),
        ("AppIcon.icns", "macos"),
        ("feathermark.desktop", "linux"),
        ("feathermark.appdata.xml", "linux"),
        ("feathermark-markdown.xml", "linux"),
    ] {
        let source = assets_root.join(subdir).join(name);
        if source.is_file() {
            println!("cargo:rerun-if-changed={}", source.display());
            fs::copy(&source, dest.join(name)).expect("copy release asset to OUT_DIR");
        } else {
            println!(
                "cargo:warning=release asset {name} not found at {}; using empty placeholder",
                source.display()
            );
            fs::write(dest.join(name), b"").expect("write empty asset placeholder");
        }
    }
}

fn read_optional(path: &str) -> Option<Vec<u8>> {
    Path::new(path)
        .exists()
        .then(|| fs::read(path).unwrap_or_else(|error| panic!("read {path}: {error}")))
}
