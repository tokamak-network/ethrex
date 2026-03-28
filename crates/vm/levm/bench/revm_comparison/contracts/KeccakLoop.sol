// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

/// @title KeccakLoop — Chained Keccak256 benchmark
/// @notice Each iteration hashes the previous result, creating a dependency chain.
///         This stresses SHA3/MSTORE/MLOAD opcodes with sequential data dependency.
contract KeccakLoop {
    function Benchmark(uint256 n) external pure returns (bytes32 result) {
        result = keccak256(abi.encodePacked(uint256(0)));
        for (uint256 i = 1; i < n; i++) {
            result = keccak256(abi.encodePacked(result));
        }
    }
}
