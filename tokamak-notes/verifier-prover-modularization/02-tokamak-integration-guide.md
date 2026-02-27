# Tokamak-zk-EVM Integration Guide

ethrex L2에 Tokamak-zk-EVM을 증명 백엔드로 통합한 전체 설계, 구현, 운영 가이드.

---

## 목차

1. [개요](#1-개요)
2. [아키텍처](#2-아키텍처)
3. [zkVM vs Tokamak-zk-EVM 비교](#3-zkvm-vs-tokamak-zk-evm-비교)
4. [스마트 컨트랙트 계층](#4-스마트-컨트랙트-계층)
5. [TokamakBackend 프루버 구현](#5-tokamakbackend-프루버-구현)
6. [E2E 증명 흐름](#6-e2e-증명-흐름)
7. [Synthesizer 분석](#7-synthesizer-분석)
8. [ABI 인코딩](#8-abi-인코딩)
9. [배포 (Deployer)](#9-배포-deployer)
10. [CLI 옵션 및 설정](#10-cli-옵션-및-설정)
11. [Localnet 운영](#11-localnet-운영)
12. [파일 인벤토리](#12-파일-인벤토리)
13. [성능 벤치마크](#13-성능-벤치마크)
14. [제약사항 및 향후 작업](#14-제약사항-및-향후-작업)

---

## 1. 개요

### 1.1 Tokamak-zk-EVM이란

Tokamak-zk-EVM은 커스텀 zkSNARK 시스템으로, SP1/RISC0 같은 범용 zkVM이 아닙니다.
자체 QAP(Quadratic Arithmetic Program) 컴파일러와 synthesizer를 통해
특정 트랜잭션 패턴(L2 TON Transfer)을 증명합니다.

- **논문**: https://eprint.iacr.org/2024/507.pdf
- **곡선**: BLS12-381
- **해시**: Poseidon (zkSNARK 친화적)
- **서명**: EdDSA on jubjub curve
- **상태 트리**: Poseidon 기반 4-ary Merkle Tree

### 1.2 통합 목표

ethrex L2의 기존 멀티-백엔드 프루버 시스템(SP1, RISC0, ZisK, OpenVM, Exec)에
Tokamak을 6번째 백엔드로 추가하여, 선택적으로 Tokamak zkSNARK 증명을
on-chain으로 검증할 수 있게 합니다.

### 1.3 설계 원칙

1. **Feature-gated**: 모든 Tokamak 코드는 `#[cfg(feature = "tokamak")]`로 격리
2. **외부 프로세스 호출**: Tokamak CLI(ICICLE/GPU 의존성)를 library link 대신 subprocess로 실행
3. **기존 패턴 준수**: ethrex의 `ProverBackend` trait, `ProverType` enum, `BatchProof` 체계 활용
4. **무파괴 통합**: tokamak feature를 끄면 기존 코드에 영향 없음

### 1.4 구현 단계

| Phase | 내용 | 상태 |
|-------|------|------|
| Phase 1 | OnChainProposer + ProverType에 Tokamak 검증기 슬롯 추가 | ✅ |
| Phase 2 | TokamakVerifier 컨트랙트 통합 + CREATE2 자동 배포 | ✅ |
| Phase 3 | TokamakBackend 프루버 구현 (CLI 호출, 출력 파싱, ABI 인코딩) | ✅ |

관련 문서:
- `00-design-overview.md` — Phase 1-2 설계
- `01-implementation-log.md` — Phase 1-2 구현 로그
- 이 문서 — Phase 3 + 전체 통합 가이드

---

## 2. 아키텍처

### 2.1 전체 시스템 구조

```
┌─────────────────────────────────────────────────────────────────┐
│                        ethrex L2 Node                           │
│                                                                 │
│  ┌──────────┐   ┌───────────────┐   ┌─────────────────────┐   │
│  │  Block    │   │    Proof      │   │   L1 Proof Sender   │   │
│  │ Producer  │──▶│  Coordinator  │──▶│                     │   │
│  └──────────┘   └───────┬───────┘   │  verifyBatch() call │   │
│                         │           └──────────┬──────────┘   │
│                         │                      │               │
└─────────────────────────┼──────────────────────┼───────────────┘
                          │                      │
                   TCP (ProofData)               │
                          │                      │
              ┌───────────▼────────────┐         │
              │     Tokamak Prover     │         │
              │                        │         │
              │  TokamakBackend        │         │
              │  ├─ write_input_config │         │
              │  ├─ run_cli(synth.)    │         │
              │  ├─ run_cli(preproc.)  │         │
              │  ├─ run_cli(prove)     │         │
              │  ├─ read_proof()       │         │
              │  └─ abi_encode()       │         │
              │         │              │         │
              │    ┌────▼────┐         │         │
              │    │tokamak- │         │         │
              │    │  cli    │         │         │
              │    └────┬────┘         │         │
              │         │              │         │
              │  ┌──────▼──────────┐   │         │
              │  │ dist/resource/  │   │         │
              │  │ ├─ synthesizer/ │   │         │
              │  │ ├─ preprocess/  │   │         │
              │  │ ├─ prove/       │   │         │
              │  │ └─ qap-compiler/│   │         │
              │  └─────────────────┘   │         │
              └────────────────────────┘         │
                                                 │ ETH tx
                                        ┌────────▼────────┐
                                        │   L1 (Ethereum)  │
                                        │                  │
                                        │ OnChainProposer  │
                                        │ ├─ verifyBatch() │
                                        │ └─ if REQUIRE_   │
                                        │    TOKAMAK_PROOF  │
                                        │    ┌────────────┐│
                                        │    │Tokamak     ││
                                        │    │Verifier    ││
                                        │    │.verify()   ││
                                        │    └────────────┘│
                                        └──────────────────┘
```

### 2.2 컴포넌트 관계

| 컴포넌트 | 역할 | 레이어 |
|----------|------|--------|
| `TokamakBackend` | tokamak-cli 호출 + 출력 파싱 + ABI 인코딩 | Rust (Prover) |
| `tokamak-cli` | synthesize/preprocess/prove 외부 바이너리 | TypeScript + Rust |
| `ProofCoordinator` | 배치 할당 + 증명 수신 | Rust (L2 Node) |
| `L1ProofSender` | 증명 calldata 구성 + L1 트랜잭션 전송 | Rust (L2 Node) |
| `OnChainProposer` | verifyBatch()에서 증명 검증 디스패치 | Solidity (L1) |
| `TokamakVerifier` | BLS12-381 pairing 기반 zkSNARK 검증 | Solidity (L1) |

---

## 3. zkVM vs Tokamak-zk-EVM 비교

| 항목 | SP1/RISC0 (zkVM) | Tokamak-zk-EVM |
|------|------------------|----------------|
| **방식** | Rust → RISC-V ELF 컴파일 → 범용 증명 | 전용 Circom 회로 → QAP → 증명 |
| **EVM 실행** | 전체 Rust EVM (ethrex) 실행 | 제한된 커스텀 EVM |
| **해시 함수** | Keccak256 (표준 EVM) | **Poseidon** (zkSNARK 친화적) |
| **서명** | secp256k1/ECDSA | **EdDSA (jubjub 커브)** |
| **상태 트리** | MPT (Merkle Patricia Trie) | **Poseidon 기반 4-ary Merkle Tree** |
| **게스트 프로그램** | ELF 바이너리로 임의 로직 실행 | 고정된 회로 구조 |
| **곡선** | BN254 또는 사용자 선택 | **BLS12-381** |
| **유연성** | 높음 (임의 Rust 코드) | 낮음 (특정 트랜잭션 패턴) |
| **통합 방식** | Library link (Rust crate) | 외부 프로세스 호출 (CLI) |

### 핵심 차이점

SP1은 ethrex EVM 전체를 RISC-V로 컴파일하여 임의 EVM 트랜잭션을 증명합니다.
Tokamak은 특정 트랜잭션 패턴(L2 TON Transfer)에 대한 전용 Circom 회로를 사용합니다.

따라서 **ethrex L2의 임의 블록 실행을 Tokamak으로 증명하는 것은 현재 불가능합니다.**

### 통합 접근 방식

zkVM 백엔드들은 `GuestProgram::elf("sp1")` → ELF 바이너리 반환 → `prove_with_elf()` 경로를 탑니다.
Tokamak은 ELF가 없으므로 (`GuestProgram::elf("tokamak")` → `None`) "legacy path"를 탑니다:

```
prover.rs prove_batch()
├─ ELF path: prove_with_elf(elf, serialized_input)  ← SP1, RISC0, etc.
└─ Legacy path: prove(ProgramInput, format)          ← Tokamak (여기)
```

---

## 4. 스마트 컨트랙트 계층

### 4.1 ITokamakVerifier 인터페이스

```solidity
// crates/l2/contracts/src/l1/interfaces/ITokamakVerifier.sol
interface ITokamakVerifier {
    function verify(
        uint128[] calldata proof_part1,       // 38개: G1 포인트 상위 16바이트
        uint256[] calldata proof_part2,       // 42개: G1 포인트 하위 32바이트 + 스칼라
        uint128[] calldata preprocess_part1,  // 4개: 순열 커밋먼트 상위 16바이트
        uint256[] calldata preprocess_part2,  // 4개: 순열 커밋먼트 하위 32바이트
        uint256[] calldata publicInputs,      // 512개: 전체 공개 입력
        uint256 smax                          // 회로 파라미터
    ) external view returns (bool);
}
```

### 4.2 OnChainProposer 통합

`OnChainProposer.sol`의 `verifyBatch()`에 Tokamak 검증 블록을 추가했습니다:

```solidity
// crates/l2/contracts/src/l1/based/OnChainProposer.sol (lines 509-538)
if (REQUIRE_TOKAMAK_PROOF) {
    (
        uint128[] memory proof_part1,
        uint256[] memory proof_part2,
        uint128[] memory preprocess_part1,
        uint256[] memory preprocess_part2,
        uint256[] memory tokamakPublicInputs,
        uint256 smax
    ) = abi.decode(
        tokamakProof,
        (uint128[], uint256[], uint128[], uint256[], uint256[], uint256)
    );

    try ITokamakVerifier(TOKAMAK_VERIFIER_ADDRESS).verify(
        proof_part1, proof_part2,
        preprocess_part1, preprocess_part2,
        tokamakPublicInputs, smax
    ) returns (bool result) {
        require(result, "OnChainProposer: Tokamak proof verification returned false");
    } catch {
        revert("OnChainProposer: Invalid Tokamak proof failed proof verification");
    }
}
```

### 4.3 컨트랙트 상태 변수

```solidity
uint8  internal constant TOKAMAK_VERIFIER_ID = 3;
bool   public REQUIRE_TOKAMAK_PROOF;      // 초기화 시 설정
address public TOKAMAK_VERIFIER_ADDRESS;   // 배포 시 설정
```

`initialize()`에서 `requireTokamakProof`가 true이면 `TOKAMAK_VERIFIER_ADDRESS`가
zero가 아닌지 검증합니다.

### 4.4 verifyBatch 시그니처 변경

기존: `verifyBatch(uint256,bytes,bytes,bytes,bytes)`
변경: `verifyBatch(uint256,bytes,bytes,bytes,bytes,bytes)`

새로운 `tokamakProof` 파라미터가 `tdxSignature` 뒤, `customPublicValues` 앞에 추가됨.
Solidity/Rust 양쪽 동기화가 필수.

---

## 5. TokamakBackend 프루버 구현

### 5.1 구조체 설계

```rust
// crates/l2/prover/src/backend/tokamak.rs

pub struct TokamakBackend {
    cli_path: PathBuf,          // tokamak-cli 바이너리 경로
    resource_dir: PathBuf,      // Tokamak-zk-EVM 레포 루트
    l2_rpc_url: Option<String>, // synthesizer가 상태를 fetch할 L2 RPC URL
}
```

### 5.2 ProverBackend trait 구현

| 메서드 | 구현 |
|--------|------|
| `prover_type()` | `ProverType::Tokamak` (u32 = 4) |
| `backend_name()` | `"tokamak"` (`backends::TOKAMAK`) |
| `serialize_input()` | No-op (Tokamak은 자체 입력 형식 사용) |
| `execute()` | synthesize + preprocess만 실행 |
| `prove()` | 전체 파이프라인: synthesize → preprocess → prove |
| `verify()` | No-op (on-chain 검증에 위임) |
| `to_batch_proof()` | ABI encode → `BatchProof::ProofCalldata` |

### 5.3 파이프라인 (`run_pipeline`)

```
write_input_config()
    │
    ├─ synthesizer .env에 RPC_URL 기록
    └─ ProgramInput을 config.json으로 직렬화
         │
tokamak-cli --synthesize <config.json>
    │  환경변수: TOKAMAK_ZK_EVM_ROOT = resource_dir
    │  출력: dist/resource/synthesizer/output/
    │        ├─ instance.json   (public inputs)
    │        ├─ permutation.json
    │        └─ placementVars.json
    │
tokamak-cli --preprocess
    │  출력: dist/resource/preprocess/output/
    │        └─ preprocess.json
    │
tokamak-cli --prove
    │  출력: dist/resource/prove/output/
    │        └─ proof.json
    │
read_proof()           → FormattedProof { part1: 38개, part2: 42개 }
read_preprocess()      → FormattedPreprocess { part1: 4개, part2: 4개 }
read_public_inputs()   → Vec<String> (512개 = 40 + 24 + 448)
read_smax()            → u64 (setupParams.json에서)
```

### 5.4 외부 프로세스 호출

```rust
fn run_cli(&self, args: &[&str]) -> Result<(), BackendError> {
    Command::new(&self.cli_path)
        .args(args)
        .env("TOKAMAK_ZK_EVM_ROOT", &self.resource_dir)
        .current_dir(&self.resource_dir)
        .output()
        // ...
}
```

`TOKAMAK_ZK_EVM_ROOT` 환경변수를 설정하여 CLI가 올바른 리소스 디렉토리를 사용하도록 합니다.

**왜 외부 프로세스 호출인가?**
- Tokamak의 synthesizer는 TypeScript로 작성됨 (tsx 런타임 필요)
- Rust 백엔드(preprocess, prove)는 ICICLE GPU 라이브러리와 강결합
- Library link 시 빌드 의존성이 과도하게 무거워짐
- CLI 래퍼를 통한 호출이 가장 깔끔한 격리 전략

### 5.5 출력 파싱 구조체

```rust
// proof.json 파싱
pub struct FormattedProof {
    pub proof_entries_part1: Vec<String>,  // 38 hex strings (uint128)
    pub proof_entries_part2: Vec<String>,  // 42 hex strings (uint256)
}

// preprocess.json 파싱
pub struct FormattedPreprocess {
    pub preprocess_entries_part1: Vec<String>,  // 4 hex strings (uint128)
    pub preprocess_entries_part2: Vec<String>,  // 4 hex strings (uint256)
}

// instance.json 파싱
pub struct InstanceJson {
    pub a_pub_user: Vec<String>,      // 40개
    pub a_pub_block: Vec<String>,     // 24개
    pub a_pub_function: Vec<String>,  // 448개
}

// setupParams.json 파싱
pub struct SetupParams {
    pub s_max: u64,  // 회로 파라미터 (유효값: 64, 128, 256, 512, 1024, 2048)
}

// 결합된 증명 출력
pub struct TokamakProveOutput {
    pub proof: FormattedProof,
    pub preprocess: FormattedPreprocess,
    pub public_inputs: Vec<String>,  // a_pub_user ++ a_pub_block ++ a_pub_function (512개)
    pub smax: u64,
}
```

### 5.6 출력 파일 경로

| 출력 | 경로 |
|------|------|
| proof.json | `dist/resource/prove/output/proof.json` |
| preprocess.json | `dist/resource/preprocess/output/preprocess.json` |
| instance.json | `dist/resource/synthesizer/output/instance.json` |
| setupParams.json | `dist/resource/qap-compiler/library/setupParams.json` |

---

## 6. E2E 증명 흐름

### 6.1 전체 데이터 흐름

```
1. ProofCoordinator (L2 Node)
   │  배치 생성 → ProgramInput 구성
   │  TCP로 ProofData::BatchResponse 전송
   │
2. Prover (tokamak 백엔드)
   │  ProgramInput 수신
   │  → TokamakBackend::prove()
   │  → run_pipeline() (synthesize → preprocess → prove)
   │  → TokamakProveOutput 생성
   │  → abi_encode_tokamak_proof() → bytes
   │  → BatchProof::ProofCalldata 구성
   │  TCP로 ProofData::ProofSubmit 전송
   │
3. ProofCoordinator
   │  증명 저장 (rollup_store)
   │
4. L1ProofSender (L2 Node)
   │  저장된 증명 로드
   │  tokamakProof = proofs[ProverType::Tokamak].calldata()
   │  verifyBatch() calldata 구성 (5개 증명 타입 모두 포함)
   │  L1 트랜잭션 전송
   │
5. OnChainProposer (L1)
   │  verifyBatch() 실행
   │  if REQUIRE_TOKAMAK_PROOF:
   │    abi.decode(tokamakProof) → 6개 파라미터
   │    ITokamakVerifier.verify() 호출
   │
6. TokamakVerifier (L1)
   │  BLS12-381 pairing 검증
   │  → true / false 반환
```

### 6.2 L1 Proof Sender 통합

`l1_proof_sender.rs`에서 Tokamak 증명을 다른 증명 타입들과 함께 calldata에 포함합니다:

```rust
// crates/l2/sequencer/l1_proof_sender.rs (lines 481-485)
proofs
    .get(&ProverType::Tokamak)
    .map(|proof| proof.calldata())
    .unwrap_or(ProverType::Tokamak.empty_calldata())
```

Tokamak 증명이 없으면 빈 bytes(`vec![]`)를 전달합니다.
`REQUIRE_TOKAMAK_PROOF`가 false이면 OnChainProposer에서 검증을 건너뜁니다.

### 6.3 에러 처리

잘못된 증명에 대한 자동 복구 로직:

```rust
// l1_proof_sender.rs (lines 520-525)
} else if message.contains("Invalid Tokamak proof")
       || message.contains("Tokamak proof verification") {
    warn!("Deleting invalid Tokamak proof");
    self.rollup_store
        .delete_proof_by_batch_and_type(batch_number, ProverType::Tokamak)
        .await?;
}
```

---

## 7. Synthesizer 분석

### 7.1 파이프라인

```
                    ┌────────────────┐
                    │   Config JSON  │  (트랜잭션 설정)
                    └───────┬────────┘
                            │
                   tokamak-cli --synthesize
                            │
                   ┌────────▼────────┐
                   │   Synthesizer   │
                   │  (TypeScript)   │
                   │                 │
                   │ 1. RPC로 상태 fetch │
                   │ 2. EVM 실행       │
                   │ 3. 회로 생성       │
                   └────────┬────────┘
                            │
              ┌─────────────┼─────────────┐
              │             │             │
    ┌─────────▼──┐ ┌────────▼───┐ ┌───────▼──────┐
    │instance.json│ │permutation │ │placementVars │
    │(public in)  │ │   .json    │ │    .json     │
    └─────────────┘ └────────────┘ └──────────────┘
                            │
                   tokamak-cli --preprocess
                            │
                   ┌────────▼────────┐
                   │  Preprocess     │
                   │  (Rust binary)  │
                   │                 │
                   │ 순열 다항식 처리    │
                   └────────┬────────┘
                            │
                  preprocess.json (4+4 entries)
                            │
                   tokamak-cli --prove
                            │
                   ┌────────▼────────┐
                   │    Prover       │
                   │  (Rust binary)  │
                   │                 │
                   │ 5단계 증명 생성:   │
                   │ prove0~prove4   │
                   │ + KZG opening   │
                   └────────┬────────┘
                            │
                   proof.json (38+42 entries)
                            │
                   tokamak-cli --verify
                            │
                   ┌────────▼────────┐
                   │   Verifier      │
                   │ (Rust/Solidity) │
                   │                 │
                   │ → true / false  │
                   └─────────────────┘
```

### 7.2 Synthesizer 입력 형식

#### Config JSON (L2TONTransferConfig)

```json
{
  "privateKeySeedsL2": [
    "Sender's L2 wallet",
    "Recipient's L2 wallet",
    "...(8개 참가자)"
  ],
  "addressListL1": [
    "0x85cc7da8Ee323325bcD678C7CFc4EB61e76657Fb",
    "0xd8eE65121e51aa8C75A6Efac74C4Bbd3C439F78f",
    "...(8개 L1 주소)"
  ],
  "userStorageSlots": [0],
  "senderIndex": 0,
  "recipientIndex": 1,
  "initStorageKey": "0x07",
  "txNonce": 0,
  "blockNumber": 23224548,
  "contractAddress": "0x2be5e8c109e2197D077D13A82dAead6a9b3433C5",
  "amount": "0x4563918244f400000",
  "transferSelector": "0xa9059cbb"
}
```

#### TypeScript 타입 정의

```typescript
type SynthesizerSimulationOpts = {
  rpcUrl: string;
  blockNumber: number;
  contractAddress: `0x${string}`;
  initStorageKeys: { L1: Uint8Array; L2: Uint8Array; }[];
  senderL2PrvKey: Uint8Array;
  txNonce: bigint;
  callData: Uint8Array;
};

type SynthesizerBlockInfo = {
  coinBase: bigint;
  timeStamp: bigint;
  blockNumber: bigint;
  prevRanDao: bigint;
  gasLimit: bigint;
  chainId: bigint;
  selfBalance: bigint;
  baseFee: bigint | undefined;
  blockHashes: bigint[];
};
```

### 7.3 RPC 의존성

**Synthesizer는 반드시 RPC 접속이 필요합니다.**

초기화 시 (`initTokamakExtendsFromRPC`) 다음을 fetch합니다:
- 컨트랙트 바이트코드
- 등록된 스토리지 키의 값
- 블록 정보 (coinbase, timestamp, etc.)
- 이전 블록 해시 배열

Non-RPC 모드는 **지원되지 않습니다**.

### 7.4 출력 형식

#### proof.json

```json
{
  "proof_entries_part1": [
    "0x14707e7c13706ad855a0110bd1a95fbe",
    "..."
  ],
  "proof_entries_part2": [
    "0x751730d725d624d4d65dc9ece3c952d054255c31b1dd297e2f1458ab472e2185",
    "..."
  ]
}
```

- **part1** (38개): G1 포인트 좌표의 상위 16바이트 → `uint128[]`
- **part2** (42개): G1 포인트 좌표의 하위 32바이트 + 스칼라 평가값 → `uint256[]`

#### preprocess.json

```json
{
  "preprocess_entries_part1": ["0x...", "0x...", "0x...", "0x..."],
  "preprocess_entries_part2": ["0x...", "0x...", "0x...", "0x..."]
}
```

- 4+4개: s^(0), s^(1) 순열 다항식의 커밋먼트 포인트

#### instance.json (Public Inputs)

```json
{
  "a_pub_user": ["0x...", ...],
  "a_pub_block": ["0x...", ...],
  "a_pub_function": ["0x...", ...]
}
```

전체 512개의 public input = `a_pub_user(40)` ++ `a_pub_block(24)` ++ `a_pub_function(448)`.

#### setupParams.json

```json
{
  "l": 512, "l_user_out": 8, "l_user": 40, "l_block": 64,
  "l_D": 2560, "m_D": 13251, "n": 2048, "s_D": 23, "s_max": 256
}
```

`s_max`는 CRS(trusted setup)에서 결정되는 회로 파라미터.
유효한 값: 64, 128, 256, 512, 1024, 2048.

---

## 8. ABI 인코딩

### 8.1 인코딩 형식

TokamakProveOutput을 Solidity의 `abi.encode()` 형식으로 인코딩합니다:

```solidity
abi.encode(
    uint256[] proof_part1,       // uint128 값을 uint256으로 확장
    uint256[] proof_part2,
    uint256[] preprocess_part1,  // uint128 값을 uint256으로 확장
    uint256[] preprocess_part2,
    uint256[] publicInputs,
    uint256   smax               // 정적 값
)
```

### 8.2 메모리 레이아웃

```
Offset  Content
──────  ───────────────────────────────────────
0x000   offset(proof_part1)          ← 동적 배열 오프셋 포인터
0x020   offset(proof_part2)
0x040   offset(preprocess_part1)
0x060   offset(preprocess_part2)
0x080   offset(publicInputs)
0x0A0   smax (정적 uint256)          ← 6번째 슬롯
0x0C0   len(proof_part1)             ← 첫 번째 배열 시작
0x0E0   proof_part1[0]
...
        len(proof_part2)
        proof_part2[0]
...
```

### 8.3 실제 데이터 크기

| 구성 요소 | 원소 수 | 인코딩 크기 |
|----------|--------|------------|
| Head (6 slots) | 6 | 192 bytes |
| proof_part1 (38) | 38 + 1(len) | 1,248 bytes |
| proof_part2 (42) | 42 + 1 | 1,376 bytes |
| preprocess_part1 (4) | 4 + 1 | 160 bytes |
| preprocess_part2 (4) | 4 + 1 | 160 bytes |
| publicInputs (512) | 512 + 1 | 16,416 bytes |
| **Total** | | **19,552 bytes** |

### 8.4 Solidity 디코딩 (OnChainProposer)

```solidity
(
    uint128[] memory proof_part1,
    uint256[] memory proof_part2,
    uint128[] memory preprocess_part1,
    uint256[] memory preprocess_part2,
    uint256[] memory tokamakPublicInputs,
    uint256 smax
) = abi.decode(
    tokamakProof,
    (uint128[], uint256[], uint128[], uint256[], uint256[], uint256)
);
```

**주의**: Solidity 측에서는 `uint128[]`로 디코딩하지만, ABI 인코딩 시에는
`uint256[]`로 패딩하여 전송합니다. Solidity의 `abi.decode`가 자동으로
상위 비트를 truncate하여 `uint128`로 변환합니다.

---

## 9. 배포 (Deployer)

### 9.1 TokamakVerifier 배포

`cmd/ethrex/l2/deployer.rs`에서 TokamakVerifier를 CREATE2로 배포합니다:

```rust
// deployer.rs (lines 589-594)
const TOKAMAK_VERIFIER_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/contracts/solc_out/TokamakVerifier.bytecode"
));
```

배포 로직:
1. `--tokamak true` 플래그가 설정되면 배포 활성화
2. `--tokamak.verifier-address`로 기존 주소 지정 가능 (재배포 방지)
3. 주소 미지정 시 CREATE2로 새 인스턴스 배포
4. 배포된 주소를 `.env` 파일에 기록: `ETHREX_DEPLOYER_TOKAMAK_VERIFIER_ADDRESS`

### 9.2 OnChainProposer 초기화

`initialize()` 호출 시 Tokamak 관련 파라미터 전달:

```rust
// deployer.rs (lines 1318-1323)
Value::Bool(opts.tokamak),                                    // requireTokamakProof
Value::Address(contract_addresses.tokamak_verifier_address),  // tokamakVerifierAddress
```

### 9.3 배포 CLI 옵션

```
--tokamak true                    배포 시 TokamakVerifier 활성화
--tokamak.verifier-address 0x...  기존 TokamakVerifier 주소 사용
```

환경변수:
```
ETHREX_L2_TOKAMAK=true
ETHREX_DEPLOYER_TOKAMAK_VERIFIER_ADDRESS=0x...
```

---

## 10. CLI 옵션 및 설정

### 10.1 배포 CLI (cmd/ethrex/l2/options.rs)

| 옵션 | 환경변수 | 설명 |
|------|---------|------|
| `--tokamak` | `ETHREX_L2_TOKAMAK` | Tokamak 증명 요구 활성화 |
| `--tokamak.verifier-address` | `ETHREX_DEPLOYER_TOKAMAK_VERIFIER_ADDRESS` | 기배포 검증자 주소 |

### 10.2 프루버 CLI (cmd/ethrex/l2/options.rs)

| 옵션 | 환경변수 | 설명 |
|------|---------|------|
| `--backend tokamak` | - | Tokamak 백엔드 선택 |
| `--tokamak-cli-path` | `ETHREX_TOKAMAK_CLI_PATH` | tokamak-cli 바이너리 경로 |
| `--tokamak-resource-dir` | `ETHREX_TOKAMAK_RESOURCE_DIR` | Tokamak-zk-EVM 레포 루트 |
| `--tokamak-l2-rpc-url` | `ETHREX_TOKAMAK_L2_RPC_URL` | Synthesizer용 L2 RPC URL |

### 10.3 ProverConfig 구조체

```rust
// crates/l2/prover/src/config.rs
pub struct ProverConfig {
    pub backend: BackendType,
    pub proof_coordinators: Vec<Url>,
    pub proving_time_ms: u64,
    pub timed: bool,
    #[cfg(feature = "tokamak")]
    pub tokamak_cli_path: Option<PathBuf>,
    #[cfg(feature = "tokamak")]
    pub tokamak_resource_dir: Option<PathBuf>,
    #[cfg(feature = "tokamak")]
    pub tokamak_l2_rpc_url: Option<String>,
}
```

### 10.4 Feature flag

```toml
# crates/l2/prover/Cargo.toml
[features]
tokamak = []  # 외부 프로세스 호출 방식이므로 추가 deps 없음
```

빌드:
```bash
cargo build --release --features l2,l2-sql,tokamak
```

---

## 11. Localnet 운영

### 11.1 Localnet 스크립트

`crates/l2/scripts/zk-dex-tokamak-localnet.sh`

전체 ZK-DEX E2E 환경을 Tokamak 백엔드로 기동합니다.

```bash
# 시작
./scripts/zk-dex-tokamak-localnet.sh start

# 프루버 없이 (앱 테스트만)
./scripts/zk-dex-tokamak-localnet.sh start --no-prover

# 중지
./scripts/zk-dex-tokamak-localnet.sh stop

# 상태 확인
./scripts/zk-dex-tokamak-localnet.sh status

# 로그 확인
./scripts/zk-dex-tokamak-localnet.sh logs [l1|l2|prover]
```

### 11.2 기동 순서

| 단계 | 작업 | 설명 |
|------|------|------|
| 1 | Initialize | 디렉토리 생성, 이전 데이터 정리 |
| 2 | Start L1 | ethrex L1 노드 (dev 모드) 기동 |
| 3 | Wait for L1 | RPC 응답 대기 (timeout: 300s) |
| 4 | Deploy Contracts | L1 + TokamakVerifier + ZK-DEX genesis 배포 |
| 5 | Start L2 | ethrex L2 노드 (tokamak feature) 기동 |
| 6 | Wait for L2 | RPC 응답 대기 (timeout: 600s) |
| 7 | Start Prover | Tokamak 프루버 기동 (--no-prover로 스킵 가능) |
| 8 | Summary | 주소, 포트, 상태 출력 |

### 11.3 환경변수

```bash
TOKAMAK_CLI_PATH       # tokamak-cli 바이너리 경로 (기본: tokamak-cli)
TOKAMAK_RESOURCE_DIR   # Tokamak 리소스 디렉토리 (기본: .)
```

### 11.4 tokamak-cli 설치

```bash
cd /path/to/Tokamak-zk-EVM

# 사전 요구사항 확인
./tokamak-cli --doctor

# 설치 (QAP 컴파일 + 백엔드 패키징 + trusted setup)
./tokamak-cli --install http://localhost:8545

# 설치 후 사전 요구사항 재확인
./tokamak-cli --doctor
```

필수 도구: `node`, `npm`, `tsx`, `cmake`, `dos2unix`, `cargo`

---

## 12. 파일 인벤토리

### 12.1 스마트 컨트랙트

| 파일 | 역할 |
|------|------|
| `contracts/src/l1/TokamakVerifier.sol` | BLS12-381 zkSNARK 검증자 구현 |
| `contracts/src/l1/interfaces/ITokamakVerifier.sol` | 검증자 인터페이스 |
| `contracts/src/l1/OnChainProposer.sol` | 시퀀서 기반 배치 검증 |
| `contracts/src/l1/based/OnChainProposer.sol` | Based 배치 검증 |
| `contracts/src/l1/Timelock.sol` | verifyBatch pass-through |
| `contracts/src/l1/interfaces/ITimelock.sol` | Timelock 인터페이스 |
| `contracts/src/l1/interfaces/IOnChainProposer.sol` | 시퀀서 기반 인터페이스 |
| `contracts/src/l1/based/interfaces/IOnChainProposer.sol` | Based 인터페이스 |

### 12.2 프루버 백엔드

| 파일 | 역할 |
|------|------|
| `prover/src/backend/tokamak.rs` | TokamakBackend 구현 (717줄) |
| `prover/src/backend/mod.rs` | BackendType::Tokamak 등록 |
| `prover/src/prover.rs` | Tokamak startup 분기 |
| `prover/src/config.rs` | Tokamak 설정 필드 |
| `prover/Cargo.toml` | tokamak feature flag |

### 12.3 L2 공통

| 파일 | 역할 |
|------|------|
| `common/src/prover.rs` | ProverType::Tokamak (ID=4), empty_calldata, verifier_getter |

### 12.4 시퀀서

| 파일 | 역할 |
|------|------|
| `sequencer/l1_proof_sender.rs` | Tokamak 증명 calldata 구성 + 에러 처리 |

### 12.5 CLI / 배포

| 파일 | 역할 |
|------|------|
| `cmd/ethrex/l2/options.rs` | --tokamak*, --backend tokamak CLI 옵션 |
| `cmd/ethrex/l2/deployer.rs` | TokamakVerifier CREATE2 배포 |

### 12.6 게스트 프로그램

| 파일 | 역할 |
|------|------|
| `guest-program/src/traits.rs` | backends::TOKAMAK 상수 |

### 12.7 스크립트 / 문서

| 파일 | 역할 |
|------|------|
| `scripts/zk-dex-tokamak-localnet.sh` | Localnet 기동/중지/상태 스크립트 |
| `tokamak-notes/verifier-prover-modularization/00-design-overview.md` | Phase 1-2 설계 |
| `tokamak-notes/verifier-prover-modularization/01-implementation-log.md` | Phase 1-2 구현 로그 |
| `tokamak-notes/verifier-prover-modularization/02-tokamak-integration-guide.md` | 이 문서 |

---

## 13. 성능 벤치마크

### 13.1 로컬 테스트 결과

환경: macOS, CPU only (Metal GPU 미지원 - ICICLE 3.8.0 제한)

| 단계 | 소요 시간 |
|------|----------|
| Synthesize | 131ms |
| Preprocess | 4.6초 |
| Prove (전체) | 67초 |
| - prove0 (산술 제약) | 8.4초 |
| - prove1 (재귀 다항식) | 1.1초 |
| - prove2 (복사 제약) | 29.9초 |
| - prove3 (KZG opening) | 1.5초 |
| - prove4 (KZG proof) | 12.6초 |
| Verify | <1초 |

GPU(CUDA) 환경에서는 상당한 성능 향상이 예상됩니다.

### 13.2 증명 데이터 크기

| 항목 | 크기 |
|------|------|
| ABI 인코딩된 전체 증명 | 19,552 bytes |
| proof (part1 + part2) | 2,624 bytes |
| preprocess (part1 + part2) | 320 bytes |
| public inputs (512개) | 16,416 bytes |
| smax | 32 bytes |

### 13.3 on-chain 검증 비용

TokamakVerifier의 가스비는 약 655K gas로 최적화되어 있습니다.

---

## 14. 제약사항 및 향후 작업

### 14.1 근본적 제약

1. **범용 EVM 증명 불가**: Tokamak은 Poseidon/EdDSA 기반 커스텀 EVM 사용
   - Keccak256 → Poseidon 대체
   - secp256k1/ECDSA → EdDSA/jubjub 대체
   - MPT → Poseidon 4-ary Merkle Tree 대체
2. **RPC 필수**: synthesizer가 직접 상태를 RPC로 fetch해야 함
3. **고정 회로**: L2 TON Transfer 패턴에 특화 (임의 컨트랙트 증명 불가)

### 14.2 완료된 통합

| 구성 요소 | 상태 |
|-----------|------|
| TokamakBackend (ProverBackend trait) | ✅ |
| CLI 호출 파이프라인 (synthesize → preprocess → prove) | ✅ |
| 출력 파싱 (proof.json, preprocess.json, instance.json, setupParams.json) | ✅ |
| ABI 인코딩 (Solidity abi.encode 호환) | ✅ |
| OnChainProposer 통합 (verifyBatch) — based + non-based | ✅ |
| ITokamakVerifier 인터페이스 + TokamakVerifier 컨트랙트 | ✅ |
| TokamakVerifier CREATE2 자동 배포 | ✅ |
| L1 Proof Sender 흐름 (calldata 구성 + 에러 처리) | ✅ |
| ProverType::Tokamak (ID=4, empty_calldata, verifier_getter) | ✅ |
| L2 RPC URL 설정 | ✅ |
| CLI 옵션 (배포 + 프루버) | ✅ |
| Feature gating (#[cfg(feature = "tokamak")]) | ✅ |
| Localnet 스크립트 (zk-dex-tokamak-localnet.sh) | ✅ |
| 단위 테스트 (14개: 파싱, ABI 인코딩, 타입 검증) | ✅ |
| Timelock.sol pass-through 수정 | ✅ |

### 14.3 미완료 작업

| 구성 요소 | 상태 | 설명 |
|-----------|------|------|
| ProgramInput → Synthesizer config 변환 | ❌ | ethrex 블록 데이터를 L2TONTransferConfig로 매핑하는 로직 필요 |
| Preprocess 캐싱 | ❌ | preprocess는 회로 setup당 1회만 필요, 매 배치마다 재실행 불필요 |
| Non-RPC 모드 | ❌ | Tokamak synthesizer 자체 수정 필요 |
| 로컬 verify | ❌ | tokamak-cli --verify 호출 통합 |

### 14.4 현실적 통합 방안

1. **L2 노드를 RPC 소스로 사용**: synthesizer가 로컬 L2 노드에 접속하여 상태 fetch
2. **Tokamak 호환 컨트랙트만 증명**: Poseidon/EdDSA를 사용하는 전용 컨트랙트
3. **범용 EVM 증명은 SP1/RISC0 사용**: 일반 EVM 트랜잭션은 기존 zkVM 백엔드 활용
4. **듀얼 증명**: 일반 트랜잭션은 SP1, Tokamak 호환 트랜잭션은 Tokamak으로 이원화

### 14.5 Tokamak-zk-EVM 로드맵

Tokamak-zk-EVM이 범용 EVM 증명을 지원하려면:
- Keccak256 회로 추가
- secp256k1/ECDSA 검증 회로 추가
- 가변 크기 스토리지 트리 지원
- 임의 바이트코드 실행 지원
