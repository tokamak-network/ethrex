# H-6 Expanded: CLI + Mempool Monitoring + Adaptive Analysis Pipeline + Auto Pause

**Created**: 2026-03-01
**Branch**: `feat/tokamak-autopsy`
**Status**: COMPLETE (implemented 2026-03-01)
**Prerequisite**: H-1~H-5 all complete (263+10 tests)

---

## Context

Phase H (Sentinel real-time attack detection) is complete (H-1~H-5, 263+10 tests). The sentinel detects attacks in committed blocks via receipt heuristics + opcode replay, but:
1. **Not wired to CLI** — operators can't enable it in production
2. **Post-execution only** — attacks detected after damage done
3. **Fixed analysis pipeline** — no dynamic step skipping, no anomaly scoring
4. **Alert-only response** — no automated protective action

H-6 expands across 4 sub-tasks to address all of these.

---

## H-6a: CLI & Configuration

### Goal
Wire `SentinelService` into the ethrex CLI so operators can activate sentinel via `--sentinel.enabled` flag or TOML config.

### Files

| File | Action | Lines |
|------|--------|-------|
| `crates/tokamak-debugger/src/sentinel/config.rs` | **NEW** | ~180 |
| `crates/tokamak-debugger/src/sentinel/mod.rs` | Edit (+1) | `pub mod config;` |
| `cmd/ethrex/Cargo.toml` | Edit (+5) | `tokamak-debugger` dep + `sentinel` feature |
| `cmd/ethrex/cli.rs` | Edit (+30) | 6 `--sentinel.*` CLI flags |
| `cmd/ethrex/initializers.rs` | Edit (+70) | `init_sentinel()` + modify `init_blockchain()` signature |

### Key Design

**config.rs** — `SentinelFullConfig` (TOML-compatible, `Deserialize`):
```rust
pub struct SentinelFullConfig {
    pub enabled: bool,
    pub prefilter: SentinelConfig,        // reuse existing type
    pub analysis: AnalysisConfig,          // reuse existing type
    pub alert: AlertOutputConfig,          // jsonl_path, webhook_url, rate_limit, dedup_window
    pub mempool: MempoolMonitorConfig,     // enabled, min_value, min_gas
    pub auto_pause: AutoPauseConfig,       // enabled, confidence_threshold, priority_threshold
    pub pipeline: AdaptivePipelineConfig,   // enabled, model_path, max_pipeline_ms
}
pub fn load_config(path: Option<&PathBuf>) -> Result<SentinelFullConfig, String>;
```

**CLI flags** (all `#[cfg(feature = "sentinel")]` gated in `Options`):
- `--sentinel.enabled` (bool, env: `ETHREX_SENTINEL_ENABLED`)
- `--sentinel.config` (PathBuf, TOML path)
- `--sentinel.alert-file` (PathBuf, JSONL output)
- `--sentinel.auto-pause` (bool)
- `--sentinel.mempool` (bool)
- `--sentinel.webhook-url` (String)

**init_sentinel()** in `initializers.rs`:
1. Load TOML config (if `--sentinel.config` provided)
2. Build alert handler pipeline: `LogAlertHandler` + optional `JsonlFileAlertHandler` + optional `WebhookAlertHandler` → `AlertDeduplicator` → `AlertRateLimiter` → `AlertDispatcher`
3. Create `SentinelService::new(store, config, analysis_config, alert_handler)`
4. Optionally create `PauseController` + `AutoPauseHandler` (H-6d)
5. Return `Arc<dyn BlockObserver>` to attach via `Blockchain::with_block_observer()`

