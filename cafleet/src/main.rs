/// The top-level exit-code mapping (SPEC §7.2): usage → 2, everything else
/// → 1, each printed as `Error: <message>` on stderr. clap's own parse
/// errors and `--help` / `--version` exit inside `cli::run`.
fn main() {
    if let Err(error) = cafleet::cli::run() {
        eprintln!("Error: {}", error.message());
        std::process::exit(error.exit_code());
    }
}
