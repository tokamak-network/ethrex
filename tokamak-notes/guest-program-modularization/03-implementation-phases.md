# 구현 단계 계획

이 문서는 Guest Program 모듈화 구현을 4개 하위 단계(Phase 2.1-2.4)로 나누어 상세하게 기술한다.
각 단계는 독립적으로 테스트 가능한 마일스톤을 가진다.

## Phase 2.1: 코어 추상화 (2-3주)

### 목표
- `GuestProgram` 트레이트를 정의한다.
- 기존 EVM-L2 코드를 `EvmL2GuestProgram`으로 래핑한다.
- `ProverBackend`에 bytes 기반 메서드를 추가한다.
- SP1/RISC0 백엔드를 새 메서드를 지원하도록 수정한다.
- **모든 기존 테스트가 통과해야 한다 (zero behavior change).**

### 작업 항목

#### 2.1.1 GuestProgram 트레이트 정의

**파일**: `crates/guest-program/src/traits.rs` (신규)

```rust
pub trait GuestProgram: Send + Sync {
    fn program_id(&self) -> &str;
    fn elf(&self, backend: BackendType) -> Option<&[u8]>;
    fn vk_bytes(&self, backend: BackendType) -> Option<Vec<u8>>;
    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError>;
    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError>;
    fn program_type_id(&self) -> u8;
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/guest-program/src/traits.rs` | **신규** — 트레이트 정의 |
| `crates/guest-program/src/lib.rs` | `pub mod traits;` 추가, `GuestProgram` re-export |

검증:
- [ ] 트레이트가 컴파일된다
- [ ] `BackendType` 의존성이 순환하지 않는다 (필요 시 별도 공유 크레이트로 분리)

> **주의**: `BackendType`은 현재 `crates/l2/prover/src/backend/mod.rs`에 정의되어 있다. `GuestProgram` 트레이트가 이를 참조하면 `guest-program` → `l2-prover` 의존성이 생겨 순환이 발생한다. 해결 방안:
> - **Option A**: `BackendType`을 `crates/l2/common/src/prover.rs`로 이동 (guest-program은 이미 l2-common에 의존하지 않으므로, 별도 공유 크레이트 필요)
> - **Option B**: `GuestProgram` 트레이트에서 `BackendType` 대신 `&str` (예: `"sp1"`, `"risc0"`)을 사용
> - **Option C**: `BackendType`을 `crates/guest-program/src/` 내부에서 재정의 (중복이지만 의존성 없음)
> - **추천**: Option A — `BackendType`을 `ethrex-common` 같은 공유 크레이트로 이동

#### 2.1.2 EvmL2GuestProgram 구현

**파일**: `crates/guest-program/src/programs/evm_l2.rs` (신규)

기존 ELF 정적 상수를 래핑:

```rust
pub struct EvmL2GuestProgram;

impl GuestProgram for EvmL2GuestProgram {
    fn program_id(&self) -> &str { "evm-l2" }

    fn elf(&self, backend: BackendType) -> Option<&[u8]> {
        match backend {
            BackendType::SP1 => Some(crate::ZKVM_SP1_PROGRAM_ELF),
            BackendType::RISC0 => Some(crate::methods::ETHREX_GUEST_RISC0_ELF),
            BackendType::ZisK => Some(crate::ZKVM_ZISK_PROGRAM_ELF),
            _ => None,
        }
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_input.to_vec())  // 기존과 동일: 이미 rkyv 직렬화된 데이터
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())  // zkVM이 반환한 public values 그대로
    }

    fn program_type_id(&self) -> u8 { 1 }
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/guest-program/src/programs/mod.rs` | **신규** — `pub mod evm_l2;` |
| `crates/guest-program/src/programs/evm_l2.rs` | **신규** — EvmL2GuestProgram |
| `crates/guest-program/src/lib.rs` | `pub mod programs;` 추가 |