**Integration**: Modify `init_blockchain()` to accept an options struct (defined in `cmd/ethrex/initializers.rs`, avoids 5-parameter signature bloat):
```rust
/// Groups all optional sentinel-related components for blockchain initialization.
/// Defined in cmd/ethrex — only the CLI layer assembles these components.
/// All fields default to `None` — zero overhead when sentinel is not configured.
pub struct SentinelComponents {
    pub block_observer: Option<Arc<dyn BlockObserver>>,
    pub mempool_observer: Option<Arc<dyn MempoolObserver>>,  // H-6b
    pub pause_controller: Option<Arc<PauseController>>,      // H-6d
}

pub fn init_blockchain(
    store: Store,
    opts: BlockchainOptions,
    sentinel: SentinelComponents,  // NEW — defaults to all-None via Default impl
) -> Arc<Blockchain>
```

### Tests (10)
- TOML roundtrip, default config, CLI override, alert pipeline composition

---

## H-6b: Mempool Monitoring

### Goal
Detect suspicious pending TXs BEFORE execution using calldata heuristics.

### Files

| File | Action | Lines |
|------|--------|-------|
| `crates/blockchain/blockchain.rs` | Edit (+25) | `MempoolObserver` trait + field + hooks |
| `crates/tokamak-debugger/src/sentinel/mempool_filter.rs` | **NEW** | ~300 |
| `crates/tokamak-debugger/src/sentinel/types.rs` | Edit (+30) | `MempoolAlert`, `MempoolSuspicionReason` |
| `crates/tokamak-debugger/src/sentinel/service.rs` | Edit (+40) | `MempoolObserver` impl, `MempoolTransaction` message |
| `crates/tokamak-debugger/src/sentinel/metrics.rs` | Edit (+20) | 3 mempool counters |
| `crates/tokamak-debugger/src/sentinel/mod.rs` | Edit (+1) | `pub mod mempool_filter;` |

### Key Design

**MempoolObserver** trait (in `blockchain.rs`, alongside `BlockObserver`):
```rust
pub trait MempoolObserver: Send + Sync {
    fn on_transaction_added(&self, tx: &Transaction, sender: Address, tx_hash: H256);
}
```

**Hook points** — After `mempool.add_transaction()` succeeds in:
- `add_transaction_to_pool()` (line 2503): `observer.on_transaction_added(&transaction, sender, hash)`
- `add_blob_transaction_to_pool()` (line 2478): same pattern

**MempoolPreFilter** — Stateless, read-only, <100μs budget:
- 5 heuristics on calldata (no receipts/logs available):
  1. **Flash loan selector** — match first 4 bytes against Aave/Balancer/dYdX selectors
  2. **High value DeFi** — `tx.value > min_value_wei` AND `tx.to` is known DeFi contract
  3. **High gas + known contract** — `gas_limit > 500k` AND target is DeFi protocol
  4. **Suspicious contract creation** — `TxKind::Create` with large init code (>10KB)
  5. **Multicall pattern** — match `multicall(bytes[])` selector (0xac9650d8) on known DeFi routers. No recursive calldata parsing — selector-only matching, same as heuristic #1.

**Known selectors database** (`FxHashSet<[u8; 4]>`):
- Aave `flashLoan` (0xab9c4b5d), Uniswap V2 `swap` (0x38ed1738), V3 `exactInputSingle` (0x414bf389)
- Balancer `flashLoan` (0x5c38449e), Compound `borrow` (0xc5ebeaec)

**SentinelService integration**:
- `MempoolPreFilter` stored on service (immutable after construction, `Send + Sync` safe)
- `on_transaction_added()` runs filter inline (<100μs), sends `MempoolAlert` via channel only if flagged
- Worker thread handles `MempoolTransaction` message → dispatches to `AlertHandler`

**New metrics**: `mempool_txs_scanned`, `mempool_txs_flagged`, `mempool_alerts_emitted`

### Tests (20)
- 5 per heuristic (positive/negative/edge), 5 integration (observer flow, metrics, channel)

---

## H-6c: Adaptive Analysis Pipeline

### Goal
Replace the fixed DeepAnalyzer flow with a dynamic multi-step pipeline that can skip/add steps at runtime and extract features for statistical anomaly scoring.

