# geth2ethrex

Geth chaindata를 다른 실행 클라이언트의 DB 포맷으로 마이그레이션하는 도구입니다.

## 지원하는 마이그레이션 경로

| 커맨드 | 소스 | 타겟 | 상태 |
|--------|------|------|------|
| `g2r` (geth2rocksdb) | Geth Pebble | ethrex RocksDB | 블록 + State + 검증 |
| `g2l` (geth2lmdb) | Geth Pebble | py-ethclient LMDB | 블록 + State + 검증 |

> Geth LevelDB (v1.9.x 이하)는 자동 감지되지만 읽기는 아직 미지원입니다.

## Quick Start

### g2r: Geth → ethrex RocksDB

```bash
# 빌드
cargo build --release --manifest-path tooling/geth2ethrex/Cargo.toml

# Dry-run (DB 감지 + 마이그레이션 플랜만 출력, 실제 쓰기 없음)
geth2ethrex g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis genesis.json \
  --dry-run

# 실행
geth2ethrex g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis genesis.json
```

마이그레이션 완료 후 ethrex 노드를 `--datadir`로 타겟 경로를 지정하여 실행합니다.

### g2l: Geth → py-ethclient LMDB

```bash
geth2ethrex g2l \
  --source /path/to/geth/chaindata \
  --target /path/to/lmdb/output \
  --dry-run

geth2ethrex g2l \
  --source /path/to/geth/chaindata \
  --target /path/to/lmdb/output
```

## 주요 기능

### 마이그레이션 단계

두 커맨드 모두 동일한 3단계를 거칩니다:

1. **블록 마이그레이션** — Header, Body, Receipt를 1000블록 배치로 읽어 타겟 DB에 기록
2. **State 마이그레이션** — Account snapshot, Storage, Contract code를 이전 (`--blocks-only`로 건너뛰기 가능)
3. **오프라인 검증** — 소스와 타겟의 canonical hash, header hash, state root를 블록 단위로 대조 (`--verify-offline false`로 비활성화)

### 검증 로직 (Offline Verification)

마이그레이션 완료 후 자동으로 소스(Geth)와 타겟 DB의 데이터 일치성을 검증합니다. (`--verify-offline true` 기본값)

#### 공통 검증 항목 (g2r, g2l 모두)

모든 블록에 대해 다음 3가지를 검증합니다:

##### 1️⃣ Canonical Hash 검증

```
각 블록 높이(block_number)에 대해 Geth의 canonical hash와 타겟 DB의 canonical hash를 비교합니다.
- 실패 시: 블록 순서 뒤섞임, 포크 체인 오류
- 복구: 마이그레이션 재실행
```

##### 2️⃣ Header Hash 검증

```
BlockHeader RLP 인코딩 후 계산한 해시를 비교합니다.
- 실패 시: 헤더 필드 불일치 (timestamp, gas_limit 등)
- 복구: RLP 인코딩/디코딩 로직 확인 후 마이그레이션 재실행
```

##### 3️⃣ State Root 검증

```
각 블록 헤더의 state_root 필드를 비교합니다.
- 실패 시: 상태 데이터 마이그레이션 오류, fork choice 실패
- 복구: --blocks-only 없이 state 마이그레이션 재실행
```

#### g2r 추가 검증 항목

##### 4️⃣ Block Body 검증

```
각 블록의 트랜잭션 개수를 비교합니다.
- 실패 시: 트랜잭션 누락, RLP 디코딩 오류
```

##### 5️⃣ Receipt 검증 (`--verify-deep` 플래그)

```
트랜잭션이 있는 블록에 대해 모든 receipt의 존재 여부를 확인합니다.
- 실패 시: Receipt 마이그레이션 누락
- 사용법: geth2ethrex g2r ... --verify-deep
```

#### 검증 범위 커스터마이징

```bash
# 특정 블록 범위만 검증
geth2ethrex g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/rocksdb \
  --genesis genesis.json \
  --verify-start-block 1000000 \
  --verify-end-block 1001000
```

#### 검증 비활성화

