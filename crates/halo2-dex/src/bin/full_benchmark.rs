use clap::Parser;
use halo2_base::{
    gates::circuit::CircuitBuilderStage,
    halo2_proofs::{
        halo2curves::bn256::Bn256,
        plonk::{verify_proof, Circuit},
        poly::{
            commitment::ParamsProver,
            kzg::{
                commitment::KZGCommitmentScheme,
                multiopen::VerifierSHPLONK,
                strategy::SingleStrategy,
            },
        },
    },
    utils::fs::gen_srs,
};
use snark_verifier_sdk::{
    gen_pk,
    halo2::{aggregation::AggregationCircuit, gen_snark_shplonk, PoseidonTranscript},
    NativeLoader, Snark, SHPLONK,
};
use std::time::Instant;

use halo2_dex::circuits::ecdsa::{self, ECDSA_K};
use halo2_dex::circuits::storage::{self, STORAGE_K};
use halo2_dex::circuits::transfer::{self, TRANSFER_K};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "halo2-dex-full-benchmark")]
#[command(about = "Full Halo2 DEX benchmark: ECDSA + Storage Proof + Transfer + Aggregation")]
struct Args {
    /// Skip SNARK aggregation (faster, sub-circuit benchmarks only)
    #[arg(long = "skip-aggregation")]
    skip_aggregation: bool,

    /// Aggregation circuit degree k (default: 23)
    #[arg(long = "agg-k", default_value = "23")]
    agg_k: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fmt_duration(d: std::time::Duration) -> String {
    if d.as_secs() >= 60 {
        format!(
            "{}m {:.2}s",
            d.as_secs() / 60,
            (d.as_millis() % 60_000) as f64 / 1000.0
        )
    } else if d.as_millis() >= 1000 {
        format!("{:.2}s", d.as_millis() as f64 / 1000.0)
    } else {
        format!("{}ms", d.as_millis())
    }
}

fn fmt_bytes(n: usize) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{} B", n)
    }
}

#[allow(dead_code)]
struct SubCircuitResult {
    name: &'static str,
    k: u32,
    keygen_time: std::time::Duration,
    prove_time: std::time::Duration,
    proof_size: usize,
    snark: Snark,
}

// ---------------------------------------------------------------------------
// Sub-circuit benchmark runner
// ---------------------------------------------------------------------------

fn bench_ecdsa() -> SubCircuitResult {
    let input = ecdsa::random_ecdsa_input();
    let k = ECDSA_K;

    print!("  [ECDSA Verification]  k={}\n", k);

    // SRS
    print!("    SRS ...        ");
    let start = Instant::now();
    let params = gen_srs(k);
    println!("{:>10}", fmt_duration(start.elapsed()));

    // Keygen
    print!("    Keygen ...     ");
    let start = Instant::now();
    let circuit = ecdsa::build_ecdsa_circuit(CircuitBuilderStage::Keygen, None, &params, &input);
    let pk = gen_pk(&params, &circuit, None);
    let c_params = circuit.params();
    let break_points = circuit.break_points();
    let keygen_time = start.elapsed();
    println!("{:>10}", fmt_duration(keygen_time));

    // Prove
    print!("    Proving ...    ");
    let start = Instant::now();
    let circuit = ecdsa::build_ecdsa_circuit(
        CircuitBuilderStage::Prover,
        Some((c_params, break_points)),
        &params,
        &input,
    );
    let snark = gen_snark_shplonk(&params, &pk, circuit, None::<String>);
    let prove_time = start.elapsed();
    let proof_size = snark.proof.len();
    println!("{:>10}  Proof: {}", fmt_duration(prove_time), fmt_bytes(proof_size));
    println!();

    SubCircuitResult {
        name: "ECDSA",
        k,
        keygen_time,
        prove_time,
        proof_size,
        snark,
    }
}

fn bench_storage() -> SubCircuitResult {
    let input = storage::generate_storage_input();
    let k = STORAGE_K;

    print!("  [Storage Proof]       k={}\n", k);

    // SRS
    print!("    SRS ...        ");
    let start = Instant::now();
    let params = gen_srs(k);
    println!("{:>10}", fmt_duration(start.elapsed()));

    // Keygen
    print!("    Keygen ...     ");
    let start = Instant::now();
    let circuit =
        storage::build_storage_circuit(CircuitBuilderStage::Keygen, None, &params, &input);
    let pk = gen_pk(&params, &circuit, None);
    let c_params = circuit.params();
    let break_points = circuit.break_points();
    let keygen_time = start.elapsed();
    println!("{:>10}", fmt_duration(keygen_time));

    // Prove
    print!("    Proving ...    ");
    let start = Instant::now();
    let circuit = storage::build_storage_circuit(
        CircuitBuilderStage::Prover,
        Some((c_params, break_points)),
        &params,
        &input,
    );
    let snark = gen_snark_shplonk(&params, &pk, circuit, None::<String>);
    let prove_time = start.elapsed();
    let proof_size = snark.proof.len();
    println!("{:>10}  Proof: {}", fmt_duration(prove_time), fmt_bytes(proof_size));
    println!();

    SubCircuitResult {
        name: "Storage",
        k,
        keygen_time,
        prove_time,
        proof_size,
        snark,
    }
}

