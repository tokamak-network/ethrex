# SP1 Prover Profiling Baseline

**Date**: 2026-02-22
**Branch**: `feat/zk/profiling-baseline`
**Commit**: ethrex `0eadb15a` (upstream main)
**SP1 Version**: v5.0.8 (succinct toolchain), Groth16 circuit v5.0.0

## Environment

| Item | Value |
|------|-------|
| Machine | Apple M4 Max |
| Architecture | x86_64 (Rosetta 2 emulation) |
| OS | macOS Darwin 24.4.0 |
| RAM | ~64GB (estimated from process usage) |
| CPU mode | CPU only (no GPU) |
| Docker | 28.x (for Groth16 gnark wrapper) |
| Rust | 1.90.0+ |
| SP1 SDK | 5.0.8 (sp1-recursion-gnark-ffi) |

> **Note**: All benchmarks were run under Rosetta 2 (x86_64 emulation on ARM).
> Native ARM performance is expected to be significantly faster (~2-3x improvement).

---

## Benchmark Results: Batch 1

### Total Proving Time

| Metric | Value |
|--------|-------|
| **Total proving time** | **1,664.55 seconds (27.7 minutes)** |
| Total execution cycles | 65,360,896 |
| Execution simulation | ~1 second |
| STARK proving (core) | ~12.4 minutes |
| Recursive compression | ~14.5 minutes |
| Groth16 wrapping (Docker) | ~17 seconds |

### Proving Time Breakdown

```
Total wall time: 1,664.55s
├── Execution simulation:    ~1s    (0.06%)
├── STARK core proving:     ~744s   (44.7%)    ← clk 0 → 60M
├── Recursive compression:  ~870s   (52.3%)    ← proof shrinking
├── Groth16 prove:          ~17s    (1.0%)     ← Docker gnark
└── Groth16 verify:         ~0.2s   (0.01%)
```

### STARK Core Proving Speed

| Checkpoint | Timestamp | Delta | Cycles/sec |
|-----------|-----------|-------|------------|
| clk = 10M | 02:16:39 | — | — |
| clk = 20M | 02:18:04 | 85s | ~117,647 |
| clk = 30M | 02:19:27 | 83s | ~120,482 |
| clk = 40M | 02:21:10 | 103s | ~97,087 |
| clk = 50M | 02:23:01 | 111s | ~90,090 |
| clk = 60M | 02:26:14 | 193s | ~51,813 |

> Proving speed degrades significantly in the last 10M cycles (~52K cycles/sec vs ~120K initially).
> This may be due to memory pressure or proof table size growth.

---

## Guest Program Cycle Profile

### Overview (65,360,896 total cycles)

| Section | Cycles | % of Total |
|---------|--------|------------|
| `read_input` | 1,012,951 | 1.55% |
| `execution` (total) | 64,345,179 | 98.45% |
| `commit_public_inputs` | 2,766 | 0.004% |

### Execution Breakdown (64,345,179 cycles)

| Function | Cycles | % of Execution | % of Total |
|----------|--------|---------------|------------|
| **`execute_block`** | **29,363,722** | **45.6%** | **44.9%** |
| `validate_receipts_root` | 4,619,876 | 7.2% | 7.1% |
| `apply_account_updates` | 2,824,380 | 4.4% | 4.3% |
| `get_final_state_root` | 1,974,096 | 3.1% | 3.0% |
| `ethrex_guest_program_state_initialization` | 741,190 | 1.2% | 1.1% |
| `get_state_transitions` | 333,823 | 0.5% | 0.5% |
| `initialize_block_header_hashes` | 29,817 | 0.05% | 0.05% |
| `validate_block` | 7,428 | 0.01% | 0.01% |
| `validate_requests_hash` | 2,319 | 0.004% | 0.004% |
| `state_trie_root` | 1,800 | 0.003% | 0.003% |
| `get_first_invalid_block_hash` | 1,668 | 0.003% | 0.003% |
| `setup_evm` | 1,636 | 0.003% | 0.003% |
| `validate_gas_and_receipts` | 1,136 | 0.002% | 0.002% |
| **(unattributed overhead)** | **24,442,288** | **38.0%** | **37.4%** |

> Subtotal of named sections: 39,902,891 cycles.
> Unattributed: 24,442,288 cycles (38%) — likely zkVM overhead, memory operations,
> and execution infrastructure not covered by named profiling spans.

### Top Optimization Targets

