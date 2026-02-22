# ZK Prover 최적화 및 병렬 실행 계획

## 목표

ethrex의 ZK Prover를 최적화하여 증명 시간과 비용을 최소화하고,
병렬 블록 실행 및 병렬 상태 루트 계산을 구현한다.
장기적으로 Tokamak 시뇨리지 마이닝 경제모델과 연결한다.

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

### 현재 병목 (증명 사이클 기준)

```
┌────────────────────────┬──────────┬─────────────────────────────────┐
│ 영역                   │ 비중     │ 설명                            │
├────────────────────────┼──────────┼─────────────────────────────────┤
│ Trie 연산              │ 50-60%   │ rebuild, apply_updates, hashing │
│ EVM 프리컴파일         │ 15-20%   │ ecpairing, ecmul 등             │
│ Trie 해싱              │ 10-15%   │ RLP 인코딩, memcpy 오버헤드     │
│ 블록 검증              │ ~5%      │ 헤더, 영수증, 요청 해시         │
│ 기타                   │ ~10%     │ 직렬화, I/O, 메시지 처리        │
└────────────────────────┴──────────┴─────────────────────────────────┘
```

### upstream이 이미 달성한 최적화

- Jumpdest 최적화: -2.9%
- Trie 최적화 (4라운드): -26.8% ~ -30.6%
- 프리컴파일 최적화 (ecpairing, ecmul): -20%
- **총 달성**: 기존 대비 30-60% 성능 향상

---

## 개발 과제

### Phase 1: 기반 이해 및 벤치마크 환경 구축

> 기간: 2-3주

#### 1.1 프루버 로컬 실행 환경 구축

- [ ] ethrex L2 로컬 실행 (Docker Compose)
- [ ] Exec 백엔드로 프루버 파이프라인 동작 확인
- [ ] SP1 백엔드 빌드 및 테스트 (GPU 없이)
- [ ] 기존 벤치마킹 도구 사용 (`tooling/bench/`)

참고: `docs/l2/prover-benchmarking.md`

#### 1.2 프루버 사이클 프로파일링

- [ ] SP1 프로파일링 (`profiling` feature flag)
- [ ] 함수별 사이클 측정 (report_cycles 매크로 활용)
- [ ] 현재 병목 직접 확인 및 문서화
- [ ] upstream 최적화 히스토리 분석 (`docs/l2/bench/prover_performance.md`)

**산출물**: 프루버 성능 베이스라인 보고서

---

### Phase 2: ZK Prover 최적화

> 기간: 4-6주

#### 2.1 Trie 연산 최적화 (최대 병목)

현재 50-60%를 차지하는 Trie 연산을 공략한다.

**현재 코드** (`crates/guest-program/src/common/execution.rs`):
```
for (i, block) in blocks.iter().enumerate() {
    // 블록별 순차 실행
    // apply_account_updates → state_trie_root
}
```

접근 방향:
- [ ] Lazy 해싱: 중간 블록의 state root 계산 생략, 최종 블록만 계산
- [ ] 노드 캐싱: 변경되지 않은 서브트리 해시 재사용
- [ ] RLP 인코딩 버퍼 재사용 (현재 이중 할당)
- [ ] 스토리지 트라이 독립 병렬 계산

#### 2.2 프리컴파일 추가 최적화

upstream이 ecpairing(-10%), ecmul(-10%)을 했지만 다른 프리컴파일이 남아있다.

- [ ] 남은 프리컴파일 사이클 프로파일링
- [ ] modexp, sha256, blake2f 등 최적화 여부 판단
- [ ] zkVM 친화적 구현으로 대체 가능한지 검토

#### 2.3 직렬화 최적화

- [ ] rkyv 직렬화 오버헤드 측정
- [ ] 입력 데이터 크기 최소화 (불필요한 witness 데이터 제거)
- [ ] 배치 크기 vs 증명 시간 트레이드오프 분석

**산출물**: 최적화 PR들 + 벤치마크 비교 보고서

---

### Phase 3: 병렬 블록 실행 (#6209)

