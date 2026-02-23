use halo2_base::gates::circuit::builder::BaseCircuitBuilder;
use halo2_base::gates::circuit::{BaseCircuitParams, CircuitBuilderStage};
use halo2_base::gates::flex_gate::MultiPhaseThreadBreakPoints;
use halo2_base::halo2_proofs::arithmetic::CurveAffine;
use halo2_base::halo2_proofs::halo2curves::bn256::{Bn256, Fr};
use halo2_base::halo2_proofs::halo2curves::ff::Field;
use halo2_base::halo2_proofs::halo2curves::secp256k1::{Fp, Fq, Secp256k1Affine};
use halo2_base::halo2_proofs::poly::kzg::commitment::ParamsKZG;
use halo2_base::utils::{biguint_to_fe, fe_to_biguint, modulus};
use halo2_base::AssignedValue;
use halo2_ecc::ecc::ecdsa::ecdsa_verify_no_pubkey_check;
use halo2_ecc::ecc::EccChip;
use halo2_ecc::fields::FieldChip;
use halo2_ecc::secp256k1::FpChip as Secp256k1FpChip;
use halo2_ecc::secp256k1::FqChip as Secp256k1FqChip;
use rand::rngs::StdRng;
use rand_core::SeedableRng;

pub const ECDSA_K: u32 = 18;
pub const ECDSA_LOOKUP_BITS: usize = 17;
pub const LIMB_BITS: usize = 88;
pub const NUM_LIMBS: usize = 3;

#[derive(Clone, Debug)]
pub struct ECDSAInput {
    pub r: Fq,
    pub s: Fq,
    pub msghash: Fq,
    pub pk: Secp256k1Affine,
}

pub fn random_ecdsa_input() -> ECDSAInput {
    let mut rng = StdRng::seed_from_u64(0);

    let sk = <Secp256k1Affine as CurveAffine>::ScalarExt::random(&mut rng);
    let pk = Secp256k1Affine::from(Secp256k1Affine::generator() * sk);
    let msghash = <Secp256k1Affine as CurveAffine>::ScalarExt::random(&mut rng);

    let k = <Secp256k1Affine as CurveAffine>::ScalarExt::random(&mut rng);
    let k_inv = k.invert().unwrap();

    let r_point = Secp256k1Affine::from(Secp256k1Affine::generator() * k)
        .coordinates()
        .unwrap();
    let x = *r_point.x();
    let x_bigint = fe_to_biguint(&x);
    let r = biguint_to_fe::<Fq>(&(x_bigint % modulus::<Fq>()));

    let s = k_inv * (msghash + (r * sk));

    ECDSAInput { r, s, msghash, pk }
}

fn ecdsa_circuit_logic(
    builder: &mut BaseCircuitBuilder<Fr>,
    input: &ECDSAInput,
    make_public: &mut Vec<AssignedValue<Fr>>,
) {
    let range = builder.range_chip();
    let ctx = builder.main(0);

    let fp_chip = Secp256k1FpChip::<Fr>::new(&range, LIMB_BITS, NUM_LIMBS);
    let fq_chip = Secp256k1FqChip::<Fr>::new(&range, LIMB_BITS, NUM_LIMBS);

    let [m, r, s] = [input.msghash, input.r, input.s].map(|x| fq_chip.load_private(ctx, x));
    let ecc_chip = EccChip::<Fr, Secp256k1FpChip<Fr>>::new(&fp_chip);
    let pk = ecc_chip.load_private_unchecked(ctx, (input.pk.x, input.pk.y));

    let res = ecdsa_verify_no_pubkey_check::<Fr, Fp, Fq, Secp256k1Affine>(
        &ecc_chip, ctx, pk, r, s, m, 4, 4,
    );

    make_public.push(res);
}

pub fn build_ecdsa_circuit(
    stage: CircuitBuilderStage,
    pinning: Option<(BaseCircuitParams, MultiPhaseThreadBreakPoints)>,
    _params: &ParamsKZG<Bn256>,
    input: &ECDSAInput,
) -> BaseCircuitBuilder<Fr> {
    let mut builder = BaseCircuitBuilder::from_stage(stage);

    if let Some((bp, break_points)) = pinning {
        builder.set_params(bp);
        builder.set_break_points(break_points);
    } else {
        builder.set_k(ECDSA_K as usize);
        builder.set_lookup_bits(ECDSA_LOOKUP_BITS);
        builder.set_instance_columns(1);
    }

    let mut assigned_instances = vec![];
    ecdsa_circuit_logic(&mut builder, input, &mut assigned_instances);
    if !assigned_instances.is_empty() {
        assert_eq!(builder.assigned_instances.len(), 1);
        builder.assigned_instances[0] = assigned_instances;
    }

    if !stage.witness_gen_only() {
        builder.calculate_params(Some(20));
    }

    builder
}