```
Rank  Section                     Cycles       Share    Optimization Path
─────────────────────────────────────────────────────────────────────────
#1    execute_block               29,363,722   45.6%    Parallel tx execution (EIP-7928, Block-STM)
#2    (unattributed overhead)     24,442,288   38.0%    Needs deeper profiling (memory, zkVM ops)
#3    validate_receipts_root       4,619,876    7.2%    Trie hashing optimization
#4    apply_account_updates        2,824,380    4.4%    Trie update batching / parallelism
#5    get_final_state_root         1,974,096    3.1%    Parallel state root (upstream #6210)
```

---

## Key Observations

### 1. Block Execution Dominates (45.6%)
`execute_block` is the single largest cycle consumer. This is where EVM transaction
execution happens inside the zkVM. Parallel transaction execution (EIP-7928 / Block-STM)
is the highest-impact optimization target, aligning with upstream issues #6209 and #6210.

### 2. Trie Operations are Significant (14.7% combined)
- `validate_receipts_root` (7.2%): RLP encoding + Merkle trie construction for receipts
- `apply_account_updates` (4.4%): State trie insert/update operations
- `get_final_state_root` (3.1%): Final state root computation

Combined, trie-related operations account for ~14.7% of execution cycles.
Optimizing trie hashing (e.g., zkVM-friendly hash functions, batch operations)
could save ~9.4M cycles.

### 3. Large Unattributed Overhead (38%)
24.4M cycles are not attributed to any named span. This needs deeper investigation.
Possible sources:
- zkVM memory allocation and management
- SP1 syscall overhead (IO, hinting)
- Rust runtime overhead in zkVM context
- Code between profiled spans

### 4. Recursive Compression is the Bottleneck (52.3% of wall time)
While STARK core proving takes ~12.4 minutes, recursive compression dominates
at ~14.5 minutes (52.3% of total time). This is an SP1-internal operation that
compresses the proof recursively before Groth16 wrapping.

### 5. Groth16 Wrapping is Fast (17s)
The final Groth16 proof generation via Docker gnark is very fast (<1% of total time).
This is not a bottleneck.

### 6. Rosetta 2 Overhead
All measurements include Rosetta 2 (x86_64 → ARM) translation overhead.
Native ARM execution should significantly reduce proving time, particularly for
the recursive compression step which is CPU-intensive.

---

## Batch Details

- **Batch number**: 1
- **Block count**: Minimal (dev environment initialization blocks)
- **Transaction count**: Low (genesis + initial state setup)
- **Total execution cycles**: 64,345,179

> For a production-representative benchmark, batches with higher transaction counts
> and diverse transaction types (ERC20 transfers, contract deployments, complex calls)
> should be profiled.

---

## Comparison with Previous Run (Run 1 — failed at Groth16)

Run 1 had identical cycle counts (same batch), validating reproducibility:

| Metric | Run 1 | Run 2 |
|--------|-------|-------|
| Total execution cycles | 64,345,179 | 64,345,179 |
| execute_block | 29,363,722 | 29,363,722 |
| validate_receipts_root | 4,619,876 | 4,619,876 |
| STARK proving speed | ~similar | ~similar |
| Groth16 | FAILED (Docker) | SUCCESS (17s) |
| Total proving time | N/A | 1,664.55s |

> Cycle counts are deterministic across runs for the same batch, confirming
> that execution in zkVM is reproducible.

---

## Next Steps

1. **Run with higher transaction load**: Profile batches with 50-100+ transactions
   to get production-representative cycle data
2. **Switch to ARM native**: Re-run benchmarks without Rosetta 2 for accurate
   timing baseline
3. **Profile unattributed overhead**: Add more granular profiling spans to identify
   the 24.4M unattributed cycles
4. **RISC0 comparison**: Run identical workload on RISC0 backend for backend comparison
5. **Cycle breakdown visualization**: Create charts for cycle distribution
6. **SP1 GPU benchmark**: Test on NVIDIA GPU instance (AWS p4d/g5) for GPU comparison

---

## Source Files Reference

| File | Description |
|------|-------------|
| `crates/guest-program/src/common/execution.rs` | Guest program execution (profiling spans defined here) |
| `crates/l2/prover/src/backend/sp1.rs` | SP1 backend implementation |
| `crates/l2/prover/src/prover.rs` | Main prover loop |
| `crates/blockchain/blockchain.rs` | `execute_block()`, state root calculation |
| `crates/vm/witness_db.rs` | `state_trie_root()` |
| `crates/l2/sequencer/proof_coordinator.rs` | Batch assignment |

## Raw Log Files

- `prover-sp1.log` — Run 1 (failed at Groth16, Docker image denied)
- `prover-sp1-run2.log` — Run 2 (successful, 1664.55s total)
