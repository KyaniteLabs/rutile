fn main() {
    if std::env::args().nth(1).as_deref() == Some("--assert-appkit-main-thread") {
        match macos_egui_wry_spike::AppKitMainThread::claim() {
            Ok(_) => println!("main-thread-ok"),
            Err(error) => {
                eprintln!("main-thread-error:{error:?}");
                std::process::exit(1);
            }
        }
    }
}
