use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use ethrex_common::U256;
use ethrex_guest_program::input::ProgramInput;
use ethrex_guest_program::traits::backends;
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProofFormat, ProverType},
};

use crate::backend::{BackendError, ProverBackend};

// ── Proof / preprocess output structures ────────────────────────────

/// Tokamak-zk-EVM proof output (parsed from proof.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedProof {
    /// 38 hex strings representing uint128 values.
    pub proof_entries_part1: Vec<String>,
    /// 42 hex strings representing uint256 values.
    pub proof_entries_part2: Vec<String>,
}

/// Tokamak-zk-EVM preprocess output (parsed from preprocess.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedPreprocess {
    /// Hex strings representing uint128 values.
    pub preprocess_entries_part1: Vec<String>,
    /// Hex strings representing uint256 values.
    pub preprocess_entries_part2: Vec<String>,
}

/// Combined output of the Tokamak proving pipeline.
pub struct TokamakProveOutput {
    pub proof: FormattedProof,
    pub preprocess: FormattedPreprocess,
    /// Public inputs as hex strings (uint256 values).
    pub public_inputs: Vec<String>,
    pub smax: u64,
}

// ── TokamakBackend ──────────────────────────────────────────────────

/// Prover backend for Tokamak-zk-EVM.
///
/// Tokamak-zk-EVM is a custom zkSNARK system (not a zkVM like SP1/RISC0).
/// It uses an external CLI (`tokamak-cli`) with a three-stage pipeline:
///   synthesize → preprocess → prove
///
/// Because the CLI has heavy GPU/ICICLE dependencies, we invoke it as an
/// external process rather than linking it as a library.
pub struct TokamakBackend {
    /// Path to the `tokamak-cli` binary.
    cli_path: PathBuf,
    /// Resource directory containing QAP, setup parameters, etc.
    resource_dir: PathBuf,
}

impl TokamakBackend {
    pub fn new(cli_path: PathBuf, resource_dir: PathBuf) -> Self {
        Self {
            cli_path,
            resource_dir,
        }
    }

    /// Run the full Tokamak proving pipeline.
    ///
    /// 1. Write ProgramInput as a Tokamak config JSON to a temp directory.
    /// 2. `tokamak-cli --synthesize <config>`
    /// 3. `tokamak-cli --preprocess`
    /// 4. (if `prove`) `tokamak-cli --prove`
    /// 5. Parse `proof.json` and `preprocess.json` from the output.
    fn run_pipeline(
        &self,
        input: &ProgramInput,
        prove: bool,
    ) -> Result<TokamakProveOutput, BackendError> {
        let work_dir = tempfile::tempdir().map_err(|e| {
            BackendError::proving(format!("failed to create temp dir: {e}"))
        })?;

        // 1. Write input config
        let config_path = work_dir.path().join("config.json");
        self.write_input_config(input, &config_path)?;

        // 2. Synthesize
        info!("Tokamak: running synthesize");
        self.run_cli(&[
            "--synthesize",
            &config_path.to_string_lossy(),
            "--output-dir",
            &work_dir.path().to_string_lossy(),
        ])?;

        // 3. Preprocess
        info!("Tokamak: running preprocess");
        self.run_cli(&[
            "--preprocess",
            "--work-dir",
            &work_dir.path().to_string_lossy(),
        ])?;

        if prove {
            // 4. Prove
            info!("Tokamak: running prove");
            self.run_cli(&[
                "--prove",
                "--work-dir",
                &work_dir.path().to_string_lossy(),
            ])?;

            // 5. Parse outputs
            let proof = self.read_proof(work_dir.path())?;
            let preprocess = self.read_preprocess(work_dir.path())?;
            let (public_inputs, smax) = self.extract_public_inputs(work_dir.path())?;

            Ok(TokamakProveOutput {
                proof,
                preprocess,
                public_inputs,
                smax,
            })
        } else {
            Err(BackendError::not_implemented(
                "execute-only mode: pipeline ran synthesize+preprocess without proving",
            ))
        }
    }

