use clap::Parser;

#[derive(Parser)]
#[command(name = "trop")]
#[command(version, about = "Manage ephemeral port reservations", long_about = None)]
struct Cli {
    /// Placeholder for future subcommands
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    // Subcommands will be added in future phases
}

fn main() {
    let _cli = Cli::parse();

    // For now, just print the version if no subcommands are provided
    println!("trop v{}", env!("CARGO_PKG_VERSION"));
    println!("Port reservation management tool");
}
