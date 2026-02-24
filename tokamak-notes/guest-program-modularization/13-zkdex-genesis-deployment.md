# ZK-DEX L2 제네시스 배포 계획

## 배경

ZkDex 컨트랙트와 Groth16 verifier를 L2 제네시스 블록에 포함시켜,
Block 0부터 DEX 기능을 사용 가능하게 하고 SP1 Prover를 즉시 시작할 수 있도록 한다.

### 현재 문제

1. **verifier 컨트랙트 부재**: `contracts/verifiers/*.sol`이 gitignore되어 있음.
   Circom 회로에서 생성해야 함.
2. **배포 순서 의존성**: ZkDex → verifier 주소 필요 → 배포 후에야 주소 확정.
   Prover는 모든 배포 완료 후 시작해야 함.
3. **DEX_CONTRACT_ADDRESS 하드코딩**: SP1 게스트 프로그램에
   `H160([0xDE; 20])`로 컴파일 타임 상수. 배포 주소와 일치해야 함.
4. **chainId 제한**: `ZkDaiBase` constructor가 `development=true` 시
   chainId 1337/31337만 허용. L2 chainId(65602535)에서는 `development=false`
   (진짜 Groth16 검증) 필수.

### 제네시스 접근법의 장점

- 컨트랙트 주소가 제네시스에서 확정 → `DEX_CONTRACT_ADDRESS` 하드코딩 가능
- 별도 배포 단계 불필요 → Docker 초기화 단순화
- Prover가 첫 트랜잭션부터 증명 가능 (배포 완료 대기 불필요)
- SP1 ZK-DEX 전용 제네시스 파일로 분리

---

## 제네시스에 포함할 컨트랙트

| # | 컨트랙트 | 역할 | 소스 |
|---|----------|------|------|
| 1 | MintBurnNoteVerifier | mint/liquidate Groth16 검증 | Circom `mint_burn_note.circom` |
| 2 | TransferNoteVerifier | spend Groth16 검증 | Circom `transfer_note.circom` |
| 3 | ConvertNoteVerifier | convertNote Groth16 검증 | Circom `convert_note.circom` |
| 4 | MakeOrderVerifier | makeOrder Groth16 검증 | Circom `make_order.circom` |
| 5 | TakeOrderVerifier | takeOrder Groth16 검증 | Circom `take_order.circom` |
| 6 | SettleOrderVerifier | settleOrder Groth16 검증 | Circom `settle_order.circom` |
| 7 | MockDai | ERC20 테스트 토큰 | `contracts/test/MockDai.sol` (현재 미사용, ETH only) |
| 8 | ZkDex | 메인 DEX 컨트랙트 | `contracts/ZkDex.sol` |

---

## 작업 순서

### Step 1: Circom 회로 컴파일 (1회, 오프라인)

Circom 회로 소스: `/Users/zena/tokamak-projects/zk-dex/circuits-circom/main/`

```bash
# 각 회로에 대해 (예: mint_burn_note)
circom circuits-circom/main/mint_burn_note.circom --r1cs --wasm -o build/

# Powers of Tau 다운로드 (회로 크기에 맞는 ptau)
# https://github.com/iden3/snarkjs#7-prepare-phase-2

# Groth16 setup
snarkjs groth16 setup build/mint_burn_note.r1cs pot_final.ptau build/mint_burn_note_0000.zkey

# (선택) contribution
snarkjs zkey contribute build/mint_burn_note_0000.zkey build/mint_burn_note_final.zkey

# Solidity verifier 추출
snarkjs zkey export solidityverifier build/mint_burn_note_final.zkey contracts/verifiers/MintBurnNoteVerifier.sol
```

6개 회로 모두 반복:
- `mint_burn_note.circom` → `MintBurnNoteVerifier.sol`
- `transfer_note.circom` → `TransferNoteVerifier.sol`
- `convert_note.circom` → `ConvertNoteVerifier.sol`
- `make_order.circom` → `MakeOrderVerifier.sol`
- `take_order.circom` → `TakeOrderVerifier.sol`
- `settle_order.circom` → `SettleOrderVerifier.sol`

도구 버전: `circom 2.1.9`, `snarkjs 0.7.6`

### Step 2: 전체 컨트랙트 컴파일 → bytecode 추출

```bash
cd /Users/zena/tokamak-projects/zk-dex

# Foundry로 컴파일 (foundry.toml 설정 필요)
forge build

# 또는 Truffle
npm install
truffle compile
```

각 컨트랙트의 **deployed bytecode** 추출 (creation bytecode가 아닌 runtime bytecode).

### Step 3: L2 제네시스 JSON에 컨트랙트 계정 추가

SP1 ZK-DEX 전용 제네시스 파일 생성 (기존 L2 제네시스와 별도):

