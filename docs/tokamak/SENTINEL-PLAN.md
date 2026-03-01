# Phase H: Sentinel — Real-Time Hack Detection System

## Implementation Plan

---

## 1. Requirements

### Functional Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| FR-1 | Monitor every new block's transactions as they are processed by the ethrex node | Must |
| FR-2 | Fast pre-filter to avoid full replay overhead on normal (benign) transactions | Must |
| FR-3 | When suspicious activity is detected, run full autopsy analysis using existing AttackClassifier, FundFlowTracer, and StepRecord enrichment | Must |
| FR-4 | Alert system: emit structured alerts via webhook, tracing log, and file output | Must |
| FR-5 | Configurable alert thresholds (confidence score, ETH value) | Should |
| FR-6 | Feature-gated behind `sentinel` feature flag to avoid affecting normal node operation | Must |
| FR-7 | Dashboard-compatible JSON alert output for integration with F-2 public dashboard | Could |
| FR-8 | Historical block scanning mode (scan past N blocks on startup) | Could |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR-1 | Normal block processing overhead | < 5% wall-clock increase |
| NFR-2 | Pre-filter latency per transaction | < 100 microseconds |
| NFR-3 | Full autopsy (when triggered) | Off critical path, async |
| NFR-4 | Memory overhead per block | < 10 MB additional |
| NFR-5 | False positive rate | < 20% at default thresholds |

---

## 2. Architecture Overview

```
                                   ethrex Node
                    ┌────────────────────────────────────┐
                    │                                    │
                    │  Blockchain::add_block_pipeline()  │
                    │          │                         │
                    │          ▼                         │
                    │  LEVM::execute_block()             │
                    │     │                              │
                    │     │  for each TX:                │
                    │     │  ┌──────────────────────┐    │
                    │     │  │ LEVM::execute_tx()   │    │
                    │     │  │   → Receipt + Logs   │────┼──────┐
                    │     │  └──────────────────────┘    │      │
                    │     │                              │      │
                    │     ▼                              │      │
                    │  Block stored                      │      │
                    └────────────────────────────────────┘      │
                                                                │
                    ┌───────────────────────────────────────────┘
                    │
                    ▼
    ┌─────────────────────────────────┐
    │  Sentinel Pre-Filter (Phase 1)  │
    │  ─────────────────────────────  │
    │  • Receipt-level heuristics     │
    │  • Log topic scanning           │
    │  • Gas anomaly detection        │
    │  • Revert + high-gas combo      │
    │  • Known exploit signatures     │
    │                                 │
    │  Cost: ~10-50μs per TX          │
    │  Input: Receipt + TX metadata   │
    └──────────────┬──────────────────┘
                   │
                   │ suspicious TX detected
                   ▼
    ┌─────────────────────────────────┐
    │  Sentinel Deep Analysis         │
    │  (Phase 2 — async thread)       │
    │  ─────────────────────────────  │
    │  • Full opcode replay via       │
    │    DebugRecorder + VM replay    │
    │  • AttackClassifier.classify()  │
    │  • FundFlowTracer.trace()       │
    │  • Confidence scoring           │
    │  • AutopsyReport generation     │
    │                                 │
    │  Cost: 50-500ms per TX          │
    │  Input: Block + TX + State DB   │
    └──────────────┬──────────────────┘
                   │
                   │ attack confirmed (confidence > threshold)
                   ▼
    ┌─────────────────────────────────┐
    │  Alert Dispatcher               │
    │  ─────────────────────────────  │
    │  • tracing::warn! structured    │
    │  • JSON file append             │
    │  • Webhook POST (optional)      │
    │  • Metrics counter update       │
    └─────────────────────────────────┘
```

### Key Design Decisions

1. **Two-tier architecture**: The pre-filter runs synchronously on the receipt data that `execute_block` already produces. No opcode recording is needed for the fast path. Only suspicious TXs trigger the expensive deep analysis.

2. **Post-execution hook, not per-opcode hook**: The sentinel does NOT use `OpcodeRecorder` in the critical path. Instead, it examines receipts (which already exist) and only re-executes suspicious transactions with recording enabled on a background thread.

