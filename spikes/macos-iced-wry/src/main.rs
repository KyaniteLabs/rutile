fn main() {
    #[cfg(all(target_os = "macos", feature = "macos-iced"))]
    if std::env::args().nth(1).as_deref() == Some("--native-seam-smoke") {
        match macos_iced_wry_spike::run_native_seam_smoke() {
            Ok(receipt) => {
                println!(
                    "native-seam-ok ipc={} resize={} focus={} iced={} webview-first={}",
                    receipt.ipc_frames,
                    receipt.resized,
                    receipt.focused,
                    receipt.iced_program,
                    receipt.webview_dropped_before_window
                );
                return;
            }
            Err(error) => {
                eprintln!("native-seam-error:{error}");
                std::process::exit(1);
            }
        }
    }

    if std::env::args().nth(1).as_deref() == Some("--assert-appkit-main-thread") {
        match macos_iced_wry_spike::AppKitMainThread::claim() {
            Ok(_) => println!("main-thread-ok"),
            Err(error) => {
                eprintln!("main-thread-error:{error:?}");
                std::process::exit(1);
            }
        }
    }
}
