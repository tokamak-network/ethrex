pub mod calldata;
pub mod merkle_tree;
pub mod messages;
pub mod privileged_transactions;
pub mod prover;
pub mod sequencer_state;
pub mod utils;

/// Maps a guest program ID string to its on-chain `programTypeId`.
///
/// This is the single source of truth for program type IDs used by
/// the deployer, committer, and proof sender.
///
/// Official programs have fixed type IDs (1-9).
/// Returns 0 for unknown programs — the deployer should use
/// `registerProgram()` (community, auto-assign typeId 10+) for these.
pub fn resolve_program_type_id(program_id: &str) -> u8 {
    match program_id {
        "evm-l2" => 1,
        "zk-dex" => 2,
        "tokamon" => 3,
        _ => 0,
    }
}

/// Parses a program spec that may include an explicit type ID.
///
/// Supported formats:
///   - `"my-app"` → (programId="my-app", typeId=None) — use registerProgram (auto-assign)
///   - `"my-app:10"` → (programId="my-app", typeId=Some(10)) — use registerOfficialProgram
///   - `"zk-dex"` → resolved via `resolve_program_type_id` as before
pub fn parse_program_spec(spec: &str) -> (String, Option<u8>) {
    if let Some((id, type_str)) = spec.split_once(':') {
        if let Ok(type_id) = type_str.parse::<u8>() {
            return (id.to_string(), Some(type_id));
        }
    }
    let known = resolve_program_type_id(spec);
    if known > 0 {
        (spec.to_string(), Some(known))
    } else {
        (spec.to_string(), None)
    }
}
