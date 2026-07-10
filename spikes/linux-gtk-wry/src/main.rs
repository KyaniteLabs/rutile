fn main() {
    #[cfg(all(target_os = "linux", feature = "linux-gtk"))]
    if std::env::args().nth(1).as_deref() == Some("--native-seam-smoke") {
        match linux_gtk_wry_spike::run_native_seam_smoke() {
            Ok(receipt) => {
                println!(
                    "native-seam-ok ipc={} resize={} focus={} webview-first={}",
                    receipt.ipc_frames,
                    receipt.resized,
                    receipt.focused,
                    receipt.webview_dropped_before_window
                );
            }
            Err(error) => {
                eprintln!("native-seam-error:{error}");
                std::process::exit(1);
            }
        }
    }
}
