use halo2_base::gates::circuit::builder::BaseCircuitBuilder;
use halo2_base::gates::circuit::{BaseCircuitParams, CircuitBuilderStage};
use halo2_base::gates::flex_gate::MultiPhaseThreadBreakPoints;
use halo2_base::gates::{GateInstructions, RangeInstructions};
use halo2_base::halo2_proofs::halo2curves::bn256::{Bn256, Fr};
use halo2_base::halo2_proofs::poly::kzg::commitment::ParamsKZG;
use halo2_base::poseidon::hasher::spec::OptimizedPoseidonSpec;
use halo2_base::poseidon::hasher::PoseidonHasher;
use halo2_base::AssignedValue;

pub const STORAGE_K: u32 = 14;
pub const STORAGE_LOOKUP_BITS: usize = 13;
pub const MERKLE_DEPTH: usize = 8;

#[derive(Clone, Debug)]
pub struct StorageProofInput {
    pub sender_balance: u64,
    pub receiver_balance: u64,
    pub amount: u64,
    /// Merkle path index bits for sender (LSB first, MERKLE_DEPTH bits)
    pub sender_index: u64,
    /// Merkle path index bits for receiver
    pub receiver_index: u64,
    /// Sibling hashes along sender's Merkle path
    pub sender_siblings: Vec<Fr>,
    /// Sibling hashes along receiver's Merkle path
    pub receiver_siblings: Vec<Fr>,
    /// Precomputed old Merkle root
    pub old_root: Fr,
}

fn poseidon_hash_two(
    hasher: &PoseidonHasher<Fr, 3, 2>,
    ctx: &mut halo2_base::Context<Fr>,
    gate: &impl GateInstructions<Fr>,
    left: AssignedValue<Fr>,
    right: AssignedValue<Fr>,
) -> AssignedValue<Fr> {
    hasher.hash_fix_len_array(ctx, gate, &[left, right])
}

fn compute_merkle_root(
    hasher: &PoseidonHasher<Fr, 3, 2>,
    ctx: &mut halo2_base::Context<Fr>,
    gate: &impl GateInstructions<Fr>,
    leaf: AssignedValue<Fr>,
    index_bits: &[AssignedValue<Fr>],
    siblings: &[AssignedValue<Fr>],
) -> AssignedValue<Fr> {
    let mut current = leaf;
    for i in 0..siblings.len() {
        let bit = index_bits[i];
        // if bit == 0: hash(current, sibling), else: hash(sibling, current)
        let left = gate.select(ctx, siblings[i], current, bit);
        let right = gate.select(ctx, current, siblings[i], bit);
        current = poseidon_hash_two(hasher, ctx, gate, left, right);
    }
    current
}

fn storage_circuit_logic(
    builder: &mut BaseCircuitBuilder<Fr>,
    input: &StorageProofInput,
    make_public: &mut Vec<AssignedValue<Fr>>,
) {
    let range = builder.range_chip();
    let gate = range.gate().clone();
    let ctx = builder.main(0);

    // Initialize Poseidon hasher (T=3, RATE=2 for 2-to-1 hashing)
    let spec = OptimizedPoseidonSpec::<Fr, 3, 2>::new::<8, 57, 0>();
    let mut hasher = PoseidonHasher::<Fr, 3, 2>::new(spec);
    hasher.initialize_consts(ctx, &gate);

    // Load witnesses
    let sender_bal = ctx.load_witness(Fr::from(input.sender_balance));
    let receiver_bal = ctx.load_witness(Fr::from(input.receiver_balance));
    let amount = ctx.load_witness(Fr::from(input.amount));
    let old_root = ctx.load_witness(input.old_root);

    // Load sender index bits (MERKLE_DEPTH bits)
    let sender_index_bits: Vec<AssignedValue<Fr>> = (0..MERKLE_DEPTH)
        .map(|i| {
            let bit = (input.sender_index >> i) & 1;
            ctx.load_witness(Fr::from(bit))
        })
        .collect();

    // Load receiver index bits
    let receiver_index_bits: Vec<AssignedValue<Fr>> = (0..MERKLE_DEPTH)
        .map(|i| {
            let bit = (input.receiver_index >> i) & 1;
            ctx.load_witness(Fr::from(bit))
        })
        .collect();

    // Constrain bits to be 0 or 1
    for bit in sender_index_bits.iter().chain(receiver_index_bits.iter()) {
        gate.assert_bit(ctx, *bit);
    }

    // Load siblings
    let sender_siblings: Vec<AssignedValue<Fr>> = input
        .sender_siblings
        .iter()
        .map(|s| ctx.load_witness(*s))
        .collect();
    let receiver_siblings: Vec<AssignedValue<Fr>> = input
        .receiver_siblings
        .iter()
        .map(|s| ctx.load_witness(*s))
        .collect();

    // Sender leaf = hash(sender_balance)
    let sender_leaf = hasher.hash_fix_len_array(ctx, &gate, &[sender_bal]);

    // Verify sender inclusion: compute root from sender leaf
    let computed_root_sender = compute_merkle_root(
        &hasher,
        ctx,
        &gate,
        sender_leaf,
        &sender_index_bits,
        &sender_siblings,
    );

    // Constrain: computed sender root == old_root
    ctx.constrain_equal(&computed_root_sender, &old_root);

    // Receiver leaf = hash(receiver_balance)
    let receiver_leaf = hasher.hash_fix_len_array(ctx, &gate, &[receiver_bal]);

    // Verify receiver inclusion
    let computed_root_receiver = compute_merkle_root(
        &hasher,
        ctx,
        &gate,
        receiver_leaf,
        &receiver_index_bits,
        &receiver_siblings,
    );
    ctx.constrain_equal(&computed_root_receiver, &old_root);

    // Compute updated balances
    let new_sender_bal = gate.sub(ctx, sender_bal, amount);
    range.range_check(ctx, new_sender_bal, 64);

    let new_receiver_bal = gate.add(ctx, receiver_bal, amount);
    range.range_check(ctx, new_receiver_bal, 64);

    // Compute new sender leaf and new root
    let new_sender_leaf = hasher.hash_fix_len_array(ctx, &gate, &[new_sender_bal]);

    // Recompute root after sender update
    let _transitional_root = compute_merkle_root(
        &hasher,
        ctx,
        &gate,
        new_sender_leaf,
        &sender_index_bits,
        &sender_siblings,
    );

    // Compute new receiver leaf and root
    let new_receiver_leaf = hasher.hash_fix_len_array(ctx, &gate, &[new_receiver_bal]);
    let new_root = compute_merkle_root(
        &hasher,
        ctx,
        &gate,
        new_receiver_leaf,
        &receiver_index_bits,
        &receiver_siblings,
    );

    // Make old_root and new_root public
    make_public.push(old_root);
    make_public.push(new_root);
}

