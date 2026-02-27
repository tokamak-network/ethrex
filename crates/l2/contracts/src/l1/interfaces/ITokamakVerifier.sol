// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title Tokamak Verifier Interface
/// @notice Interface for the Tokamak custom zkSNARK Verifier
interface ITokamakVerifier {
    /// @notice Verifies a custom zkSNARK proof
    /// @param proof_part1 First part of the proof (uint128 array)
    /// @param proof_part2 Second part of the proof (uint256 array)
    /// @param preprocess_part1 First part of the preprocessing data (uint128 array)
    /// @param preprocess_part2 Second part of the preprocessing data (uint256 array)
    /// @param publicInputs The public inputs for verification
    /// @param smax The smax parameter
    /// @return True if the proof is valid
    function verify(
        uint128[] calldata proof_part1,
        uint256[] calldata proof_part2,
        uint128[] calldata preprocess_part1,
        uint256[] calldata preprocess_part2,
        uint256[] calldata publicInputs,
        uint256 smax
    ) external view returns (bool);
}
