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

/// Public inputs from the Tokamak synthesizer (instance.json).
///
/// Contains three arrays that are concatenated to form the full public
/// inputs vector for the verifier:
///   publicInputs = a_pub_user ++ a_pub_block ++ a_pub_function
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceJson {
    pub a_pub_user: Vec<String>,
    pub a_pub_block: Vec<String>,
    pub a_pub_function: Vec<String>,
}

/// Setup parameters from setupParams.json.
#[derive(Debug, Clone, Deserialize)]
pub struct SetupParams {
    pub s_max: u64,
}

/// Combined output of the Tokamak proving pipeline.
pub struct TokamakProveOutput {
    pub proof: FormattedProof,
    pub preprocess: FormattedPreprocess,
    /// Public inputs as hex strings (uint256 values).
    /// Concatenation of a_pub_user + a_pub_block + a_pub_function.
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
/// The CLI operates from the Tokamak-zk-EVM repository root and writes
/// all outputs to `{root}/dist/resource/`. We set `TOKAMAK_ZK_EVM_ROOT`
/// env var to control the root directory.
///
/// Because the CLI has heavy GPU/ICICLE dependencies, we invoke it as an
/// external process rather than linking it as a library.
pub struct TokamakBackend {
    /// Path to the `tokamak-cli` binary.
    cli_path: PathBuf,
    /// Tokamak-zk-EVM repository root directory.
    /// The CLI reads/writes to `{resource_dir}/dist/resource/`.
    resource_dir: PathBuf,
}

impl TokamakBackend {
    pub fn new(cli_path: PathBuf, resource_dir: PathBuf) -> Self {
        Self {
            cli_path,
            resource_dir,
        }
    }

    /// Returns the `dist/` directory under the Tokamak resource root.
    fn dist_dir(&self) -> PathBuf {
        self.resource_dir.join("dist")
    }

    /// Run the full Tokamak proving pipeline.
    ///
    /// The CLI operates from the Tokamak-zk-EVM repo root and writes
    /// outputs to fixed paths under `{root}/dist/resource/`.
    ///
    /// Pipeline:
    /// 1. Write ProgramInput as a Tokamak synthesizer config JSON.
    /// 2. `tokamak-cli --synthesize <config.json>`
    /// 3. `tokamak-cli --preprocess`
    /// 4. (if `prove`) `tokamak-cli --prove`
    /// 5. Parse outputs from `dist/resource/`.
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
        self.run_cli(&["--synthesize", &config_path.to_string_lossy()])?;

        // 3. Preprocess
        info!("Tokamak: running preprocess");
        self.run_cli(&["--preprocess"])?;

        if prove {
            // 4. Prove
            info!("Tokamak: running prove");
            self.run_cli(&["--prove"])?;

            // 5. Parse outputs from dist/resource/
            let dist = self.dist_dir();
            let proof = self.read_proof(&dist)?;
            let preprocess = self.read_preprocess(&dist)?;
            let public_inputs = self.read_public_inputs(&dist)?;
            let smax = self.read_smax(&dist)?;

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
    ///
    /// Sets `TOKAMAK_ZK_EVM_ROOT` so the CLI uses our resource directory
    /// as the repo root.
    fn run_cli(&self, args: &[&str]) -> Result<(), BackendError> {
        info!(
            "Tokamak: executing {} {}",
            self.cli_path.display(),
            args.join(" ")
        );

        let output = Command::new(&self.cli_path)
            .args(args)
            .env("TOKAMAK_ZK_EVM_ROOT", &self.resource_dir)
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

    /// Read and parse `proof.json` from `dist/resource/prove/output/`.
    fn read_proof(&self, dist_dir: &Path) -> Result<FormattedProof, BackendError> {
        let path = dist_dir.join("resource/prove/output/proof.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;
        serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse proof.json: {e}"))
        })
    }

    /// Read and parse `preprocess.json` from `dist/resource/preprocess/output/`.
    fn read_preprocess(&self, dist_dir: &Path) -> Result<FormattedPreprocess, BackendError> {
        let path = dist_dir.join("resource/preprocess/output/preprocess.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;
        serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse preprocess.json: {e}"))
        })
    }

    /// Read public inputs from `instance.json`.
    ///
    /// The instance.json produced by the Tokamak synthesizer has this structure:
    /// ```json
    /// {
    ///   "a_pub_user": ["0x...", ...],
    ///   "a_pub_block": ["0x...", ...],
    ///   "a_pub_function": ["0x...", ...]
    /// }
    /// ```
    ///
    /// The full public inputs vector is the concatenation of all three arrays.
    fn read_public_inputs(&self, dist_dir: &Path) -> Result<Vec<String>, BackendError> {
        let path = dist_dir.join("resource/synthesizer/output/instance.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;
        let instance: InstanceJson = serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse instance.json: {e}"))
        })?;

        // Concatenate: a_pub_user ++ a_pub_block ++ a_pub_function
        let mut public_inputs = instance.a_pub_user;
        public_inputs.extend(instance.a_pub_block);
        public_inputs.extend(instance.a_pub_function);
        Ok(public_inputs)
    }

