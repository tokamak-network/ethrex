// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

/// @title BitwiseOps — Bitwise operation benchmark
/// @notice Exercises AND, OR, XOR, SHL, SHR opcodes in a tight loop.
///         Pure arithmetic with no memory/storage access.
contract BitwiseOps {
    function Benchmark(uint256 n) external pure returns (uint256 result) {
        uint256 a = 0xdeadbeef;
        uint256 b = 0xcafebabe;

        for (uint256 i = 0; i < n; i++) {
            a = (a ^ b) | (a & (b << 3));
            b = (b ^ a) & (b | (a >> 2));
            a = a ^ (b << 1);
        }

        result = a ^ b;
    }
}
