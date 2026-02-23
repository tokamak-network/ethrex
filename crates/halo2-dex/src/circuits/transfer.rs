use halo2_base::gates::circuit::builder::BaseCircuitBuilder;
use halo2_base::gates::circuit::{BaseCircuitParams, CircuitBuilderStage};
use halo2_base::gates::flex_gate::MultiPhaseThreadBreakPoints;
use halo2_base::gates::{GateInstructions, RangeInstructions};
use halo2_base::halo2_proofs::halo2curves::bn256::{Bn256, Fr};
use halo2_base::halo2_proofs::halo2curves::ff::PrimeField;
use halo2_base::halo2_proofs::poly::kzg::commitment::ParamsKZG;
use halo2_base::AssignedValue;

pub const TRANSFER_K: u32 = 10;
pub const TRANSFER_LOOKUP_BITS: usize = 9;

#[derive(Clone, Debug)]
pub struct TransferInput {
    pub from_balance: String,
    pub to_balance: String,
    pub amount: String,
}

fn transfer_circuit_logic(
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

        let new_from = range.gate().sub(ctx, from_bal, amount);
        range.range_check(ctx, new_from, 64);

        let new_to = range.gate().add(ctx, to_bal, amount);
        range.range_check(ctx, new_to, 64);

        make_public.push(from_bal);
        make_public.push(to_bal);
        make_public.push(amount);
        make_public.push(new_from);
        make_public.push(new_to);
    }
}

pub fn generate_transfers(n: usize) -> Vec<TransferInput> {
    (0..n)
        .map(|i| TransferInput {
            from_balance: format!("{}", 10_000 + i * 100),
            to_balance: format!("{}", 500 + i * 50),
            amount: format!("{}", 100 + i * 10),
        })
        .collect()
}

pub fn build_transfer_circuit(
    stage: CircuitBuilderStage,
    pinning: Option<(BaseCircuitParams, MultiPhaseThreadBreakPoints)>,
    _params: &ParamsKZG<Bn256>,
    transfers: &[TransferInput],
) -> BaseCircuitBuilder<Fr> {
    let mut builder = BaseCircuitBuilder::from_stage(stage);

    if let Some((bp, break_points)) = pinning {
        builder.set_params(bp);
        builder.set_break_points(break_points);
    } else {
        builder.set_k(TRANSFER_K as usize);
        builder.set_lookup_bits(TRANSFER_LOOKUP_BITS);
        builder.set_instance_columns(1);
    }

    let mut assigned_instances = vec![];
    transfer_circuit_logic(&mut builder, transfers, &mut assigned_instances);
    if !assigned_instances.is_empty() {
        assert_eq!(builder.assigned_instances.len(), 1);
        builder.assigned_instances[0] = assigned_instances;
    }

    if !stage.witness_gen_only() {
        builder.calculate_params(Some(20));
    }

    builder
}
