# Proving System Comparison: Halo2 Full vs SP1 ZK-DEX vs EVM L2

**Date**: 2026-02-23 (Full Benchmark 업데이트)
**Branch**: `feat/zk/halo2-dex-benchmark`
**Machine**: Apple M4 Max (Rosetta 2, x86_64 emulation, CPU only)

---

## TL;DR

Halo2 DEX Full Benchmark는 **ECDSA + Storage Proof + Transfer + SNARK Aggregation**을
모두 포함하여 SP1 ZK-DEX와 구조적으로 동등한 범위를 증명한다.

| | EVM L2 | SP1 ZK-DEX | **Halo2 Full** |
|---|---|---|---|
| **Proving** | 27m 44s | 3m 26s | **1m 0s** |
| **Verification** | -- | 229ms | **2ms** |
| **Proof size** | -- | ~260 B (Groth16) | **1.5 KB** (SHPLONK) |

**Halo2 Full은 SP1 대비 proving 3.4x, verification 114x 빠르다.**

단, Halo2는 Keccak-MPT 대신 **Poseidon-Merkle**을 사용하므로
해시 연산에서 Halo2에 유리한 비교이다. 이 점을 고려한 분석은 본문 참조.

---

## 1. 각 시스템이 증명하는 것

### 1.1 증명 범위 비교

| 검증 항목 | EVM L2 | SP1 ZK-DEX | Halo2 Transfer Only | **Halo2 Full** |
|-----------|--------|------------|---------------------|----------------|
| 트랜잭션 서명 검증 (ECDSA) | O | O | X | **O** (secp256k1) |
| Merkle proof 검증 (state/storage) | O | O (keccak-MPT) | X | **O** (Poseidon-Merkle) |
| 잔액 충분성 검증 | O | O | O | **O** |
| 잔액 업데이트 + 오버플로 방지 | O | O | O | **O** (range check) |
| State root 재계산 | O | O (keccak) | X | **O** (Poseidon) |
| SNARK Aggregation | -- | O (recursive + Groth16) | X | **O** (SHPLONK agg) |
| 트랜잭션 파싱 (ABI 디코딩) | O | O | X | X |
| Nonce / Gas 검증 | O | O | X | X |
| Receipt/log 생성 | O | O | X | X |
| EVM 바이트코드 인터프리터 | O | X (app-specific) | X | X |

### 1.2 정리

- **EVM L2**: 임의 스마트 컨트랙트 포함 전체 블록 실행 증명
- **SP1 ZK-DEX**: DEX 특화, 서명 검증 ~ state root 업데이트 전 과정
- **Halo2 Full**: ECDSA + Poseidon-Merkle 기반 storage proof + transfer + aggregation
  - SP1과 **구조적으로 동등** (서명 → storage 검증 → 잔액 연산 → state 업데이트)
  - 단, 해시 함수가 다름 (Poseidon vs keccak) — 아래 분석 참조

---

## 2. 해시 함수 차이의 영향

### 2.1 Poseidon vs Keccak

| | Keccak-256 (SP1) | Poseidon (Halo2) |
|---|---|---|
| **사용처** | Ethereum MPT, state root | ZK-friendly Merkle tree |
| **ZK 회로 비용** | 매우 높음 (~15만 constraint/호출) | 매우 낮음 (~수백 constraint/호출) |
| **SP1 cycle 비용** | ~수천 cycles (precompile) | N/A |
| **Ethereum 호환** | O | **X** |

### 2.2 비교 공정성

- SP1은 keccak을 **VM 내부에서 네이티브 실행** (precompile)
- Halo2는 Poseidon을 **산술 회로에서 직접 실행**
- "해시 함수를 검증 가능하게 실행한다"는 점에서 **구조적으로 동등**
- 단, Poseidon이 산술 회로에서 훨씬 저렴 → **Halo2에 유리한 비교**

> axiom-eth(Keccak-MPT)를 사용하면 완전히 공정한 비교가 가능하지만,
> community-edition 브랜치와 의존성 충돌(zkevm-hashes 누락, snark-verifier-sdk 버전 pin)이
> 확인되어 Poseidon-Merkle로 fallback했다.

### 2.3 Poseidon 독자 L2의 실용성

Poseidon-Merkle을 사용하면 Ethereum L1의 Keccak-MPT와 호환되지 않지만,
**실질적 단점은 거의 없다.**

