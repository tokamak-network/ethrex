# ZK Prover 최적화 및 병렬 실행 계획

## 목표

Guest Program을 모듈화하여 앱 전용 L2를 쉽게 배포할 수 있는 구조를 만들고,
ethrex의 ZK Prover를 최적화하여 증명 시간과 비용을 최소화하고,
병렬 블록 실행 및 병렬 상태 루트 계산을 구현한다.
장기적으로 Tokamak 시뇨리지 마이닝 경제모델과 연결한다.

### 최적화 전략

Tokamak은 단일 범용 L2가 아니라, **다수의 앱 전용 L2를 제공하는 플랫폼**이다.
사용자는 용도에 맞는 L2를 골라 사용한다.

```
Tokamak Platform
├── L2-A (DEX 전용)     → DEX Circuit        → 증명 빠름
├── L2-B (결제 전용)    → 전송 Circuit        → 증명 매우 빠름
├── L2-C (게임 전용)    → 게임 Circuit        → 증명 빠름
├── L2-D (범용 EVM)     → 기본 Guest Program  → 범용
└── ...
```

이를 위해 Guest Program(증명 대상 프로그램)이 교체 가능한 플러그인 구조여야 한다.
모듈화가 먼저 이루어져야 이후 최적화(Phase 3-5)가 모듈 단위로 적용 가능하다.

## 현재 상태 분석

### ethrex ZK Prover 아키텍처

```
Prover Client ──TCP──▶ Proof Coordinator ──▶ L1 Committer
     │                     (배치 할당)          (L1 제출)
     ▼
Backend (SP1 / RISC0 / ZisK / OpenVM / TEE)
     │
     ▼
Guest Program (zkVM 내부 실행)
     ├── execute_blocks()    ← 순차 실행 (병목)
     ├── state_trie_root()   ← 순차 해싱 (병목)
     ├── verify messages
     └── compute output
```

### 주요 파일 맵

| 영역 | 파일 | 설명 |
|------|------|------|
| 프루버 진입점 | `crates/l2/prover/src/prover.rs` | 메인 루프, 코디네이터 폴링 |
| 백엔드 트레이트 | `crates/l2/prover/src/backend/mod.rs` | `ProverBackend` 트레이트 정의 |
| SP1 백엔드 | `crates/l2/prover/src/backend/sp1.rs` | SP1 증명 생성 (GPU 지원) |
| RISC0 백엔드 | `crates/l2/prover/src/backend/risc0.rs` | RISC0 증명 생성 |
| ZisK 백엔드 | `crates/l2/prover/src/backend/zisk.rs` | ZisK 증명 (실험적) |
| 게스트 실행 | `crates/guest-program/src/common/execution.rs` | **블록 순차 실행 (핵심 병목)** |
| L2 프로그램 | `crates/guest-program/src/l2/program.rs` | L2 증명 엔트리포인트 |
| 상태 DB | `crates/vm/witness_db.rs` | `state_trie_root()` 계산 |
| 블록 실행 | `crates/blockchain/blockchain.rs` | `execute_block()` — 트랜잭션 순차 실행 |
| 코디네이터 | `crates/l2/sequencer/proof_coordinator.rs` | 배치 할당, 증명 수집 |
| 상태 루트 | `crates/blockchain/blockchain.rs:514-533` | 16 샤드 병렬 해싱 (내부는 순차) |

### SP1 프로파일링 베이스라인 (Phase 1 결과)

Batch 1 기준, 총 65.4M cycles, 증명 시간 1,664.55초 (Rosetta 2 / CPU):

| 최적화 타겟 | 사이클 | 비율 |
|-------------|--------|------|
| `execute_block` | 29,363,722 | **45.6%** |
| (미분류 오버헤드) | 24,442,288 | **38.0%** |
| `validate_receipts_root` | 4,619,876 | 7.2% |
| `apply_account_updates` | 2,824,380 | 4.4% |
| `get_final_state_root` | 1,974,096 | 3.1% |
| 기타 | 2,136,534 | 1.7% |

> 상세: `tokamak-notes/sp1-profiling-baseline.md`

### upstream이 이미 달성한 최적화

- Jumpdest 최적화: -2.9%
- Trie 최적화 (4라운드): -26.8% ~ -30.6%
- 프리컴파일 최적화 (ecpairing, ecmul): -20%
- **총 달성**: 기존 대비 30-60% 성능 향상

---

## 개발 과제

### Phase 1: 기반 이해 및 벤치마크 환경 구축 ✅

> 기간: 2-3주 | 상태: **완료**

#### 1.1 프루버 로컬 실행 환경 구축

- [x] ethrex L2 로컬 실행 (Docker + L2 시퀀서)
- [x] SP1 백엔드 빌드 및 테스트 (CPU 모드)
- [x] 기존 벤치마킹 도구 확인 (`scripts/bench_metrics.sh`)