3. **Async deep analysis**: Deep analysis runs on a separate thread pool, never blocking block processing. Results are delivered via an alert channel.

4. **State replay, not live recording**: Deep analysis uses the committed state (after `store_block`) to re-execute the suspicious TX with `OpcodeRecorder` attached. This means the state is immutable and safe to read from a background thread.

---

## 3. Implementation Phases

### Phase H-1: Pre-Filter Engine (Core)

**Goal**: Build the lightweight receipt-based pre-filter that can scan every TX in a block with < 100 microseconds overhead per TX.

#### Files to Create

| File | Description |
|------|-------------|
| `crates/tokamak-debugger/src/sentinel/mod.rs` | Module root, public API |
| `crates/tokamak-debugger/src/sentinel/pre_filter.rs` | Receipt-based heuristic scanner |
| `crates/tokamak-debugger/src/sentinel/types.rs` | Sentinel-specific types (SuspiciousTx, AlertLevel, SentinelConfig) |

#### Files to Modify

| File | Change |
|------|--------|
| `crates/tokamak-debugger/Cargo.toml` | Add `sentinel` feature flag |
| `crates/tokamak-debugger/src/lib.rs` | Add `#[cfg(feature = "sentinel")] pub mod sentinel;` |

#### Pre-Filter Design

The `PreFilter` struct is stateless and examines `(Transaction, Receipt, BlockHeader)` tuples:

```rust
// crates/tokamak-debugger/src/sentinel/pre_filter.rs

pub struct PreFilter {
    config: SentinelConfig,
}

pub struct SuspiciousTx {
    pub tx_hash: H256,
    pub tx_index: usize,
    pub reasons: Vec<SuspicionReason>,
    pub priority: AlertPriority,
}

pub enum SuspicionReason {
    HighValueTransfer { value_wei: U256 },
    LargeLogCount { count: usize },
    FlashLoanSignature { provider: Address },
    ReentrantCallPattern,
    HighGasWithRevert { gas_used: u64 },
    KnownExploitSelector { selector: [u8; 4] },
    MultipleErc20Transfers { count: usize },
    UnusualCallDepth,
    PriceOracleInteraction { oracle: Address },
}
```

#### Heuristics (detail in Section 4)

Each heuristic scores independently. If the combined score exceeds `config.suspicion_threshold`, the TX is flagged for deep analysis.

### Phase H-2: Deep Analysis Engine

**Goal**: Re-execute flagged transactions with full opcode recording, leveraging existing autopsy infrastructure.

#### Files to Create

| File | Description |
|------|-------------|
| `crates/tokamak-debugger/src/sentinel/analyzer.rs` | Deep analysis orchestrator |
| `crates/tokamak-debugger/src/sentinel/replay.rs` | TX re-execution with OpcodeRecorder |

#### Files to Modify

| File | Change |
|------|-------------|
| `crates/tokamak-debugger/Cargo.toml` | `sentinel` feature needs `dep:ethrex-storage`, `dep:ethrex-vm`, `dep:ethrex-blockchain`, `dep:rustc-hash`, `dep:serde_json`, `dep:tokio` |

#### Deep Analysis Flow

```rust
// crates/tokamak-debugger/src/sentinel/analyzer.rs

pub struct DeepAnalyzer;

impl DeepAnalyzer {
    /// Re-execute a suspicious transaction with full opcode recording.
    ///
    /// 1. Load parent state from Store
    /// 2. Create VM with OpcodeRecorder attached
    /// 3. Execute all TXs in the block up to and including the target TX
    /// 4. Extract StepRecords from the recorder
    /// 5. Run AttackClassifier::classify_with_confidence()
    /// 6. Run FundFlowTracer::trace()
    /// 7. Build AutopsyReport
    /// 8. Return SentinelAlert if confidence > threshold
    pub fn analyze(
        store: &Store,
        block: &Block,
        tx_index: usize,
        suspicion: &SuspiciousTx,
    ) -> Result<Option<SentinelAlert>, SentinelError> {
        // ... implementation
    }
}
```

