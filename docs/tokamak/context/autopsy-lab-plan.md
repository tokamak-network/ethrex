# Smart Contract Autopsy Lab — Implementation Plan

> Time-Travel Debugger(E-1/E-2/E-3)를 해킹 사후 분석 서비스로 전환.
> Volkov Review R5 예상 점수 7.5 (PROCEED) — 인프라 완성, 고객 명확, 즉시 가능.

## 현재 인프라 (이미 완성)

| Component | Status | Path | Notes |
|-----------|--------|------|-------|
| ReplayEngine | ✅ | `tokamak-debugger/src/engine.rs` | forward/backward/goto |
| OpcodeRecorder | ✅ | `levm/src/debugger_hook.rs` | 매 스텝: pc, opcode, gas, stack, depth, address |
| debug_timeTravel RPC | ✅ | `networking/rpc/debug/time_travel.rs` | HTTP/WS 접근 가능 |
| prepare_state_for_tx | ✅ | `blockchain/tracing.rs` | 로컬 블록 상태 재구성 |
| VmDatabase trait | ✅ | `vm/db.rs` | 확장 가능한 DB 인터페이스 |
| GeneralizedDatabase | ✅ | `levm/src/db/gen_db.rs` | initial vs current 상태 비교 |

## 빠진 것 4가지

| Gap | 설명 | 중요도 |
|-----|------|--------|
| Remote State Fetching | 메인넷 해킹 TX 분석에 archive node 접근 필요 | **Critical** |
| Attack Pattern Detection | 리엔트런시, 플래시론 등 자동 분류 | High |
| Fund Flow Tracing | ETH/토큰 이동 추적 | High |
| Report Generation | 구조화된 사후 분석 보고서 | Medium |

## 아키텍처

```
┌─ Input ────────────────────────────────────────────┐
│  TX hash + Archive RPC URL (Alchemy/Infura)        │
└────────────────────┬───────────────────────────────┘
                     ▼
┌─ Phase 1: State Reconstruction ────────────────────┐
│  RemoteVmDatabase                                  │
│    eth_getCode(addr, block)                        │
│    eth_getStorageAt(addr, key, block)              │
│    eth_getBalance(addr, block)                     │
│  → GeneralizedDatabase에 lazy-load                 │
└────────────────────┬───────────────────────────────┘
                     ▼
┌─ Phase 2: Replay ──────────────────────────────────┐
│  ReplayEngine::record() [이미 있음]                │
│  + 확장: CALL/DELEGATECALL value 추적              │
│  + 확장: storage diff (slot별 before/after)        │
│  → Vec<StepRecord> + Vec<CallTrace> + StorageDiff  │
└────────────────────┬───────────────────────────────┘
                     ▼
┌─ Phase 3: Analysis ────────────────────────────────┐
│  AttackClassifier                                  │
│    - Reentrancy: CALL → (같은 컨트랙트) SSTORE    │
│    - Flash Loan: 큰 CALL value → 끝에 repay       │
│    - Price Oracle: STATICCALL(oracle) → SWAP →     │
│                    STATICCALL(oracle)               │
│    - Access Control: SSTORE without CALLER check   │
│  FundFlowTracer                                    │
│    - CALL/CREATE의 value 추적                      │
│    - ERC20 Transfer 이벤트 디코딩 (LOG3)           │
│  4byteDirectory                                    │
│    - function selector → human-readable name       │
└────────────────────┬───────────────────────────────┘
                     ▼
┌─ Phase 4: Report ──────────────────────────────────┐
│  AutopsyReport {                                   │
│    summary, timeline, attack_vector,               │
│    fund_flow, storage_diff, suggested_fix          │
│  }                                                 │
│  → JSON + Markdown 출력                            │
└────────────────────────────────────────────────────┘
```

## Step 1: RemoteVmDatabase (Critical Path)

현재 VmDatabase trait을 구현하는 새 백엔드. Foundry의 fork mode와 동일한 역할.

```rust
/// Archive RPC를 통해 메인넷 상태를 lazy-load하는 DB 백엔드.
pub struct RemoteVmDatabase {
    rpc_url: String,
    block_number: u64,
    client: reqwest::Client,
    // 한 번 가져온 데이터는 캐시 (같은 slot을 반복 조회하지 않음)
    account_cache: RwLock<FxHashMap<Address, AccountState>>,
    storage_cache: RwLock<FxHashMap<(Address, H256), U256>>,
    code_cache: RwLock<FxHashMap<H256, Code>>,
}
```

각 trait 메서드는 캐시 miss 시 RPC 호출:
- `get_account_state()` → `eth_getBalance` + `eth_getTransactionCount` + `eth_getCode`
- `get_storage_slot()` → `eth_getStorageAt`
- `get_block_hash()` → `eth_getBlockByNumber`
- `get_account_code()` → `eth_getCode`

**이것만 있으면 어떤 메인넷 TX든 리플레이 가능.**

예상 작업량: **2-3일**

## Step 2: StepRecord 확장

현재 StepRecord에 없는 필드 추가:

```rust
pub struct StepRecord {
    // ... 기존 필드 유지
    pub call_value: Option<U256>,          // CALL/CREATE의 전송 금액
    pub storage_writes: Vec<StorageWrite>, // SSTORE 시 {addr, slot, old, new}
    pub log_topics: Option<Vec<H256>>,     // LOG0-LOG4의 topics (이벤트 추적)
}

pub struct StorageWrite {
    pub address: Address,
    pub slot: H256,
    pub old_value: U256,
    pub new_value: U256,
}
```

OpcodeRecorder hook을 확장해서 SSTORE/CALL/LOG 시 추가 데이터 캡처.
기존 hook 구조에 필드만 추가하면 됨.