검증:
- [ ] `EvmL2GuestProgram`이 `GuestProgram` 트레이트를 구현한다
- [ ] `elf()` 반환 값이 기존 `ZKVM_SP1_PROGRAM_ELF` 등과 동일하다

#### 2.1.3 ProverBackend에 bytes 기반 메서드 추가

**파일**: `crates/l2/prover/src/backend/mod.rs` (수정)

기존 `prove()` 메서드를 유지하면서 `prove_with_elf()` 추가:

```rust
pub trait ProverBackend {
    // ... 기존 메서드 유지 ...

    fn backend_type(&self) -> BackendType;

    fn execute_with_elf(&self, elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError> {
        Err(BackendError::not_implemented("execute_with_elf"))
    }

    fn prove_with_elf(&self, elf: &[u8], serialized_input: &[u8], format: ProofFormat)
        -> Result<Self::ProofOutput, BackendError>
    {
        Err(BackendError::not_implemented("prove_with_elf"))
    }

    fn verify_with_vk(&self, proof: &Self::ProofOutput, vk: &[u8]) -> Result<(), BackendError> {
        Err(BackendError::not_implemented("verify_with_vk"))
    }
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/mod.rs` | `backend_type()`, `prove_with_elf()`, `execute_with_elf()`, `verify_with_vk()` 추가 |
| `crates/l2/prover/src/backend/error.rs` | `BackendError::NotImplemented` 변형 추가 |

검증:
- [ ] 기존 백엔드가 새 메서드의 기본 구현으로 컴파일된다
- [ ] 기존 `prove()` 호출이 변경 없이 동작한다

#### 2.1.4 SP1 백엔드 수정

**파일**: `crates/l2/prover/src/backend/sp1.rs` (수정)

```rust
impl ProverBackend for Sp1Backend {
    fn backend_type(&self) -> BackendType { BackendType::SP1 }

    fn execute_with_elf(&self, elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError> {
        let mut stdin = SP1Stdin::new();
        stdin.write_slice(serialized_input);
        let client = self.get_or_init_client();
        client.execute(elf, &stdin).map_err(BackendError::execution)?;
        Ok(())
    }

    fn prove_with_elf(&self, elf: &[u8], serialized_input: &[u8], format: ProofFormat)
        -> Result<Self::ProofOutput, BackendError>
    {
        let mut stdin = SP1Stdin::new();
        stdin.write_slice(serialized_input);
        let client = self.get_or_init_client();
        let (pk, vk) = client.setup(elf);
        let sp1_format = Self::convert_format(format);
        let proof = client.prove(&pk, &stdin, sp1_format).map_err(BackendError::proving)?;
        Ok(Sp1ProveOutput::new(proof, vk))
    }

    // 기존 메서드 유지 (변경 없음)
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/sp1.rs` | `backend_type()`, `execute_with_elf()`, `prove_with_elf()` 구현 |

검증:
- [ ] 기존 `prove()` + `execute()` 테스트 통과
- [ ] `prove_with_elf()` 단위 테스트: 기존 ELF를 명시적으로 전달하여 동일 결과 확인

#### 2.1.5 RISC0 백엔드 수정

**파일**: `crates/l2/prover/src/backend/risc0.rs` (수정)

```rust
impl ProverBackend for Risc0Backend {
    fn backend_type(&self) -> BackendType { BackendType::RISC0 }

    fn execute_with_elf(&self, elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError> {
        let env = ExecutorEnv::builder()
            .write_slice(serialized_input)
            .build()
            .map_err(BackendError::execution)?;
        let executor = default_executor();
        executor.execute(env, elf).map_err(BackendError::execution)?;
        Ok(())
    }

    fn prove_with_elf(&self, elf: &[u8], serialized_input: &[u8], format: ProofFormat)
        -> Result<Self::ProofOutput, BackendError>
    {
        let env = ExecutorEnv::builder()
            .write_slice(serialized_input)
            .build()
            .map_err(BackendError::execution)?;
        let prover = default_prover();
        let prover_opts = Self::convert_format(format);
        let prove_info = prover.prove_with_opts(env, elf, &prover_opts)
            .map_err(BackendError::proving)?;
        Ok(prove_info.receipt)
    }
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/backend/risc0.rs` | `backend_type()`, `execute_with_elf()`, `prove_with_elf()` 구현 |