```
crates/l2/fixtures/genesis/l2-zk-dex.json
```

제네시스 JSON의 `alloc` 섹션에 컨트랙트 추가:

```json
{
  "alloc": {
    "0x<VERIFIER_1_ADDRESS>": {
      "code": "0x<MintBurnNoteVerifier deployed bytecode>",
      "balance": "0x0"
    },
    "0x<VERIFIER_2_ADDRESS>": {
      "code": "0x<TransferNoteVerifier deployed bytecode>",
      "balance": "0x0"
    },
    ...
    "0x<ZKDEX_ADDRESS>": {
      "code": "0x<ZkDex deployed bytecode>",
      "balance": "0x0",
      "storage": {
        "0x0": "<development(false) + dai(MockDai주소) packed>",
        "0x1": "<requestVerifier = MintBurnNoteVerifier 주소>",
        "0x7": "<mintNoteVerifier 주소>",
        "0x8": "<spendNoteVerifier 주소>",
        "0x9": "<liquidateNoteVerifier 주소>",
        "0xa": "<convertNoteVerifier 주소>",
        "0xb": "<makeOrderVerifier 주소>",
        "0xc": "<takeOrderVerifier 주소>",
        "0xd": "<settleOrderVerifier 주소>",
        "0xe": "0x0"
      }
    }
  }
}
```

### Step 4: DEX_CONTRACT_ADDRESS 확정

제네시스에서 ZkDex에 할당한 주소를 게스트 프로그램에 반영:

```rust
// crates/guest-program/src/programs/zk_dex/mod.rs:10
const DEX_CONTRACT_ADDRESS: Address = H160([...]); // ← 제네시스 주소와 일치
```

현재: `H160([0xDE; 20])` = `0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE`
→ 제네시스에서 이 주소를 그대로 사용하면 코드 변경 불필요.

### Step 5: Docker 파이프라인 수정

`zk-dex-docker.sh` 및 `docker-compose-zk-dex.overrides.yaml` 수정:

```
현재: L1 시작 → L1 컨트랙트 배포 → L2 시작 → (Prover)
변경: L1 시작 → L1 컨트랙트 배포 → L2 시작 (ZK-DEX 제네시스 사용) → Prover 시작
```

L2 시작 시 `--network` 옵션으로 ZK-DEX 제네시스 파일 지정:
```
--network /genesis/l2-zk-dex.json
```

### Step 6: E2E 검증

1. Docker 환경 시작
2. ZkDex 함수 호출 (mint, spend, liquidate, convertNote, makeOrder, takeOrder, settleOrder)
3. 배치 커밋 확인
4. SP1 증명 생성 확인
5. L1 검증 확인

---

## ZkDex Storage Layout (제네시스 세팅용)

ZkDex 상속 체인: `ZkDaiBase → MintNotes → SpendNotes → LiquidateNotes → ZkDai → ZkDex`

| Slot | 변수 | 값 |
|------|------|-----|
| 0 | `development` (bool) + `dai` (address) | `0x00..00` + `<MockDai주소>` (packed) |
| 1 | `requestVerifier` | MintBurnNoteVerifier 주소 |
| 2 | (gap) | 0 |
| 3 | `encryptedNotes` mapping | 비어있음 (mapping base) |
| 4 | `notes` mapping | 비어있음 (mapping base) |
| 5 | `requestedNoteProofs` mapping | 비어있음 |
| 6 | `verifiedProofs` mapping | 비어있음 |
| 7 | `mintNoteVerifier` | MintBurnNoteVerifier 주소 |
| 8 | `spendNoteVerifier` | TransferNoteVerifier 주소 |
| 9 | `liquidateNoteVerifier` | MintBurnNoteVerifier 주소 |
| 10 | `convertNoteVerifier` | ConvertNoteVerifier 주소 |
| 11 | `makeOrderVerifier` | MakeOrderVerifier 주소 |
| 12 | `takeOrderVerifier` | TakeOrderVerifier 주소 |
| 13 | `settleOrderVerifier` | SettleOrderVerifier 주소 |
| 14 | `orders.length` | 0 |

> `forge inspect ZkDex storage-layout`으로 슬롯 번호 확인 필수.

---

## 참고 파일

- Circom 회로: `/Users/zena/tokamak-projects/zk-dex/circuits-circom/main/`
- ZkDex 컨트랙트: `/Users/zena/tokamak-projects/zk-dex/contracts/`
- SP1 게스트 프로그램: `crates/guest-program/src/programs/zk_dex/`
- Docker 스크립트: `crates/l2/scripts/zk-dex-docker.sh`
- 현재 L2 제네시스: `crates/l2/fixtures/genesis/`
- 도구: `circom 2.1.9`, `snarkjs 0.7.6`, `forge 1.5.1`
