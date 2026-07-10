use super::PlatformAdapter;

/// Compile-ready production seam; native widget ownership is supplied by the
/// platform-shell lane after the approved spike adapter is promoted.
pub struct LinuxGtkAdapter;

impl PlatformAdapter for LinuxGtkAdapter {
    fn run() -> Result<(), String> {
        Ok(())
    }
}
