fn main() {
    if let Err(error) = feathermark_app::run() {
        eprintln!("FeatherMark failed to start: {error}");
        std::process::exit(1);
    }
}
