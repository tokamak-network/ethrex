use clap::Parser;
use halo2_base::gates::circuit::builder::BaseCircuitBuilder;
use halo2_base::gates::{GateInstructions, RangeInstructions};
use halo2_base::halo2_proofs::halo2curves::bn256::Fr;
use halo2_base::halo2_proofs::halo2curves::ff::PrimeField;
use halo2_base::AssignedValue;
use halo2_scaffold::scaffold::cmd::Cli;
use halo2_scaffold::scaffold::run;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitInput {
    pub from_balance: String,
    pub to_balance: String,
    pub amount: String,
}

fn dex_transfer_circuit(
    builder: &mut BaseCircuitBuilder<Fr>,
    input: CircuitInput,
    make_public: &mut Vec<AssignedValue<Fr>>,
) {
    let range = builder.range_chip();
    let ctx = builder.main(0);

    // Load witnesses from input
    let from_bal = ctx.load_witness(Fr::from_str_vartime(&input.from_balance).unwrap());
    let to_bal = ctx.load_witness(Fr::from_str_vartime(&input.to_balance).unwrap());
    let amount = ctx.load_witness(Fr::from_str_vartime(&input.amount).unwrap());

    // Compute new_from = from_balance - amount
    let new_from = range.gate().sub(ctx, from_bal, amount);

    // Range check: new_from fits in 64 bits (ensures from_balance >= amount)
    range.range_check(ctx, new_from, 64);

    // Compute new_to = to_balance + amount
    let new_to = range.gate().add(ctx, to_bal, amount);

    // Range check: new_to fits in 64 bits (no overflow)
    range.range_check(ctx, new_to, 64);

    // Make inputs and outputs public
    make_public.push(from_bal);
    make_public.push(to_bal);
    make_public.push(amount);
    make_public.push(new_from);
    make_public.push(new_to);
}

fn main() {
    env_logger::init();
    let args = Cli::parse();
    run(dex_transfer_circuit, args);
}