검증:
- [ ] 기존 RISC0 테스트 통과
- [ ] `prove_with_elf()` 단위 테스트

#### 2.1.6 Exec 백엔드 수정

**파일**: `crates/l2/prover/src/backend/exec.rs` (수정)

Exec 백엔드는 ELF를 사용하지 않고 직접 실행하므로 특수 처리:

```rust
impl ProverBackend for ExecBackend {
    fn backend_type(&self) -> BackendType { BackendType::Exec }

    fn execute_with_elf(&self, _elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError> {
        // Exec 모드에서는 ELF를 무시하고 직접 실행
        let input: ProgramInput = rkyv::from_bytes(serialized_input)
            .map_err(BackendError::serialization)?;
        Self::execute_core(input)?;
        Ok(())
    }
}
```

### Phase 2.1 완료 기준

- [ ] `GuestProgram` 트레이트가 정의되고 `EvmL2GuestProgram`이 구현됨
- [ ] `ProverBackend`에 `prove_with_elf()` 등 새 메서드가 추가됨
- [ ] SP1, RISC0, Exec 백엔드가 새 메서드를 구현
- [ ] **모든 기존 테스트 통과** (cargo test)
- [ ] 새 메서드 단위 테스트 추가

---

## Phase 2.2: 레지스트리 & 멀티프로그램 (2-3주)

### 목표
- `GuestProgramRegistry` 구현
- `ProofData` 프로토콜에 `program_id` 추가
- Proof Coordinator가 프로그램별 배치 할당
- Transfer Circuit Guest Program 레퍼런스 구현

### 작업 항목

#### 2.2.1 GuestProgramRegistry 구현

**파일**: `crates/l2/prover/src/registry.rs` (신규)

```rust
pub struct GuestProgramRegistry {
    programs: HashMap<String, Arc<dyn GuestProgram>>,
    default_program_id: String,
}

impl GuestProgramRegistry {
    pub fn new(default_program_id: &str) -> Self { ... }
    pub fn register(&mut self, program: Arc<dyn GuestProgram>) { ... }
    pub fn get(&self, program_id: &str) -> Option<&Arc<dyn GuestProgram>> { ... }
    pub fn default_program(&self) -> Option<&Arc<dyn GuestProgram>> { ... }
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/registry.rs` | **신규** — GuestProgramRegistry |
| `crates/l2/prover/src/lib.rs` | `pub mod registry;` 추가 |

#### 2.2.2 ProofData 프로토콜 확장

**파일**: `crates/l2/common/src/prover.rs` (수정)

```rust
pub enum ProofData {
    BatchRequest {
        commit_hash: String,
        prover_type: ProverType,
        #[serde(default)]
        supported_programs: Vec<String>,
    },
    BatchResponse {
        batch_number: Option<u64>,
        input: Option<ProverInputData>,
        format: Option<ProofFormat>,
        #[serde(default)]
        program_id: Option<String>,
    },
    ProofSubmit {
        batch_number: u64,
        batch_proof: BatchProof,
        #[serde(default = "default_program_id")]
        program_id: String,
    },
    // ...
}

fn default_program_id() -> String {
    "evm-l2".to_string()
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/common/src/prover.rs` | `ProofData` 변형에 `program_id` 관련 필드 추가 |

하위 호환성:
- `#[serde(default)]`를 사용하여 이전 프루버가 새 필드 없이 통신 가능
- `program_id`가 None이면 기본값 `"evm-l2"` 사용

#### 2.2.3 Proof Coordinator 수정

**파일**: `crates/l2/sequencer/proof_coordinator.rs` (수정)

