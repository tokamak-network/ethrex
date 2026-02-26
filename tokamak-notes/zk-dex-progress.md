# ZK-DEX 프로젝트 진행현황

**작성일**: 2026-02-23
**브랜치**: `feat/zk/sp1-zk-dex-e2e` (from `feat/zk/guest-program-modularization`)

---

## 1. 전체 작업 타임라인

### Phase 1: 기반 구축 + 프로파일링 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-22 | Guest Program 모듈화 프레임워크 구현 | `f6be4be`~`048a766` |
| | - ZK-DEX, Tokamon 커스텀 타입 및 실행 로직 | |
| | - SP1 zkVM 엔트리포인트 생성 | |
| | - SDK 스캐폴딩, E2E 테스트, ELF 검증 | |
| | - DynamicGuestProgram (런타임 ELF 로딩) | |
| | - Production hardening | |
| 2026-02-23 | App-specific circuit 프레임워크 + DexCircuit | `2107f49`~`4a070dc` |
| | - AppCircuit trait, AppState, incremental MPT | |
| | - DexCircuit: token transfer 로직 구현 | |
| 2026-02-23 | SP1 벤치마크 바이너리 + cycle-tracker 계측 | `4af8122`, `61ce982` |
| 2026-02-23 | 프루버 파이프라인 모듈화 | `25d9025`~`5753e75` |
| | - configurable guest_program_id (시퀀서/CLI) | |
| | - programs.toml 설정, docker-compose 오버라이드 | |
| 2026-02-23 | L1 컨트랙트 + 플랫폼 UI | `318a06f`~`e29718d` |
| | - guestProgramRegistry 컨트랙트 연동 | |
| | - /launch, /guide 페이지, 공식 프로그램 시딩 | |
| 2026-02-23 | SP1 ZK-DEX 바이너리 완성 + 프로파일링 | `881e31d`~`d8cc431` |
| | - SP1 바이너리를 DexCircuit + AppProgramInput으로 재작성 | |
| | - ProgramInput → AppProgramInput 변환 | |
| | - build_trie_from_proofs 수정 + signed tx 벤치마크 | |
| | - **SP1 crypto precompile 패치 → 182x 사이클 감소** | |

---

## 2. SP1 ZK-DEX 프로파일링 결과

### 환경
- **머신**: Apple M4 Max (Rosetta 2 / x86_64 에뮬레이션, CPU only)
- **SP1**: v5.0.8 (Groth16 circuit v5.0.0)

### 핵심 결과 (3-way 비교)

| | EVM L2 (baseline) | ZK-DEX (패치 전) | ZK-DEX (패치 후) | baseline 대비 |
|---|---|---|---|---|
| **총 실행 사이클** | 65,360,896 | 11,449,345 | **357,761** | **182x 감소** |
| **총 proving 시간** | 27분 44초 | 5분 05초 | **3분 26초** | **8.1x 단축** |

> 사이클 182x 감소 대비 proving 시간 8.1x만 단축된 이유:
> recursive compression + Groth16 wrapping 고정 오버헤드(~3분)가 전체의 대부분을 차지.

### 사이클 분석

**EVM L2 주요 병목**:
- `execute_block`: 29.4M cycles (45.6%) — EVM 인터프리터
- 미분류 오버헤드: 24.4M cycles (38.0%) — zkVM 오버헤드
- trie 연산: 9.4M cycles (14.7%) — receipts, state root 등

**ZK-DEX 패치 후 (337K cycles)**:
- ECDSA 서명 검증: ~20K cycles (~6%) — SP1 precompile 가속
- Merkle proof 검증: ~100K cycles (~30%) — sha3 패치 가속
- State root 재계산: ~100K cycles (~30%)
- Token 잔액 업데이트: ~50K cycles (~15%)

### SP1 Crypto Precompile 패치 효과

| 연산 | 패치 전 | 패치 후 | 감소율 |
|------|---------|---------|--------|
| secp256k1 ecrecover | ~10,000,000 cycles | ~20,000 cycles | **~500x** |
| keccak256 해싱 | 소프트웨어 | sha3 precompile | 수배 |

추가된 패치: `k256`, `ecdsa`, `crypto-bigint`, `sha2`, `sha3`, `p256` (기존 `tiny-keccak`, `secp256k1`에 추가)

### 스케일링 전망

| Transfers | ZK-DEX 사이클 (추정) | 예상 Proving 시간 |
|-----------|---------------------|------------------|
| 1 | 357,761 | 3분 26초 (실측) |
| 10 | ~3,500,000 | ~3.5-4분 |
| 100 | ~35,000,000 | ~5-8분 |
| 1,000 | ~350,000,000 | ~30-60분 |

---

## 3. Halo2 vs SP1 비교 및 전략적 결정

### 결론: SP1 선택

| 기준 | Halo2 (직접 구현) | SP1 (zkVM) |
|------|-------------------|------------|
| 개발 생산성 | 회로를 직접 작성 (수개월) | Rust 코드 그대로 실행 (수일) |
| 유연성 | 회로 변경 시 전면 재작성 | Rust 코드 수정만으로 변경 |
| 성능 | 이론적으로 최적 | crypto precompile로 충분한 성능 |
| 검증 비용 | Groth16 (저렴) | Groth16 wrapping 지원 (동일) |
| 생태계 | 직접 유지보수 | SP1 생태계 활용 |