#### 1.2 프루버 사이클 프로파일링

- [x] SP1 프로파일링 (`PROVER_CLIENT_TIMED=true`)
- [x] 함수별 사이클 측정 (report_cycles 매크로)
- [x] 현재 병목 직접 확인 및 문서화

**산출물**: `sp1-profiling-baseline.md`, `local-setup-guide.md`

---

### Phase 2: Guest Program 모듈화 (앱 전용 L2)

> 기간: 6-8주
> **최우선 과제** — 이후 모든 최적화가 이 모듈 구조 위에서 이루어진다.

#### 배경

현재 ethrex의 Guest Program은 범용 EVM 실행을 고정적으로 수행한다.
Tokamak은 앱 전용 L2 플랫폼을 목표로 하므로, Guest Program을
교체 가능한 플러그인 구조로 만들어 앱 개발자가 자기 앱에 최적화된
Circuit을 넣을 수 있어야 한다.

```
현재:
  Prover → [고정된 Guest Program (EVM)] → Proof

목표:
  Prover → [교체 가능한 Guest Program] → Proof
              │
              ├── 기본 EVM (범용)
              ├── DEX Circuit (주문 매칭 특화)
              ├── Transfer Circuit (단순 전송 특화)
              └── Custom Circuit (개발자 제작)
```

#### 2.1 Guest Program 인터페이스 추상화

**수정 대상**: `crates/guest-program/src/`

현재 Guest Program은 L1/L2 구분만 있고, 실행 로직은 `execute_blocks()`에 고정되어 있다.
이를 트레이트 기반으로 추상화한다.

```rust
/// 모든 Guest Program이 구현해야 하는 인터페이스
pub trait GuestProgram {
    /// 입력 타입
    type Input;
    /// 출력 타입 (public values)
    type Output;

    /// zkVM 안에서 실행되는 핵심 로직
    fn execute(input: Self::Input) -> Result<Self::Output, Error>;

    /// 출력을 검증 가능한 public values로 변환
    fn commit(output: Self::Output);
}
```

- [ ] `GuestProgram` 트레이트 정의
- [ ] 기존 EVM 실행 로직을 `EvmGuestProgram`으로 리팩토링
- [ ] Input/Output 타입 일반화 (현재 `ExecutionWitness` → 범용 인터페이스)
- [ ] `ProverBackend` 트레이트와의 연결 구조 설계

#### 2.2 프루버 파이프라인 모듈화

**수정 대상**: `crates/l2/prover/src/backend/`

프루버가 어떤 Guest Program을 사용할지 선택할 수 있어야 한다.

- [ ] Guest Program 바이너리 선택 메커니즘 (ELF 교체)
- [ ] SP1/RISC0 백엔드에서 커스텀 ELF 로딩 지원
- [ ] 배치 입력 직렬화를 Guest Program 타입에 따라 분기
- [ ] Proof Coordinator가 L2 타입별 Guest Program 매핑 관리

#### 2.3 앱 전용 Guest Program 레퍼런스 구현

최소 2개의 앱 전용 Guest Program을 레퍼런스로 구현한다.

**Transfer Circuit** (단순 전송 전용):
- [ ] EVM 인터프리터 없이 잔액 이동만 증명
- [ ] 예상 사이클: 현재 대비 10-50x 감소
- [ ] 상태 모델: 계정 → 잔액 매핑 (Merkle Tree)

**Token Circuit** (ERC20 전용):
- [ ] ERC20 transfer/approve/transferFrom만 지원
- [ ] 토큰 잔액 + allowance 상태 증명
- [ ] 예상 사이클: 현재 대비 5-20x 감소

각 레퍼런스 구현에 포함할 것:
- [ ] Guest Program 코드
- [ ] 입력/출력 스키마 정의
- [ ] 검증자 컨트랙트 (L1에 동일 검증 로직)
- [ ] 벤치마크: EVM 대비 사이클/시간 비교

#### 2.4 앱 전용 L2 배포 파이프라인

앱 개발자가 커스텀 Guest Program을 만들어 L2를 배포할 수 있는 파이프라인:

- [ ] Guest Program 템플릿/스캐폴딩 도구
- [ ] 빌드 도구: Guest Program → zkVM ELF 컴파일
- [ ] 검증자 컨트랙트 자동 생성 (Guest Program의 output 스키마 기반)
- [ ] L2 배포 CLI: `ethrex l2 deploy --guest-program <path>`
- [ ] 문서: 앱 전용 Guest Program 개발 가이드

#### 2.5 보안 고려사항