```rust
async fn handle_request(
    &self,
    stream: &mut TcpStream,
    commit_hash: String,
    prover_type: ProverType,
    supported_programs: Vec<String>,  // 추가
) -> Result<(), ProofCoordinatorError> {
    // 배치의 program_id 결정 (현재는 항상 "evm-l2")
    let program_id = self.determine_program_for_batch(batch_to_prove).await?;

    // 프루버가 해당 프로그램을 지원하는지 확인
    if !supported_programs.is_empty() && !supported_programs.contains(&program_id) {
        // 이 프루버는 해당 프로그램을 지원하지 않음
        send_response(stream, &ProofData::empty_batch_response()).await?;
        return Ok(());
    }

    let response = ProofData::BatchResponse {
        batch_number: Some(batch_to_prove),
        input: Some(input),
        format: Some(format),
        program_id: Some(program_id),
    };
    send_response(stream, &response).await?;
    Ok(())
}

/// 배치에 사용할 Guest Program 결정.
/// 현재는 항상 "evm-l2". 향후 배치별 프로그램 매핑 추가.
async fn determine_program_for_batch(&self, _batch_number: u64) -> Result<String, ProofCoordinatorError> {
    Ok("evm-l2".to_string())
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/sequencer/proof_coordinator.rs` | `handle_request()` 시그니처 및 로직 확장, `handle_connection()` 파싱 수정 |

#### 2.2.4 Prover 메인 루프 수정

**파일**: `crates/l2/prover/src/prover.rs` (수정)

```rust
struct Prover<B: ProverBackend> {
    backend: B,
    registry: GuestProgramRegistry,  // 추가
    // ...
}

impl<B: ProverBackend> Prover<B> {
    pub fn new(backend: B, cfg: &ProverConfig, registry: GuestProgramRegistry) -> Self {
        Self { backend, registry, ... }
    }

    async fn start(&self) {
        loop {
            // 기존 루프 유지, prove_batch() 호출 시 registry 사용
        }
    }
}
```

`request_new_input()` 수정:
```rust
async fn request_new_input(&self, endpoint: &Url) -> Result<InputRequest, String> {
    let request = ProofData::BatchRequest {
        commit_hash: self.commit_hash.clone(),
        prover_type: self.backend.prover_type(),
        supported_programs: self.registry.program_ids().iter().map(|s| s.to_string()).collect(),
    };
    // ... 기존 응답 처리 ...
    // program_id를 ProverData에 저장
}
```

`start()` 내부 증명 생성 수정:
```rust
// program_id에 따라 GuestProgram 조회
let program_id = prover_data.program_id.as_deref().unwrap_or("evm-l2");
let program = self.registry.get(program_id).ok_or("Unknown program")?;

let elf = program.elf(self.backend.backend_type()).ok_or("No ELF for backend")?;
let serialized = program.serialize_input(&raw_input_bytes)?;

let batch_proof = self.backend.prove_with_elf(elf, &serialized, prover_data.format)
    .and_then(|output| self.backend.to_batch_proof(output, prover_data.format))?;
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/prover/src/prover.rs` | `Prover` 구조체에 `registry` 추가, `start_prover()` 수정, 증명 생성 로직 수정 |

#### 2.2.5 Transfer Circuit 레퍼런스 구현

**디렉토리**: `crates/guest-program/programs/transfer/` (신규)

단순 잔액 이동만 증명하는 최소 Guest Program:

```rust
// crates/guest-program/programs/transfer/src/main.rs

/// Transfer Circuit: EVM 없이 잔액 이동만 증명
///
/// 입력: 발신자, 수신자, 금액, 서명, 현재 잔액
/// 검증: 서명 유효성, 잔액 충분성
/// 출력: 이전 상태 루트, 새 상태 루트
fn main() {
    // zkVM 엔트리포인트
    let input: TransferInput = read_input();

    // 서명 검증
    verify_signature(&input.sender, &input.signature, &input.tx_hash)?;

    // 잔액 검증
    assert!(input.sender_balance >= input.amount);

    // 상태 전이
    let new_sender_balance = input.sender_balance - input.amount;
    let new_receiver_balance = input.receiver_balance + input.amount;

    // 상태 루트 계산 (간소화된 Merkle tree)
    let new_state_root = compute_state_root(/* ... */);

    commit_output(TransferOutput {
        initial_state_root: input.state_root,
        final_state_root: new_state_root,
    });
}
```

