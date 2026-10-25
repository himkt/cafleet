fn main() {
    if let Err(error) = cafleet::cli::run() {
        eprintln!("Error: {}", error.message());
        std::process::exit(error.exit_code());
    }
}