> **Naming note**: "Adaptive" (not "Agentic") — the pipeline uses conditional branching and dynamic step injection, not LLM-based autonomous reasoning. The `AnalysisStep` trait enables extensibility, but orchestration follows a deterministic pipeline pattern.

### Files

| File | Action | Lines |
|------|--------|-------|
| `crates/tokamak-debugger/src/sentinel/pipeline.rs` | **NEW** | ~400 |
| `crates/tokamak-debugger/src/sentinel/ml_model.rs` | **NEW** | ~150 |
| `crates/tokamak-debugger/src/sentinel/mod.rs` | Edit (+2) | modules |
| `crates/tokamak-debugger/src/sentinel/analyzer.rs` | Edit (+15) | delegate to pipeline |
| `crates/tokamak-debugger/src/sentinel/service.rs` | Edit (+10) | pipeline init in worker |
| `crates/tokamak-debugger/src/sentinel/types.rs` | Edit (+15) | `FeatureVector` |

### Key Types

**AnalysisContext** — Accumulated findings shared across steps:
```rust
pub struct AnalysisContext {
    pub replay_result: Option<ReplayResult>,
    #[cfg(feature = "autopsy")]
    pub patterns: Vec<DetectedPattern>,
    #[cfg(feature = "autopsy")]
    pub fund_flows: Vec<FundFlow>,
    pub features: Option<FeatureVector>,
    pub anomaly_score: Option<f64>,
    pub final_confidence: Option<f64>,
    pub evidence: Vec<String>,
    pub dismissed: bool,  // short-circuit flag
}
```

**FeatureVector** — Numeric features extracted from opcode trace:
```rust
pub struct FeatureVector {
    pub total_steps: u32,
    pub unique_addresses: u32,
    pub max_call_depth: u32,
    pub sstore_count: u32, pub sload_count: u32,
    pub call_count: u32, pub delegatecall_count: u32,
    pub staticcall_count: u32, pub create_count: u32,
    pub selfdestruct_count: u32, pub log_count: u32,
    pub revert_count: u32,
    pub reentrancy_depth: u32,
    pub eth_transferred_wei: f64,
    pub gas_ratio: f64,
    pub calldata_entropy: f64,
}
```

**AnalysisStep** trait — Each step in the pipeline:
```rust
pub trait AnalysisStep: Send {
    fn name(&self) -> &'static str;
    fn execute(
        &self,
        ctx: &mut AnalysisContext,
        store: &Store,
        block: &Block,
        suspicion: &SuspiciousTx,
        config: &AnalysisConfig,
    ) -> Result<StepResult, SentinelError>;
}

pub enum StepResult {
    Continue,
    Dismiss,                               // early exit — TX is benign
    AddSteps(Vec<Box<dyn AnalysisStep>>),  // dynamic follow-up
}
```

### Pipeline Steps (6)

| # | Step | Reuses | Key Logic |
|---|------|--------|-----------|
| 1 | `TraceAnalyzer` | `replay.rs` | Replay TX, extract StepRecords |
| 2 | `PatternMatcher` | `classifier.rs` | Run AttackClassifier. **Dismiss** if no CALL opcodes (skip reentrancy/flash loan) |
| 3 | `FundFlowAnalyzer` | `fund_flow.rs` | Trace ETH + ERC-20 movements |
| 4 | `AnomalyDetector` | — | Extract `FeatureVector`, run `AnomalyModel::predict()` |
| 5 | `ConfidenceScorer` | — | Weighted combination: pattern confidence × 0.4 + anomaly score × 0.3 + prefilter score × 0.2 + fund flow magnitude × 0.1 (see rationale below) |
| 6 | `ReportGenerator` | — | Build `SentinelAlert` from `AnalysisContext` |

**ConfidenceScorer Weight Rationale** — initial values ranked by signal reliability:

