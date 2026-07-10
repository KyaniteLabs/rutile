use super::PlatformAdapter;

/// Compile-ready production seam; the ADR-selected native shell supplies the
/// concrete widget/webview ownership without leaking it into the shared core.
pub struct MacosAdapter;

impl PlatformAdapter for MacosAdapter {
    fn run() -> Result<(), String> {
        Ok(())
    }
}
