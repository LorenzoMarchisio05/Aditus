fn main() {
    if let Err(e) = aditus_lib::run_browser() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
