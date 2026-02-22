mod cli;
mod utils;

use crate::cli::CLI;
use clap::Parser;

#[tokio::main]
async fn main() {
    let CLI { command } = CLI::parse();

    if let Err(error) = command.run().await {
        eprintln!("Migration failed: {error:?}");
        std::process::exit(1);
    }
}
