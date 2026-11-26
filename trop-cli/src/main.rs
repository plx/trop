use clap::Parser;
use std::io::Write;
use trop_cli::Cli;

fn main() {
    // Parse CLI arguments. `--help` and `--version` exit 0; every other
    // parse failure is a usage error and exits with the invalid-arguments
    // code, never with clap's default 2 (reserved for lock timeouts).
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Printing (or flushing) can only fail if the stream is closed;
            // the exit code still reports the failure. Flush both streams
            // before exiting, as clap's own `Error::exit` does.
            let _ = error.print();
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(trop_cli::parse_failure_exit_code(error.kind()));
        }
    };

    let result = trop_cli::run(cli);

    // Handle errors and set exit code
    match result {
        Ok(()) => std::process::exit(trop_cli::success_exit_code()),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(e.exit_code());
        }
    }
}
