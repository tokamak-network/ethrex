# Sentinel Architecture — Design Decisions

**Date**: 2026-02-28
**Branch**: `feat/tokamak-autopsy`
**Scope**: H-1 (Pre-Filter), H-2 (Deep Analysis), H-3 (Block Processing Integration)

---

## Core Constraint

ethrex is a full node. Any analysis added to the block processing hot path must not degrade synchronization performance or risk consensus divergence. This constraint drives every design decision below.

---

## 1. Two-Stage Pipeline (PreFilter → DeepAnalyzer)

**Analogy**: Airport security screening. Everyone passes through the metal detector (PreFilter); only those who trigger an alarm get a full pat-down (DeepAnalyzer).

```
Block (200 TXs)
  │
  ├─ PreFilter: receipt-based, ~10-50μs/TX  ← scans ALL TXs
  │   └─ 197 benign → discard
  │   └─ 3 suspicious → forward to DeepAnalyzer
  │
  └─ DeepAnalyzer: opcode replay, ~10-100ms/TX  ← suspicious only
      └─ AttackClassifier + FundFlowTracer
      └─ SentinelAlert generation
```

### Why not a single stage?

Opcode replay re-executes the transaction from scratch. Replaying all 200 TXs in a block would double block processing time. Receipts already exist as execution output — scanning them costs nearly zero.

### Why receipts for the pre-filter?

Receipts contain logs (events), `gas_used`, and `succeeded` status. Flash loan event signatures, ERC-20 Transfer counts, and gas patterns can be extracted without opcode-level tracing. False positives are filtered by the DeepAnalyzer.

---

## 2. BlockObserver Trait Placement

### Dependency graph

```
ethrex-blockchain  ──depends──>  ethrex-vm, ethrex-storage
tokamak-debugger   ──depends──>  ethrex-blockchain, ethrex-vm, ethrex-storage
```

tokamak-debugger depends on ethrex-blockchain, not the reverse. If blockchain directly referenced `SentinelService`, it would create a **circular dependency**.

### Solution: Dependency Inversion Principle (DIP)

Define the `BlockObserver` trait in ethrex-blockchain (the interface). Implement it (`SentinelService`) in tokamak-debugger (the concrete). blockchain only knows `dyn BlockObserver`.

```rust
// ethrex-blockchain — interface only
pub trait BlockObserver: Send + Sync {
    fn on_block_committed(&self, block: Block, receipts: Vec<Receipt>);
}

// tokamak-debugger — implementation
impl BlockObserver for SentinelService { ... }
```

No feature gates needed in blockchain. The `block_observer` field defaults to `None` — zero overhead when sentinel is not configured.

---

## 3. Background Worker Thread + mpsc Channel

**Analogy**: Restaurant kitchen. The waiter (block processing thread) hangs the order ticket (`send`) and immediately serves the next customer. The cook (worker thread) processes orders at their own pace.

```rust
// Block processing hot path — non-blocking
fn on_block_committed(&self, block: Block, receipts: Vec<Receipt>) {
    let _ = sender.send(SentinelMessage::BlockCommitted { ... });
    // Returns immediately — analysis happens on the worker
}
```

### Why std::sync::mpsc instead of async?

LEVM's `Database` trait is synchronous. `replay_tx_from_store` calls LEVM directly. Using `block_on()` inside a tokio async context panics (learned from the autopsy `reqwest::blocking` experience). A dedicated OS thread is the safest choice.

### Why `Mutex<mpsc::Sender>`?

`mpsc::Sender` is `Send` but NOT `Sync`. The `BlockObserver: Send + Sync` bound requires wrapping with `Mutex`. Lock contention is effectively zero — `send()` takes microseconds.

---

## 4. Autopsy Infrastructure Reuse

E-4's `AttackClassifier`, `FundFlowTracer`, and `DetectedPattern` are reused directly.

```
E-4 (Autopsy Lab)          H-2 (Sentinel Deep Analysis)
─────────────────          ──────────────────────────────
RemoteVmDatabase           StoreVmDatabase (local Store)
    ↓                          ↓
OpcodeRecorder             OpcodeRecorder (identical)
    ↓                          ↓
AttackClassifier           AttackClassifier (identical)
FundFlowTracer             FundFlowTracer (identical)
    ↓                          ↓
AutopsyReport              SentinelAlert (different output format)
```

### The only difference is the data source

Autopsy fetches state from an external archive RPC. Sentinel reads from the node's own Store. The analysis logic is 100% identical.

### `#[cfg(feature = "autopsy")]` gating

Sentinel can run with only the `sentinel` feature (without `autopsy`). In this mode, only the PreFilter operates and deep analysis is skipped. This excludes heavy dependencies like `reqwest` (HTTP client) from sentinel-only builds.

---

## 5. Store-Based Replay vs Remote RPC

```rust
// Autopsy (E-4) — depends on external archive node
let db = RemoteVmDatabase::new(rpc_url, block_number);

// Sentinel (H-2) — reads directly from local Store
let db = StoreVmDatabase::new(store.clone(), parent_header);
```

Why Sentinel uses the local Store:

| Factor | Remote RPC | Local Store |
|--------|-----------|-------------|
| Latency | ~ms per call | ~μs per read |
| Availability | External node may fail | Always available (just stored) |
| Consistency | May lag or differ | Block was just committed |
| Dependencies | reqwest, network | None |

---

## 6. Block Clone Timing

```rust
// Clone BEFORE store_block — block is consumed by store_block()
let observer_data = self.block_observer.as_ref()
    .map(|_| (block.clone(), res.receipts.clone()));

let result = self.store_block(block, ...);  // block consumed here

// Notify observer ONLY after successful store
if result.is_ok() {
    if let Some((block_clone, receipts)) = observer_data {
        observer.on_block_committed(block_clone, receipts);
    }
}
```

