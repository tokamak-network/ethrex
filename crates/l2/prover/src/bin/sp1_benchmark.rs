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
use ethrex_common::types::{
    AccountState, Block, BlockBody, BlockHeader, EIP1559Transaction, Transaction, TxKind,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_guest_program::{
    ZKVM_SP1_TOKAMON_ELF, ZKVM_SP1_ZK_DEX_ELF,
    programs::tokamon::types::{ActionType, GameAction, TokammonProgramInput},
    programs::zk_dex::circuit,
};
use ethrex_rlp::encode::{PayloadRLPEncode as _, RLPEncode as _};
use ethrex_trie::{EMPTY_TRIE_HASH, Trie};
use rkyv::rancor::Error as RkyvError;
use secp256k1::{Message, SECP256K1, SecretKey};
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

/// DEX contract address (must match the guest binary constant).
const DEX_CONTRACT: Address = H160([0xDE; 20]);

/// Chain ID used for benchmark transactions.
const BENCH_CHAIN_ID: u64 = 42;

/// Derive an Ethereum address from a secp256k1 secret key.
fn address_from_secret_key(sk: &SecretKey) -> Address {
    let pk = sk.public_key(SECP256K1);
    let hash = keccak_hash(&pk.serialize_uncompressed()[1..]);
    Address::from_slice(&hash[12..])
}

/// Sign an EIP-1559 transaction in place using the given secret key.
///
/// Computes the signing hash per EIP-1559 (0x02 || RLP(chain_id, nonce, ...)),
/// signs with ECDSA, and sets signature_r, signature_s, signature_y_parity.
fn sign_eip1559_tx(tx: &mut EIP1559Transaction, sk: &SecretKey) {
    // Build signing payload: tx_type byte + RLP-encoded unsigned fields.
    let mut buf = vec![0x02u8]; // EIP-1559 tx type
    buf.extend_from_slice(&tx.encode_payload_to_vec());

    let hash = keccak_hash(&buf);
    let msg = Message::from_digest(hash);
    let (recovery_id, signature) = SECP256K1
        .sign_ecdsa_recoverable(&msg, sk)
        .serialize_compact();

    // Extract r (32 bytes) and s (32 bytes).
    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    r_bytes.copy_from_slice(&signature[..32]);
    s_bytes.copy_from_slice(&signature[32..64]);

    tx.signature_r = U256::from_big_endian(&r_bytes);
    tx.signature_s = U256::from_big_endian(&s_bytes);
    tx.signature_y_parity = i32::from(recovery_id) != 0;
}

/// Generate a deterministic secret key for benchmark user `index`.
fn deterministic_secret_key(index: u32) -> SecretKey {
    let mut key_bytes = [0u8; 32];
    // Use a non-zero prefix + index to ensure valid keys.
    key_bytes[0] = 0x01;
    let idx_bytes = index.to_be_bytes();
    key_bytes[28..32].copy_from_slice(&idx_bytes);
    #[expect(
        clippy::expect_used,
        reason = "benchmark utility with deterministic inputs"
    )]
    SecretKey::from_slice(&key_bytes).expect("deterministic key should be valid")
}

