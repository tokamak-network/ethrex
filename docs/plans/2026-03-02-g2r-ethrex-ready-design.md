# g2r Migration → ethrex-Ready Output Design

**Date**: 2026-03-02
**Target**: Sepolia testnet
**Goal**: `geth2ethrex g2r` 마이그레이션 후 `ethrex --datadir <path>`로 바로 실행 가능하게 만들기
**Expected behavior**: P2P 싱크 재개 + RPC 서버 (과거 데이터 조회) 모두 가능

## Background

현재 g2r은 블록/상태 데이터를 마이그레이션하지만, ethrex가 직접 로드하기에는 몇 가지 갭이 존재:

| Gap | Impact |
|-----|--------|
| `metadata.json` 미생성 | ethrex startup `NotFoundDBVersion` 에러 |
| Receipts 마이그레이션 불확실 | `eth_getTransactionReceipt` RPC 실패 |
| Transaction Locations 미확인 | `eth_getTransactionByHash` RPC 실패 |
| Finalized/Safe Block Number 미기록 | `eth_getBlockByNumber("finalized")` null |
| 검증 커버리지 ~30% | body/receipt/code 손상 미감지 |

## Design

### Section 1: Data Gap Completion

#### 1.1 metadata.json
마이그레이션 완료 시 target 디렉토리에 자동 생성:
```json
{"schema_version": 1}
```
ethrex `validate_store_schema_version()` 호환.

#### 1.2 Receipts Migration
`add_blocks()` → `forkchoice_update()` 경로에서 receipts 기록 여부 확인.
미작성 시: `geth_reader.read_receipts(block_num, hash)` → ethrex `RECEIPTS` 테이블에 `(block_hash, tx_idx) → Receipt` 저장.

#### 1.3 Transaction Locations Index
각 블록 body의 tx hash에서 `TRANSACTION_LOCATIONS` 테이블에 `tx_hash → (block_num, block_hash, tx_idx)` 매핑 저장.

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
