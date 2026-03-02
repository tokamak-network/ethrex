# g2r Migration → ethrex-Ready Output Design

**Date**: 2026-03-02
**Target**: Sepolia testnet
**Goal**: `geth2ethrex g2r` 마이그레이션 후 `ethrex --datadir <path>`로 바로 실행 가능하게 만들기
**Expected behavior**: P2P 싱크 재개 + RPC 서버 (과거 데이터 조회) 모두 가능

## Background

현재 g2r은 블록/상태 데이터를 마이그레이션하지만, ethrex가 직접 로드하기에는 몇 가지 갭이 존재:

| Gap | Status | Impact |
|-----|--------|--------|
| `metadata.json` 미생성 | **이미 해결됨** (`Store::new_from_genesis()`가 자동 생성) | - |
| Receipts 마이그레이션 | **미작성** → 구현 완료 | `eth_getTransactionReceipt` RPC 실패 |
| Transaction Locations | **이미 해결됨** (`add_blocks()`가 자동 생성) | - |
| Finalized/Safe Block Number 미기록 | **미기록** → 구현 완료 | `eth_getBlockByNumber("finalized")` null |
| 검증 커버리지 ~30% | **개선 완료** (body + receipt 검증 추가) | body/receipt 손상 미감지 |

## Design

### Section 1: Data Gap Completion

#### 1.1 metadata.json (이미 해결됨)
`Store::new_from_genesis()`가 `validate_store_schema_version()`을 통해 자동 생성. 추가 작업 불필요.

#### 1.2 Receipts Migration (구현 완료)
`add_blocks()`는 receipts를 쓰지 않음을 확인. `geth_reader.read_receipts(block_num, hash, body)` 구현 후 migration 루프에 receipt 쓰기 추가. Geth stored receipt에는 tx_type이 없으므로 block body의 트랜잭션에서 파생.

#### 1.3 Transaction Locations Index (이미 해결됨)
`add_blocks()`가 `TRANSACTION_LOCATIONS` 테이블에 자동으로 `tx_hash + block_hash → (block_num, block_hash, tx_idx)` 매핑 저장. 추가 작업 불필요.

#### 1.4 Finalized/Safe Block Number
마이그레이션 최종 단계:
- `CHAIN_DATA[FinalizedBlockNumber]` = last migrated block
- `CHAIN_DATA[SafeBlockNumber]` = same value

### Section 2: Enhanced Verification

#### 2.1 Existing (retained)
- Canonical hash match (Geth vs ethrex)
- Block header hash match
- State root match
- State trie node existence (optional)

#### 2.2 New: Default Verification (`--verify-offline`)
| Check | Method | Purpose |
|-------|--------|---------|
| Body hash | Geth body → RLP → hash vs ethrex body hash | Tx data corruption |
| Transactions root | Build trie from ethrex body.txs → root vs header.txs_root | Tx missing/order error |

#### 2.3 New: Deep Verification (`--verify-deep`)
All default checks plus:
| Check | Method | Purpose |
|-------|--------|---------|
| Receipts root | Build trie from receipts → root vs header.receipts_root | Receipt corruption |
| Contract code sampling | Random N accounts → code_hash → keccak256 recompute | Bytecode corruption |
| Tx location spot check | Random N tx hashes → TRANSACTION_LOCATIONS → block match | Index accuracy |

#### 2.4 Reporting
Extended `OfflineVerificationSummary` with per-check pass/fail counts:
```
[verify] 1000/5000 (20%) body_ok=1000 receipt_ok=1000 code_sampled=50/50 mismatches=0
```

### Section 3: ethrex Startup Compatibility Check

#### 3.1 `--ethrex-ready` flag (default: enabled)
Post-migration smoke test:

1. `metadata.json` exists with correct `schema_version`
2. `CHAIN_DATA[LatestBlockNumber]` exists and readable
3. Latest block header loadable from `HEADERS` table
4. `CHAIN_DATA[ChainConfig]` parseable
5. Genesis block (0) has canonical hash + header + body
6. Latest block's state_root has trie nodes present

#### 3.2 Report Format
```json
{
  "ethrex_ready": true,
  "checks": {
    "metadata_json": "pass",
    "latest_block_number": "pass (block 5000000)",
    "latest_header": "pass",
    "chain_config": "pass (sepolia)",
    "genesis_block": "pass",
    "state_root_valid": "pass"
  }
}
```

#### 3.3 Opt-out
`--no-ethrex-ready` to skip this check.

## Non-Goals
- LevelDB reader implementation (separate task)
- Block re-execution validation (Approach C scope)
- Mainnet-specific optimizations
- Integration test stabilization (separate task)