    /// Invoke `tokamak-cli` with the given arguments.
    fn run_cli(&self, args: &[&str]) -> Result<(), BackendError> {
        let output = Command::new(&self.cli_path)
            .args(args)
            .current_dir(&self.resource_dir)
            .output()
            .map_err(|e| {
                BackendError::proving(format!(
                    "failed to execute tokamak-cli at {}: {e}",
                    self.cli_path.display()
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(BackendError::proving(format!(
                "tokamak-cli failed (exit {:?}):\nstderr: {stderr}\nstdout: {stdout}",
                output.status.code()
            )));
        }

        Ok(())
    }

    /// Serialize ProgramInput into a Tokamak-compatible config JSON.
    ///
    /// TODO: The exact mapping from ethrex block data to Tokamak circuit
    /// inputs depends on the Tokamak-zk-EVM synthesizer input format.
    /// For now we write a minimal JSON with the block data.
    fn write_input_config(
        &self,
        input: &ProgramInput,
        config_path: &Path,
    ) -> Result<(), BackendError> {
        let json = serde_json::to_string_pretty(input).map_err(|e| {
            BackendError::serialization(format!("failed to serialize ProgramInput: {e}"))
        })?;
        std::fs::write(config_path, json).map_err(|e| {
            BackendError::serialization(format!("failed to write config: {e}"))
        })?;
        Ok(())
    }

    /// Read and parse `proof.json` from the work directory.
    fn read_proof(&self, work_dir: &Path) -> Result<FormattedProof, BackendError> {
        let path = work_dir.join("proof.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;
        serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse proof.json: {e}"))
        })
    }

    /// Read and parse `preprocess.json` from the work directory.
    fn read_preprocess(&self, work_dir: &Path) -> Result<FormattedPreprocess, BackendError> {
        let path = work_dir.join("preprocess.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;
        serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse preprocess.json: {e}"))
        })
    }

    /// Extract public inputs and smax from the work directory.
    ///
    /// Reads `public_inputs.json` which contains:
    /// ```json
    /// { "public_inputs": ["0x...", ...], "smax": 12345 }
    /// ```
    fn extract_public_inputs(
        &self,
        work_dir: &Path,
    ) -> Result<(Vec<String>, u64), BackendError> {
        let path = work_dir.join("public_inputs.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;

        #[derive(Deserialize)]
        struct PublicInputsFile {
            public_inputs: Vec<String>,
            smax: u64,
        }

        let parsed: PublicInputsFile = serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse public_inputs.json: {e}"))
        })?;

        Ok((parsed.public_inputs, parsed.smax))
    }
}

// ── ProverBackend implementation ────────────────────────────────────

impl ProverBackend for TokamakBackend {
    type ProofOutput = TokamakProveOutput;
    type SerializedInput = ();

    fn prover_type(&self) -> ProverType {
        ProverType::Tokamak
    }