/// Generate consistent test input with valid Merkle proofs.
/// Uses an in-circuit Poseidon hasher (in Mock mode) to compute native hash values.
pub fn generate_storage_input() -> StorageProofInput {
    let sender_balance: u64 = 10_000;
    let receiver_balance: u64 = 500;
    let amount: u64 = 100;
    let sender_index: u64 = 3;
    let receiver_index: u64 = 200;

    let num_leaves = 1u64 << MERKLE_DEPTH; // 256

    // Use a throwaway BaseCircuitBuilder in Mock mode to compute Poseidon hashes natively
    let mut builder = BaseCircuitBuilder::<Fr>::from_stage(CircuitBuilderStage::Mock);
    builder.set_k(10);
    builder.set_lookup_bits(9);
    builder.set_instance_columns(1);

    let range = builder.range_chip();
    let gate = range.gate().clone();
    let ctx = builder.main(0);

    let spec = OptimizedPoseidonSpec::<Fr, 3, 2>::new::<8, 57, 0>();
    let mut hasher = PoseidonHasher::<Fr, 3, 2>::new(spec);
    hasher.initialize_consts(ctx, &gate);

    // Build leaves: leaf_i = poseidon_hash(balance_i)
    let mut leaf_values = Vec::with_capacity(num_leaves as usize);
    for i in 0..num_leaves {
        let val = if i == sender_index {
            Fr::from(sender_balance)
        } else if i == receiver_index {
            Fr::from(receiver_balance)
        } else {
            Fr::from(i + 1000)
        };
        let assigned = ctx.load_witness(val);
        let leaf = hasher.hash_fix_len_array(ctx, &gate, &[assigned]);
        leaf_values.push(*leaf.value());
    }

    // Build tree layers bottom-up
    let mut layers: Vec<Vec<Fr>> = vec![leaf_values];
    for depth in 0..MERKLE_DEPTH {
        let prev = &layers[depth];
        let mut next = Vec::with_capacity(prev.len() / 2);
        for j in (0..prev.len()).step_by(2) {
            let l = ctx.load_witness(prev[j]);
            let r = ctx.load_witness(prev[j + 1]);
            let h = hasher.hash_fix_len_array(ctx, &gate, &[l, r]);
            next.push(*h.value());
        }
        layers.push(next);
    }

    let old_root = layers[MERKLE_DEPTH][0];

    // Extract siblings for sender and receiver
    let sender_siblings = extract_siblings(&layers, sender_index as usize);
    let receiver_siblings = extract_siblings(&layers, receiver_index as usize);

    StorageProofInput {
        sender_balance,
        receiver_balance,
        amount,
        sender_index,
        receiver_index,
        sender_siblings,
        receiver_siblings,
        old_root,
    }
}

fn extract_siblings(layers: &[Vec<Fr>], index: usize) -> Vec<Fr> {
    let mut siblings = Vec::with_capacity(MERKLE_DEPTH);
    let mut idx = index;
    for depth in 0..MERKLE_DEPTH {
        let sibling_idx = idx ^ 1;
        siblings.push(layers[depth][sibling_idx]);
        idx >>= 1;
    }
    siblings
}

pub fn build_storage_circuit(
    stage: CircuitBuilderStage,
    pinning: Option<(BaseCircuitParams, MultiPhaseThreadBreakPoints)>,
    _params: &ParamsKZG<Bn256>,
    input: &StorageProofInput,
) -> BaseCircuitBuilder<Fr> {
    let mut builder = BaseCircuitBuilder::from_stage(stage);

    if let Some((bp, break_points)) = pinning {
        builder.set_params(bp);
        builder.set_break_points(break_points);
    } else {
        builder.set_k(STORAGE_K as usize);
        builder.set_lookup_bits(STORAGE_LOOKUP_BITS);
        builder.set_instance_columns(1);
    }

    let mut assigned_instances = vec![];
    storage_circuit_logic(&mut builder, input, &mut assigned_instances);
    if !assigned_instances.is_empty() {
        assert_eq!(builder.assigned_instances.len(), 1);
        builder.assigned_instances[0] = assigned_instances;
    }

    if !stage.witness_gen_only() {
        builder.calculate_params(Some(20));
    }

    builder
}
