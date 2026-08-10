fn main() {
    if let Err(error) = xtask::runner_native::launcher_main() {
        eprintln!("rutile-runner-launcher: {error}");
        std::process::exit(1);
    }
}