1. **Pattern matching (0.4)**: Deterministic, lowest false-positive rate among our classifiers.
2. **Anomaly score (0.3)**: Statistical, useful for novel attacks but prone to false positives on unusual-but-benign TXs.
3. **Prefilter score (0.2)**: Receipt-level heuristics, fast but coarse.
4. **Fund flow magnitude (0.1)**: High-value transfers correlate with attacks but also with legitimate DeFi operations.

These weights are stored in `AnalysisConfig` and configurable via TOML (`[sentinel.analysis]` section). Calibration against the baseline dataset (see ML Model section) should be performed before production deployment.

### AnalysisPipeline Orchestrator

```rust
pub struct AnalysisPipeline {
    steps: Vec<Box<dyn AnalysisStep>>,
}

impl AnalysisPipeline {
    pub fn analyze(&self, store, block, suspicion, config) -> Result<Option<SentinelAlert>> {
        let mut ctx = AnalysisContext::default();
        let mut dynamic: VecDeque<Box<dyn AnalysisStep>> = VecDeque::new();

        // Phase 1: Run initial steps
        for step in &self.steps {
            if ctx.dismissed { break; }
            match step.execute(&mut ctx, store, block, suspicion, config)? {
                StepResult::Continue => {},
                StepResult::Dismiss => { ctx.dismissed = true; },
                StepResult::AddSteps(new) => dynamic.extend(new),
            }
        }
        // Phase 2: Run dynamically added steps
        while let Some(step) = dynamic.pop_front() {
            if ctx.dismissed { break; }
            match step.execute(&mut ctx, store, block, suspicion, config)? {
                StepResult::Continue => {},
                StepResult::Dismiss => { ctx.dismissed = true; },
                StepResult::AddSteps(new) => dynamic.extend(new),
            }
        }
        if ctx.dismissed { return Ok(None); }
        ctx.to_alert(block, suspicion)
    }
}
```

### Pipeline Observability

Each `AnalysisStep::execute()` call is wrapped with `Instant::now()` timing. The following counters are added to `SentinelMetrics`:
- `pipeline_steps_executed: u64` — total step executions across all analyzed TXs
- `pipeline_steps_dismissed: u64` — steps that returned `StepResult::Dismiss`
- `pipeline_duration_ms: u64` — cumulative pipeline execution time (all steps per TX)
- `pipeline_step_durations: HashMap<&'static str, u64>` — per-step cumulative duration (keyed by `step.name()`)

These metrics are exposed via the existing Prometheus text endpoint (`/metrics`) alongside H-5's block-level counters. When `pipeline_duration_ms / pipeline_steps_executed` exceeds the 500ms budget, the bottleneck step is identifiable from `pipeline_step_durations`.

### Feature Gating

All H-6c code (`pipeline.rs`, `ml_model.rs`, `AnalysisStep` impls, `FeatureVector`) lives under the existing `sentinel` feature — **no separate `pipeline` feature is needed**. Rationale: the adaptive pipeline is the natural evolution of the sentinel's deep analysis stage. Splitting into a sub-feature would complicate the test matrix without meaningful build-size savings (the code adds ~550 lines of pure Rust with zero new external dependencies).

Test commands remain unchanged from the Verification section — `--features sentinel` includes all pipeline code.

### ML Model (No External Dependencies)

**AnomalyModel** trait + `StatisticalAnomalyDetector` (built-in):
```rust
pub trait AnomalyModel: Send + Sync {
    fn predict(&self, features: &FeatureVector) -> f64;  // 0.0 benign -> 1.0 malicious
}

pub struct StatisticalAnomalyDetector {
    means: FeatureVector,     // from mainnet baseline (see calibration below)
    stddevs: FeatureVector,   // from mainnet baseline (see calibration below)
}
// Computes average |z-score| across features, maps to 0-1 via sigmoid
```

No `linfa` or external ML crate needed. The `AnomalyModel` trait allows future swap to any model (Linfa, ONNX, etc.) without pipeline changes.

