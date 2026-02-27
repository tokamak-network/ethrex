# TokamakVerifier 모듈화 구현 로그

## 변경된 파일 목록

### 신규 파일
| 파일 | 설명 |
|------|------|
| `contracts/src/l1/interfaces/ITokamakVerifier.sol` | Tokamak zkSNARK 검증기 인터페이스 |
| `tokamak-notes/verifier-prover-modularization/00-design-overview.md` | 설계 문서 |
| `tokamak-notes/verifier-prover-modularization/01-implementation-log.md` | 구현 로그 (이 파일) |

### 수정된 파일
| 파일 | 변경 내용 |
|------|-----------|
| `contracts/src/l1/interfaces/IOnChainProposer.sol` | verifyBatch에 `tokamakProof` 파라미터 추가 |
| `contracts/src/l1/based/interfaces/IOnChainProposer.sol` | verifyBatch에 `tokamakProof` 파라미터 추가 |
| `contracts/src/l1/interfaces/ITimelock.sol` | verifyBatch에 `tokamakProof` 파라미터 추가 |
| `contracts/src/l1/OnChainProposer.sol` | import, 상수, 스토리지, initialize, verifyBatch 확장 |
| `contracts/src/l1/based/OnChainProposer.sol` | import, 상수, 스토리지, initialize, verifyBatch 확장 |
| `contracts/src/l1/Timelock.sol` | verifyBatch pass-through에 tokamakProof 추가 |
| `crates/l2/common/src/prover.rs` | ProverType::Tokamak 추가 (값=4) |
| `crates/l2/sequencer/l1_proof_sender.rs` | 시그니처, calldata, 에러 핸들링 업데이트 |
| `cmd/ethrex/l2/deployer.rs` | CLI 옵션, ContractAddresses, initialize 시그니처, calldata 업데이트 |

---

## 구현 세부 사항

### Step 1: ITokamakVerifier.sol 인터페이스
- PR #82의 TokamakVerifier.verify() 시그니처 기반
- 6개 파라미터: proof_part1, proof_part2, preprocess_part1, preprocess_part2, publicInputs, smax
- `view` 함수로 `bool` 반환

### Step 2: IOnChainProposer.sol 인터페이스 수정
- non-based, based 양쪽 모두 수정
- `bytes memory tokamakProof` 파라미터 추가 (tdxSignature 뒤, customPublicValues 앞)

### Step 3: OnChainProposer.sol (non-based) 수정
- `TOKAMAK_VERIFIER_ID = 3` 상수 추가
- `TOKAMAK_VERIFIER_ADDRESS`, `REQUIRE_TOKAMAK_PROOF` 스토리지 추가
- `initialize()`: `requireTokamakProof`, `tokamakverifier` 파라미터 추가
- `verifyBatch()`: TDX 블록 뒤에 Tokamak 검증 블록 추가
  - abi.decode로 6개 파라미터 디코딩
  - ITokamakVerifier.verify() 호출
  - try-catch 패턴 사용 (기존 패턴 동일)
  - 에러 코드: "00t" (returned false), "00u" (verify failed)

### Step 4: based/OnChainProposer.sol 수정
- Step 3과 동일한 패턴

### Step 5: ProverType Rust enum 확장
- `Tokamak = 4` variant 추가
- `all()`, `empty_calldata()`, `verifier_getter()`, `Display` 모두 확장

### Step 6: l1_proof_sender.rs 수정
- `VERIFY_FUNCTION_SIGNATURE`: bytes 5개로 변경
- calldata 배열에 Tokamak proof 슬롯 추가
- 에러 핸들링: "Invalid Tokamak proof" / "Tokamak proof verification" 매칭 추가

### Step 7: deployer.rs 수정
- CLI: `--tokamak` (bool), `--tokamak.verifier-address` (Address) 추가
- ContractAddresses: `tokamak_verifier_address` 필드 추가
- initialize 시그니처: based/non-based 양쪽에 `bool`, `address` 추가
- calldata: `Value::Bool(opts.tokamak)`, `Value::Address(contract_addresses.tokamak_verifier_address)` 추가
- env 파일: `ETHREX_DEPLOYER_TOKAMAK_VERIFIER_ADDRESS` 추가
- defaults: `tokamak: false`, `tokamak_verifier_address: None`

### 추가 발견: Timelock.sol 수정
- 계획에 없었으나, Timelock이 OnChainProposer.verifyBatch()를 pass-through하므로 함께 수정
- ITimelock.sol도 동일하게 수정