`TransferGuestProgram` 구현:

```rust
// crates/guest-program/src/programs/transfer.rs (신규)

pub struct TransferGuestProgram;

impl GuestProgram for TransferGuestProgram {
    fn program_id(&self) -> &str { "transfer" }
    fn program_type_id(&self) -> u8 { 2 }

    fn elf(&self, backend: BackendType) -> Option<&[u8]> {
        // 빌드된 Transfer ELF 참조
        match backend {
            #[cfg(feature = "sp1")]
            BackendType::SP1 => Some(include_bytes!("../../bin/sp1/transfer/out/transfer-sp1-elf")),
            _ => None,
        }
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // Transfer 전용 입력 직렬화
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // Transfer 전용 출력 인코딩 (간소화)
        Ok(raw_output.to_vec())
    }
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/guest-program/programs/transfer/` | **신규 디렉토리** — Transfer Circuit 크레이트 |
| `crates/guest-program/src/programs/transfer.rs` | **신규** — TransferGuestProgram |
| `crates/guest-program/src/programs/mod.rs` | `pub mod transfer;` 추가 |
| `crates/guest-program/build.rs` | Transfer 프로그램 빌드 추가 |

### Phase 2.2 완료 기준

- [ ] `GuestProgramRegistry`가 동작하고 `EvmL2GuestProgram`이 등록됨
- [ ] `ProofData` 프로토콜에 `program_id`가 추가됨 (하위 호환)
- [ ] Proof Coordinator가 `program_id`를 포함한 응답을 전송
- [ ] Prover가 레지스트리를 통해 ELF를 조회하여 증명 생성
- [ ] Transfer Circuit 레퍼런스가 SP1에서 컴파일/실행됨
- [ ] 기존 EVM-L2 흐름이 변경 없이 동작 (end-to-end 테스트)

---

## Phase 2.3: L1 컨트랙트 & 검증 (2-3주)

### 목표
- `OnChainProposer.sol`의 VK 매핑에 `programTypeId` 추가
- `verifyBatch()` 프로그램 타입별 검증 디스패치
- 배포 스크립트 멀티프로그램 VK 지원
- `l1_committer.rs`와 `l1_proof_sender.rs` 수정

### 작업 항목

#### 2.3.1 OnChainProposer.sol 수정

**파일**: `crates/l2/contracts/src/l1/OnChainProposer.sol` (수정)

VK 매핑 확장:
```solidity
// 기존:
// mapping(bytes32 => mapping(uint8 => bytes32)) public verificationKeys;

// 변경:
mapping(bytes32 commitHash => mapping(uint8 programTypeId => mapping(uint8 verifierId => bytes32 vk)))
    public verificationKeys;
```