> 기간: 6-8주
> 참조: [EIP-7928](https://eips.ethereum.org/EIPS/eip-7928), [Block-STM](https://arxiv.org/abs/2203.06871)

#### 배경

EIP-7928 Block Access Lists (Amsterdam 하드포크)가 제공하는 정보:
- 블록 내 모든 트랜잭션이 접근하는 계정/스토리지 슬롯 목록
- 트랜잭션별 `block_access_index` — 의존성 그래프 구축 가능

#### 3.1 의존성 그래프 엔진

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

#### 3.2 Block-STM 스타일 투기적 실행

- [ ] 비충돌 트랜잭션 병렬 실행 엔진
- [ ] 충돌 트랜잭션: 투기적 실행 + 충돌 감지 + 재시도
- [ ] 결정론적 최종 상태 보장
- [ ] MEV 번들, DEX 풀 등 높은 경합 워크로드 처리

#### 3.3 zkVM 내 병렬 실행 적용

**수정 대상**: `crates/guest-program/src/common/execution.rs`

Guest Program 내에서도 병렬 실행이 가능한지 검토:
- [ ] zkVM 환경에서의 멀티스레딩 제약 조사 (SP1, RISC0)
- [ ] 가능하다면 게스트 프로그램에도 병렬 실행 적용
- [ ] 불가능하다면 배치 단위 병렬화로 우회

**산출물**: 병렬 블록 실행 구현 + 벤치마크

---

### Phase 4: 병렬 상태 루트 계산 (#6210)

> 기간: 4-6주

#### 배경

현재 상태 (`blockchain.rs:514-533`):
- 주소 해시 프리픽스로 16 샤드 분할 → 샤드별 병렬
- **샤드 내부는 순차**: 트라이 업데이트 + 해싱이 직렬

#### 4.1 BAL 기반 작업 분배

- [ ] BAL의 per-account 구조를 활용한 균등 분배
- [ ] 프리픽스 기반이 아닌 실제 변경량 기반 샤딩

#### 4.2 샤드 내 병렬화

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

#### 4.3 zkVM 내 적용

- [ ] 게스트 프로그램의 `state_trie_root()` 최적화
- [ ] 증명 사이클에서 trie 연산 비중 감소 목표: 50-60% → 30% 이하

**산출물**: 병렬 상태 루트 계산 구현 + 벤치마크

---

### Phase 5: 시뇨리지 마이닝 경제모델 설계

> 기간: 3-4주 (설계), 이후 구현

#### 5.1 비용 모델링

- [ ] 프루버 운영 비용 분석 (서버, GPU, 전기)
- [ ] 배치당 증명 비용 산출
- [ ] 최적화 전후 비용 비교

#### 5.2 시뇨리지 마이닝 메커니즘 설계

- [ ] 증명 완료 = 마이닝 보상 매핑
- [ ] Tokamak 시뇨리지와 연결 구조
- [ ] 프루버 간 경쟁/협력 모델
- [ ] 인센티브 균형: 빠른 증명 vs 비용 효율

#### 5.3 프로토콜 설계

- [ ] 프루버 등록/탈퇴 메커니즘
- [ ] 보상 분배 로직
- [ ] 슬래싱 조건 (잘못된 증명 제출)

**산출물**: 경제모델 설계 문서 + 프로토타입 스마트 컨트랙트

---

## 우선순위 및 타임라인

```
Phase 1 ████████░░░░░░░░░░░░░░░░░░░░░░░░  (2-3주)
         기반 구축 + 프로파일링

Phase 2 ░░░░░░░░████████████████░░░░░░░░░  (4-6주)
         ZK Prover 최적화

Phase 3 ░░░░░░░░░░░░░░░░░░░░████████████████████  (6-8주)
         병렬 블록 실행 (#6209)

Phase 4 ░░░░░░░░░░░░░░░░░░░░░░░░░░██████████████  (4-6주, Phase 3과 일부 병행)
         병렬 상태 루트 (#6210)

Phase 5 ░░░░░░░░░░░░████████████░░░░░░░░░░░░░░░░  (Phase 2와 병행 가능)
         경제모델 설계
```

**Phase 1 → 2 → 3 → 4** 는 순차 의존성이 있다.
**Phase 5** 는 설계 작업이므로 Phase 2와 병행 가능.

## upstream 기여 전략

| Phase | upstream 기여 가능성 | 방식 |
|-------|---------------------|------|
| Phase 2 (Prover 최적화) | **높음** — upstream도 원한다 | `main`에서 브랜치 → upstream PR |
| Phase 3 (#6209) | **높음** — 이미 이슈 등록됨 | upstream과 협업 또는 독립 구현 후 PR |
| Phase 4 (#6210) | **높음** — 이미 이슈 등록됨 | 위와 동일 |
| Phase 5 (경제모델) | **없음** — Tokamak 전용 | `tokamak-dev`에서만 개발 |

Phase 2-4는 upstream에 기여하면 유지보수 부담이 줄고, Tokamak의 기술 신뢰도도 올라간다.
Phase 5만 Tokamak 전용으로 `tokamak-dev` 브랜치에서 관리.

## 관련 참고자료

- [EIP-7928: Block-Level Access Lists](https://eips.ethereum.org/EIPS/eip-7928)
- [Block-STM (PPoPP 2023)](https://arxiv.org/abs/2203.06871)
- [ParallelEVM (EuroSys 2025)](https://yajin.org/papers/EuroSys_2025_ParallelEVM.pdf)
- ethrex 프루버 벤치마크: `docs/l2/bench/prover_performance.md`
- ethrex 프루버 아키텍처: `docs/l2/architecture/prover.md`
- ethrex 벤치마킹 가이드: `docs/l2/prover-benchmarking.md`
