use clap::Parser;
use trop_cli::Cli;

fn main() {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging based on verbosity
    let _logger = trop::init_logger(cli.verbose, cli.quiet);

    let result = trop_cli::run(cli);

    // Handle errors and set exit code
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(e.exit_code());
        }
    }
}
