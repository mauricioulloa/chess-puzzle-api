use anyhow::Result;
use chess_puzzle_api::{import, serve};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "chess-puzzle-api",
    version,
    about = "Open-source chess puzzle API backed by the Lichess puzzle database"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build the puzzle database from the Lichess dump
    Import(import::ImportArgs),

    /// Run the HTTP API
    Serve(serve::ServeArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "chess_puzzle_api=info".into()),
        )
        .with_target(false)
        .init();

    match Cli::parse().command {
        Command::Import(args) => import::run(args).await,
        Command::Serve(args) => serve::run(args).await,
    }
}