- [ ] Guest Program 인터페이스의 안전성 보장 (상태 루트 검증 필수)
- [ ] 커스텀 Guest Program이 L1 검증을 우회할 수 없는 구조
- [ ] 악의적 Guest Program 방지: 최소 검증 요구사항 정의
- [ ] 공식 감사 범위 설정

**산출물**: Guest Program SDK + 레퍼런스 구현 2개 + 배포 파이프라인 + 개발 가이드

---

### Phase 3: ZK Prover 최적화

> 기간: 4-6주

#### 3.1 Trie 연산 최적화

프로파일링 결과 trie 관련 연산이 전체의 14.7%를 차지한다.

접근 방향:
- [ ] Lazy 해싱: 중간 블록의 state root 계산 생략, 최종 블록만 계산
- [ ] 노드 캐싱: 변경되지 않은 서브트리 해시 재사용
- [ ] RLP 인코딩 버퍼 재사용 (현재 이중 할당)
- [ ] 스토리지 트라이 독립 병렬 계산

#### 3.2 미분류 오버헤드 심층 프로파일링

프로파일링에서 38%가 미분류 상태이다. 정확한 타겟 식별이 필요하다.

- [ ] `execute_block` 내부에 세부 span 추가 (EVM opcode, storage access, precompile)
- [ ] keccak256 해싱이 차지하는 비중 측정
- [ ] zkVM 시스템콜/메모리 오버헤드 측정
- [ ] 결과에 따라 Phase 3-4 우선순위 조정

#### 3.3 프리컴파일 추가 최적화

- [ ] 남은 프리컴파일 사이클 프로파일링
- [ ] modexp, sha256, blake2f 등 최적화 여부 판단
- [ ] zkVM 친화적 구현으로 대체 가능한지 검토

#### 3.4 직렬화 최적화

- [ ] rkyv 직렬화 오버헤드 측정
- [ ] 입력 데이터 크기 최소화 (불필요한 witness 데이터 제거)
- [ ] 배치 크기 vs 증명 시간 트레이드오프 분석

**산출물**: 최적화 PR들 + 벤치마크 비교 보고서

---

### Phase 4: 병렬 블록 실행 (#6209)