예상 작업량: **1-2일**

## Step 3: AttackClassifier + FundFlowTracer

### Attack Pattern Types

```rust
pub enum AttackPattern {
    Reentrancy {
        vulnerable_function: String,    // "withdraw(uint256)"
        reentrant_call_step: usize,     // 재진입이 발생한 step index
        state_modified_step: usize,     // 재진입 후 SSTORE step
    },
    FlashLoan {
        borrow_step: usize,
        borrow_amount: U256,
        repay_step: usize,
        profit: U256,
    },
    PriceManipulation {
        oracle_read_before: usize,
        swap_step: usize,
        oracle_read_after: usize,
        price_delta_percent: f64,
    },
    AccessControlBypass {
        sstore_step: usize,
        missing_check: String,          // "no CALLER verification"
    },
    Unknown,
}
```

### Classification Logic

```rust
fn classify(steps: &[StepRecord]) -> Vec<AttackPattern> {
    let mut patterns = vec![];

    // Reentrancy: depth가 증가한 후 같은 address에서 SSTORE
    // Flash Loan: 첫 CALL의 value가 마지막 CALL의 value와 비슷
    // Price Manipulation: STATICCALL(oracle) 패턴이 2번, 사이에 SWAP

    patterns
}
```

### Fund Flow Tracing

```rust
pub struct FundFlow {
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub token: Option<Address>,  // None = ETH, Some = ERC20
    pub step_index: usize,
}

fn trace_funds(steps: &[StepRecord]) -> Vec<FundFlow> {
    // CALL with value > 0 → ETH transfer
    // LOG3 with Transfer(address,address,uint256) topic → ERC20 transfer
}
```

예상 작업량: **3-5일** (초기 버전은 reentrancy + flash loan만)

## Step 4: Report Generation

### Output Structure

```rust
pub struct AutopsyReport {
    pub tx_hash: H256,
    pub block_number: u64,
    pub timestamp: u64,
    pub summary: String,               // "Reentrancy attack on Vault.withdraw()"
    pub attack_patterns: Vec<AttackPattern>,
    pub fund_flow: Vec<FundFlow>,
    pub storage_diff: Vec<StorageWrite>,
    pub total_steps: usize,
    pub key_steps: Vec<AnnotatedStep>,  // 중요 스텝만 주석 포함
    pub affected_contracts: Vec<ContractInfo>,
    pub suggested_fixes: Vec<String>,
}
```

### Example Markdown Output

```markdown
# Autopsy Report: 0xabc...def

## Summary
Reentrancy attack on Vault contract (0x742d...8f44).
Attacker drained 1,847 ETH ($4.2M) in a single transaction.

## Timeline
| Step | Opcode | Depth | Event |
|------|--------|-------|-------|
| 42   | CALL   | 1→2   | Attacker calls withdraw(1847 ether) |
| 89   | CALL   | 2→3   | ⚠️ Reentrant call to withdraw() |
| 134  | SSTORE | 3     | Balance set to 0 (too late) |

## Fund Flow
  Vault (0x742d) --[1847 ETH]--> Attacker (0x9f3a)
  Attacker (0x9f3a) --[1847 ETH]--> Tornado (0x...)

## Suggested Fix
Add `nonReentrant` modifier to `withdraw()`.
Move `balances[msg.sender] = 0` before external `call`.
```

예상 작업량: **2-3일**

## 타임라인

```
Week 1: RemoteVmDatabase + StepRecord 확장         (Step 1+2)
Week 2: AttackClassifier + FundFlowTracer           (Step 3)
Week 3: Report Generation + CLI/API                 (Step 4)
Week 4: 실제 해킹 케이스 3건으로 검증               (Euler, Curve, Ronin 등)
```

**4주 MVP.** 핵심은 Step 1 (RemoteVmDatabase) — VmDatabase trait 구현뿐이라 구조적으로 깔끔.

## 비즈니스 모델

### 고객

| Segment | Pain Point | Willingness to Pay |
|---------|------------|-------------------|
| 해킹당한 DeFi 프로토콜 | 30분 안에 원인 파악 필요 | 매우 높음 ($10K-$50K/건) |
| 보안 감사 회사 | 해킹 분석 자동화 | 높음 (연간 라이선스) |
| 보험사 | 해킹 보험 청구 검증 | 높음 (건당 과금) |
| DeFi 개발팀 | 배포 전 공격 시뮬레이션 | 중간 (SaaS 구독) |

### 수익 구조

- **Incident Response**: 해킹 발생 시 긴급 분석 ($10K-$50K/건)
- **SaaS API**: 자동화된 TX 분석 API (월 $1K-$5K)
- **Self-hosted**: 기업용 온프레미스 라이선스 (연 $50K+)

### 시장 규모

- 2025년 DeFi 해킹 피해: ~$1.7B (DeFiLlama)
- 보안 감사 시장: ~$200M+
- 해킹 건수: 월 평균 2-3건 (major), 10-20건 (minor)

## Volkov PROCEED 조건 대조

| PROCEED 조건 | Autopsy Lab 충족 여부 |
|---|---|
| Q1-Q4 의사결정 완료 | ✅ 제품 정체성 명확 (보안 분석 서비스) |
| 6개월 로드맵 | ✅ 4주 MVP → 검증 → SaaS 전환 |
| 구체적 인력/예산 | 1 Senior Rust + 1 Security Researcher |
| 경쟁 대비 차별점 3가지 | ① 내장 Time-Travel (Tenderly는 trace만) ② JIT dual-validation ③ 자동 공격 분류 |
| EXIT 기준 | 3건 실제 해킹 분석 실패 시 피벗 |
| Tier S PoC 결과 | ✅ Debugger 이미 완성, RemoteVmDatabase만 추가 |