/// Generate a deterministic, valid `AppProgramInput` for zk-dex benchmarking.
///
/// Builds an `AppProgramInput` directly with:
/// - Merkle proofs extracted from an in-memory state/storage trie
/// - Properly signed EIP-1559 transfer transactions
/// - Account proofs for all senders, receivers, and the DEX contract
/// - Storage proofs for token balance slots
///
/// This constructs the input that the guest binary expects without going through
/// the `ProgramInput` → `AppProgramInput` conversion pipeline (which is tested
/// separately in unit tests). The benchmark focuses on guest execution cycles.
#[expect(clippy::indexing_slicing)]
fn generate_zk_dex_input(transfer_count: u32) -> anyhow::Result<Vec<u8>> {
    use ethrex_guest_program::common::app_types::{AccountProof, AppProgramInput, StorageProof};

    let token = H160([0xAA; 20]);
    let count = usize::try_from(transfer_count).context("transfer count overflow")?;

    // Generate deterministic key pairs: each transfer has a sender and receiver.
    let mut sender_keys: Vec<SecretKey> = Vec::with_capacity(count);
    let mut sender_addrs: Vec<Address> = Vec::with_capacity(count);
    let mut receiver_addrs: Vec<Address> = Vec::with_capacity(count);

    for i in 0..transfer_count {
        let sender_sk = deterministic_secret_key(i.saturating_mul(2));
        let sender_addr = address_from_secret_key(&sender_sk);
        let receiver_sk = deterministic_secret_key(i.saturating_mul(2).saturating_add(1));
        let receiver_addr = address_from_secret_key(&receiver_sk);

        sender_keys.push(sender_sk);
        sender_addrs.push(sender_addr);
        receiver_addrs.push(receiver_addr);
    }

    // Collect all unique user addresses.
    let mut all_users: Vec<Address> = sender_addrs.clone();
    all_users.extend_from_slice(&receiver_addrs);
    all_users.sort();
    all_users.dedup();

    // Build storage trie with balance slots for all users.
    let mut storage_trie = Trie::empty_in_memory();
    for user in &all_users {
        let slot = circuit::balance_storage_slot(token, *user);
        let hashed_slot = keccak_hash(slot.as_bytes()).to_vec();
        let balance = U256::from(1_000_000u64);
        storage_trie
            .insert(hashed_slot, balance.encode_to_vec())
            .map_err(|e| anyhow::anyhow!("storage trie insert: {e}"))?;
    }
    let storage_root = storage_trie.hash_no_commit();

    // Build state trie with user accounts + DEX contract.
    let mut state_trie = Trie::empty_in_memory();

    let dex_account = AccountState {
        nonce: 0,
        balance: U256::zero(),
        storage_root,
        code_hash: H256::zero(),
    };
    let dex_path = keccak_hash(DEX_CONTRACT.as_bytes()).to_vec();
    state_trie
        .insert(dex_path, dex_account.encode_to_vec())
        .map_err(|e| anyhow::anyhow!("state trie insert dex: {e}"))?;

    let user_balance = U256::from(10u64) * U256::from(10u64).pow(U256::from(18u64));
    for user in &all_users {
        let account = AccountState {
            nonce: 0,
            balance: user_balance,
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: H256::zero(),
        };
        let path = keccak_hash(user.as_bytes()).to_vec();
        state_trie
            .insert(path, account.encode_to_vec())
            .map_err(|e| anyhow::anyhow!("state trie insert user: {e}"))?;
    }
    let prev_state_root = state_trie.hash_no_commit();

    // Extract account proofs from the state trie.
    let mut account_proofs: Vec<AccountProof> = Vec::new();

    // DEX contract account proof.
    let dex_proof = state_trie
        .get_proof(&keccak_hash(DEX_CONTRACT.as_bytes()).to_vec())
        .map_err(|e| anyhow::anyhow!("dex proof: {e}"))?;
    account_proofs.push(AccountProof {
        address: DEX_CONTRACT,
        nonce: 0,
        balance: U256::zero(),
        storage_root: storage_root,
        code_hash: H256::zero(),
        proof: dex_proof,
    });

    // User account proofs.
    for user in &all_users {
        let proof = state_trie
            .get_proof(&keccak_hash(user.as_bytes()).to_vec())
            .map_err(|e| anyhow::anyhow!("user proof: {e}"))?;
        account_proofs.push(AccountProof {
            address: *user,
            nonce: 0,
            balance: user_balance,
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: H256::zero(),
            proof,
        });
    }

    // Extract storage proofs for each sender/receiver balance slot.
    let mut storage_proofs: Vec<StorageProof> = Vec::new();
    let mut seen_slots: std::collections::BTreeSet<(Address, H256)> =
        std::collections::BTreeSet::new();

    for i in 0..transfer_count {
        let idx = usize::try_from(i).context("index overflow")?;
        let sender = sender_addrs.get(idx).copied().context("sender addr")?;
        let receiver = receiver_addrs.get(idx).copied().context("receiver addr")?;

        // Both sender and receiver need balance slots.
        for user in [sender, receiver] {
            let slot = circuit::balance_storage_slot(token, user);
            if !seen_slots.insert((DEX_CONTRACT, slot)) {
                continue; // Already added.
            }

            let hashed_slot = keccak_hash(slot.as_bytes()).to_vec();
            let storage_proof = storage_trie
                .get_proof(&hashed_slot)
                .map_err(|e| anyhow::anyhow!("storage proof: {e}"))?;
            let account_proof = state_trie
                .get_proof(&keccak_hash(DEX_CONTRACT.as_bytes()).to_vec())
                .map_err(|e| anyhow::anyhow!("account proof for storage: {e}"))?;

            storage_proofs.push(StorageProof {
                address: DEX_CONTRACT,
                slot,
                value: U256::from(1_000_000u64),
                account_proof,
                storage_proof,
            });
        }
    }

    // Build signed transactions.
    let mut transactions = Vec::with_capacity(count);
    for i in 0..transfer_count {
        let idx = usize::try_from(i).context("index overflow")?;
        let receiver = receiver_addrs.get(idx).copied().context("receiver")?;
        let sender_sk = sender_keys.get(idx).context("sender key")?;

        let calldata = circuit::encode_transfer_calldata(receiver, token, U256::from(100u64));

        let mut eip1559_tx = EIP1559Transaction {
            to: TxKind::Call(DEX_CONTRACT),
            data: bytes::Bytes::from(calldata),
            nonce: 0,
            max_fee_per_gas: 1000,
            max_priority_fee_per_gas: 1,
            gas_limit: 100_000,
            chain_id: BENCH_CHAIN_ID,
            ..Default::default()
        };
        sign_eip1559_tx(&mut eip1559_tx, sender_sk);
        transactions.push(Transaction::EIP1559Transaction(eip1559_tx));
    }

    let block = Block {
        header: BlockHeader {
            number: 1,
            base_fee_per_gas: Some(100),
            gas_limit: u64::try_from(count)
                .unwrap_or(u64::MAX)
                .saturating_mul(100_000),
            ..Default::default()
        },
        body: BlockBody {
            transactions,
            ..Default::default()
        },
    };

    // Build AppProgramInput directly.
    let app_input = AppProgramInput {
        blocks: vec![block],
        prev_state_root,
        storage_proofs,
        account_proofs,
        elasticity_multiplier: 2,
        fee_configs: vec![],
        blob_commitment: [0u8; 48],
        blob_proof: [0u8; 48],
        chain_id: BENCH_CHAIN_ID,
    };

    // Serialize via rkyv.
    let bytes = rkyv::to_bytes::<RkyvError>(&app_input)
        .map_err(|e| anyhow::anyhow!("rkyv serialize AppProgramInput: {e}"))?;
    Ok(bytes.to_vec())
}

