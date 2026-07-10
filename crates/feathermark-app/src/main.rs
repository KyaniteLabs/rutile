fn main() {
    let mut smoke = false;
    let mut path = None;
    for argument in std::env::args_os().skip(1) {
        if argument == "--native-smoke" {
            smoke = true;
        } else if path.is_none() {
            path = Some(std::path::PathBuf::from(argument));
        }
    }

    #[cfg(all(target_os = "macos", feature = "macos-shell"))]
    let result = {
        feathermark_app::platform::macos::run_cli(path, smoke).map_err(|error| error.to_string())
    };

    #[cfg(all(target_os = "linux", feature = "linux-gtk"))]
    let result = {
        // The Linux GTK adapter consumes the smoke entrypoint through a typed
        // environment interface so the default/headless binary stays free of
        // platform dependencies. Translate --native-smoke here once.
        if smoke {
            // SAFETY: this is the process main thread before any worker threads
            // are spawned; the environment is read only by the GTK adapter on
            // the same thread to select the auto-close smoke path.
            unsafe { std::env::set_var("FEATHERMARK_SMOKE_AUTOCLOSE_MS", "5000") };
        }
        let _ = path;
        feathermark_app::run()
    };

    #[cfg(not(any(
        all(target_os = "macos", feature = "macos-shell"),
        all(target_os = "linux", feature = "linux-gtk"),
    )))]
    let result = {
        let _ = smoke;
        let _ = path;
        feathermark_app::run()
    };

    if let Err(error) = result {
        eprintln!(
            "{} failed to start: {error}",
            feathermark_app::brand::PRODUCT_NAME
        );
        std::process::exit(1);
    }
}
