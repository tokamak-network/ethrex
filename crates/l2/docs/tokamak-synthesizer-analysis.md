# Tokamak-zk-EVM Synthesizer 분석

## 개요

Tokamak-zk-EVM은 커스텀 zkSNARK 시스템으로, SP1/RISC0 같은 범용 zkVM이 아닙니다.
자체 회로 컴파일러(QAP)와 synthesizer를 통해 EVM 트랜잭션을 증명합니다.

이 문서는 Tokamak synthesizer의 입력 형식, 동작 방식, 제약사항을 분석합니다.

---

## 1. zkVM vs Tokamak-zk-EVM 아키텍처 비교

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

### 핵심 차이점

SP1은 ethrex EVM 전체를 RISC-V로 컴파일하여 임의 EVM 트랜잭션을 증명합니다.
Tokamak은 특정 트랜잭션 패턴(L2 TON Transfer)에 대한 전용 Circom 회로를 사용합니다.

따라서 **ethrex L2의 임의 블록 실행을 Tokamak으로 증명하는 것은 현재 불가능합니다.**

---

## 2. Synthesizer 파이프라인

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

---

## 3. Synthesizer 입력 형식

### 3.1 Config JSON (L2TONTransferConfig)

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

### 3.2 TypeScript 타입 정의

```typescript
// 고수준 시뮬레이션 옵션
type SynthesizerSimulationOpts = {
  rpcUrl: string;          // Ethereum RPC URL (상태 fetch용)
  blockNumber: number;     // 상태를 가져올 블록 번호
  contractAddress: `0x${string}`;  // 대상 컨트랙트
  initStorageKeys: {
    L1: Uint8Array;        // L1 스토리지 키
    L2: Uint8Array;        // L2 스토리지 키 (Poseidon MPT)
  }[];
  senderL2PrvKey: Uint8Array;  // 송신자의 EdDSA 개인키
  txNonce: bigint;
  callData: Uint8Array;    // 트랜잭션 calldata
};

// 저수준 Synthesizer 옵션
interface SynthesizerOpts {
  signedTransaction: TokamakL2Tx;       // EdDSA 서명된 트랜잭션
  blockInfo: SynthesizerBlockInfo;       // 블록 컨텍스트
  stateManager: TokamakL2StateManager;  // Poseidon Merkle 상태
}

// 블록 정보
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

### 3.3 RPC 의존성

**Synthesizer는 반드시 RPC 접속이 필요합니다.**

초기화 시 (`initTokamakExtendsFromRPC`) 다음을 fetch합니다:
- 컨트랙트 바이트코드
- 등록된 스토리지 키의 값
- 블록 정보 (coinbase, timestamp, etc.)
- 이전 블록 해시 배열

Non-RPC 모드는 **지원되지 않습니다**.

---

## 4. 출력 형식

### 4.1 proof.json

```json
{
  "proof_entries_part1": [
    "0x14707e7c13706ad855a0110bd1a95fbe",  // uint128 값 38개
    "..."
  ],
  "proof_entries_part2": [
    "0x751730d725d624d4d65dc9ece3c952d054255c31b1dd297e2f1458ab472e2185",  // uint256 값 42개
    "..."
  ]
}
```

- **part1** (38개): G1 포인트 좌표의 상위 16바이트 → `uint128[]`
- **part2** (42개): G1 포인트 좌표의 하위 32바이트 + 스칼라 평가값 → `uint256[]`

### 4.2 preprocess.json

```json
{
  "preprocess_entries_part1": ["0x...", "0x...", "0x...", "0x..."],  // 4개 uint128
  "preprocess_entries_part2": ["0x...", "0x...", "0x...", "0x..."]   // 4개 uint256
}
```

- s^(0), s^(1) 순열 다항식의 커밋먼트 포인트

### 4.3 instance.json (Public Inputs)

```json
{
  "a_pub_user": ["0x...", ...],       // 40개 - 사용자 공개 입력
  "a_pub_block": ["0x...", ...],      // 24개 - 블록 공개 입력
  "a_pub_function": ["0x...", ...]    // 448개 - 함수 공개 입력
}
```

전체 512개의 public input은 이 3개 배열을 연결하여 구성합니다.

### 4.4 setupParams.json (s_max)

```json
{
  "l": 512,
  "l_user_out": 8,
  "l_user": 40,
  "l_block": 64,
  "l_D": 2560,
  "m_D": 13251,
  "n": 2048,
  "s_D": 23,
  "s_max": 256
}
```

`s_max`는 CRS(trusted setup)에서 결정되는 회로 파라미터입니다.
유효한 값: 64, 128, 256, 512, 1024, 2048

---

## 5. Solidity Verifier 인터페이스

```solidity
interface IVerifierV3 {
    function verify(
        uint128[] calldata _proof_part1,       // proof 상위 16바이트
        uint256[] calldata _proof_part2,       // proof 하위 32바이트 + 스칼라
        uint128[] calldata preprocessedPart1,  // preprocess 상위 16바이트
        uint256[] calldata preprocessedPart2,  // preprocess 하위 32바이트
        uint256[] calldata publicInputs,       // 전체 공개 입력 (512개)
        uint256 smax                           // 회로 파라미터
    ) external view returns (bool);
}
```

ethrex의 OnChainProposer에서:
```solidity
if (REQUIRE_TOKAMAK_PROOF) {
    (uint128[] memory proof_part1, uint256[] memory proof_part2,
     uint128[] memory preprocess_part1, uint256[] memory preprocess_part2,
     uint256[] memory tokamakPublicInputs, uint256 smax
    ) = abi.decode(tokamakProof, (uint128[], uint256[], uint128[], uint256[], uint256[], uint256));

    ITokamakVerifier(TOKAMAK_VERIFIER_ADDRESS).verify(
        proof_part1, proof_part2,
        preprocess_part1, preprocess_part2,
        tokamakPublicInputs, smax
    );
}
```

---

## 6. 성능 벤치마크 (로컬 테스트)

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

---

## 7. ethrex 통합 현황

### 완료된 부분

| 구성 요소 | 상태 | 파일 |
|-----------|------|------|
| TokamakBackend (ProverBackend trait) | ✅ | `prover/src/backend/tokamak.rs` |
| CLI 호출 파이프라인 | ✅ | `tokamak.rs` → `tokamak-cli` |
| Proof 파싱 (proof.json, preprocess.json) | ✅ | `tokamak.rs` |
| Public inputs 파싱 (instance.json) | ✅ | `tokamak.rs` |
| ABI 인코딩 (Solidity 호환) | ✅ | `tokamak.rs` |
| OnChainProposer 통합 | ✅ | `OnChainProposer.sol` |
| TokamakVerifier 배포 | ✅ | `deployer.rs` |
| Proof sender 흐름 | ✅ | `l1_proof_sender.rs` |
| L2 RPC URL 설정 | ✅ | `config.rs`, `options.rs` |
| Localnet 스크립트 | ✅ | `zk-dex-tokamak-localnet.sh` |

### 미완료 부분

| 구성 요소 | 상태 | 설명 |
|-----------|------|------|
| ProgramInput → Synthesizer config 변환 | ❌ | ethrex 블록 데이터를 synthesizer 입력으로 매핑 필요 |
| Preprocess 캐싱 | ❌ | 회로당 1회만 필요, 매 배치마다 재실행 불필요 |
| Non-RPC 모드 | ❌ | Tokamak synthesizer 자체 수정 필요 |

---

## 8. 제약사항 및 향후 작업

### 근본적 제약

1. **범용 EVM 증명 불가**: Tokamak은 Poseidon/EdDSA 기반 커스텀 EVM 사용
2. **RPC 필수**: synthesizer가 직접 상태를 fetch해야 함
3. **고정 회로**: L2 TON Transfer 패턴에 특화 (임의 컨트랙트 증명 불가)

### 현실적 통합 방안

1. **L2 노드를 RPC 소스로 사용**: synthesizer가 로컬 L2 노드에 접속하여 상태 fetch
2. **Tokamak 호환 컨트랙트만 증명**: Poseidon/EdDSA를 사용하는 전용 컨트랙트
3. **범용 EVM 증명은 SP1/RISC0 사용**: 일반 EVM 트랜잭션은 기존 zkVM 백엔드 활용

### Tokamak-zk-EVM 로드맵

Tokamak-zk-EVM이 향후 범용 EVM 증명을 지원하려면:
- Keccak256 회로 추가
- secp256k1/ECDSA 검증 회로 추가
- 가변 크기 스토리지 트리 지원
- 임의 바이트코드 실행 지원