| 우려 | 실제 영향 |
|------|-----------|
| L1에 Poseidon 검증 컨트랙트 배포 필요 | 어차피 L2 검증 컨트랙트는 배포해야 함. 추가 비용 무시 가능 |
| L1 검증 가스비 증가? | **아님**. ZK proof 검증은 pairing check 1회 (~300K gas)로 동일 |
| L1에서 L2 state 직접 조회 불가 | L2→L1 통신은 항상 proof 기반. 직접 조회는 현실적으로 거의 없음 |

**선례**: StarkNet, zkSync Era 등 주요 L2가 이미 Poseidon/자체 해시 기반 state trie를 사용하며
프로덕션에서 문제 없이 운영 중.

**결론**: Halo2 + Poseidon 독자 L2는 실용적으로 불리한 점이 거의 없으며,
ZK 증명 성능 이점(SP1 대비 3.4x proving, 114x verification)을 그대로 유지할 수 있다.

---

## 3. 벤치마크 결과 (Raw Numbers)

### 3.1 Halo2 Full Benchmark (ECDSA + Storage + Transfer + Aggregation)

```
============================================
  Halo2 DEX Full Benchmark (1 transfer)
============================================

  [ECDSA Verification]  k=18
    SRS ...              24ms  (cached)
    Keygen ...         1.13s
    Proving ...        1.99s   Proof: 1.5 KB

  [Storage Proof]       k=14
    SRS ...               0ms  (cached)
    Keygen ...          224ms
    Proving ...         321ms   Proof: 3.7 KB

  [Transfer Logic]      k=10
    SRS ...               0ms  (cached)
    Keygen ...            8ms
    Proving ...          27ms   Proof: 992 B

  [Aggregation]         k=23
    SRS ...          1m 54s  (first run, cached after)
    Keygen ...        28.34s
    Proving ...       58.14s   Proof: 1.5 KB
    Verifying ...        2ms

============================================
  Results Summary
============================================

  Sub-circuit proving times:
    ECDSA (k=18):        1.99s
    Storage (k=14):      321ms
    Transfer (k=10):      27ms
    ────────────────────────────
    Sub-total:           2.34s

  Aggregation (k=23):
    Proving:            58.14s
    Final verify:          2ms
    Final proof:        1.5 KB

  ────────────────────────────────────
  Total proving:     1m 0.48s
  Final verification:      2ms
============================================
```

### 3.2 Halo2 Transfer Only (기존, 참고용)

```
  Circuit degree (k): 10
  Proof generation:         32ms
  Verification:              1ms
  Proof size:             992 B
```

### 3.3 SP1 ZK-DEX (1 transfer, Groth16, 패치 후)

```
Total cycles:      357,761
Execution time:    56.88ms
Proving time:      3m 26.1s (206s)
Verification time: 229.17ms
```

### 3.4 SP1 ZK-DEX (1 transfer, 패치 전)

```
Total cycles:      11,449,345
Proving time:      5m 05.3s (305s)
```

### 3.5 EVM L2 Baseline (Batch 1, Groth16)

```
Total cycles:      65,360,896
Proving time:      27m 44s (1,664.55s)
```

---

## 4. 비교 분석

### 4.1 수치 비교 (1 transfer, 실측)

| 지표 | EVM L2 | SP1 ZK-DEX | **Halo2 Full** | Halo2 Sub-only |
|------|--------|------------|----------------|----------------|
| **Proving time** | 27m 44s | 3m 26s | **1m 0s** | 2.34s |
| **Verification** | -- | 229ms | **2ms** | N/A |
| **Proof size** | -- | ~260 B | **1.5 KB** | -- |
| **Execution cycles** | 65.4M | 357K | N/A | N/A |

### 4.2 이전 추정치 vs 실측 비교

이전 문서에서는 Halo2 Full 구현 시 "~15-40초"로 추정했다.

| | 이전 추정 | **실측** | 비고 |
|---|---|---|---|
| ECDSA proving | ~2초 | **1.99s** | 추정 정확 |
| Storage proving | ~3-5초 (keccak 기준) | **321ms** (Poseidon) | Poseidon이 10-15x 저렴 |
| Transfer proving | 32ms | **27ms** | 일치 |
| Aggregation proving | ~10-30초 | **58.14s** | 추정보다 2x 느림 (k=23) |
| **총 proving** | ~15-40초 | **1m 0s** | Aggregation이 지배적 |
| Verification | ~1ms | **2ms** | 일치 |

> Aggregation이 예상보다 느린 이유: k=23 (8M rows)으로 3개 sub-proof(k=18,14,10)를
> VerifierUniversality::Full로 합성. Sub-proof 간 k 차이가 클수록 aggregation 비용 증가.

### 4.3 Proving Time 구조 비교

