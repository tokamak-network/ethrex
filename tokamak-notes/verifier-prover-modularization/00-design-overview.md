# TokamakVerifier 모듈화 통합 설계 문서

## 1. 개요

PR #82 (tokamak-network/Tokamak-zk-EVM-contracts)의 TokamakVerifier를 SP1/RISC0/TDX와 동일하게 선택 가능한 검증기/프루버 백엔드로 통합한다.

### 배경
- TokamakVerifier는 커스텀 zkSNARK 기반 검증기
- 가스비 655K로 최적화됨
- 기존 SP1, RISC0, TDX와 동일한 패턴으로 통합 필요

### 브랜치
`feat/zk/verifier-prover-modularization`

---

## 2. 아키텍처

### 현재 검증기 구조
```
OnChainProposer.verifyBatch()
├── RISC0 검증 (REQUIRE_RISC0_PROOF)
├── SP1 검증 (REQUIRE_SP1_PROOF)
└── TDX 검증 (REQUIRE_TDX_PROOF)
```

### 목표 검증기 구조
```
OnChainProposer.verifyBatch()
├── RISC0 검증 (REQUIRE_RISC0_PROOF)
├── SP1 검증 (REQUIRE_SP1_PROOF)
├── TDX 검증 (REQUIRE_TDX_PROOF)
└── Tokamak 검증 (REQUIRE_TOKAMAK_PROOF)  ← NEW
```

### ProverType 열거형 확장
```
Exec  = 0  (기존)
RISC0 = 1  (기존)
SP1   = 2  (기존)
TDX   = 3  (기존)
Tokamak = 4  ← NEW
```

---

## 3. 변경 파일 요약

| # | 파일 | 작업 | 레이어 |
|---|------|------|--------|
| 1 | `contracts/src/l1/interfaces/ITokamakVerifier.sol` | 신규 인터페이스 | Solidity |
| 2 | `contracts/src/l1/interfaces/IOnChainProposer.sol` | verifyBatch 시그니처 변경 | Solidity |
| 3 | `contracts/src/l1/based/interfaces/IOnChainProposer.sol` | verifyBatch 시그니처 변경 | Solidity |
| 4 | `contracts/src/l1/OnChainProposer.sol` | Tokamak 검증기 통합 (non-based) | Solidity |
| 5 | `contracts/src/l1/based/OnChainProposer.sol` | Tokamak 검증기 통합 (based) | Solidity |
| 6 | `crates/l2/common/src/prover.rs` | ProverType에 Tokamak 추가 | Rust |
| 7 | `crates/l2/sequencer/l1_proof_sender.rs` | verifyBatch calldata에 Tokamak 추가 | Rust |
| 8 | `cmd/ethrex/l2/deployer.rs` | CLI 옵션 + initialize 시그니처 + calldata | Rust |

---

## 4. 상세 설계

### 4.1 ITokamakVerifier 인터페이스

TokamakVerifier의 `verify` 함수는 6개의 파라미터를 받는 커스텀 zkSNARK 검증 인터페이스:

```solidity
interface ITokamakVerifier {
    function verify(
        uint128[] calldata proof_part1,
        uint256[] calldata proof_part2,
        uint128[] calldata preprocess_part1,
        uint256[] calldata preprocess_part2,
        uint256[] calldata publicInputs,
        uint256 smax
    ) external view returns (bool);
}
```

### 4.2 verifyBatch 시그니처 변경

기존 `verifyBatch(uint256,bytes,bytes,bytes,bytes)` → `verifyBatch(uint256,bytes,bytes,bytes,bytes,bytes)`

새로운 `tokamakProof` 파라미터가 `tdxSignature` 뒤, `customPublicValues` 앞에 추가됨.

### 4.3 OnChainProposer 변경사항

#### 새 상수/스토리지
```solidity
uint8 internal constant TOKAMAK_VERIFIER_ID = 3;
address public TOKAMAK_VERIFIER_ADDRESS;
bool public REQUIRE_TOKAMAK_PROOF;
```

#### initialize() 확장
- `bool requireTokamakProof` 파라미터 추가
- `address tokamakverifier` 파라미터 추가
- require 검증: `!REQUIRE_TOKAMAK_PROOF || tokamakverifier != address(0)`

#### verifyBatch() 확장
TDX 검증 블록 뒤에 Tokamak 검증 블록 추가:
```solidity
if (REQUIRE_TOKAMAK_PROOF) {
    (
        uint128[] memory proof_part1,
        uint256[] memory proof_part2,
        uint128[] memory preprocess_part1,
        uint256[] memory preprocess_part2,
        uint256[] memory tokamakPublicInputs,
        uint256 smax
    ) = abi.decode(tokamakProof, (uint128[], uint256[], uint128[], uint256[], uint256[], uint256));

    bool result = ITokamakVerifier(TOKAMAK_VERIFIER_ADDRESS).verify(
        proof_part1, proof_part2,
        preprocess_part1, preprocess_part2,
        tokamakPublicInputs, smax
    );
    require(result, "Invalid Tokamak proof");
}
```

### 4.4 ProverType Rust 열거형

```rust
pub enum ProverType {
    Exec,    // 0
    RISC0,   // 1
    SP1,     // 2
    TDX,     // 3
    Tokamak, // 4  ← NEW
}
```

메서드 확장:
- `From<ProverType> for u32`: `Tokamak => 4`
- `all()`: Tokamak 포함
- `empty_calldata()`: `vec![Value::Bytes(vec![].into())]`
- `verifier_getter()`: `Some("REQUIRE_TOKAMAK_PROOF()")`
- `Display`: `"Tokamak"`

### 4.5 l1_proof_sender calldata 변경

`VERIFY_FUNCTION_SIGNATURE` → `"verifyBatch(uint256,bytes,bytes,bytes,bytes,bytes)"` (bytes 5개)

calldata 배열에 Tokamak proof 추가 (TDX 뒤, customPublicValues 앞).

### 4.6 deployer 변경

- DeployerOptions: `tokamak: bool`, `tokamak_verifier_address: Option<Address>`
- ContractAddresses: `tokamak_verifier_address: Address`
- initialize 시그니처에 `bool`, `address` 2개 추가
- calldata_values에 `Value::Bool(opts.tokamak)`, `Value::Address(...)` 추가

---

## 5. 호환성

- `REQUIRE_TOKAMAK_PROOF=false` (기본값)일 때 기존 동작에 영향 없음
- verifyBatch 시그니처가 변경되므로 Solidity/Rust 양쪽 동기화 필수
- 기존 ProverType 값 (0-3)은 변경 없음, Tokamak은 4로 추가

---

## 6. 검증 방법

1. **컴파일 확인**: Solidity 컴파일 + `cargo build`
2. **기존 테스트**: `cargo test -p ethrex-l2-common` 통과 확인
3. **기능 테스트**: `REQUIRE_TOKAMAK_PROOF=false`일 때 기존 동작 확인
4. **인터페이스 일관성**: verifyBatch 시그니처 Solidity/Rust 일치 확인
