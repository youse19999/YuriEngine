fn main() {
    if let Err(error) = yuriengine::engine::cli_main() {
        eprintln!("YuriEngine failed: {error}");
        std::process::exit(1);
    }
}