The key insight is that `DeepAnalyzer` reuses exactly the same pipeline as the CLI autopsy command but reads from the local node's `Store` instead of a remote archive RPC. This is much faster since all state is local.

### Phase H-3: Integration with Block Processing

**Goal**: Hook the sentinel into the ethrex block processing pipeline without degrading performance.

#### Files to Modify

| File | Change |
|------|-------------|
| `crates/blockchain/blockchain.rs` | Add sentinel hook after `execute_block` / `execute_block_pipeline` in `add_block` and `add_block_pipeline` |
| `crates/blockchain/Cargo.toml` | Add optional `tokamak-debugger` dependency with `sentinel` feature |

#### Integration Strategy

The sentinel is injected into `Blockchain` as an optional field:

```rust
// In Blockchain struct:
#[cfg(feature = "sentinel")]
sentinel: Option<Arc<SentinelService>>,
```

The hook point is AFTER `store_block()` succeeds (state committed, block stored). This ensures:
- Block processing is not blocked by sentinel analysis
- State is available for re-execution
- Receipts are available for pre-filtering

```rust
// In add_block_pipeline(), after store_block():
#[cfg(feature = "sentinel")]
if let Some(sentinel) = &self.sentinel {
    sentinel.on_block_committed(&block, &execution_result.receipts);
}
```

The `on_block_committed` method is non-blocking: it sends the block data to a channel that the sentinel's background thread pool consumes.

### Phase H-4: Alert System

**Goal**: Dispatch alerts through multiple channels.

#### Files to Create

| File | Description |
|------|-------------|
| `crates/tokamak-debugger/src/sentinel/alert.rs` | Alert types and dispatcher |
| `crates/tokamak-debugger/src/sentinel/webhook.rs` | HTTP webhook client (optional) |

#### Alert Types

```rust
pub struct SentinelAlert {
    pub timestamp: u64,
    pub block_number: u64,
    pub block_hash: H256,
    pub tx_hash: H256,
    pub tx_index: usize,
    pub alert_level: AlertLevel,
    pub detected_patterns: Vec<DetectedPattern>,
    pub fund_flows: Vec<FundFlow>,
    pub total_value_at_risk: U256,
    pub summary: String,
    pub report: Option<AutopsyReport>,
}

pub enum AlertLevel {
    /// Informational — suspicious pattern but low confidence (0.3–0.5)
    Info,
    /// Warning — moderate confidence attack pattern (0.5–0.7)
    Warning,
    /// Critical — high confidence exploit detected (> 0.7)
    Critical,
}
```

#### Dispatch Channels

1. **tracing::warn!/error!**: Structured log output with `sentinel` target for log filtering
2. **JSON file**: Append-only JSONL file at `<data_dir>/sentinel/alerts.jsonl`
3. **Webhook**: HTTP POST to configured URL with JSON alert body (optional, requires `webhook` sub-feature)

### Phase H-5: Service Orchestration

**Goal**: Tie everything together into a `SentinelService` that manages the background thread pool.

#### Files to Create

| File | Description |
|------|-------------|
| `crates/tokamak-debugger/src/sentinel/service.rs` | Main service (owns thread pool, channels, config) |
| `crates/tokamak-debugger/src/sentinel/config.rs` | Configuration struct with serde support |

#### Service Architecture

```rust
pub struct SentinelService {
    config: SentinelConfig,
    store: Store,
    /// Channel for block notifications from the critical path
    block_tx: crossbeam_channel::Sender<BlockNotification>,
    /// Background worker handles
    workers: Vec<std::thread::JoinHandle<()>>,
    /// Metrics
    metrics: Arc<SentinelMetrics>,
}

struct BlockNotification {
    block: Arc<Block>,
    receipts: Arc<Vec<Receipt>>,
}
```

The service spawns 1 worker thread (configurable) that:
1. Receives `BlockNotification` from the channel
2. Runs `PreFilter::scan_block()` on all receipts
3. For each suspicious TX, runs `DeepAnalyzer::analyze()`
4. Dispatches alerts via `AlertDispatcher`

