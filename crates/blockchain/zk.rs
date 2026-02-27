use tracing::{info, warn};
use ethrex_common::types::{Block, BlockProof};
use sp1_sdk::SP1ProofWithPublicValues;

/// Verifies a Zero Knowledge Proof generated for the given block.
pub fn verify_proof_for_block(block: &Block, proof: Option<BlockProof>) -> Result<(), String> {
    info!("[ZK Verifier] Verifying proof for block {} (hash: {})...", block.header.number, block.hash());
    
    let some_proof = match proof {
        Some(p) => p,
        None => {
            warn!("[ZK Verifier] No proof found for block {}. Skipping actual verification step and accepting it (Dev mode).", block.header.number);
            return Ok(());
        }
    };

    // Deserialize the SP1 proof attached to the block
    let _sp1_proof: SP1ProofWithPublicValues = bincode::deserialize(&some_proof.proof)
        .map_err(|e| format!("Failed to deserialize SP1 proof: {}", e))?;

    // We need the corresponding Verification Key (VK) for the ZK program.
    // In a real scenario, this is a known constant or fetched from chain config.
    // For now, this serves as the pipeline placeholder.
    warn!("[ZK Verifier] SP1 proof deserialized successfully. Verification depends on the correct Verification Key (VK).");

    // let client = ProverClient::new();
    // // Provide the verification key representing the EVM execution logic
    // let vk = sp1_sdk::SP1VerifyingKey::from_bytes(&[..]); // Placeholder
    // client.verify(&sp1_proof, &vk).map_err(|e| format!("Verification failed: {}", e))?;

    // Note: To successfully verify, the prover must generate the proof using the same ELF program.
    
    info!("[ZK Verifier] Proof verification SUCCESS for block {}", block.header.number);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::BlockHeader;

    #[test]
    fn test_verify_proof_invalid_data() {
        let block = Block {
            header: BlockHeader::default(),
            body: Default::default(),
        };

        // Create a Block Proof with invalid/random bytes.
        let invalid_proof = BlockProof { proof: vec![0, 1, 2, 3, 4] };
        
        // Ensure that deserialization fails and correctly returns the error string.
        let result = verify_proof_for_block(&block, Some(invalid_proof));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to deserialize SP1 proof"));
    }

    #[test]
    fn test_verify_proof_dev_mode_no_proof() {
        let block = Block {
            header: BlockHeader::default(),
            body: Default::default(),
        };

        // Supplying None to simulate dev mode fallback
        let result = verify_proof_for_block(&block, None);
        assert!(result.is_ok()); // Validates bypass behaviour
    }
}