```
SP1 Proving Time Breakdown (1 transfer, 206s):
┌─────────────────────────────────────────────────┐
│ STARK core proving (~30s)  ← cycle 비례        │
│ Recursive compression (~150s) ← 고정 오버헤드  │
│ Groth16 wrapping (~20s) ← 고정 오버헤드        │
├─────────────────────────────────────────────────┤
│ 고정 오버헤드 비율: ~83%                          │
└─────────────────────────────────────────────────┘

Halo2 Full Proving Time Breakdown (1 transfer, 60.5s):
┌─────────────────────────────────────────────────┐
│ ECDSA sub-proof (2.0s)     ← k=18              │
│ Storage sub-proof (0.3s)   ← k=14 (Poseidon)   │
│ Transfer sub-proof (0.03s) ← k=10              │
│ SNARK Aggregation (58.1s)  ← k=23              │
├─────────────────────────────────────────────────┤
│ Aggregation 비율: ~96%                           │
└─────────────────────────────────────────────────┘
```

**핵심 관찰**: 두 시스템 모두 고정 오버헤드(SP1: recursive+Groth16, Halo2: aggregation)가
전체 시간의 80-96%를 차지한다. 실제 연산 증명은 둘 다 수 초 수준.

### 4.4 Sub-circuit만 비교 (Aggregation 제외)

Aggregation/wrapping 오버헤드를 제외하고 순수 연산 증명만 비교:

| | SP1 core STARK | Halo2 Sub-total |
|---|---|---|
| Proving | ~30s | **2.34s** |
| 비율 | 1x | **~13x 빠름** |

이 비교가 "증명 시스템 효율성" 차이에 가장 가까운 수치.

---

## 5. 스케일링 전망

### 5.1 SP1 ZK-DEX 스케일링 (패치 후)

| Transfers | 추정 Cycles | 추정 Proving | 고정 오버헤드 비율 |
|-----------|------------|-------------|-------------------|
| 1 | 357,761 | 3m 26s (실측) | ~83% |
| 10 | ~3,500,000 | ~3.5-4분 | ~75% |
| 100 | ~35,000,000 | ~5-8분 | ~37% |
| 1,000 | ~350,000,000 | ~30-60분 | ~5% |

### 5.2 Halo2 Full 스케일링 (추정)

| Transfers | 추정 Sub-proving | 추정 Aggregation | 총 Proving |
|-----------|-----------------|-----------------|------------|
| 1 | 2.34s (실측) | 58s (실측) | **~1분** |
| 10 | ~5-10s | ~60s (동일 k) | **~1분 10초** |
| 100 | ~30-60s (k 증가) | ~2-3분 (k 증가) | **~3-4분** |

> Halo2는 배치 증가 시 sub-circuit k가 올라가므로 proving 비용도 증가.
> SP1은 cycle이 선형 증가하지만 고정 오버헤드가 amortize.
> **100+ transfers에서는 격차가 좁아질 것으로 예상.**

---

## 6. 결론

### 6.1 벤치마크 결과의 의미

| 항목 | 결론 |
|------|------|
| "Halo2 Full이 SP1보다 3.4x 빠르다" | **조건부 사실**. Poseidon-Merkle 사용으로 Halo2에 유리 |
| "Halo2 검증이 114x 빠르다" | **사실**. SHPLONK 2ms vs Groth16 229ms |
| "Halo2 sub-circuit 합계가 2.3초" | **사실**. Aggregation 없이 ECDSA+Storage+Transfer |
| "Aggregation이 전체의 96%를 차지" | **사실**. k=23 aggregation이 병목 |
| "이전 추정(15-40초)이 정확했나" | **부분적**. Sub-circuit은 정확, aggregation은 2x 과소추정 |

### 6.2 각 접근법의 적합한 용도

| 접근법 | 적합 | 부적합 |
|--------|------|--------|
| **EVM L2** | 범용 스마트 컨트랙트 증명 | 특정 앱 최적화 |
| **SP1 ZK-DEX** | 프로덕션 DEX, Ethereum 호환 필수 | 초저지연 요구 |
| **Halo2 Full** | 특화앱 최적화, 빠른 검증 필요 | Ethereum MPT 호환 필수 시 |

### 6.3 핵심 인사이트

1. **Halo2 네이티브 회로는 실제로 빠르다**: ECDSA 2초, Storage 321ms, Transfer 27ms.
   Sub-circuit 합계 2.34초는 SP1 core STARK (~30초) 대비 13x 빠름.

2. **Aggregation이 새로운 병목**: SNARK aggregation(k=23)이 58초로 전체의 96%.
   SP1의 recursive compression(~150초)과 유사한 역할이지만 더 빠름.