### Why clone before store?

`store_block()` consumes `Block` by value (`fn store_block(self, block: Block, ...)`). The clone cannot be deferred to after the store call.

### Why skip clone when no observer?

The `.map(|_| ...)` pattern skips cloning entirely when `block_observer` is `None`. Zero overhead when sentinel is not configured.

---

## 7. Graceful Shutdown via Drop

```rust
impl Drop for SentinelService {
    fn drop(&mut self) {
        self.shutdown();              // Send Shutdown message
        if let Some(h) = handle.take() {
            let _ = h.join();         // Wait for worker to finish
        }
    }
}
```

The worker thread holds a `Store` reference. If the process exits while the worker is mid-analysis, in-flight data is lost. Graceful shutdown ensures the worker finishes its current block before terminating.

---

## Design Summary

| Decision | Rationale |
|----------|-----------|
| Two-stage pipeline | 99% of TXs filtered at stage 1, saving replay cost |
| BlockObserver trait in blockchain | Avoids circular dependency (DIP) |
| Dedicated OS thread | Compatible with LEVM sync traits, prevents async deadlock |
| Autopsy reuse | Proven classifiers reused, no code duplication |
| Local Store replay | No external dependency, μs latency |
| Clone before store | Block is consumed by `store_block()` — unavoidable |
| Feature flag separation | sentinel-only builds exclude reqwest and other heavy deps |
| Mutex around mpsc::Sender | mpsc::Sender is Send but not Sync; Mutex satisfies BlockObserver bounds |

---

## File Map

| File | Purpose | Lines |
|------|---------|-------|
| `sentinel/mod.rs` | Module declarations | 13 |
| `sentinel/types.rs` | SentinelConfig, SuspiciousTx, AlertPriority, SentinelAlert, SentinelError, AnalysisConfig | 208 |
| `sentinel/pre_filter.rs` | 7 receipt-based heuristics, known address DB | 396 |
| `sentinel/replay.rs` | replay_tx_from_store, load_block_header | 149 |
| `sentinel/analyzer.rs` | DeepAnalyzer orchestration with autopsy-gated classification | 163 |
| `sentinel/service.rs` | SentinelService background worker, AlertHandler, BlockObserver impl | 189 |
| `sentinel/tests.rs` | 85 sentinel tests (H-1: 32, H-2: 20, H-3: 11, extras) | 1,642 |
| `blockchain/blockchain.rs` | BlockObserver trait + hooks in add_block/add_block_pipeline | +73 |

---

## Known Limitations

1. **Silent error in worker loop**: Deep analysis errors are suppressed in the worker. Addressed in H-4 (structured logging via AlertDispatcher).
2. **Unbounded channel**: `mpsc::channel()` is unbounded. Under sustained block bursts, memory could grow. H-4 adds AlertRateLimiter for downstream backpressure.
3. **Single worker thread**: One worker processes blocks sequentially. If deep analysis is slow, blocks queue up. Acceptable for current throughput requirements.
4. **Alert persistence**: H-4 added JSONL file logging, H-5 added AlertHistory query engine and dashboard.
5. **PreFilter blind spot for stealthy attacks**: The 7 receipt-based heuristics are optimized for "loud" attacks (flash loans, high-value reverts, mass ERC-20 transfers). A minimal reentrancy attack (1 wei value, ~82k gas, successful execution, no ERC-20 transfers) triggers zero heuristics. The E2E pipeline test validates this gap using lowered thresholds (`suspicion_threshold: 0.1, min_gas_used: 50_000`) and `prefilter_alert_mode: true`. Production mitigation options: calldata pattern analysis, ML-based scoring, or mempool-level inspection (H-6 optional scope).

---

## 8. E2E Validation

The live reentrancy E2E test (`examples/reentrancy_demo.rs`) validates the full 6-phase pipeline with real bytecode execution:

```
Phase 1: Deploy & Execute  → LEVM executes attacker/victim contracts
Phase 2: Verify Attack      → call depth >= 3, SSTORE count >= 2
Phase 3: Classify            → AttackClassifier detects Reentrancy (conf >= 0.7)
Phase 4: Fund Flow           → FundFlowTracer traces ETH transfers
Phase 5: Sentinel Pipeline   → real receipt → PreFilter → SentinelService → alert
Phase 6: Alert Validation    → alert content + metrics verification
```

Key insight: stealthy attacks bypass PreFilter entirely. The `prefilter_alert_mode` flag ensures alerts are still emitted when deep analysis is unavailable (no Store for replay).

---

## 9. H-6 Expansion (Planned)

H-6 extends the sentinel with 4 sub-tasks documented in `docs/tokamak/H6-EXPANDED-PLAN.md`:

| Sub-task | Purpose |
|----------|---------|
| H-6a | CLI integration + TOML configuration |
| H-6b | Mempool monitoring (pre-execution calldata heuristics) |
| H-6c | Adaptive analysis pipeline (dynamic multi-step, statistical anomaly scoring) |
| H-6d | Auto-pause (block processing suspension on Critical alerts + resume RPC) |

Key design decisions carried from H-1~H-5:
- **DIP pattern reuse**: `MempoolObserver` trait in blockchain (same pattern as `BlockObserver`)
- **PauseController**: `AtomicBool` + `Condvar` with `wait_timeout()` auto-resume — no permanent halt risk
- **Feature containment**: All H-6 code gated under `sentinel` feature (no new sub-features)
- **Adaptive pipeline**: Replaces fixed DeepAnalyzer with `AnalysisStep` trait chain + dynamic step injection, while reusing existing `AttackClassifier`/`FundFlowTracer` as step implementations