### Phase H-6: CLI & Configuration

**Goal**: Add sentinel configuration to the ethrex CLI and configuration files.

#### Files to Modify

| File | Change |
|------|--------|
| `cmd/ethrex/cli.rs` or equivalent | Add `--sentinel` flag and `--sentinel-config` path |
| `crates/blockchain/blockchain.rs` | `BlockchainOptions` gets `sentinel_config: Option<SentinelConfig>` |

#### Configuration

```toml
# sentinel.toml
[sentinel]
enabled = true
suspicion_threshold = 0.5
min_alert_confidence = 0.6
min_value_wei = "1000000000000000000"  # 1 ETH
worker_threads = 1
alert_file = "data/sentinel/alerts.jsonl"

[sentinel.webhook]
url = "https://hooks.example.com/sentinel"
timeout_ms = 5000
```

---

## 4. Pre-Filter Heuristics Design

The pre-filter operates on data that is already produced during normal block execution: `Receipt` (containing `Log` entries with address, topics, and data) and `Transaction` metadata. No additional VM execution is required.

### Heuristic 1: Flash Loan Signature Detection

**Input**: Receipt logs
**Cost**: O(n) scan over logs
**Logic**: Check if any log topic matches known flash loan event signatures:

| Protocol | Event | Topic Prefix |
|----------|-------|-------------|
| Aave V2/V3 | `FlashLoan(address,address,address,uint256,uint256,uint16)` | `0x631042c8` |
| dYdX | `LogOperation(address)` | Custom |
| Balancer | `FlashLoan(address,address,uint256,uint256)` | `0x0d7d75e0` |
| Uniswap V3 | `Flash(address,address,uint256,uint256,uint256,uint256)` | `0xbdbdb716` |

**Score**: +0.4 per match

### Heuristic 2: High Value Transfer with Revert

**Input**: Receipt (succeeded flag, cumulative_gas), TX (value, gas_limit)
**Cost**: O(1)
**Logic**: TX reverted AND gas_used > 100k AND (tx.value > 1 ETH OR large ERC-20 Transfer in logs)

**Score**: +0.3

### Heuristic 3: Multiple ERC-20 Transfer Events

**Input**: Receipt logs
**Cost**: O(n) scan over logs
**Logic**: Count LOG3 events with Transfer topic prefix `0xddf252ad`. If count > 5 in a single TX, flag as suspicious (normal swaps produce 2-3 transfers; exploit chains produce many).

**Score**: +0.2 for 5-10 transfers, +0.4 for > 10

### Heuristic 4: Known Contract Interaction

**Input**: TX `to` address, Receipt log addresses
**Cost**: O(n) lookup in static hash set
**Logic**: Check if the TX interacts with known high-value DeFi protocols (lending pools, DEX routers, bridges). Cross-reference with the `known_label()` function from `report.rs`.

**Score**: +0.1 (contextual — amplifies other signals)

### Heuristic 5: Unusual Gas Usage Pattern

**Input**: Receipt gas_used, TX gas_limit
**Cost**: O(1)
**Logic**: If `gas_used / gas_limit > 0.95` (near-exact gas estimation, typical of automated exploit scripts) AND gas_used > 500k.

**Score**: +0.15

### Heuristic 6: Self-Destruct in Logs

**Input**: Receipt logs
**Cost**: O(n)
**Logic**: Contract self-destructs are rare in legitimate transactions. If the post-state shows account removal (detectable from logs or account updates), flag.

**Score**: +0.3

### Heuristic 7: Price Oracle Interaction + Swap

**Input**: Receipt log addresses
**Cost**: O(n)
**Logic**: If the TX touches both a known oracle address (from `known_label()`) AND a DEX router/pool in the same transaction.

**Score**: +0.2

### Scoring Formula

```
total_score = sum(heuristic_scores)
suspicious = total_score >= config.suspicion_threshold (default: 0.5)
```

