use clap::Parser;

/// Toku — a private, offline-first personal book manager.
#[derive(Parser)]
#[command(name = "toku", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Show version information
    Version,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Version) | None => {
            println!("toku {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}
