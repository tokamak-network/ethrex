mod cli;
mod utils;

use crate::cli::{CLI, emit_error_report};
use clap::Parser;

#[tokio::main]
async fn main() {
    let CLI { command } = CLI::parse();
    let json = command.json_output();

    if let Err(error) = command.run().await {
        emit_error_report(json, &error);
        std::process::exit(1);
    }
}
