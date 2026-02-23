use clap::Parser;
use halo2_base::{
    gates::{
        circuit::{builder::BaseCircuitBuilder, BaseCircuitParams, CircuitBuilderStage},
        flex_gate::MultiPhaseThreadBreakPoints,
        GateInstructions, RangeInstructions,
    },
    halo2_proofs::{
        dev::MockProver,
        halo2curves::{
            bn256::{Bn256, Fr},
            ff::PrimeField,
        },
        plonk::{verify_proof, Circuit},
        poly::{
            commitment::{Params, ParamsProver},
            kzg::{
                commitment::{KZGCommitmentScheme, ParamsKZG},
                multiopen::VerifierSHPLONK,
                strategy::SingleStrategy,
            },
        },
    },
    utils::fs::gen_srs,
    AssignedValue,
};
use snark_verifier_sdk::{
    gen_pk,
    halo2::{gen_snark_shplonk, PoseidonTranscript},
    CircuitExt, NativeLoader,
};
use std::time::Instant;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "halo2-dex-benchmark")]
#[command(about = "Benchmark Halo2 DEX transfer circuit (keygen / prove / verify)")]
struct Args {
    /// Circuit degree k (circuit has 2^k rows)
    #[arg(short = 'k', long = "degree", default_value = "10")]
    degree: u32,

    /// Number of transfers to batch in a single proof
    #[arg(short = 'n', long = "transfers", default_value = "1")]
    num_transfers: usize,
}

// ---------------------------------------------------------------------------
// Circuit
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct TransferInput {
    from_balance: String,
    to_balance: String,
    amount: String,
}

fn dex_transfer_circuit(
    builder: &mut BaseCircuitBuilder<Fr>,
    transfers: &[TransferInput],
    make_public: &mut Vec<AssignedValue<Fr>>,
) {
    let range = builder.range_chip();
    let ctx = builder.main(0);

    for t in transfers {
        let from_bal = ctx.load_witness(Fr::from_str_vartime(&t.from_balance).unwrap());
        let to_bal = ctx.load_witness(Fr::from_str_vartime(&t.to_balance).unwrap());
        let amount = ctx.load_witness(Fr::from_str_vartime(&t.amount).unwrap());

        // new_from = from_balance - amount  (range-check proves no underflow)
        let new_from = range.gate().sub(ctx, from_bal, amount);
        range.range_check(ctx, new_from, 64);

        // new_to = to_balance + amount  (range-check proves no overflow)
        let new_to = range.gate().add(ctx, to_bal, amount);
        range.range_check(ctx, new_to, 64);

        make_public.push(from_bal);
        make_public.push(to_bal);
        make_public.push(amount);
        make_public.push(new_from);
        make_public.push(new_to);
    }
}