```bash
# 빠른 마이그레이션을 위해 검증 생략
geth2ethrex g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/rocksdb \
  --genesis genesis.json \
  --verify-offline false
```

### Merge block 자동 감지 (g2r)

ethrex는 post-merge (PoS) 네트워크만 지원합니다. `--from-block`을 지정하지 않으면 genesis의 `merge_netsplit_block`을 읽어 merge block부터 자동으로 마이그레이션을 시작합니다.

### TUI 대시보드

`--json` 플래그 없이 실행하면 터미널에 실시간 대시보드가 표시됩니다:
- 진행률 게이지 바 (현재 블록 / 전체 블록)
- 실시간 처리 속도 (blocks/s) 및 예상 완료 시간
- 배치 진행, 에러, 건너뛴 블록 로그

TUI는 기본 feature로 포함됩니다. `--no-default-features`로 빌드하면 비활성화됩니다.

### Ancient (Freezer) DB 지원

Geth의 ancient/freezer DB (`chaindata/ancient/chain/`)를 투명하게 읽습니다. Hot DB에 없는 오래된 블록은 자동으로 ancient DB에서 조회됩니다.

### DB 타입 자동 감지

소스 디렉토리의 파일 패턴으로 Geth DB 타입을 자동 감지합니다:
- `OPTIONS-*` 파일 → Pebble
- `*.ldb` 파일 → LevelDB
- `*.sst` 파일 → Pebble

## CLI Reference

### `g2r` — Geth → ethrex RocksDB

```
geth2ethrex g2r [OPTIONS] --source <PATH> --target <PATH> --genesis <PATH>
```

| 플래그 | 기본값 | 설명 |
|--------|--------|------|
| `--source` | 필수 | Geth chaindata 디렉토리 경로 |
| `--target` | 필수 | ethrex RocksDB 출력 경로 |
| `--genesis` | 필수 | ethrex 초기화용 genesis 파일 경로 |
| `--dry-run` | `false` | DB 감지 및 플랜만 출력, 실제 쓰기 없음 |
| `--blocks-only` | `false` | 블록만 마이그레이션 (state 건너뜀) |
| `--from-block` | 자동 | 시작 블록 번호 (미지정 시 merge block 자동 감지) |
| `--verify-offline` | `true` | 마이그레이션 후 DB-to-DB 검증 실행 |
| `--verify-start-block` | — | 검증 시작 블록 오버라이드 |
| `--verify-end-block` | — | 검증 종료 블록 오버라이드 |
| `--skip-state-trie-check` | `false` | 검증 시 state trie 존재 체크 건너뜀 |
| `--json` | `false` | 구조화된 JSON 출력 (TUI 비활성화) |
| `--report-file` | — | 리포트 추가 저장 경로 (JSONL) |
| `--retry-attempts` | `3` | 재시도 횟수 (1~10) |
| `--retry-base-delay-ms` | `1000` | 재시도 기반 지연 ms (지수 백오프) |
| `--continue-on-error` | `false` | 블록 오류 시 건너뛰고 계속 진행 |

### `g2l` — Geth → py-ethclient LMDB

```
geth2ethrex g2l [OPTIONS] --source <PATH> --target <PATH>
```

| 플래그 | 기본값 | 설명 |
|--------|--------|------|
| `--source` | 필수 | Geth chaindata 디렉토리 경로 |
| `--target` | 필수 | LMDB 출력 디렉토리 경로 |
| `--dry-run` | `false` | 플랜만 출력, 실제 쓰기 없음 |
| `--blocks-only` | `false` | 블록만 마이그레이션 (state 건너뜀) |
| `--map-size-gb` | `16` | LMDB 맵 크기 (GB) |
| `--skip-receipts` | `false` | Receipt 데이터 마이그레이션 건너뜀 |
| `--verify-offline` | `true` | 마이그레이션 후 DB-to-DB 검증 실행 |
| `--verify-start-block` | — | 검증 시작 블록 오버라이드 |
| `--verify-end-block` | — | 검증 종료 블록 오버라이드 |
| `--json` | `false` | 구조화된 JSON 출력 |
| `--report-file` | — | 리포트 추가 저장 경로 |
| `--continue-on-error` | `false` | 오류 시 건너뛰고 계속 진행 |