### SP1 선택 이유

1. **개발 속도**: DexCircuit을 Rust로 구현하여 수일 만에 동작하는 프로토타입 완성
2. **충분한 성능**: crypto precompile 패치만으로 182x 사이클 감소 달성
3. **모듈화 용이**: 새 AppCircuit 추가가 Rust 코드 작성만으로 가능
4. **플랫폼 전략 적합**: 다수의 앱 전용 L2를 빠르게 지원해야 하는 Tokamak 모델에 적합
5. **Halo2는 옵션으로 유지**: 극한 최적화가 필요한 특정 회로에 한해 Halo2 직접 구현 가능

---

## 4. L1/L2 인프라 현황

### 이미 구현된 것

**Guest Program 모듈화 (Phase 2)**:
- [x] AppCircuit trait + AppState 추상화
- [x] Incremental MPT (Merkle proof 기반 상태 관리)
- [x] DexCircuit 레퍼런스 구현 (token transfer)
- [x] DynamicGuestProgram (런타임 ELF 로딩)
- [x] ELF 헤더 검증 + fuzz-style 테스트
- [x] SDK 스캐폴딩 도구 (모듈 구조 자동 생성)
- [x] programs.toml 설정 + docker-compose 오버라이드
- [x] configurable guest_program_id (시퀀서/CLI 관통)

**L1 컨트랙트**:
- [x] `guestProgramRegistry` → `initialize()` 연동
- [x] `programTypeId` → `verify()` 연동

**GP Store 플랫폼** (Next.js + Express + SQLite):
- [x] 공식 프로그램 3종 시딩 (EVM L2, ZK-DEX, Tokamon)
- [x] /store, /launch, /guide 페이지
- [x] L2 설정 생성 + TOML/docker-compose 다운로드
- [x] 세션 관리 (SQLite 마이그레이션)

**SP1 ZK-DEX 바이너리**:
- [x] SP1 게스트 바이너리 (`sp1-zk-dex`)
- [x] ProgramInput → AppProgramInput 변환 (serialize_input)
- [x] SP1 crypto precompile 패치 (k256, ecdsa, sha2, sha3 등)
- [x] SP1 벤치마크 바이너리
- [x] Cycle-tracker 계측

### Phase 2: L1/L2 환경 구성 + 배포 자동화 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-23 | L1 deployer에 ZK-DEX 게스트 프로그램 등록 파이프라인 추가 | `fe62d40` |
| | - `--register-guest-programs` CLI 옵션 | |
| | - GuestProgramRegistry 초기화 + 공식 프로그램 등록 | |
| | - Timelock을 통한 SP1 VK 등록 | |
| 2026-02-23 | SP1 게스트 크래시 수정 + Timelock VK 등록 수정 | `808d428` |
| 2026-02-24 | **로컬 환경 자동 구축 스크립트** | `b31fed5`, `057383d` |
| | - `crates/l2/scripts/zk-dex-localnet.sh` 신규 작성 | |
| | - 한 명령으로 L1→배포→L2→프로버 자동 시작 | |
| | - `start/stop/status/logs` 명령 + `--no-prover` 옵션 | |
| | - Makefile 타겟 4개 추가 | |
| | - 실제 로컬넷 구동 검증 완료 | |

### Phase 3: L2 제네시스 배포 파이프라인 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-26 | **ZK-DEX L2 제네시스 배포 파이프라인 구축** | |
| | - `IGroth16Verifier.sol` 인터페이스 생성 (6개 verifier) | |
| | - `foundry.toml` 생성 (Forge 빌드 지원) | |
| | - `generate_verifiers.sh` pragma 호환성 수정 (snarkjs 0.7.x) | |
| | - `generate-zk-dex-genesis.sh` 스크립트 작성 | |
| | - forge build → bytecode 추출 → storage layout 검증 → genesis JSON 생성 자동화 | |
| | - localnet/Docker 스크립트 `l2-zk-dex.json` 전환 | |
| 2026-02-26 | **Storage slot 버그 수정 + Localnet E2E 검증** | |
| | - `storage.rs` 슬롯 상수 수정: ENCRYPTED_NOTES 3→2, NOTES 4→3, ORDERS 14→13 | |
| | - L2 서버 `--no-monitor` 플래그 추가 (TUI→stdout 로깅 전환) | |
| | - L2 P2P 포트 충돌 해결 (`--p2p.port 30304 --discovery.port 30304`) | |
| | - Localnet E2E 성공: L1→Deploy→L2 전체 파이프라인 검증 | |
| | - ZkDex 7개 컨트랙트 + storage slots L2 제네시스 배포 확인 | |
| 2026-02-26 | **SP1 E2E 증명 + L1 온체인 검증 성공** | |
| | - `ETHREX_GUEST_PROGRAM_ID=zk-dex` env var 누락 수정 (programTypeId 1→2) | |
| | - Batch 1: SP1 zk-dex 게스트 실행 (4.9M cycles) → Groth16 증명 → L1 검증 성공 | |
| | - 전체 파이프라인: L1→Deploy→L2→Prover→L1 Verify 완전 동작 확인 | |

