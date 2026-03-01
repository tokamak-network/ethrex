// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

/// @title Exponentiation — Modular exponentiation benchmark
/// @notice Computes repeated modular exponentiation using MUL and MOD opcodes.
///         Stresses arithmetic opcodes without memory/storage overhead.
contract Exponentiation {
    function Benchmark(uint256 n) external pure returns (uint256 result) {
        uint256 base = 3;
        uint256 modulus = 1000000007;
        result = 1;

        for (uint256 i = 0; i < n; i++) {
            result = mulmod(result, base, modulus);
            base = addmod(base, result, modulus);
        }
    }
}