`BatchCommitmentInfo` 확장:
```solidity
struct BatchCommitmentInfo {
    // ... 기존 필드 ...
    uint8 programTypeId;  // 추가
}
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | VK 매핑 3차원, BatchCommitmentInfo에 programTypeId, commitBatch/verifyBatch 수정, VK 업그레이드 함수 수정 |
| `crates/l2/contracts/src/l1/interfaces/IOnChainProposer.sol` | 인터페이스 시그니처 업데이트 |

#### 2.3.2 VK 마이그레이션 함수

```solidity
/// 기존 VK 데이터를 새 3차원 매핑으로 마이그레이션.
/// 기존 VK는 programTypeId=1 (EVM-L2)로 이전.
function migrateVerificationKeys(
    bytes32[] calldata commitHashes
) external onlyOwner {
    uint8 DEFAULT_PROGRAM_TYPE = 1; // EVM-L2
    for (uint256 i = 0; i < commitHashes.length; i++) {
        bytes32 ch = commitHashes[i];
        // 이전 슬롯에서 읽어 새 슬롯에 쓰기
        // 주의: 스토리지 슬롯 계산이 달라지므로 실제 구현 시 검증 필요
        verificationKeys[ch][DEFAULT_PROGRAM_TYPE][SP1_VERIFIER_ID] = oldVk_sp1;
        verificationKeys[ch][DEFAULT_PROGRAM_TYPE][RISC0_VERIFIER_ID] = oldVk_risc0;
    }
}
```

#### 2.3.3 upgradeVerificationKey 범용화

```solidity
function upgradeVerificationKey(
    bytes32 commit_hash,
    uint8 programTypeId,
    uint8 verifierId,
    bytes32 new_vk
) public onlyOwner {
    require(commit_hash != bytes32(0), "OnChainProposer: commit hash is zero");
    require(programTypeId > 0, "OnChainProposer: invalid program type");
    require(verifierId > 0 && verifierId <= 2, "OnChainProposer: invalid verifier ID");
    verificationKeys[commit_hash][programTypeId][verifierId] = new_vk;
    emit VerificationKeyUpgraded(programTypeId, verifierId, commit_hash, new_vk);
}
```

#### 2.3.4 l1_committer.rs 수정

`commitBatch()` 호출에 `programTypeId` 추가.

**파일**: `crates/l2/sequencer/l1_committer.rs` (수정)

```rust
// commitBatch 호출 시 program_type_id 추가
let calldata_values = [
    // ... 기존 필드 ...
    &[Value::Uint(U256::from(program_type_id as u64))],  // 추가
].concat();
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/sequencer/l1_committer.rs` | `commitBatch()` calldata에 `programTypeId` 추가 |

#### 2.3.5 l1_proof_sender.rs 수정

`verifyBatch()` 호출은 시그니처가 변경되지 않으므로 수정 불필요.
`_getPublicInputsFromCommitment()`이 내부적으로 `programTypeId`를 사용하므로 L1에서 자동 처리.

단, 배치별 `program_type_id` 결정 로직 필요:

```rust
// 배치의 program_type_id를 store에서 조회
let program_type_id = self.rollup_store
    .get_program_type_for_batch(batch_to_send)
    .await?
    .unwrap_or(1); // 기본값: EVM-L2
```

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| `crates/l2/sequencer/l1_proof_sender.rs` | 배치별 program_type_id 조회 로직 (있으면) |

#### 2.3.6 배포 스크립트 수정

VK 등록 시 `programTypeId` 포함:

변경 파일:
| 파일 | 변경 내용 |
|------|----------|
| 배포 스크립트 (Rust/JS) | `initialize()` 호출 시 `programTypeId` 파라미터 추가 |
| 테스트 스크립트 | VK 등록 테스트 업데이트 |

### Phase 2.3 완료 기준

- [ ] `OnChainProposer.sol`이 3차원 VK 매핑을 사용
- [ ] `commitBatch()`가 `programTypeId`를 받고 저장
- [ ] `verifyBatch()`가 `programTypeId`에 따라 VK를 조회하고 public inputs를 재구성
- [ ] 기존 VK 마이그레이션 함수 동작
- [ ] `l1_committer`가 `programTypeId`를 전송
- [ ] Solidity 단위 테스트 + 통합 테스트 통과
- [ ] 기존 EVM-L2 배치가 `programTypeId=1`로 정상 동작

---

## Phase 2.4: SDK & 개발자 도구 (1-2주)

### 목표
- Guest Program 개발 템플릿
- 빌드 도구
- 개발자 문서

### 작업 항목

#### 2.4.1 Guest Program 템플릿

`cargo-generate` 또는 간단한 스크립트로 새 Guest Program 크레이트를 생성:

```bash
# 사용법
./scripts/new-guest-program.sh my-app