fn build_circuit(
    stage: CircuitBuilderStage,
    pinning: Option<(BaseCircuitParams, MultiPhaseThreadBreakPoints)>,
    params: &ParamsKZG<Bn256>,
    transfers: &[TransferInput],
) -> BaseCircuitBuilder<Fr> {
    let mut builder = BaseCircuitBuilder::from_stage(stage);

    if let Some((bp, break_points)) = pinning {
        builder.set_params(bp);
        builder.set_break_points(break_points);
    } else {
        let k = params.k() as usize;
        builder.set_k(k);
        builder.set_lookup_bits(k - 1);
        builder.set_instance_columns(1);
    }

    let mut assigned_instances = vec![];
    dex_transfer_circuit(&mut builder, transfers, &mut assigned_instances);
    if !assigned_instances.is_empty() {
        assert_eq!(builder.assigned_instances.len(), 1);
        builder.assigned_instances[0] = assigned_instances;
    }

    if !stage.witness_gen_only() {
        builder.calculate_params(Some(20));
    }

    builder
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn generate_transfers(n: usize) -> Vec<TransferInput> {
    (0..n)
        .map(|i| TransferInput {
            from_balance: format!("{}", 10_000 + i * 100),
            to_balance: format!("{}", 500 + i * 50),
            amount: format!("{}", 100 + i * 10),
        })
        .collect()
}

fn fmt_duration(d: std::time::Duration) -> String {
    if d.as_secs() >= 60 {
        format!("{}m {:.2}s", d.as_secs() / 60, (d.as_millis() % 60_000) as f64 / 1000.0)
    } else if d.as_millis() >= 1000 {
        format!("{:.2}s", d.as_millis() as f64 / 1000.0)
    } else {
        format!("{}ms", d.as_millis())
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    env_logger::init();
    let args = Args::parse();
    let k = args.degree;
    let n = args.num_transfers;
    let transfers = generate_transfers(n);

    println!("============================================");
    println!("  Halo2 DEX Transfer Benchmark");
    println!("============================================");
    println!("  Circuit degree (k): {}", k);
    println!("  Number of rows:     {}", 1u64 << k);
    println!("  Transfers / proof:  {}", n);
    println!("============================================\n");

    // 1. SRS generation (cached to ./params/)
    print!("[1/5] SRS generation ...");
    let start = Instant::now();
    let params = gen_srs(k);
    let srs_time = start.elapsed();
    println!(" {}", fmt_duration(srs_time));

    // 2. Mock proof – sanity check that the circuit is satisfiable
    print!("[2/5] Mock proof (sanity check) ...");
    let start = Instant::now();
    let circuit = build_circuit(CircuitBuilderStage::Mock, None, &params, &transfers);
    MockProver::run(k, &circuit, circuit.instances())
        .unwrap()
        .assert_satisfied();
    let mock_time = start.elapsed();
    println!(" OK ({})", fmt_duration(mock_time));

    // 3. Key generation (pk + vk)
    //    break_points are set during gen_pk → synthesize (interior mutability)
    print!("[3/5] Key generation ...");
    let start = Instant::now();
    let circuit = build_circuit(CircuitBuilderStage::Keygen, None, &params, &transfers);
    let pk = gen_pk(&params, &circuit, None);
    let c_params = circuit.params();
    let break_points = circuit.break_points();
    let keygen_time = start.elapsed();
    println!(" {}", fmt_duration(keygen_time));
    println!("       params: {:?}", c_params);

    // 4. Proof generation (SHPLONK)
    print!("[4/5] Proof generation ...");
    let start = Instant::now();
    let circuit = build_circuit(
        CircuitBuilderStage::Prover,
        Some((c_params, break_points)),
        &params,
        &transfers,
    );
    let snark = gen_snark_shplonk(&params, &pk, circuit, None::<String>);
    let prove_time = start.elapsed();
    println!(" {}", fmt_duration(prove_time));
    println!("       proof size: {} bytes", snark.proof.len());

    // 5. Verification
    print!("[5/5] Verification ...");
    let verifier_params = params.verifier_params();
    let strategy = SingleStrategy::new(&params);
    let mut transcript =
        PoseidonTranscript::<NativeLoader, &[u8]>::new::<0>(&snark.proof[..]);
    let instance = &snark.instances[0][..];

    let start = Instant::now();
    verify_proof::<
        KZGCommitmentScheme<Bn256>,
        VerifierSHPLONK<'_, Bn256>,
        _,
        _,
        SingleStrategy<'_, Bn256>,
    >(verifier_params, pk.get_vk(), strategy, &[&[instance]], &mut transcript)
        .expect("verification failed");
    let verify_time = start.elapsed();
    println!(" {}", fmt_duration(verify_time));

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------
    let total = srs_time + keygen_time + prove_time + verify_time;

    println!();
    println!("============================================");
    println!("  Results Summary  (k={}, n={})", k, n);
    println!("============================================");
    println!("  SRS generation:   {:>12}", fmt_duration(srs_time));
    println!("  Key generation:   {:>12}", fmt_duration(keygen_time));
    println!("  Proof generation: {:>12}", fmt_duration(prove_time));
    println!("  Verification:     {:>12}", fmt_duration(verify_time));
    println!("  ─────────────────────────────");
    println!("  Total:            {:>12}", fmt_duration(total));
    println!("  Proof size:       {:>9} B", snark.proof.len());
    println!("  Public inputs:    {:>9}", snark.instances[0].len());
    println!("============================================");
    println!();
    println!("  3-way Comparison (1 transfer)");
    println!("  ─────────────────────────────────────────────────────");
    println!("  {:20} {:>12} {:>12} {:>12}", "", "EVM L2", "SP1 ZK-DEX", "Halo2");
    println!("  ─────────────────────────────────────────────────────");
    println!("  {:20} {:>12} {:>12} {:>12}",
        "Proving time", "27m 44s", "3m 26s", fmt_duration(prove_time));
    println!("  {:20} {:>12} {:>12} {:>12}",
        "Verification", "—", "229ms", fmt_duration(verify_time));
    println!("  {:20} {:>12} {:>12} {:>9} B",
        "Proof size", "—", "—", snark.proof.len());
    println!("  {:20} {:>12} {:>12} {:>12}",
        "Execution cycles", "65,360,896", "357,761", "N/A");
    println!("  ─────────────────────────────────────────────────────");
    println!("  * SP1 = Groth16 on M4 Max (Rosetta 2), 1 DEX transfer");
    println!("  * Halo2 = SHPLONK (KZG), same machine, pure transfer circuit");
    println!("============================================");
}