    fn backend_name(&self) -> &'static str {
        backends::TOKAMAK
    }

    fn serialize_input(&self, _input: &ProgramInput) -> Result<(), BackendError> {
        // Tokamak uses its own input format; serialization happens in run_pipeline.
        Ok(())
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        // Run synthesize + preprocess only (no proof generation).
        // run_pipeline returns Err(NotImplemented) for execute-only, which is
        // the expected "no output" signal.
        match self.run_pipeline(&input, false) {
            Ok(_) => Ok(()),
            Err(BackendError::NotImplemented(_)) => {
                info!("Tokamak: execute-only pipeline completed (synthesize + preprocess)");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn prove(
        &self,
        input: ProgramInput,
        _format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        self.run_pipeline(&input, true)
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        // On-chain verification is handled by the TokamakVerifier contract.
        // Local verification could invoke `tokamak-cli --verify` in the future.
        warn!("Tokamak: local verify not yet implemented, relying on on-chain verification");
        Ok(())
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        let encoded = abi_encode_tokamak_proof(&proof)?;
        Ok(BatchProof::ProofCalldata(ProofCalldata {
            prover_type: ProverType::Tokamak,
            calldata: vec![Value::Bytes(encoded.into())],
        }))
    }
}

// ── ABI encoding ────────────────────────────────────────────────────

/// Parse a hex string (with or without "0x" prefix) into a U256.
fn parse_hex_u256(hex: &str) -> Result<U256, BackendError> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    U256::from_str_radix(stripped, 16)
        .map_err(|e| BackendError::batch_proof(format!("invalid hex value '{hex}': {e}")))
}

/// ABI-encode the 6 Tokamak proof parameters into raw bytes.
///
/// The encoding matches the Solidity signature:
/// ```solidity
/// abi.encode(
///     uint256[] proof_part1,      // u128 values stored as u256
///     uint256[] proof_part2,
///     uint256[] preprocess_part1, // u128 values stored as u256
///     uint256[] preprocess_part2,
///     uint256[] publicInputs,
///     uint256   smax
/// )
/// ```
///
/// We use a manual ABI encoding that matches Solidity's `abi.encode` for
/// dynamic types. Each dynamic array is:
///   - offset pointer (32 bytes) in the head
///   - length (32 bytes) + elements (32 bytes each) at the offset
fn abi_encode_tokamak_proof(output: &TokamakProveOutput) -> Result<Vec<u8>, BackendError> {
    let proof_part1: Vec<U256> = output
        .proof
        .proof_entries_part1
        .iter()
        .map(|h| parse_hex_u256(h))
        .collect::<Result<_, _>>()?;

    let proof_part2: Vec<U256> = output
        .proof
        .proof_entries_part2
        .iter()
        .map(|h| parse_hex_u256(h))
        .collect::<Result<_, _>>()?;

    let preprocess_part1: Vec<U256> = output
        .preprocess
        .preprocess_entries_part1
        .iter()
        .map(|h| parse_hex_u256(h))
        .collect::<Result<_, _>>()?;

    let preprocess_part2: Vec<U256> = output
        .preprocess
        .preprocess_entries_part2
        .iter()
        .map(|h| parse_hex_u256(h))
        .collect::<Result<_, _>>()?;

    let public_inputs: Vec<U256> = output
        .public_inputs
        .iter()
        .map(|h| parse_hex_u256(h))
        .collect::<Result<_, _>>()?;

    let smax = U256::from(output.smax);

    // ABI encode: 6 parameters where 5 are dynamic (uint256[]) and 1 is static (uint256).
    // Head: 6 × 32 bytes (offsets for dynamic, value for static).
    // Tail: each array = 32 bytes (length) + N × 32 bytes (elements).
    let arrays: [&[U256]; 5] = [
        &proof_part1,
        &proof_part2,
        &preprocess_part1,
        &preprocess_part2,
        &public_inputs,
    ];

    let head_size: usize = 6 * 32; // 5 offsets + 1 static value
    let mut tail_size: usize = 0;
    let mut offsets: Vec<usize> = Vec::with_capacity(5);

    for arr in &arrays {
        offsets.push(head_size + tail_size);
        tail_size += 32 + arr.len() * 32; // length word + elements
    }

    let total = head_size + tail_size;
    let mut buf = vec![0u8; total];

    // Write head: 5 offsets for dynamic arrays, then smax as static value.
    for (i, offset) in offsets.iter().enumerate() {
        let offset_u256 = U256::from(*offset);
        buf[i * 32..(i + 1) * 32].copy_from_slice(&offset_u256.to_big_endian());
    }
    // Slot 5: smax (static uint256)
    buf[5 * 32..6 * 32].copy_from_slice(&smax.to_big_endian());

    // Write tail: each array as [length, elem0, elem1, ...]
    for (i, arr) in arrays.iter().enumerate() {
        let base = offsets[i];
        let len_u256 = U256::from(arr.len());
        buf[base..base + 32].copy_from_slice(&len_u256.to_big_endian());
        for (j, val) in arr.iter().enumerate() {
            let pos = base + 32 + j * 32;
            buf[pos..pos + 32].copy_from_slice(&val.to_big_endian());
        }
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_u256_with_prefix() {
        let val = parse_hex_u256("0xff").unwrap();
        assert_eq!(val, U256::from(255));
    }

    #[test]
    fn parse_hex_u256_without_prefix() {
        let val = parse_hex_u256("1a").unwrap();
        assert_eq!(val, U256::from(26));
    }

    #[test]
    fn parse_hex_u256_invalid() {
        assert!(parse_hex_u256("not_hex").is_err());
    }

    #[test]
    fn abi_encode_empty_arrays() {
        let output = TokamakProveOutput {
            proof: FormattedProof {
                proof_entries_part1: vec![],
                proof_entries_part2: vec![],
            },
            preprocess: FormattedPreprocess {
                preprocess_entries_part1: vec![],
                preprocess_entries_part2: vec![],
            },
            public_inputs: vec![],
            smax: 42,
        };

        let encoded = abi_encode_tokamak_proof(&output).unwrap();

        // Head: 6 * 32 = 192 bytes
        // Tail: 5 arrays × 32 bytes (just length word, no elements) = 160 bytes
        // Total: 352 bytes
        assert_eq!(encoded.len(), 352);

        // Check smax is at slot 5 (offset 160..192)
        let smax_bytes = U256::from(42).to_big_endian();
        assert_eq!(&encoded[160..192], &smax_bytes);
    }

    #[test]
    fn abi_encode_with_values() {
        let output = TokamakProveOutput {
            proof: FormattedProof {
                proof_entries_part1: vec!["0x1".to_string(), "0x2".to_string()],
                proof_entries_part2: vec!["0x3".to_string()],
            },
            preprocess: FormattedPreprocess {
                preprocess_entries_part1: vec!["0x4".to_string()],
                preprocess_entries_part2: vec![],
            },
            public_inputs: vec!["0x5".to_string(), "0x6".to_string(), "0x7".to_string()],
            smax: 100,
        };

        let encoded = abi_encode_tokamak_proof(&output).unwrap();

        // Head: 6 * 32 = 192
        // proof_part1: 32 + 2*32 = 96
        // proof_part2: 32 + 1*32 = 64
        // preprocess_part1: 32 + 1*32 = 64
        // preprocess_part2: 32 + 0*32 = 32
        // public_inputs: 32 + 3*32 = 128
        // Total: 192 + 96 + 64 + 64 + 32 + 128 = 576
        assert_eq!(encoded.len(), 576);
    }

    #[test]
    fn formatted_proof_serde_roundtrip() {
        let proof = FormattedProof {
            proof_entries_part1: vec!["0xabc".into(), "0xdef".into()],
            proof_entries_part2: vec!["0x123".into()],
        };
        let json = serde_json::to_string(&proof).unwrap();
        let parsed: FormattedProof = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.proof_entries_part1.len(), 2);
        assert_eq!(parsed.proof_entries_part2.len(), 1);
    }

    #[test]
    fn formatted_preprocess_serde_roundtrip() {
        let preprocess = FormattedPreprocess {
            preprocess_entries_part1: vec!["0x1".into()],
            preprocess_entries_part2: vec!["0x2".into(), "0x3".into()],
        };
        let json = serde_json::to_string(&preprocess).unwrap();
        let parsed: FormattedPreprocess = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.preprocess_entries_part1.len(), 1);
        assert_eq!(parsed.preprocess_entries_part2.len(), 2);
    }

    #[test]
    fn backend_name_is_tokamak() {
        let backend = TokamakBackend::new("tokamak-cli".into(), ".".into());
        assert_eq!(backend.backend_name(), "tokamak");
    }

    #[test]
    fn prover_type_is_tokamak() {
        let backend = TokamakBackend::new("tokamak-cli".into(), ".".into());
        assert_eq!(backend.prover_type(), ProverType::Tokamak);
    }
}