Priority mapping:
- `total_score >= 0.8` → `AlertPriority::Critical`
- `total_score >= 0.5` → `AlertPriority::High`
- `total_score >= 0.3` → `AlertPriority::Medium`

---

## 5. Integration Points with Existing ethrex Code

### 5.1 Block Processing (Critical Path)

**File**: `crates/blockchain/blockchain.rs`
**Function**: `add_block_pipeline()` (line ~1908)
**Hook point**: After `store_block()` returns `Ok(())`

```rust
// After line ~1977: let result = self.store_block(block, account_updates_list, res);
#[cfg(feature = "sentinel")]
if result.is_ok() {
    if let Some(sentinel) = &self.sentinel {
        // Non-blocking: sends to channel
        sentinel.on_block_committed(
            Arc::new(block.clone()),
            Arc::new(res.receipts.clone()),
        );
    }
}
```

Also hook into `add_block()` (line ~1871) for the non-pipeline path.

### 5.2 Evm / LEVM Replay (Deep Analysis)

**File**: `crates/vm/backends/levm/mod.rs`
**Reuse**: `LEVM::execute_tx()` is called with a fresh VM that has `opcode_recorder` set.

The deep analyzer creates a VM with `OpcodeRecorder` to capture the full trace:

```rust
// In sentinel/replay.rs
let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), VMType::L1)?;

// Attach recorder
let recorder = Rc::new(RefCell::new(DebugRecorder::new(ReplayConfig::default())));
vm.opcode_recorder = Some(recorder.clone());

// Execute
let report = vm.execute()?;

// Extract steps
let steps = std::mem::take(&mut recorder.borrow_mut().steps);
```

### 5.3 Autopsy Components (Reuse)

| Component | Location | Sentinel Usage |
|-----------|----------|----------------|
| `AttackClassifier::classify_with_confidence()` | `autopsy/classifier.rs` | Called on deep analysis StepRecords |
| `FundFlowTracer::trace()` | `autopsy/fund_flow.rs` | Called on deep analysis StepRecords |
| `AutopsyReport::build()` | `autopsy/report.rs` | Optional full report generation |
| `enrich_storage_writes()` | `autopsy/enrichment.rs` | Enriches SSTORE old_value fields |
| `known_label()` | `autopsy/report.rs` | Used in pre-filter for contract identification |
| `DetectedPattern` | `autopsy/types.rs` | Included in SentinelAlert |

### 5.4 Store (State Access)

**File**: `crates/blockchain/vm.rs`
**Struct**: `StoreVmDatabase`

The deep analyzer reads committed state from the Store using `StoreVmDatabase`, which provides read-only access to the post-parent-block state. This is thread-safe since Store uses RocksDB underneath.

### 5.5 Feature Flag Propagation

The `sentinel` feature must propagate through the dependency chain:

```
tokamak-debugger (sentinel feature)
    └─ requires: autopsy + ethrex-storage + ethrex-vm + ethrex-blockchain

ethrex-blockchain (sentinel feature)
    └─ enables tokamak-debugger/sentinel

ethrex-levm (tokamak-debugger feature)
    └─ already exists, enables OpcodeRecorder

cmd/ethrex (sentinel feature)
    └─ enables ethrex-blockchain/sentinel
```

---

## 6. Alert System Design

### 6.1 Structured Log Alert

```rust
tracing::warn!(
    target: "sentinel",
    block_number = %alert.block_number,
    tx_hash = %format!("0x{:x}", alert.tx_hash),
    alert_level = %alert.alert_level,
    patterns = %pattern_names,
    value_at_risk = %alert.total_value_at_risk,
    "Attack detected: {}", alert.summary
);
```

### 6.2 JSON File Alert

Append to `<data_dir>/sentinel/alerts.jsonl` (one JSON object per line):

