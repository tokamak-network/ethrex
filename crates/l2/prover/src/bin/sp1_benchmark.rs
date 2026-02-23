//! SP1 benchmark binary for comparing guest program proving performance.
//!
//! Measures execution cycles and proving time for different guest programs
//! using the SP1 backend. This enables direct comparison between the standard
//! evm-l2 guest program (baseline) and application-specific programs like
//! zk-dex and tokamon.
//!
//! The benchmark uses the same SP1 SDK code paths as the production prover:
//! - rkyv serialization for input encoding
//! - `Prover::execute()` for cycle counting
//! - `Prover::prove()` for proof generation
//! - `Prover::verify()` for proof verification
//!
//! # Usage
//!
//! ```bash
//! # Benchmark zk-dex with 100 transfers
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program zk-dex --actions 100
//!
//! # Benchmark tokamon with 100 game actions
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program tokamon --actions 100
//!
//! # Execute only (cycle count, skip proving)
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program zk-dex --execute-only
//!
//! # Benchmark with Groth16 proof
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program zk-dex --format groth16
//! ```

use std::time::Instant;

use anyhow::Context;
use clap::Parser;
use ethrex_guest_program::{
    programs::tokamon::types::{ActionType, GameAction, TokammonProgramInput},
    programs::zk_dex::types::{DexProgramInput, DexTransfer},
    ZKVM_SP1_TOKAMON_ELF, ZKVM_SP1_ZK_DEX_ELF,
};
use rkyv::rancor::Error as RkyvError;
#[cfg(not(feature = "gpu"))]
use sp1_sdk::CpuProver;
#[cfg(feature = "gpu")]
use sp1_sdk::cuda::builder::CudaProverBuilder;
use sp1_sdk::{Prover, SP1ProofMode, SP1Stdin};

#[derive(Parser)]
#[command(name = "sp1_benchmark")]
#[command(about = "SP1 guest program performance benchmark")]
struct Args {
    /// Guest program to benchmark: zk-dex or tokamon
    #[arg(long)]
    program: String,

    /// Number of actions/transfers to include in the input
    #[arg(long, default_value_t = 100)]
    actions: u32,

    /// Proof format: compressed or groth16
    #[arg(long, default_value = "compressed")]
    format: String,

    /// Run execution only (skip proving and verification)
    #[arg(long)]
    execute_only: bool,
}

/// Generate a deterministic, valid `DexProgramInput`.
#[expect(clippy::indexing_slicing)]
fn generate_zk_dex_input(transfer_count: u32) -> anyhow::Result<DexProgramInput> {
    let initial_state_root = [0x42u8; 32];
    let count = usize::try_from(transfer_count).context("transfer count overflow")?;
    let mut transfers = Vec::with_capacity(count);

    for i in 0..transfer_count {
        let idx = i.to_le_bytes();

        let mut from = [0u8; 20];
        from[0] = idx[0];
        from[1] = idx[1];
        from[2] = idx[2];
        from[3] = idx[3];
        from[4] = 0x01;

        let mut to = [0u8; 20];
        to[0] = idx[0];
        to[1] = idx[1];
        to[2] = idx[2];
        to[3] = idx[3];
        to[4] = 0x02;

        let mut token = [0u8; 20];
        token[0] = idx[0];
        token[1] = idx[1];
        token[2] = idx[2];
        token[3] = idx[3];
        token[4] = 0x03;

        transfers.push(DexTransfer {
            from,
            to,
            token,
            amount: u64::from(i) + 1,
            nonce: u64::from(i),
        });
    }

    Ok(DexProgramInput {
        initial_state_root,
        transfers,
    })
}

/// Generate a deterministic, valid `TokammonProgramInput`.
///
/// Creates a mix of game action types (CreateSpot, ClaimReward, FeedTokamon,
/// Battle) cycling through them. Each action has valid payloads matching the
/// validation requirements of the execution function.
#[expect(clippy::indexing_slicing)]
fn generate_tokamon_input(action_count: u32) -> anyhow::Result<TokammonProgramInput> {
    let initial_state_root = [0x42u8; 32];
    let count = usize::try_from(action_count).context("action count overflow")?;
    let mut actions = Vec::with_capacity(count);

    let action_types = [
        ActionType::CreateSpot,
        ActionType::ClaimReward,
        ActionType::FeedTokamon,
        ActionType::Battle,
    ];

    for i in 0..action_count {
        let idx = i.to_le_bytes();

        let mut player = [0u8; 20];
        player[0] = idx[0];
        player[1] = idx[1];
        player[2] = idx[2];
        player[3] = idx[3];
        player[4] = 0xAA;

        // Cycle through action types: 0,1,2,3,0,1,2,3,...
        let action_type = action_types[(i % 4) as usize].clone();

        // Generate valid payload per action type.
        let payload = match &action_type {
            ActionType::CreateSpot => {
                // Requires >= 16 bytes (lat: 8 + lon: 8).
                let mut p = vec![0u8; 16];
                p[0] = idx[0];
                p[8] = idx[1];
                p
            }
            ActionType::Battle => {
                // Requires >= 8 bytes (random seed).
                let mut p = vec![0u8; 8];
                p[0] = idx[0];
                p[1] = idx[1];
                p
            }
            // ClaimReward and FeedTokamon: no payload requirement.
            _ => vec![],
        };

        actions.push(GameAction {
            player,
            action_type,
            target_id: u64::from(i),
            payload,
        });
    }

    Ok(TokammonProgramInput {
        initial_state_root,
        actions,
    })
}