3. **해시 함수 선택이 결정적**: Poseidon(321ms) vs Keccak(추정 수 초~수십 초).
   Ethereum 호환이 필요하면 Keccak-MPT가 필수이고, 이 경우 Halo2 이점이 감소.

4. **두 시스템의 병목 구조가 유사**: 둘 다 "실제 연산 증명은 빠르지만 집계/wrapping이 느림".
   SP1: 고정 오버헤드 83% / Halo2: aggregation 96%.

5. **배치 크기 증가 시 격차 축소**: SP1은 고정 오버헤드가 amortize,
   Halo2는 k가 증가. 100+ transfers에서는 격차가 크게 줄어들 전망.

---

## 7. Keccak-MPT vs Poseidon: 어떤 전략이 유리한가

### 7.1 Keccak-MPT로 교체 시 예상 (가상 시나리오)

axiom-eth 통합이 가능해지면 Poseidon → Keccak-MPT로 교체할 수 있다.

| 컴포넌트 | 현재 (Poseidon) | Keccak-MPT 예상 |
|----------|----------------|-----------------|
| Storage proving | **321ms** | ~3-5초 (10-15x 증가) |
| Storage k | 14 | 19-20 |
| Aggregation k | 23 | 24-25 |
| Aggregation proving | **58초** | ~2-3분 |
| **총 proving** | **1분** | **~3-4분** (미구현, 순수 추정) |
| SP1 대비 | 3.4x 빠름 | **~1x 수준** (추정) |

> **주의**: Keccak-MPT 수치는 axiom-eth 미통합 상태의 추정치.
> 실측 시 aggregation k 증가, 메모리 요구량 등으로 더 느려질 수 있다.

### 7.2 Poseidon 독자 L2가 더 합리적인 선택

Keccak-MPT로 교체하면 Halo2 성능 이점이 사라지고 SP1과 동등해진다.
그 상태에서 SP1(일반 Rust, 수일) 대비 Halo2(회로 프로그래밍, 수주~수개월)의
개발 비용 차이만 남으므로, **L1 호환을 위해 Keccak-MPT를 쓸 이유가 없다.**

Poseidon 독자 L2를 선택하면:
- Halo2 성능 이점 유지 (SP1 대비 3.4x proving, 114x verification)
- L1 검증 가스비는 동일 (ZK proof pairing check ~300K gas)
- L1에서 L2 state 직접 조회 불가이지만, L2→L1 통신은 어차피 proof 기반
- StarkNet, zkSync Era 등이 이미 동일 전략으로 프로덕션 운영 중

**결론: Halo2를 선택한다면 Poseidon 독자 L2로 가는 것이 성능과 실용성 모두에서 최적.**
L1 호환이 반드시 필요한 경우에만 Keccak-MPT(또는 SP1)를 고려.

---

## 8. 환경 정보

| 항목 | 값 |
|------|---|
| Machine | Apple M4 Max |
| Architecture | x86_64 (Rosetta 2 emulation) |
| OS | macOS Darwin 24.4.0 |
| CPU mode | CPU only (no GPU) |
| SP1 SDK | 5.0.8, Groth16 circuit v5.0.0 |
| Halo2 | halo2-base + halo2-ecc community-edition (axiom-crypto) |
| Proof scheme | SP1: STARK → Groth16 / Halo2: PLONK + SHPLONK (KZG) |
| Aggregation | snark-verifier-sdk, VerifierUniversality::Full |

---

## 9. Source Files

| 파일 | 설명 |
|------|------|
| `crates/halo2-dex/src/bin/full_benchmark.rs` | **Halo2 Full 벤치마크 바이너리** |
| `crates/halo2-dex/src/circuits/ecdsa.rs` | ECDSA secp256k1 sub-circuit (halo2-ecc) |
| `crates/halo2-dex/src/circuits/storage.rs` | Poseidon-Merkle storage proof sub-circuit |
| `crates/halo2-dex/src/circuits/transfer.rs` | Transfer logic sub-circuit |
| `crates/halo2-dex/src/bin/benchmark.rs` | Halo2 Transfer-only 벤치마크 (기존) |
| `crates/guest-program/src/programs/zk_dex/circuit.rs` | SP1 DexCircuit (full logic) |
| `crates/l2/prover/src/bin/sp1_benchmark.rs` | SP1 벤치마크 바이너리 |
| `tokamak-notes/sp1-zk-dex-vs-baseline.md` | SP1 vs EVM L2 상세 비교 |

---

## 10. 실행 방법

