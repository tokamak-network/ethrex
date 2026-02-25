mod cli;
mod detect;
mod readers;
mod utils;

use crate::cli::{CLI, emit_error_report};
use clap::Parser;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let started_at = Instant::now();
    let CLI { command } = CLI::parse();
    let json = command.json_output();
    let retry_attempts = command.retry_attempts();
    let report_file = command.report_file().map(|path| path.to_path_buf());

    if let Err(error) = command.run().await {
        emit_error_report(
            json,
            retry_attempts,
            started_at,
            &error,
            report_file.as_deref(),
        );
        std::process::exit(1);
    }
}