fn parse_proof_mode(s: &str) -> anyhow::Result<SP1ProofMode> {
    match s {
        "compressed" => Ok(SP1ProofMode::Compressed),
        "groth16" => Ok(SP1ProofMode::Groth16),
        other => anyhow::bail!("Unsupported proof format: '{other}'. Use 'compressed' or 'groth16'."),
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs >= 60 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        let tenths = d.subsec_millis() / 100;
        format!("{mins}m {secs}.{tenths}s")
    } else {
        format!("{:.2?}", d)
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    println!("=== SP1 Benchmark: {} ===\n", args.program);

    // Resolve ELF binary for the selected guest program.
    let elf: &[u8] = match args.program.as_str() {
        "zk-dex" => {
            if ZKVM_SP1_ZK_DEX_ELF.is_empty() {
                anyhow::bail!(
                    "zk-dex SP1 ELF not found. Build it first:\n  \
                     cd crates/guest-program && GUEST_PROGRAMS=zk-dex \
                     cargo build --release -p ethrex-guest-program --features sp1"
                );
            }
            ZKVM_SP1_ZK_DEX_ELF
        }
        "tokamon" => {
            if ZKVM_SP1_TOKAMON_ELF.is_empty() {
                anyhow::bail!(
                    "tokamon SP1 ELF not found. Build it first:\n  \
                     cd crates/guest-program && GUEST_PROGRAMS=tokamon \
                     cargo build --release -p ethrex-guest-program --features sp1"
                );
            }
            ZKVM_SP1_TOKAMON_ELF
        }
        other => anyhow::bail!(
            "Unsupported program: '{other}'. Supported: zk-dex, tokamon"
        ),
    };
    println!("ELF size: {} bytes", elf.len());

    // Generate program-specific input and serialize with rkyv.
    let serialized: Vec<u8> = match args.program.as_str() {
        "zk-dex" => {
            println!("Generating input: {} transfers", args.actions);
            let input = generate_zk_dex_input(args.actions)?;
            rkyv::to_bytes::<RkyvError>(&input)
                .map_err(|e| anyhow::anyhow!("rkyv serialization: {e}"))?
                .to_vec()
        }
        "tokamon" => {
            println!("Generating input: {} game actions", args.actions);
            let input = generate_tokamon_input(args.actions)?;
            rkyv::to_bytes::<RkyvError>(&input)
                .map_err(|e| anyhow::anyhow!("rkyv serialization: {e}"))?
                .to_vec()
        }
        _ => anyhow::bail!("Unsupported program for input generation"),
    };
    println!("Serialized input: {} bytes\n", serialized.len());

    // Prepare SP1 stdin.
    let mut stdin = SP1Stdin::new();
    stdin.write_slice(&serialized);

    // Initialize SP1 prover client.
    println!("--- SP1 Prover Initialization ---");
    let init_start = Instant::now();
    #[cfg(not(feature = "gpu"))]
    let client = CpuProver::new();
    #[cfg(feature = "gpu")]
    let client = CudaProverBuilder::default().local().build();
    println!("Client created: {}", format_duration(init_start.elapsed()));

    // Setup proving/verifying keys for this ELF.
    let setup_start = Instant::now();
    let (pk, vk) = client.setup(elf);
    println!("Key setup: {}\n", format_duration(setup_start.elapsed()));

    // Execute (cycle counting).
    println!("--- Execution (cycle counting) ---");
    let exec_start = Instant::now();
    let (_public_values, report) = client
        .execute(elf, &stdin)
        .run()
        .context("SP1 execution failed")?;
    let exec_duration = exec_start.elapsed();

    println!("Execution time: {}", format_duration(exec_duration));
    println!(
        "Total instruction count: {}",
        report.total_instruction_count()
    );
    println!();

    // Proving (unless --execute-only).
    let mut prove_duration = None;
    if !args.execute_only {
        let mode = parse_proof_mode(&args.format)?;

        println!("--- Proving ({}) ---", args.format);
        let prove_start = Instant::now();
        let proof = client
            .prove(&pk, &stdin)
            .mode(mode)
            .run()
            .context("SP1 proving failed")?;
        let duration = prove_start.elapsed();
        prove_duration = Some(duration);
        println!("Proving time: {}\n", format_duration(duration));

        // Verification.
        println!("--- Verification ---");
        let verify_start = Instant::now();
        client
            .verify(&proof, &vk)
            .context("SP1 verification failed")?;
        println!("Verification time: {}\n", format_duration(verify_start.elapsed()));
    }

    // Summary.
    println!("=== Summary ===");
    println!("Program:           {}", args.program);
    println!("Actions:           {}", args.actions);
    println!("Proof format:      {}", args.format);
    println!("ELF size:          {} bytes", elf.len());
    println!("Input size:        {} bytes", serialized.len());
    println!(
        "Instruction count: {}",
        report.total_instruction_count()
    );
    println!("Execution time:    {}", format_duration(exec_duration));
    if let Some(d) = prove_duration {
        println!("Proving time:      {}", format_duration(d));
    }

    Ok(())
}