```bash
cd crates/halo2-dex

# Sub-circuit만 (aggregation 없이, 빠른 테스트)
cargo run --release --bin full_benchmark -- --skip-aggregation

# Full 벤치마크 (aggregation 포함, ~8GB RAM, ~2분)
cargo run --release --bin full_benchmark

# Transfer-only (기존 벤치마크)
cargo run --release --bin benchmark
```

---

## 11. 아키텍처

```
┌──────────────────────────────────────────────────────┐
│              Halo2 ZK-DEX Full Benchmark             │
│                                                      │
│  ┌────────────┐  ┌────────────┐  ┌────────────────┐  │
│  │   ECDSA    │  │  Storage   │  │   Transfer     │  │
│  │  secp256k1 │  │  Proof     │  │   Logic        │  │
│  │            │  │  (Poseidon │  │                │  │
│  │ halo2-ecc  │  │   Merkle)  │  │ sub + add +    │  │
│  │ k=18       │  │ k=14       │  │ range_check    │  │
│  │ 1.99s      │  │ 321ms      │  │ k=10, 27ms     │  │
│  └─────┬──────┘  └─────┬──────┘  └───────┬────────┘  │
│        │               │                 │           │
│        └───────────┬───┘─────────────────┘           │
│                    │                                 │
│           ┌────────▼────────┐                        │
│           │ SNARK Aggregation│                       │
│           │ snark-verifier   │                       │
│           │ Universality::Full│                      │
│           │ k=23             │                       │
│           │ 58.14s           │                       │
│           └────────┬────────┘                        │
│                    │                                 │
│           ┌────────▼────────┐                        │
│           │ Final Proof     │                        │
│           │ 1.5 KB          │                        │
│           │ Verify: 2ms     │                        │
│           └─────────────────┘                        │
└──────────────────────────────────────────────────────┘
```

---

## 12. 전략 선택: 두 가지 옵션

벤치마크 결과를 종합하면, ZK-DEX 구현 전략은 두 가지로 수렴한다.

### Option A: SP1 zkVM + Keccak-MPT (Ethereum 호환)

| 항목 | 내용 |
|------|------|
| **Proving** | 3분 26초 (1 transfer) |
| **Verification** | 229ms |
| **Proof size** | ~260 B (Groth16) |
| **State trie** | Keccak-MPT (Ethereum L1 동일) |
| **L1 호환** | 완전 호환 — 표준 Merkle proof로 L2 state 직접 증명 가능 |
| **개발 방식** | 일반 Rust 코드, zkVM이 자동 회로화 |
| **생태계** | SP1/Succinct 활발, 업계 주류 방향 |

**적합**: Ethereum L1과의 직접적 상호운용이 핵심 요구사항인 경우.

### Option B: Halo2 네이티브 + Poseidon 독자 L2

| 항목 | 내용 |
|------|------|
| **Proving** | **1분 0초** (SP1 대비 3.4x) |
| **Verification** | **2ms** (SP1 대비 114x) |
| **Proof size** | 1.5 KB (SHPLONK) |
| **State trie** | Poseidon-Merkle (독자 설계) |
| **L1 호환** | ZK proof 검증만 가능 (state 직접 조회 불가, 실질 영향 미미) |
| **개발 방식** | 회로 프로그래밍 (AI 활용으로 생산성 격차 해소 가능) |
| **선례** | StarkNet, zkSync Era 등이 동일 전략으로 프로덕션 운영 |

**적합**: 증명 성능과 검증 속도가 핵심이고, L1 state 직접 호환이 필수가 아닌 경우.

### 비교 요약

| | **Option A (SP1)** | **Option B (Halo2)** |
|---|---|---|
| Proving 속도 | 3m 26s | **1m 0s (3.4x)** |
| Verification 속도 | 229ms | **2ms (114x)** |
| L1 검증 가스비 | ~300K gas | ~300K gas (동일) |
| L1 state 직접 조회 | O | X (실질 영향 미미) |
| 개발 난이도 | 쉬움 (Rust) | 회로 프로그래밍 (AI 활용 시 격차 축소) |
| 배치 100+ tx 시 | 고정 오버헤드 amortize | aggregation k 증가, 격차 축소 |
| 생태계 방향 | 업계 주류 (zkVM) | 특화앱 최적화 (네이티브 회로) |

### 핵심 판단 기준

- **L1 state 직접 호환이 필수** → Option A (SP1)
- **증명/검증 성능 최적화가 우선** → Option B (Halo2)
- **둘 다 필요** → Option B 기반 + L1 브릿지 컨트랙트 (StarkNet 모델)