**Baseline Calibration Methodology**:
1. **Data source**: Mainnet blocks 18,000,000–19,000,000 (~2.5M transactions, Oct 2023–Jan 2024). This range includes both normal activity and known exploits (e.g., Curve pool reentrancy aftermath, KyberSwap).
2. **Labeling**: "Attack" TXs are identified from published exploit databases (rekt.news, DeFiLlama hacks tracker). All other TXs in sampled blocks are labeled "benign". Minimum sample: 50 labeled attacks, 10,000 benign TXs.
3. **Feature extraction**: Run `FeatureVector` extraction on sampled TXs via a one-time offline script (not shipped in production). Compute per-feature mean and stddev from the benign-only set.
4. **Validation**: Verify that known attack TXs produce anomaly scores >0.7 and benign TXs score <0.3 on the calibration set. If separation is poor, adjust feature weights or add discriminative features.
5. **Deployment**: Hardcode validated mean/stddev into `StatisticalAnomalyDetector::default()`. Include a `const CALIBRATION_BLOCK_RANGE: (u64, u64) = (18_000_000, 19_000_000);` for traceability.
6. **Re-calibration**: When false positive rate exceeds 5% in production (measured via `SentinelMetrics`), re-run calibration on a more recent block range. The `AnomalyModel` trait allows hot-swapping without pipeline changes.

**Scope note**: The calibration script is **outside H-6 scope**. `StatisticalAnomalyDetector::default()` ships with conservative placeholder values (high stddev = low sensitivity) that avoid false positives at the cost of recall. Actual calibration is a follow-up task requiring archive node access and labeled attack data.

### SentinelAlert Field Mapping

The `AnalysisContext.final_confidence` maps to the existing `SentinelAlert.suspicion_score` field (reuse, not a new field). When the adaptive pipeline produces a `final_confidence`, it replaces the prefilter's `suspicion_score` with a more refined value. Additionally, `AnalysisContext.features` is stored in a new optional field `SentinelAlert.feature_vector: Option<FeatureVector>` for downstream consumers (dashboard, JSONL logs) that want raw feature data.

### DeepAnalyzer Integration

`DeepAnalyzer::analyze()` gets optional `pipeline` parameter:
```rust
pub fn analyze(store, block, suspicion, config, pipeline: Option<&AnalysisPipeline>)
    -> Result<Option<SentinelAlert>>
{
    if let Some(pipeline) = pipeline {
        return pipeline.analyze(store, block, suspicion, config);
    }
    // ... existing fixed pipeline as fallback
}
```

### Tests (28)
- Step unit tests (6: one per step), dismiss/skip tests (4), dynamic AddSteps (3)
- FeatureVector extraction (3), anomaly scoring (3), confidence scoring (3)
- Full pipeline integration (6: reentrancy, flash loan, benign, timeout, no-autopsy fallback, dismiss-early)

---

## H-6d: Auto Pause (Block Processing Suspension)

### Goal
Automatically pause block execution when a Critical attack is detected with high confidence.

### Files

| File | Action | Lines |
|------|--------|-------|
| `crates/blockchain/blockchain.rs` | Edit (+45) | `PauseController` struct + field + 2 checkpoints |
| `crates/tokamak-debugger/src/sentinel/auto_pause.rs` | **NEW** | ~80 |
| `crates/tokamak-debugger/src/sentinel/mod.rs` | Edit (+1) | module |
| `crates/networking/rpc/rpc.rs` | Edit (+30) | `sentinel_resume` + `sentinel_status` RPC methods |
| `cmd/ethrex/initializers.rs` | Edit (+15) | wire PauseController to Blockchain + RPC |

### PauseController (in `blockchain.rs`, NOT feature-gated)

```rust
pub struct PauseController {
    paused: AtomicBool,
    lock: Mutex<()>,
    condvar: Condvar,
    auto_resume_secs: Option<u64>,  // default: Some(300) = 5 minutes
    paused_at: Mutex<Option<Instant>>,
}
```