/// Generate a deterministic, valid `TokammonProgramInput`.
///
/// Creates a mix of game action types (CreateSpot, ClaimReward, FeedTokamon,
/// Battle) cycling through them. Each action has valid payloads matching the
/// validation requirements of the execution function.
#[expect(clippy::indexing_slicing, clippy::as_conversions)]
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
        other => {
            anyhow::bail!("Unsupported proof format: '{other}'. Use 'compressed' or 'groth16'.")
        }
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
        other => anyhow::bail!("Unsupported program: '{other}'. Supported: zk-dex, tokamon"),
    };
    println!("ELF size: {} bytes", elf.len());

    // Generate program-specific input and serialize with rkyv.
    let serialized: Vec<u8> = match args.program.as_str() {
        "zk-dex" => {
            println!("Generating input: {} transfers", args.actions);
            generate_zk_dex_input(args.actions)?
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
        println!(
            "Verification time: {}\n",
            format_duration(verify_start.elapsed())
        );
    }

    // Summary.
    println!("=== Summary ===");
    println!("Program:           {}", args.program);
    println!("Actions:           {}", args.actions);
    println!("Proof format:      {}", args.format);
    println!("ELF size:          {} bytes", elf.len());
    println!("Input size:        {} bytes", serialized.len());
    println!("Instruction count: {}", report.total_instruction_count());
    println!("Execution time:    {}", format_duration(exec_duration));
    if let Some(d) = prove_duration {
        println!("Proving time:      {}", format_duration(d));
    }

    Ok(())
}