> 기간: 6-8주
> 참조: [EIP-7928](https://eips.ethereum.org/EIPS/eip-7928), [Block-STM](https://arxiv.org/abs/2203.06871)

#### 배경

EIP-7928 Block Access Lists (Amsterdam 하드포크)가 제공하는 정보:
- 블록 내 모든 트랜잭션이 접근하는 계정/스토리지 슬롯 목록
- 트랜잭션별 `block_access_index` — 의존성 그래프 구축 가능

#### 4.1 의존성 그래프 엔진

**수정 대상**: `crates/blockchain/blockchain.rs` — `execute_block()`

```
현재: tx1 → tx2 → tx3 → tx4 → tx5 (순차)

목표: BAL 분석 후
      tx1 ──┐
      tx2 ──┼──▶ 병렬 실행 (disjoint accounts)
      tx3 ──┘
      tx4 ──┐
      tx5 ──┘──▶ 병렬 실행 (tx1-3 완료 후)
```

- [ ] BAL 파서 구현 (EIP-7928 스펙)
- [ ] 트랜잭션 의존성 그래프 빌더
- [ ] 충돌 감지: 동일 계정/슬롯 접근 트랜잭션 식별
- [ ] 비충돌 트랜잭션 그룹 분류

#### 4.2 Block-STM 스타일 투기적 실행

- [ ] 비충돌 트랜잭션 병렬 실행 엔진
- [ ] 충돌 트랜잭션: 투기적 실행 + 충돌 감지 + 재시도
- [ ] 결정론적 최종 상태 보장
- [ ] MEV 번들, DEX 풀 등 높은 경합 워크로드 처리

#### 4.3 zkVM 내 병렬 실행 적용

**수정 대상**: `crates/guest-program/src/common/execution.rs`

Guest Program 내에서도 병렬 실행이 가능한지 검토:
- [ ] zkVM 환경에서의 멀티스레딩 제약 조사 (SP1, RISC0)
- [ ] 가능하다면 게스트 프로그램에도 병렬 실행 적용
- [ ] 불가능하다면 배치 단위 병렬화로 우회

**산출물**: 병렬 블록 실행 구현 + 벤치마크

---

### Phase 5: 병렬 상태 루트 계산 (#6210)

> 기간: 4-6주

#### 배경

현재 상태 (`blockchain.rs:514-533`):
- 주소 해시 프리픽스로 16 샤드 분할 → 샤드별 병렬
- **샤드 내부는 순차**: 트라이 업데이트 + 해싱이 직렬

#### 5.1 BAL 기반 작업 분배

- [ ] BAL의 per-account 구조를 활용한 균등 분배
- [ ] 프리픽스 기반이 아닌 실제 변경량 기반 샤딩

#### 5.2 샤드 내 병렬화

```
현재:
  shard[0x0_] → [node1 → node2 → node3] 순차

목표:
  shard[0x0_] → branch node의 16개 children 병렬 해싱
```

- [ ] 브랜치 노드 자식 16개의 서브트리 해시 병렬 계산
- [ ] 스토리지 트라이: 계정별 독립 → 계정 간 병렬, 계정 내 브랜치 병렬
- [ ] spawn 오버헤드 vs 이득 트레이드오프 측정
- [ ] 공유 부모 노드 락 경합 최소화

#### 5.3 zkVM 내 적용

- [ ] 게스트 프로그램의 `state_trie_root()` 최적화
- [ ] 증명 사이클에서 trie 연산 비중 감소 목표: 14.7% → 5% 이하

**산출물**: 병렬 상태 루트 계산 구현 + 벤치마크

---

### Phase 6: 시뇨리지 마이닝 경제모델 설계

> 기간: 3-4주 (설계), 이후 구현
> Phase 3과 병행 가능

#### 6.1 비용 모델링

- [ ] 프루버 운영 비용 분석 (서버, GPU, 전기)
- [ ] 배치당 증명 비용 산출
- [ ] 최적화 전후 비용 비교

#### 6.2 시뇨리지 마이닝 메커니즘 설계

- [ ] 증명 완료 = 마이닝 보상 매핑
- [ ] Tokamak 시뇨리지와 연결 구조
- [ ] 프루버 간 경쟁/협력 모델
- [ ] 인센티브 균형: 빠른 증명 vs 비용 효율

#### 6.3 프로토콜 설계

- [ ] 프루버 등록/탈퇴 메커니즘
- [ ] 보상 분배 로직
- [ ] 슬래싱 조건 (잘못된 증명 제출)

**산출물**: 경제모델 설계 문서 + 프로토타입 스마트 컨트랙트

---

## 우선순위 및 타임라인

```
Phase 1 ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  (2-3주) ✅ 완료
         기반 구축 + 프로파일링

Phase 2 ░░░░░░░░████████████████████████░░░░░░░░░  (6-8주) ★ 최우선
         Guest Program 모듈화 (앱 전용 L2)

Phase 3 ░░░░░░░░░░░░░░░░░░░░░░░░████████████████  (4-6주)
         ZK Prover 최적화

Phase 4 ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░████████████████████  (6-8주)
         병렬 블록 실행 (#6209)

Phase 5 ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████████████  (4-6주)
         병렬 상태 루트 (#6210)

Phase 6 ░░░░░░░░░░░░░░░░░░░░░░░░████████████░░░░░░░░░░░░░░░░  (Phase 3과 병행)
         경제모델 설계
```

**Phase 1 → 2** 는 순차 의존성이 있다.
**Phase 2 → 3 → 4 → 5** 는 순차 진행. 모듈화(Phase 2) 완료 후 최적화가 모듈 단위로 적용된다.
**Phase 6** 은 설계 작업이므로 Phase 3과 병행 가능.

## upstream 기여 전략

| Phase | upstream 기여 가능성 | 방식 |
|-------|---------------------|------|
| Phase 2 (모듈화) | **중간** — 구조 변경이 크므로 upstream 논의 필요 | 설계안 먼저 제안 → 합의 후 PR |
| Phase 3 (Prover 최적화) | **높음** — upstream도 원한다 | `main`에서 브랜치 → upstream PR |
| Phase 4 (#6209) | **높음** — 이미 이슈 등록됨 | upstream과 협업 또는 독립 구현 후 PR |
| Phase 5 (#6210) | **높음** — 이미 이슈 등록됨 | 위와 동일 |
| Phase 6 (경제모델) | **없음** — Tokamak 전용 | `tokamak-dev`에서만 개발 |

Phase 3-5는 upstream에 기여하면 유지보수 부담이 줄고, Tokamak의 기술 신뢰도도 올라간다.
Phase 2는 upstream 구조를 크게 바꾸므로 사전 논의 후 진행.
Phase 6만 Tokamak 전용으로 `tokamak-dev` 브랜치에서 관리.

## 관련 참고자료

- [EIP-7928: Block-Level Access Lists](https://eips.ethereum.org/EIPS/eip-7928)
- [Block-STM (PPoPP 2023)](https://arxiv.org/abs/2203.06871)
- [ParallelEVM (EuroSys 2025)](https://yajin.org/papers/EuroSys_2025_ParallelEVM.pdf)
- ethrex 프루버 벤치마크: `docs/l2/bench/prover_performance.md`
- ethrex 프루버 아키텍처: `docs/l2/architecture/prover.md`
- ethrex 벤치마킹 가이드: `docs/l2/prover-benchmarking.md`
