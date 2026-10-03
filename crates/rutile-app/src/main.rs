fn main() {
    #[cfg(feature = "test-control")]
    let mut smoke = false;
    #[cfg(all(
        not(feature = "test-control"),
        not(all(target_os = "linux", feature = "linux-gtk"))
    ))]
    let smoke = false;
    let mut path = None;
    let mut positional_only = false;
    for argument in std::env::args_os().skip(1) {
        if !positional_only && argument == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && (argument == "--help" || argument == "-h") {
            println!(
                "Rutile — local Markdown editor\n\nUsage: rutile [FILE]\n       rutile -- [FILE]\n\nOptions:\n  -h, --help     Show help\n  -V, --version  Show version"
            );
            return;
        }
        if !positional_only && (argument == "--version" || argument == "-V") {
            println!("Rutile {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        #[cfg(feature = "test-control")]
        if !positional_only && argument == "--native-smoke" {
            smoke = true;
            continue;
        }
        if !positional_only && argument.to_string_lossy().starts_with('-') {
            eprintln!(
                "Unsupported option: {}. Use --help for usage.",
                argument.to_string_lossy()
            );
            std::process::exit(2);
        }
        if path.is_some() {
            eprintln!("Open one file per command; additional paths are not supported.");
            std::process::exit(2);
        }
        path = Some(std::path::PathBuf::from(argument));
    }

    #[cfg(all(target_os = "macos", feature = "macos-shell"))]
    let result =
        { rutile_app::platform::macos::run_cli(path, smoke).map_err(|error| error.to_string()) };

    #[cfg(all(target_os = "linux", feature = "linux-gtk"))]
    let result = {
        // The Linux GTK adapter consumes the smoke entrypoint through a typed
        // environment interface so the default/headless binary stays free of
        // platform dependencies. Translate --native-smoke here once.
        #[cfg(feature = "test-control")]
        if smoke {
            // SAFETY: this is the process main thread before any worker threads
            // are spawned; the environment is read only by the GTK adapter on
            // the same thread to select the auto-close smoke path.
            unsafe { std::env::set_var("RUTILE_SMOKE_AUTOCLOSE_MS", "5000") };
        }
        let _ = path;
        rutile_app::run()
    };

    #[cfg(not(any(
        all(target_os = "macos", feature = "macos-shell"),
        all(target_os = "linux", feature = "linux-gtk"),
    )))]
    let result = {
        let _ = smoke;
        let _ = path;
        rutile_app::run()
    };

    if let Err(error) = result {
        eprintln!(
            "{} failed to start: {error}",
            rutile_app::brand::PRODUCT_NAME
        );
        std::process::exit(1);
    }
}
