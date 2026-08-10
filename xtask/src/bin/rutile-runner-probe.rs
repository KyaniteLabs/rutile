fn main() {
    if let Err(error) = xtask::runner_native::probe_main() {
        eprintln!("rutile-runner-probe: {error}");
        std::process::exit(1);
    }
}