```json
{
  "timestamp": 1709136000,
  "block_number": 19500000,
  "block_hash": "0xabc...",
  "tx_hash": "0xdef...",
  "tx_index": 42,
  "alert_level": "Critical",
  "detected_patterns": [
    {
      "pattern": { "FlashLoan": { "borrow_step": 100, "repay_step": 5000, "..." : "..." } },
      "confidence": 0.9,
      "evidence": ["Borrow at step 100, repay at step 5000", "..."]
    }
  ],
  "fund_flows": [...],
  "total_value_at_risk": "50000000000000000000",
  "summary": "Flash Loan + Price Manipulation detected targeting Uniswap V3 pool"
}
```

### 6.3 Webhook Alert

POST to configured URL with the same JSON body. Includes:
- Retry logic (3 attempts with exponential backoff)
- Configurable timeout (default 5 seconds)
- Non-blocking (fires and forgets, errors logged)

### 6.4 Metrics

The `SentinelMetrics` struct tracks:

| Metric | Type | Description |
|--------|------|-------------|
| `blocks_scanned` | Counter | Total blocks processed by sentinel |
| `txs_scanned` | Counter | Total TXs pre-filtered |
| `txs_flagged` | Counter | TXs that passed pre-filter |
| `deep_analyses` | Counter | Full autopsy replays performed |
| `alerts_emitted` | Counter | Alerts dispatched (by level) |
| `prefilter_latency_us` | Histogram | Per-TX pre-filter time |
| `deep_analysis_latency_ms` | Histogram | Per-TX deep analysis time |
| `false_positive_overrides` | Counter | Manual FP markings (future) |

---

## 7. Performance Considerations

### 7.1 Critical Path Impact

The sentinel adds exactly ONE non-blocking operation to the block processing critical path:

```
channel.send(BlockNotification { block, receipts })
```

This is a `crossbeam_channel::Sender::send()` call, which takes ~50-200 nanoseconds. The block's `Arc<Block>` and `Arc<Vec<Receipt>>` avoid data cloning.

**Expected overhead**: < 0.001% of block processing time.

### 7.2 Pre-Filter Performance

The pre-filter runs on the background thread, not the critical path. Per-TX cost:

| Heuristic | Cost |
|-----------|------|
| Flash loan topic scan | O(L) where L = number of logs |
| Value/revert check | O(1) |
| ERC-20 count | O(L) |
| Known contract lookup | O(L) with FxHashSet |
| Gas pattern | O(1) |
| Total per TX | ~10-50 microseconds |

For a block with 200 TXs, pre-filtering takes ~2-10ms total.

### 7.3 Deep Analysis Performance

Deep analysis re-executes the transaction with opcode recording. Costs:

| Phase | Typical Duration |
|-------|-----------------|
| State setup (StoreVmDatabase) | 1-5ms |
| TX re-execution with recording | 10-200ms (depends on TX complexity) |
| Classification + fund flow tracing | 1-10ms |
| Report generation | < 1ms |
| Total | 15-250ms |

Since this runs on a background thread, it does not impact block processing.

### 7.4 Memory Budget

| Component | Memory |
|-----------|--------|
| BlockNotification channel buffer (16 deep) | ~1 MB |
| Pre-filter state (static hash sets) | ~10 KB |
| Deep analysis per TX (StepRecords) | 1-5 MB (cleared after analysis) |
| Alert file buffer | < 100 KB |
| Total steady-state | ~2-6 MB |

### 7.5 Backpressure

If the deep analysis thread cannot keep up (e.g., many suspicious TXs in consecutive blocks), the channel has a bounded capacity of 16 blocks. If full, older blocks are dropped with a warning log. This ensures the block processing critical path is never blocked.

---

## 8. Risk Assessment

### 8.1 Technical Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| State replay fails due to concurrent state pruning | Deep analysis cannot run | Low | Verify state exists before replay; use recent blocks only |
| False positive flood from pre-filter | Alert fatigue | Medium | Tunable thresholds; mandatory deep analysis before alert |
| `OpcodeRecorder` memory bloat on large TXs | OOM on analysis thread | Low | Cap StepRecords at 1M steps; skip analysis if exceeded |
| Background thread panic | Silent failure | Low | Catch panics in thread; restart worker; log error |
| Clock skew between block processing and analysis | Stale state for replay | Very Low | Always use committed state; verify block hash exists |