# 생성되는 구조
crates/guest-program/programs/my-app/
├── Cargo.toml
└── src/
    ├── main.rs          # zkVM 엔트리포인트 템플릿
    ├── input.rs         # 입력 타입 정의
    └── output.rs        # 출력 타입 정의
```

#### 2.4.2 빌드 도구

```bash
# 특정 Guest Program만 빌드
GUEST_PROGRAMS=transfer cargo build --features sp1

# 모든 Guest Program 빌드
GUEST_PROGRAMS=evm-l2,transfer cargo build --features sp1

# 기본 (evm-l2만)
cargo build --features sp1
```

#### 2.4.3 개발자 문서

`docs/l2/guest-program-development.md`:
- Guest Program 개요
- GuestProgram 트레이트 설명
- 새 Guest Program 생성 가이드
- 빌드 및 테스트 방법
- L1 검증 설정 방법

### Phase 2.4 완료 기준

- [ ] Guest Program 템플릿 스크립트 동작
- [ ] 빌드 환경변수로 멀티프로그램 빌드 가능
- [ ] 개발자 가이드 문서 작성

---

## 전체 파일 변경 요약

| 파일 | Phase | 변경 유형 |
|------|-------|----------|
| `crates/guest-program/src/traits.rs` | 2.1 | **신규** |
| `crates/guest-program/src/programs/mod.rs` | 2.1 | **신규** |
| `crates/guest-program/src/programs/evm_l2.rs` | 2.1 | **신규** |
| `crates/guest-program/src/lib.rs` | 2.1 | 수정 |
| `crates/l2/prover/src/backend/mod.rs` | 2.1 | 수정 |
| `crates/l2/prover/src/backend/sp1.rs` | 2.1 | 수정 |
| `crates/l2/prover/src/backend/risc0.rs` | 2.1 | 수정 |
| `crates/l2/prover/src/backend/exec.rs` | 2.1 | 수정 |
| `crates/l2/prover/src/backend/error.rs` | 2.1 | 수정 |
| `crates/l2/prover/src/registry.rs` | 2.2 | **신규** |
| `crates/l2/common/src/prover.rs` | 2.2 | 수정 |
| `crates/l2/prover/src/prover.rs` | 2.2 | 수정 |
| `crates/l2/sequencer/proof_coordinator.rs` | 2.2 | 수정 |
| `crates/guest-program/programs/transfer/` | 2.2 | **신규 디렉토리** |
| `crates/guest-program/src/programs/transfer.rs` | 2.2 | **신규** |
| `crates/guest-program/build.rs` | 2.2 | 수정 |
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | 2.3 | 수정 |
| `crates/l2/contracts/src/l1/interfaces/IOnChainProposer.sol` | 2.3 | 수정 |
| `crates/l2/sequencer/l1_committer.rs` | 2.3 | 수정 |
| `crates/l2/sequencer/l1_proof_sender.rs` | 2.3 | 수정 (가능) |
| `scripts/new-guest-program.sh` | 2.4 | **신규** |
| `docs/l2/guest-program-development.md` | 2.4 | **신규** |

## 의존성 관계

```
Phase 2.1 ──▶ Phase 2.2 ──▶ Phase 2.3 ──▶ Phase 2.4
(코어 추상화)  (레지스트리)   (L1 컨트랙트)  (SDK)
   │              │              │
   │              │              └─ 2.3은 2.2 없이도 부분 진행 가능
   │              │                 (VK 매핑 확장은 독립적)
   │              │
   │              └─ 2.2는 반드시 2.1 완료 후
   │
   └─ 2.1은 독립적으로 시작 가능
```

- Phase 2.1 → 2.2: 트레이트가 정의되어야 레지스트리 구현 가능
- Phase 2.2 → 2.3: L1 변경은 프로토콜 확장 후 진행
- Phase 2.3 ↔ 2.4: 독립적 (병행 가능)