fn bench_transfer() -> SubCircuitResult {
    let transfers = transfer::generate_transfers(1);
    let k = TRANSFER_K;

    print!("  [Transfer Logic]      k={}\n", k);

    // SRS
    print!("    SRS ...        ");
    let start = Instant::now();
    let params = gen_srs(k);
    println!("{:>10}", fmt_duration(start.elapsed()));

    // Keygen
    print!("    Keygen ...     ");
    let start = Instant::now();
    let circuit = transfer::build_transfer_circuit(
        CircuitBuilderStage::Keygen,
        None,
        &params,
        &transfers,
    );
    let pk = gen_pk(&params, &circuit, None);
    let c_params = circuit.params();
    let break_points = circuit.break_points();
    let keygen_time = start.elapsed();
    println!("{:>10}", fmt_duration(keygen_time));

    // Prove
    print!("    Proving ...    ");
    let start = Instant::now();
    let circuit = transfer::build_transfer_circuit(
        CircuitBuilderStage::Prover,
        Some((c_params, break_points)),
        &params,
        &transfers,
    );
    let snark = gen_snark_shplonk(&params, &pk, circuit, None::<String>);
    let prove_time = start.elapsed();
    let proof_size = snark.proof.len();
    println!("{:>10}  Proof: {}", fmt_duration(prove_time), fmt_bytes(proof_size));
    println!();

    SubCircuitResult {
        name: "Transfer",
        k,
        keygen_time,
        prove_time,
        proof_size,
        snark,
    }
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

struct AggregationResult {
    keygen_time: std::time::Duration,
    prove_time: std::time::Duration,
    verify_time: std::time::Duration,
    proof_size: usize,
}

fn bench_aggregation(snarks: Vec<Snark>, agg_k: u32) -> AggregationResult {
    use snark_verifier_sdk::halo2::aggregation::{AggregationConfigParams, VerifierUniversality};

    println!("  [Aggregation]         k={}", agg_k);
    println!("    NOTE: Requires ~8GB RAM. First run generates SRS (~67MB).");

    // SRS for aggregation
    print!("    SRS ...        ");
    let start = Instant::now();
    let params = gen_srs(agg_k);
    println!("{:>10}", fmt_duration(start.elapsed()));

    let agg_config = AggregationConfigParams {
        degree: agg_k,
        lookup_bits: (agg_k - 1) as usize,
        ..Default::default()
    };

    // Keygen with the actual snarks
    print!("    Keygen ...     ");
    let start = Instant::now();
    let mut agg_circuit = AggregationCircuit::new::<SHPLONK>(
        CircuitBuilderStage::Keygen,
        agg_config,
        &params,
        snarks.clone(),
        VerifierUniversality::Full,
    );
    let agg_config_final = agg_circuit.calculate_params(Some(10));
    let pk = gen_pk(&params, &agg_circuit, None);
    let break_points = agg_circuit.builder.break_points();
    let keygen_time = start.elapsed();
    println!("{:>10}", fmt_duration(keygen_time));

    // Prove
    print!("    Proving ...    ");
    let start = Instant::now();
    let mut agg_circuit = AggregationCircuit::new::<SHPLONK>(
        CircuitBuilderStage::Prover,
        agg_config_final,
        &params,
        snarks,
        VerifierUniversality::Full,
    );
    agg_circuit.builder.set_break_points(break_points);

    let agg_snark = gen_snark_shplonk(&params, &pk, agg_circuit, None::<String>);
    let prove_time = start.elapsed();
    let proof_size = agg_snark.proof.len();
    println!("{:>10}  Proof: {}", fmt_duration(prove_time), fmt_bytes(proof_size));

    // Verify
    print!("    Verifying ...  ");
    let verifier_params = params.verifier_params();
    let strategy = SingleStrategy::new(&params);
    let mut transcript =
        PoseidonTranscript::<NativeLoader, &[u8]>::new::<0>(&agg_snark.proof[..]);
    let instance = &agg_snark.instances[0][..];

    let start = Instant::now();
    verify_proof::<
        KZGCommitmentScheme<Bn256>,
        VerifierSHPLONK<'_, Bn256>,
        _,
        _,
        SingleStrategy<'_, Bn256>,
    >(
        verifier_params,
        pk.get_vk(),
        strategy,
        &[&[instance]],
        &mut transcript,
    )
    .expect("aggregation verification failed");
    let verify_time = start.elapsed();
    println!("{:>10}", fmt_duration(verify_time));
    println!();

    AggregationResult {
        keygen_time,
        prove_time,
        verify_time,
        proof_size,
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    env_logger::init();
    let args = Args::parse();

    println!();
    println!("============================================");
    println!("  Halo2 DEX Full Benchmark (1 transfer)");
    println!("============================================");
    println!();

    // Sub-circuit benchmarks
    let ecdsa_result = bench_ecdsa();
    let storage_result = bench_storage();
    let transfer_result = bench_transfer();

    let sub_total_prove = ecdsa_result.prove_time + storage_result.prove_time + transfer_result.prove_time;

    // Aggregation (optional)
    let agg_result = if !args.skip_aggregation {
        let snarks = vec![
            ecdsa_result.snark,
            storage_result.snark,
            transfer_result.snark,
        ];
        Some(bench_aggregation(snarks, args.agg_k))
    } else {
        println!("  [Aggregation] SKIPPED (--skip-aggregation)");
        println!();
        None
    };

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------
    println!("============================================");
    println!("  Results Summary");
    println!("============================================");
    println!();
    println!("  Sub-circuit proving times:");
    println!("    ECDSA (k={}):     {:>10}", ECDSA_K, fmt_duration(ecdsa_result.prove_time));
    println!("    Storage (k={}):   {:>10}", STORAGE_K, fmt_duration(storage_result.prove_time));
    println!("    Transfer (k={}):  {:>10}", TRANSFER_K, fmt_duration(transfer_result.prove_time));
    println!("    ────────────────────────────");
    println!("    Sub-total:        {:>10}", fmt_duration(sub_total_prove));
    println!();

    if let Some(ref agg) = agg_result {
        let total_prove = sub_total_prove + agg.prove_time;
        println!("  Aggregation (k={}):", args.agg_k);
        println!("    Keygen:           {:>10}", fmt_duration(agg.keygen_time));
        println!("    Proving:          {:>10}", fmt_duration(agg.prove_time));
        println!("    Final verify:     {:>10}", fmt_duration(agg.verify_time));
        println!("    Final proof:      {:>10}", fmt_bytes(agg.proof_size));
        println!();
        println!("  ────────────────────────────────────");
        println!("  Total proving:      {:>10}", fmt_duration(total_prove));
        println!("  Final verification: {:>10}", fmt_duration(agg.verify_time));
        println!();
        println!("  Comparison (1 transfer)");
        println!("  ─────────────────────────────────────────────────────");
        println!("  {:20} {:>12} {:>12} {:>12}", "", "EVM L2", "SP1 ZK-DEX", "Halo2 Full");
        println!("  ─────────────────────────────────────────────────────");
        println!(
            "  {:20} {:>12} {:>12} {:>12}",
            "Proving",
            "27m 44s",
            "3m 26s",
            fmt_duration(total_prove)
        );
        println!(
            "  {:20} {:>12} {:>12} {:>12}",
            "Verify",
            "\u{2014}",
            "229ms",
            fmt_duration(agg.verify_time)
        );
        println!(
            "  {:20} {:>12} {:>12} {:>12}",
            "Final proof",
            "\u{2014}",
            "\u{2014}",
            fmt_bytes(agg.proof_size)
        );
        println!("  ─────────────────────────────────────────────────────");
    } else {
        println!("  (Aggregation skipped — sub-circuit times only)");
        println!();
        println!("  Comparison (1 transfer, no aggregation)");
        println!("  ─────────────────────────────────────────────────────");
        println!("  {:20} {:>12} {:>12} {:>12}", "", "EVM L2", "SP1 ZK-DEX", "Halo2 Sub");
        println!("  ─────────────────────────────────────────────────────");
        println!(
            "  {:20} {:>12} {:>12} {:>12}",
            "Proving (sub-total)",
            "27m 44s",
            "3m 26s",
            fmt_duration(sub_total_prove)
        );
        println!("  ─────────────────────────────────────────────────────");
    }

    println!();
    println!("  Notes:");
    println!("  * SP1 = Groth16 wrap on M4 Max (Rosetta 2), 1 DEX transfer");
    println!("  * Halo2 = SHPLONK (KZG), same machine");
    println!("  * ECDSA: secp256k1 via halo2-ecc");
    println!("  * Storage: Poseidon-Merkle (depth 8), NOT keccak-MPT");
    println!("    (Poseidon is circuit-friendly => Halo2 advantage in this comparison)");
    if agg_result.is_some() {
        println!("  * Aggregation: VerifierUniversality::Full (k=18+14+10 -> k={})", args.agg_k);
    }
    println!("============================================");
}