## State 마이그레이션 상세

State 마이그레이션은 Geth의 snapshot layer에서 데이터를 읽습니다:

| 데이터 | Geth key | 설명 |
|--------|----------|------|
| Account snapshot | `"a" + account_hash` | SlimAccount RLP (nonce, balance, root, codehash) |
| Storage snapshot | `"o" + account_hash + slot_hash` | Storage slot 값 |
| Contract code | `"c" + code_hash` | EVM bytecode |
| Preimage | `"secure-key-" + hash` | hash → 원본 address/slot 복원 |

**g2r**: State trie와 storage trie를 처음부터 구축합니다. 마이그레이션 후 계산된 state root와 head block의 state root를 대조하여 정합성을 검증합니다.

**g2l**: Preimage DB를 통해 hash → address 복원을 시도합니다. 복원에 성공한 계정은 address 기반 DB(`accounts`, `storage`)에도 기록되고, 항상 hash 기반 DB(`snap_accounts`, `snap_storage`)에 기록됩니다. Preimage가 없는 계정/슬롯은 snap DB에만 저장되며 경고가 출력됩니다.

## JSON 출력 (--json)

`--json` 플래그를 사용하면 스크립팅/CI에 적합한 구조화된 JSON을 출력합니다.

**성공 리포트:**

```json
{
  "schema_version": 1,
  "status": "planned|in_progress|completed|up_to_date",
  "phase": "planning|execution",
  "source_head": 20000000,
  "target_head": 19000000,
  "plan": { "start_block": 19000001, "end_block": 20000000 },
  "dry_run": false,
  "imported_blocks": 1000000,
  "skipped_blocks": 0,
  "elapsed_ms": 360000,
  "retry_attempts": 3,
  "retries_performed": 0
}
```

**실패 리포트:**

```json
{
  "schema_version": 1,
  "status": "failed",
  "phase": "execution",
  "error_type": "transient|fatal",
  "error_classification": "retry_failure|io_kind|message_marker|default_fatal",
  "retryable": true,
  "retry_attempts": 3,
  "retry_attempts_used": 2,
  "error": "human-readable error message",
  "elapsed_ms": 27
}
```

`--report-file`을 지정하면 리포트를 파일에도 JSONL 형식으로 추가 저장합니다.

## LMDB 스키마 (g2l)

g2l이 생성하는 py-ethclient 호환 LMDB의 named database 목록:

| DB 이름 | key | value |
|---------|-----|-------|
| `headers` | block_hash (32B) | RLP BlockHeader |
| `bodies` | block_hash (32B) | RLP BlockBody |
| `canonical` | block_number (8B BE) | block_hash (32B) |
| `header_numbers` | block_number (8B BE) | block_hash (32B) |
| `tx_index` | tx_hash (32B) | block_hash (32B) + tx_idx (4B BE) |
| `receipts` | block_hash (32B) | RLP receipt list |
| `accounts` | address (20B) | RLP Account |
| `code` | code_hash (32B) | raw bytecode |
| `storage` | address (20B) + slot (32B) | minimal BE int |
| `original_storage` | address (20B) + slot (32B) | minimal BE int |
| `snap_accounts` | account_hash (32B) | RLP Account |
| `snap_storage` | account_hash (32B) + slot_hash (32B) | raw value |
| `meta` | string key | `latest_block` (i64 BE), `snap_progress` (JSON) |

## 개발 환경 요구사항

- Rust toolchain: workspace의 `rust-toolchain.toml` 참조
- `libclang`: `librocksdb-sys` 빌드에 필요

```bash
# Ubuntu/Debian
sudo apt-get install -y libclang-dev clang

# macOS (Xcode Command Line Tools에 포함)
xcode-select --install

# Arch Linux
sudo pacman -S --needed clang
```

### 빌드 및 테스트

```bash
# 빌드
cargo build --release --manifest-path tooling/geth2ethrex/Cargo.toml

# 테스트
cargo test --manifest-path tooling/geth2ethrex/Cargo.toml
```