### 8.2 Design Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Pre-filter heuristics miss novel attack vectors | False negatives | Medium | Extensible heuristic system; add new patterns over time |
| Known exploit signatures become outdated | Reduced detection | Medium | Configurable signature DB; update via config file |
| Alert webhook DoS during attack surge | Alert loss | Low | Bounded retry; queue alerts locally |

### 8.3 Integration Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Feature flag not properly gated | Performance regression on main build | Low | CI job: `cargo clippy` without sentinel; `cargo clippy --features sentinel` |
| Upstream ethrex API changes break sentinel | Build failure | Medium | Pin to specific interfaces; integration tests |
| LEVM `tokamak-debugger` feature interaction | Unexpected opcode recording in production | Low | Sentinel uses separate recorder instance, not global |

---

## 9. Testing Strategy

### 9.1 Unit Tests

| Module | Test Count (Est.) | Focus |
|--------|-------------------|-------|
| `pre_filter.rs` | 20-25 | Each heuristic individually; scoring; threshold edge cases |
| `analyzer.rs` | 8-10 | Analysis pipeline; error handling; timeout |
| `replay.rs` | 5-8 | TX re-execution; recorder attachment; state setup |
| `alert.rs` | 8-10 | Alert formatting; JSON serialization; file append |
| `config.rs` | 5-6 | Config parsing; defaults; validation |
| `service.rs` | 5-8 | Service start/stop; channel backpressure; metrics |

### 9.2 Pre-Filter Heuristic Tests

Each heuristic needs:
1. **Positive case**: Construct a Receipt that should trigger the heuristic
2. **Negative case**: Construct a Receipt that should NOT trigger
3. **Edge case**: Boundary values (exactly at threshold)

Example for flash loan detection:
```rust
#[test]
fn test_flash_loan_topic_detected() {
    let log = Log {
        address: aave_v3_pool_address(),
        topics: vec![flash_loan_topic()],
        data: Bytes::new(),
    };
    let receipt = make_receipt(true, 500_000, vec![log]);
    let result = PreFilter::default().scan_tx(&receipt, &tx, &header);
    assert!(result.is_some());
    assert!(result.unwrap().reasons.iter().any(|r|
        matches!(r, SuspicionReason::FlashLoanSignature { .. })
    ));
}
```

### 9.3 Integration Tests

| Test | Description |
|------|-------------|
| `test_sentinel_end_to_end` | Create a block with a reentrancy-like TX, process through sentinel, verify alert is emitted |
| `test_sentinel_benign_block` | Process a block of simple ETH transfers, verify no alerts |
| `test_sentinel_backpressure` | Flood the channel, verify blocks are dropped gracefully |
| `test_sentinel_deep_analysis_replay` | Create a known attack pattern in LEVM, verify deep analysis detects it |
| `test_alert_file_output` | Verify JSONL file is correctly written and parseable |

### 9.4 Performance Tests

| Test | Description | Target |
|------|-------------|--------|
| `test_prefilter_latency` | Measure pre-filter time on 200-TX block | < 20ms total |
| `test_critical_path_overhead` | Measure `add_block` time with and without sentinel | < 5% increase |
| `test_deep_analysis_timeout` | Verify analysis respects timeout on pathologically large TX | < 10 seconds |

### 9.5 Test Commands

```bash
# Base sentinel tests (no deep analysis)
cargo test -p tokamak-debugger --features sentinel

# Full sentinel tests (with deep analysis requiring LEVM)
cargo test -p tokamak-debugger --features "sentinel,autopsy"

# Clippy for sentinel feature
cargo clippy -p tokamak-debugger --features sentinel
cargo clippy -p tokamak-debugger --features "sentinel,autopsy"

# Clippy without sentinel (verify no leakage)
cargo clippy -p tokamak-debugger
```

### 9.6 CI Jobs

Add to `tokamak-ci.yaml`:

