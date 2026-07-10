fn main() {
    if let Err(error) = xtask::runner_native::probe_main() {
        eprintln!("feathermark-runner-probe: {error}");
        std::process::exit(1);
    }
}
