fn main() {
    if let Err(error) = else_daemon::run_from_args() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