```yaml
sentinel-tests:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Test sentinel (pre-filter only)
      run: cargo test -p tokamak-debugger --features sentinel
    - name: Test sentinel + autopsy (full pipeline)
      run: cargo test -p tokamak-debugger --features "sentinel,autopsy"
    - name: Clippy sentinel
      run: |
        cargo clippy -p tokamak-debugger --features sentinel -- -D warnings
        cargo clippy -p tokamak-debugger --features "sentinel,autopsy" -- -D warnings
```

---

## 10. Dependency Map

```
Phase H-1: Pre-Filter Engine
    ├── No dependencies on other phases
    ├── Reuses: known_label() from autopsy/report.rs
    └── Estimated: 2-3 days

Phase H-2: Deep Analysis Engine
    ├── Depends on: H-1 (types)
    ├── Reuses: AttackClassifier, FundFlowTracer, AutopsyReport, DebugRecorder
    └── Estimated: 2-3 days

Phase H-3: Block Processing Integration
    ├── Depends on: H-1 + H-2
    ├── Modifies: blockchain.rs (minimal, feature-gated)
    └── Estimated: 1-2 days

Phase H-4: Alert System
    ├── Depends on: H-1 (types)
    ├── Independent of H-2 (alerts can come from pre-filter alone)
    └── Estimated: 1-2 days

Phase H-5: Service Orchestration
    ├── Depends on: H-1 + H-2 + H-3 + H-4
    ├── Ties everything together
    └── Estimated: 1-2 days

Phase H-6: CLI & Configuration
    ├── Depends on: H-5
    ├── Light integration work
    └── Estimated: 0.5-1 day

Total estimated: 8-13 days
```

---

## 11. File Summary

### New Files (10)

| File | Lines (Est.) | Phase |
|------|-------------|-------|
| `crates/tokamak-debugger/src/sentinel/mod.rs` | 30 | H-1 |
| `crates/tokamak-debugger/src/sentinel/types.rs` | 150 | H-1 |
| `crates/tokamak-debugger/src/sentinel/pre_filter.rs` | 350 | H-1 |
| `crates/tokamak-debugger/src/sentinel/analyzer.rs` | 250 | H-2 |
| `crates/tokamak-debugger/src/sentinel/replay.rs` | 200 | H-2 |
| `crates/tokamak-debugger/src/sentinel/alert.rs` | 250 | H-4 |
| `crates/tokamak-debugger/src/sentinel/webhook.rs` | 120 | H-4 |
| `crates/tokamak-debugger/src/sentinel/service.rs` | 300 | H-5 |
| `crates/tokamak-debugger/src/sentinel/config.rs` | 100 | H-5 |
| `crates/tokamak-debugger/src/sentinel/metrics.rs` | 120 | H-5 |

### Modified Files (5)

| File | Change Size | Phase |
|------|------------|-------|
| `crates/tokamak-debugger/Cargo.toml` | +15 lines | H-1 |
| `crates/tokamak-debugger/src/lib.rs` | +2 lines | H-1 |
| `crates/blockchain/blockchain.rs` | +20 lines | H-3 |
| `crates/blockchain/Cargo.toml` | +5 lines | H-3 |
| `cmd/ethrex` CLI config | +10 lines | H-6 |

### Total New Code: ~1,870 lines (excluding tests)
### Total Test Code: ~800-1,000 lines (estimated 55-70 tests)

---

## 12. Open Questions

1. **Pruning interaction**: If the node prunes old state aggressively, deep analysis of blocks more than N blocks old may fail. Should sentinel have a configurable analysis window (e.g., only analyze last 128 blocks)?

2. **Mempool monitoring**: Should sentinel also scan pending transactions in the mempool (pre-execution)? This would enable earlier detection but requires different heuristics (no receipts available).

3. **False positive feedback**: Should there be a mechanism to mark alerts as false positives and feed that back to improve heuristic thresholds?

4. **Multi-TX attack detection**: Some exploits span multiple transactions in the same block (e.g., sandwich attacks). Should the pre-filter consider TX pairs/groups?

5. **L2 support**: Should sentinel work on L2 mode (`VMType::L2`)? The current design assumes L1, but the architecture is compatible with L2 with minor adjustments to fee handling.
