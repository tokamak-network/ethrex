mod cli;
mod utils;

use crate::cli::{CLI, emit_error_report};
use clap::Parser;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let started_at = Instant::now();
    let CLI { command } = CLI::parse();
    let json = command.json_output();

    if let Err(error) = command.run().await {
        emit_error_report(json, started_at, &error);
        std::process::exit(1);
    }
}