- `new(auto_resume_secs: Option<u64>)` — constructor. `None` = no auto-resume (manual only). `Some(300)` = 5 min default.
- `pause()` — set flag, record `Instant::now()` in `paused_at`, returns immediately
- `resume()` — idempotent: uses `paused.compare_exchange(true, false, ...)` so double-resume is safe. Clears `paused_at`, wakes all blocked threads via `condvar.notify_all()`
- `wait_if_paused()` — fast path: single atomic load (<1ns). Slow path: `condvar.wait_timeout(Duration::from_secs(auto_resume_secs))`. On timeout expiry: auto-call `resume()` + `eprintln!("[SENTINEL] Auto-resume after {}s timeout", secs)`.
- `is_paused()` — check without blocking

### Checkpoint Locations

1. **`add_block_pipeline()`** (line 1965) — BEFORE `find_parent_header`:
```rust
if let Some(pause) = &self.pause_controller {
    pause.wait_if_paused();
}
```

2. **`add_blocks_in_batch()`** (line 2336) — alongside `cancellation_token.is_cancelled()`:
```rust
if let Some(pause) = &self.pause_controller {
    pause.wait_if_paused();
}
```

Both are BETWEEN complete block executions — state always consistent.

### AutoPauseHandler (in `tokamak-debugger`, sentinel feature-gated)

Implements `AlertHandler` trait:
```rust
pub struct AutoPauseHandler {
    controller: Arc<PauseController>,
    confidence_threshold: f64,      // default: 0.8
    priority_threshold: AlertPriority,  // default: Critical
}

impl AlertHandler for AutoPauseHandler {
    fn on_alert(&self, alert: SentinelAlert) {
        if meets_priority && meets_confidence {
            eprintln!("[SENTINEL AUTO-PAUSE] ...");
            self.controller.pause();
        }
    }
}
```

### Safety Guarantees
- P2P continues operating — only block execution methods check pause
- No mid-execution pause — only at block boundaries
- Auto-resume timeout via `condvar.wait_timeout()` (configurable, default 5min) to prevent permanent halt
- Resume via: `sentinel_resume` RPC endpoint, auto-resume timeout, or node restart

### Resume RPC Endpoint

Two new JSON-RPC methods in `ethrex-rpc` (feature-gated under `sentinel`):

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `sentinel_resume` | none | `bool` | Calls `PauseController::resume()`. Returns `true` if was paused, `false` if already running. |
| `sentinel_status` | none | `{ paused: bool, paused_for_secs: Option<u64>, auto_resume_in: Option<u64> }` | Non-blocking status check. `paused_for_secs` = `Instant::now() - paused_at` (duration, not timestamp — avoids `Instant` to epoch conversion). `auto_resume_in` = remaining seconds before auto-resume. |

**File**: `crates/networking/rpc/rpc.rs` — add route handler + `PauseController` reference via `Arc` stored in RPC context.

**Access control**: `sentinel_resume` and `sentinel_status` are placed in the `admin` RPC namespace, following ethrex's existing namespace-based access control. By default, the `admin` namespace is only exposed on the IPC/localhost interface, not on the public HTTP endpoint. Operators who expose admin RPCs publicly are responsible for firewall-level restrictions.

> **Note**: The `PauseController` is passed to both `Blockchain` (for checkpoint blocking) and `RpcHandler` (for resume control). Both hold `Arc<PauseController>` — no ownership conflict.

