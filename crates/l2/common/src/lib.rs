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

/// Official typeId range: 2–9 (matching GuestProgramRegistry.sol STORE_PROGRAM_START_ID = 10).
const OFFICIAL_TYPE_ID_MIN: u8 = 2;
const OFFICIAL_TYPE_ID_MAX: u8 = 9;

/// Parses a program spec that may include an explicit type ID.
///
/// Supported formats:
///   - `"my-app"` → Ok((programId="my-app", typeId=None)) — use registerProgram (auto-assign)
///   - `"my-app:5"` → Ok((programId="my-app", typeId=Some(5))) — use registerOfficialProgram
///   - `"zk-dex"` → resolved via `resolve_program_type_id` as before
///
/// Returns Err for:
///   - Reserved typeIds (0, 1): these are system-internal
///   - Out-of-range typeIds (≥10): these are auto-assigned by the contract
///   - Empty program IDs
pub fn parse_program_spec(spec: &str) -> Result<(String, Option<u8>), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("Empty program spec".to_string());
    }

    // Check for explicit "programId:typeId" format
    if let Some((id, type_str)) = spec.split_once(':') {
        let id = id.trim();
        let type_str = type_str.trim();
        if id.is_empty() {
            return Err(format!("Empty program ID in spec: {spec:?}"));
        }
        if let Ok(type_id) = type_str.parse::<u8>() {
            if type_id <= 1 {
                return Err(format!(
                    "Reserved typeId {type_id} in spec {spec:?}. TypeIds 0 and 1 are system-internal."
                ));
            }
            if type_id > OFFICIAL_TYPE_ID_MAX {
                return Err(format!(
                    "typeId {type_id} in spec {spec:?} is out of official range ({OFFICIAL_TYPE_ID_MIN}-{OFFICIAL_TYPE_ID_MAX}). \
                     Community programs get auto-assigned typeIds (10+). Use \"{id}\" without a suffix."
                ));
            }
            return Ok((id.to_string(), Some(type_id)));
        }
        // Non-numeric suffix — treat as part of program name (e.g., "my-app:v2" is invalid)
        return Err(format!(
            "Invalid typeId suffix in spec {spec:?}. Expected format: \"programId:typeId\" with numeric typeId."
        ));
    }
    // No colon — check known programs
    let known = resolve_program_type_id(spec);
    if known > 0 {
        Ok((spec.to_string(), Some(known)))
    } else {
        Ok((spec.to_string(), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_known_programs() {
        assert_eq!(resolve_program_type_id("evm-l2"), 1);
        assert_eq!(resolve_program_type_id("zk-dex"), 2);
        assert_eq!(resolve_program_type_id("tokamon"), 3);
        assert_eq!(resolve_program_type_id("unknown"), 0);
    }

    #[test]
    fn test_parse_known_program() {
        assert_eq!(
            parse_program_spec("zk-dex").unwrap(),
            ("zk-dex".into(), Some(2))
        );
        assert_eq!(
            parse_program_spec("evm-l2").unwrap(),
            ("evm-l2".into(), Some(1))
        );
    }

    #[test]
    fn test_parse_community_program() {
        assert_eq!(
            parse_program_spec("my-app").unwrap(),
            ("my-app".into(), None)
        );
    }

    #[test]
    fn test_parse_explicit_type_id_in_range() {
        // Official range: 2-9
        assert_eq!(
            parse_program_spec("custom:5").unwrap(),
            ("custom".into(), Some(5))
        );
        assert_eq!(
            parse_program_spec("custom:9").unwrap(),
            ("custom".into(), Some(9))
        );
    }

    #[test]
    fn test_parse_explicit_type_id_out_of_range() {
        // typeId >= 10 is auto-assigned by contract, not valid for registerOfficialProgram
        assert!(parse_program_spec("my-app:10").is_err());
        assert!(parse_program_spec("my-app:255").is_err());
    }

    #[test]
    fn test_parse_rejects_reserved_type_ids() {
        // typeId 0 and 1 are system-internal — must be rejected
        assert!(parse_program_spec("bad:0").is_err());
        assert!(parse_program_spec("bad:1").is_err());
    }

    #[test]
    fn test_parse_invalid_type_id() {
        // Non-numeric suffix — error
        assert!(parse_program_spec("my-app:abc").is_err());
    }

    #[test]
    fn test_parse_trims_whitespace() {
        assert_eq!(
            parse_program_spec("  my-app  ").unwrap(),
            ("my-app".into(), None)
        );
        assert_eq!(
            parse_program_spec("  custom : 5  ").unwrap(),
            ("custom".into(), Some(5))
        );
    }

    #[test]
    fn test_parse_rejects_empty() {
        assert!(parse_program_spec("").is_err());
        assert!(parse_program_spec("  ").is_err());
    }
}