    /// Read `s_max` from `setupParams.json`.
    ///
    /// `s_max` is a circuit-level parameter set during trusted setup.
    /// Valid values: 64, 128, 256, 512, 1024, 2048.
    fn read_smax(&self, dist_dir: &Path) -> Result<u64, BackendError> {
        let path = dist_dir.join("resource/qap-compiler/library/setupParams.json");
        let data = std::fs::read_to_string(&path).map_err(|e| {
            BackendError::proving(format!("failed to read {}: {e}", path.display()))
        })?;
        let params: SetupParams = serde_json::from_str(&data).map_err(|e| {
            BackendError::proving(format!("failed to parse setupParams.json: {e}"))
        })?;
        Ok(params.s_max)
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

    /// Test parsing actual proof.json from Tokamak-zk-EVM output.
    #[test]
    fn parse_real_proof_json() {
        let json = r#"{
            "proof_entries_part1": [
                "0x14707e7c13706ad855a0110bd1a95fbe",
                "0x10d6824c08b9845d68190898015afe56"
            ],
            "proof_entries_part2": [
                "0x751730d725d624d4d65dc9ece3c952d054255c31b1dd297e2f1458ab472e2185",
                "0xec91fa306f8fd2dc4254d1a93e0c634da644750b1373c57843c45bd87dd2ed1b"
            ]
        }"#;
        let proof: FormattedProof = serde_json::from_str(json).unwrap();
        assert_eq!(proof.proof_entries_part1.len(), 2);
        assert_eq!(proof.proof_entries_part2.len(), 2);
        // part1 values should fit in u128
        let val = parse_hex_u256(&proof.proof_entries_part1[0]).unwrap();
        assert!(val <= U256::from(u128::MAX));
    }

    /// Test parsing actual preprocess.json from Tokamak-zk-EVM output.
    #[test]
    fn parse_real_preprocess_json() {
        let json = r#"{
            "preprocess_entries_part1": [
                "0x0592ea049faa4b7c5464a5779e3d59a3",
                "0x100a2db829d1551e0980d52175ce0ea6",
                "0x08c69f68d7c3c93aad10d3e65c3bf996",
                "0x0c0aab59c18db9b3269af60b2eb8b947"
            ],
            "preprocess_entries_part2": [
                "0x193316eb55d3413d82ac47f71a412d5eaee3e1f39469a882a0d0cffc84aeadf9",
                "0x8631b5cb57934d8ac49c9c7e1f294ceed762aab6e7c2b42d268e9fa43641cbb3",
                "0xfe348e5ecc24c936bb564a82b96d1795942b6297b3c174b8e44f7673075552bd",
                "0x1305c8a83e9950c060dc85b4964fc2ebc96261e8f9d7148a988acc81782c936c"
            ]
        }"#;
        let pp: FormattedPreprocess = serde_json::from_str(json).unwrap();
        assert_eq!(pp.preprocess_entries_part1.len(), 4);
        assert_eq!(pp.preprocess_entries_part2.len(), 4);
    }

    /// Test parsing instance.json (public inputs) format.
    #[test]
    fn parse_instance_json() {
        let json = r#"{
            "a_pub_user": ["0x1", "0x2", "0x3"],
            "a_pub_block": ["0x4", "0x5"],
            "a_pub_function": ["0x6"]
        }"#;
        let instance: InstanceJson = serde_json::from_str(json).unwrap();
        assert_eq!(instance.a_pub_user.len(), 3);
        assert_eq!(instance.a_pub_block.len(), 2);
        assert_eq!(instance.a_pub_function.len(), 1);

        // Concatenation order
        let mut all = instance.a_pub_user;
        all.extend(instance.a_pub_block);
        all.extend(instance.a_pub_function);
        assert_eq!(all, vec!["0x1", "0x2", "0x3", "0x4", "0x5", "0x6"]);
    }

    /// Test parsing setupParams.json for s_max.
    #[test]
    fn parse_setup_params() {
        let json = r#"{"l":512,"l_user_out":8,"l_user":40,"l_block":64,"l_D":2560,"m_D":13251,"n":2048,"s_D":23,"s_max":256}"#;
        let params: SetupParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.s_max, 256);
    }

    /// Test ABI encoding with real-sized proof data (38 part1 + 42 part2).
    #[test]
    fn abi_encode_real_sized_proof() {
        let proof_part1: Vec<String> = (0..38)
            .map(|i| format!("0x{:032x}", i + 1))
            .collect();
        let proof_part2: Vec<String> = (0..42)
            .map(|i| format!("0x{:064x}", i + 100))
            .collect();
        let preprocess_part1: Vec<String> = (0..4)
            .map(|i| format!("0x{:032x}", i + 200))
            .collect();
        let preprocess_part2: Vec<String> = (0..4)
            .map(|i| format!("0x{:064x}", i + 300))
            .collect();
        // 40 + 24 + 448 = 512 public inputs (real size)
        let public_inputs: Vec<String> = (0..512)
            .map(|i| format!("0x{:x}", i))
            .collect();

        let output = TokamakProveOutput {
            proof: FormattedProof {
                proof_entries_part1: proof_part1,
                proof_entries_part2: proof_part2,
            },
            preprocess: FormattedPreprocess {
                preprocess_entries_part1: preprocess_part1,
                preprocess_entries_part2: preprocess_part2,
            },
            public_inputs,
            smax: 256,
        };

        let encoded = abi_encode_tokamak_proof(&output).unwrap();

        // Head: 6 * 32 = 192
        // proof_part1: 32 + 38*32 = 1248
        // proof_part2: 32 + 42*32 = 1376
        // preprocess_part1: 32 + 4*32 = 160
        // preprocess_part2: 32 + 4*32 = 160
        // public_inputs: 32 + 512*32 = 16416
        // Total: 192 + 1248 + 1376 + 160 + 160 + 16416 = 19552
        assert_eq!(encoded.len(), 19552);

        // Verify smax at slot 5
        let smax_bytes = U256::from(256).to_big_endian();
        assert_eq!(&encoded[160..192], &smax_bytes);
    }
}