### 아직 구현되지 않은 것

- [x] ~~실제 L1 네트워크에 컨트랙트 배포~~ → 로컬 L1 (ethrex --dev) 배포 완료
- [x] ~~실제 L2 노드를 Guest Program과 함께 가동~~ → `zk-dex-localnet.sh`로 자동화
- [x] ~~ZK-DEX 컨트랙트 L2 제네시스 배포~~ → `generate-zk-dex-genesis.sh` 파이프라인 구축
- [x] ~~Circom 회로 컴파일 + trusted setup 실행 (1회, 오프라인)~~ → 6개 회로 컴파일 + PTAU 14 setup 완료
- [x] End-to-end 증명 생성 및 L1 검증 — Batch 1 SP1 Groth16 증명 + L1 온체인 검증 성공 (2026-02-26)
- [ ] 프론트엔드 (platform/client) 연결하여 L2 RPC 동작 확인
- [ ] 대규모 배치 벤치마크 (100+ transfers)
- [ ] Native ARM 벤치마크 (Rosetta 2 없이)

---

## 5. 로컬 환경 구축 가이드

### 자동 구축 (권장)

```bash
cd crates/l2

# 전체 환경 시작 (L1 + 컨트랙트 배포 + L2 + SP1 프로버)
make zk-dex-localnet

# 프로버 없이 시작 (앱/프론트엔드 테스트용, 더 빠름)
make zk-dex-localnet-no-prover

# 상태 확인
make zk-dex-localnet-status

# 로그 확인
./scripts/zk-dex-localnet.sh logs        # 전체
./scripts/zk-dex-localnet.sh logs l1     # L1만
./scripts/zk-dex-localnet.sh logs l2     # L2만

# 종료
make zk-dex-localnet-stop
```

### 엔드포인트

| 서비스 | URL |
|--------|-----|
| L1 RPC | `http://localhost:8545` |
| L2 RPC | `http://localhost:1729` |
| Proof Coordinator | `tcp://127.0.0.1:3900` |
| Prometheus Metrics | `http://localhost:3702` |

### 배포된 컨트랙트 (로컬)

| 컨트랙트 | 주소 |
|-----------|------|
| OnChainProposer | `cmd/.env` → `ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS` |
| Bridge | `cmd/.env` → `ETHREX_WATCHER_BRIDGE_ADDRESS` |
| SP1 Verifier | `cmd/.env` → `ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS` |
| Timelock | `cmd/.env` → `ETHREX_TIMELOCK_ADDRESS` |

### 검증

```bash
# L1 RPC 동작 확인
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:8545

# L2 RPC 동작 확인
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:1729
```

### 주의사항

- 첫 실행 시 릴리스 빌드가 포함되어 10분 이상 소요될 수 있음
- 이전 ethrex 프로세스가 남아있으면 포트 충돌 → `pkill ethrex` 후 재시작
- 런타임 파일은 `crates/l2/.zk-dex-localnet/`에 저장 (`.gitignore` 처리됨)

---

## 6. 다음 단계

### 목표

로컬 환경에서 ZK-DEX 트랜잭션의 **End-to-end 증명 생성 + L1 검증**을 완료한다.

### 할 일

1. **프로버 연결**
   - 로컬넷에 SP1 프로버 추가 (`make zk-dex-localnet` 또는 별도 터미널)
   - L2 배치 → SP1 증명 생성 → L1 제출 → 검증 파이프라인 확인

2. **프론트엔드 연결**
   - `platform/client/` (Next.js) → L2 RPC (`http://localhost:1729`) 연결
   - DEX token transfer UI 동작 확인

3. **E2E 테스트**
   - DEX token transfer 트랜잭션 전송
   - 배치 생성 → SP1 증명 생성 → L1 제출 → 온체인 검증
   - 전체 파이프라인 동작 확인

4. **성능 측정**
   - 실제 L2 배치로 proving 시간 측정
   - Mock 데이터와의 차이 비교

---

## 참조 문서

| 문서 | 설명 |
|------|------|
| `tokamak-notes/sp1-profiling-baseline.md` | EVM L2 베이스라인 프로파일링 |
| `tokamak-notes/sp1-zk-dex-vs-baseline.md` | ZK-DEX vs EVM L2 상세 비교 |
| `tokamak-notes/zk-optimization-plan.md` | 전체 최적화 계획 (Phase 1-6) |
| `tokamak-notes/guest-program-modularization/` | 모듈화 설계 문서 (12개) |
| `tokamak-notes/local-setup-guide.md` | 로컬 실행 가이드 |
| `crates/l2/scripts/ZK-DEX-LOCALNET.md` | 로컬넷 스크립트 사용 가이드 |
| `tokamak-notes/zk-dex-e2e-design.md` | E2E 아키텍처 설계 |
