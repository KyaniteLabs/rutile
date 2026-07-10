fn main() {
    if let Err(error) = xtask::runner_native::launcher_main() {
        eprintln!("feathermark-runner-launcher: {error}");
        std::process::exit(1);
    }
}