### Tests (12)
- PauseController unit (5: fast path, pause/resume, concurrent, auto-resume timeout expiry, manual resume before timeout)
- AutoPauseHandler (3: critical triggers, high doesn't trigger, below confidence)
- RPC (2: sentinel_resume when paused, sentinel_status fields)
- Integration (2: pause in add_block_pipeline + resume, add_blocks_in_batch + auto-resume)

---

## Feature Propagation

```
cmd/ethrex (sentinel feature)
  |-- tokamak-debugger/sentinel  (includes adaptive pipeline, mempool filter, auto-pause handler)
  |-- tokamak-debugger/autopsy   (deep analysis — pattern matcher + fund flow steps need this)
  |-- ethrex-blockchain          (BlockObserver, MempoolObserver, PauseController — NOT feature-gated)
  '-- ethrex-rpc                 (sentinel_resume/sentinel_status — feature-gated)

Blockchain struct fields:
  block_observer: Option<Arc<dyn BlockObserver>>       (existing)
  mempool_observer: Option<Arc<dyn MempoolObserver>>   (H-6b, NEW)
  pause_controller: Option<Arc<PauseController>>       (H-6d, NEW)

Note: All H-6c code (pipeline.rs, ml_model.rs) is gated under `sentinel` feature — no separate
`pipeline` sub-feature. The adaptive pipeline is integral to the sentinel's deep analysis stage.
```

---

## Execution Order

```
Phase 1 (H-6a + H-6d struct):  CLI flags, config.rs, PauseController struct
  |-- No deps on H-6b/H-6c
  '-- ~2-3 days

Phase 2 (H-6b):  MempoolObserver trait, mempool_filter.rs, service extension
  |-- Depends: H-6a (config types)
  '-- ~2-3 days

Phase 3 (H-6c):  pipeline.rs, ml_model.rs, AnalysisStep adaptive pipeline
  |-- Depends: H-6a (config types)
  |-- Independent of H-6b
  '-- ~3-4 days

Phase 4 (H-6d wiring):  auto_pause.rs, checkpoint insertion, RPC endpoints, init glue
  |-- Depends: Phase 1 (PauseController), H-6a (init_sentinel)
  '-- ~2-3 days

Phase 5 (Integration):  Wire everything in init_sentinel, test full flow
  |-- Depends: All above
  '-- ~1 day
```

H-6b and H-6c are **parallelizable** after Phase 1.

---

## Summary

| Sub-task | New Files | Modified Files | New Lines | Tests |
|----------|-----------|---------------|-----------|-------|
| H-6a CLI/Config | 1 (config.rs) | 4 | ~285 | 10 |
| H-6b Mempool | 1 (mempool_filter.rs) | 5 | ~415 | 20 |
| H-6c Adaptive Pipeline | 2 (pipeline.rs, ml_model.rs) | 4 | ~580 | 28 |
| H-6d Auto Pause | 1 (auto_pause.rs) | 4 (+rpc.rs) | ~175 | 12 |
| **Total** | **5 new** | **9 modified** (deduplicated) | **~1,455** | **70** |

Modified file count is deduplicated — `mod.rs`, `types.rs`, `service.rs`, `blockchain.rs`, `initializers.rs` are touched by multiple sub-tasks.

Expected test count: **333 + 10 ignored** (263 existing + 70 new)

---

## Verification

```bash
# Full test suite
cargo test -p tokamak-debugger --features "cli,autopsy,sentinel"

# Sentinel-only (no autopsy -- pipeline falls back to fixed analysis)
cargo test -p tokamak-debugger --features sentinel

# PauseController tests
cargo test -p ethrex-blockchain pause_controller

# Clippy
cargo clippy -p tokamak-debugger --features "cli,autopsy,sentinel" -- -D warnings
cargo clippy -p ethrex-blockchain -- -D warnings

# Build verification (sentinel disabled -- zero overhead)
cargo check -p ethrex
```

### E2E Integration Scenario

Beyond unit tests, the following end-to-end scenario must pass (implemented as an integration test or executable example, extending `examples/reentrancy_demo.rs`):

1. **Init**: Create `SentinelFullConfig` with `auto_pause.enabled = true`, `auto_pause.confidence_threshold = 0.5`, `mempool.enabled = true`
2. **Wire**: `init_sentinel()` → `SentinelService` + `AutoPauseHandler` + `PauseController` + `MempoolPreFilter`
3. **Attach**: `Blockchain::with_block_observer()` + `with_mempool_observer()` + `with_pause_controller()`
4. **Inject Mempool TX**: Submit a pending TX with flash loan selector (0xab9c4b5d) via mempool → verify `mempool_txs_flagged >= 1`
5. **Execute Block**: Process a test block containing a reentrancy attack TX (reuse E2E bytecode)
6. **Verify PreFilter**: `SentinelMetrics.blocks_scanned >= 1`, `txs_flagged >= 1`
7. **Verify Pipeline**: Adaptive pipeline runs all 6 steps, `pipeline_steps_executed >= 6`
8. **Verify Auto-Pause**: `PauseController::is_paused() == true` (Critical alert with confidence > 0.5)
9. **Verify Resume**: Call `PauseController::resume()`, confirm `is_paused() == false`
10. **Verify Metrics**: All pipeline metrics (steps, durations, dismissals) + mempool metrics (`mempool_txs_scanned`, `mempool_alerts_emitted`) are non-zero

This scenario validates the full chain: mempool scan → block commit → observer → prefilter → adaptive pipeline → alert → auto-pause → resume.

---

## Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| MempoolObserver latency on TX pool hot path | Medium | Filter is read-only <100μs, benchmark in test |
| PauseController deadlock if resume never called | High | `condvar.wait_timeout()` auto-resume (default 5min) + `sentinel_resume` RPC |
| Adaptive pipeline exceeds 500ms budget | Medium | Per-step `Instant::now()` timing + `pipeline_step_durations` metrics for bottleneck identification |
| StatisticalAnomalyDetector calibration wrong | Medium | Calibration against mainnet 18M-19M block range (see methodology), `AnomalyModel` trait allows hot-swap, re-calibrate when FP rate >5% |
| ConfidenceScorer weight tuning | Low | Weights stored in `AnalysisConfig` (TOML-configurable), initial values based on signal reliability ranking, calibration before production |
| Feature propagation breaks default build | High | All `#[cfg(feature = "sentinel")]` gated, CI verifies |

---

## Architecture Diagram

```
                          ethrex Node
         +------------------------------------------+
         |                                          |
         |  RPC/P2P  -->  Mempool.add_transaction() |
         |                    |                     |
         |                    v                     |
         |  [H-6b] MempoolObserver.on_tx_added()   |
         |           |  (<100us calldata scan)      |
         |           v                              |
         |  Blockchain.add_block_pipeline()         |
         |     |                                    |
         |  [H-6d] PauseController.wait_if_paused() |
         |     |                                    |
         |     v                                    |
         |  LEVM execute_block()                    |
         |     |                                    |
         |     v                                    |
         |  store_block()                           |
         |     |                                    |
         |  [H-1] BlockObserver.on_block_committed()|
         +-----|------------------------------------+
               |
               v
         Sentinel Worker Thread (mpsc channel)
         +------------------------------------------+
         |                                          |
         |  Stage 1: PreFilter (7 heuristics)       |
         |     |                                    |
         |     v  suspicious TXs                    |
         |                                          |
         |  Stage 2: [H-6c] Adaptive Pipeline        |
         |     Step 1: TraceAnalyzer (replay)       |
         |     Step 2: PatternMatcher (classifier)  |
         |     Step 3: FundFlowAnalyzer             |
         |     Step 4: AnomalyDetector (ML/stats)   |
         |     Step 5: ConfidenceScorer             |
         |     Step 6: ReportGenerator              |
         |     |                                    |
         |     v  SentinelAlert                     |
         |                                          |
         |  AlertHandler Pipeline                   |
         |     RateLimiter -> Deduplicator ->        |
         |     Dispatcher -> [Log, JSONL, Webhook,  |
         |                    WS, AutoPause]        |
         |                         |                |
         |                [H-6d]   v                |
         |              PauseController.pause()     |
         +------------------------------------------+
```
