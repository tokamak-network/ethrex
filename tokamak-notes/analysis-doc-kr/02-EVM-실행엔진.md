# Ethrex 프로젝트 분석 - EVM 실행 엔진 (LEVM)

## 1. LEVM 개요

**LEVM (Lambda EVM)**은 ethrex의 자체 구현 이더리움 가상 머신이다. 순수 Rust로 작성되었으며, 정확성, 성능, 가독성, 확장성을 설계 목표로 한다.

- **위치**: `crates/vm/levm/src/`
- **특징**: 포스트-머지(Post-Merge) 전용, L1/L2 모드 지원

## 2. 핵심 아키텍처

### 2.1 VM 구조

```
VM
├── Stack (1024 항목 제한)
├── Memory (확장 가능, 32바이트 정렬)
├── CallFrame (호출별 실행 컨텍스트)
├── Substate (트랜잭션 수준 상태 변경)
└── Environment (블록/트랜잭션 컨텍스트)
```

### 2.2 서브스테이트 (Substate)

```rust
Substate {
    selfdestruct_set: FxHashSet<Address>,                      // 삭제 대상 계정
    accessed_addresses: FxHashSet<Address>,                    // 워밍된 주소 (EIP-2929)
    accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>>, // 워밍된 슬롯
    created_accounts: FxHashSet<Address>,                      // 생성된 계정
    refunded_gas: u64,                                         // 가스 환불
    transient_storage: TransientStorage,                       // EIP-1153 (트랜잭션당 초기화)
    logs: Vec<Log>,                                            // 이벤트 로그
}
```

### 2.3 VM 타입

```rust
enum VMType {
    L1,                  // 표준 이더리움 L1
    L2(FeeConfig),       // L2 롤업 (추가 수수료 처리)
}
```

## 3. 옵코드 지원

### 3.1 전체 옵코드 목록

#### 정지 및 산술 연산 (0x00-0x0B)
| 옵코드 | 이름 | 가스 | 설명 |
|--------|------|------|------|
| 0x00 | STOP | 0 | 실행 중지 |
| 0x01 | ADD | 3 | 덧셈 |
| 0x02 | MUL | 5 | 곱셈 |
| 0x03 | SUB | 3 | 뺄셈 |
| 0x04 | DIV | 5 | 부호 없는 나눗셈 |
| 0x05 | SDIV | 5 | 부호 있는 나눗셈 |
| 0x06 | MOD | 5 | 나머지 |
| 0x07 | SMOD | 5 | 부호 있는 나머지 |
| 0x08 | ADDMOD | 8 | 모듈러 덧셈 |
| 0x09 | MULMOD | 8 | 모듈러 곱셈 |
| 0x0A | EXP | 10+ | 거듭제곱 (동적 가스) |
| 0x0B | SIGNEXTEND | 5 | 부호 확장 |

#### 비교 및 비트 연산 (0x10-0x1D)
| 옵코드 | 이름 | 가스 | 설명 |
|--------|------|------|------|
| 0x10 | LT | 3 | 미만 비교 |
| 0x11 | GT | 3 | 초과 비교 |
| 0x12 | SLT | 3 | 부호 있는 미만 비교 |
| 0x13 | SGT | 3 | 부호 있는 초과 비교 |
| 0x14 | EQ | 3 | 동등 비교 |
| 0x15 | ISZERO | 3 | 제로 검사 |
| 0x16 | AND | 3 | 비트 AND |
| 0x17 | OR | 3 | 비트 OR |
| 0x18 | XOR | 3 | 비트 XOR |
| 0x19 | NOT | 3 | 비트 NOT |
| 0x1A | BYTE | 3 | N번째 바이트 추출 |
| 0x1B | SHL | 3 | 좌측 시프트 |
| 0x1C | SHR | 3 | 우측 논리 시프트 |
| 0x1D | SAR | 3 | 우측 산술 시프트 |

#### 암호화 (0x20)
| 옵코드 | 이름 | 가스 | 설명 |
|--------|------|------|------|
| 0x20 | KECCAK256 | 30+ | Keccak-256 해시 |

#### 환경 정보 (0x30-0x4B)
| 옵코드 | 이름 | 설명 |
|--------|------|------|
| 0x30 | ADDRESS | 현재 계정 주소 |
| 0x31 | BALANCE | 계정 잔액 조회 |
| 0x32 | ORIGIN | 트랜잭션 발신자 |
| 0x33 | CALLER | 직접 호출자 |
| 0x34 | CALLVALUE | 전송된 ETH 양 |
| 0x35 | CALLDATALOAD | 콜데이터 로드 (32바이트) |
| 0x36 | CALLDATASIZE | 콜데이터 크기 |
| 0x37 | CALLDATACOPY | 콜데이터를 메모리에 복사 |
| 0x38 | CODESIZE | 코드 크기 |
| 0x39 | CODECOPY | 코드를 메모리에 복사 |
| 0x3A | GASPRICE | 가스 가격 |
| 0x3B | EXTCODESIZE | 외부 코드 크기 |
| 0x3C | EXTCODECOPY | 외부 코드 복사 |
| 0x3D | RETURNDATASIZE | 반환 데이터 크기 |
| 0x3E | RETURNDATACOPY | 반환 데이터 복사 |
| 0x3F | EXTCODEHASH | 외부 코드 해시 |
| 0x40 | BLOCKHASH | 블록 해시 (최근 256개) |
| 0x41 | COINBASE | 블록 보상 수신자 |
| 0x42 | TIMESTAMP | 블록 타임스탬프 |
| 0x43 | NUMBER | 블록 번호 |
| 0x44 | PREVRANDAO | RANDAO 값 (포스트-머지) |
| 0x45 | GASLIMIT | 블록 가스 리밋 |
| 0x46 | CHAINID | 체인 ID |
| 0x47 | SELFBALANCE | 현재 계정 잔액 (최적화) |
| 0x48 | BASEFEE | 기본 수수료 (EIP-1559) |
| 0x49 | BLOBHASH | 블롭 해시 (EIP-4844) |
| 0x4A | BLOBBASEFEE | 블롭 기본 수수료 |
| 0x4B | SLOTNUM | 비콘 슬롯 번호 (Amsterdam) |

#### 스택/메모리/스토리지/흐름 (0x50-0x5E)
| 옵코드 | 이름 | 설명 |
|--------|------|------|
| 0x50 | POP | 스택에서 제거 |
| 0x51 | MLOAD | 메모리에서 로드 |
| 0x52 | MSTORE | 메모리에 저장 (32바이트) |
| 0x53 | MSTORE8 | 메모리에 저장 (1바이트) |
| 0x54 | SLOAD | 스토리지에서 로드 |
| 0x55 | SSTORE | 스토리지에 저장 |
| 0x56 | JUMP | 무조건 점프 |
| 0x57 | JUMPI | 조건부 점프 |
| 0x58 | PC | 프로그램 카운터 |
| 0x59 | MSIZE | 메모리 크기 |
| 0x5A | GAS | 남은 가스 |
| 0x5B | JUMPDEST | 점프 목적지 마커 |
| 0x5C | TLOAD | 트랜지언트 스토리지 로드 (EIP-1153) |
| 0x5D | TSTORE | 트랜지언트 스토리지 저장 (EIP-1153) |
| 0x5E | MCOPY | 메모리 복사 (EIP-5656) |

#### PUSH 연산 (0x5F-0x7F)
- **PUSH0** (0x5F): 0을 스택에 푸시 (EIP-3855)
- **PUSH1-PUSH32** (0x60-0x7F): 1-32바이트를 스택에 푸시

#### DUP 연산 (0x80-0x8F)
- **DUP1-DUP16**: 스택의 N번째 항목 복제

#### SWAP 연산 (0x90-0x9F)
- **SWAP1-SWAP16**: 스택 최상위와 N+1번째 항목 교환

#### 로깅 (0xA0-0xA4)
| 옵코드 | 이름 | 설명 |
|--------|------|------|
| 0xA0 | LOG0 | 토픽 0개 로그 |
| 0xA1 | LOG1 | 토픽 1개 로그 |
| 0xA2 | LOG2 | 토픽 2개 로그 |
| 0xA3 | LOG3 | 토픽 3개 로그 |
| 0xA4 | LOG4 | 토픽 4개 로그 |

#### 시스템 연산 (0xF0-0xFF)
| 옵코드 | 이름 | 가스 | 설명 |
|--------|------|------|------|
| 0xF0 | CREATE | 32000 | 컨트랙트 생성 |
| 0xF1 | CALL | 700+ | 외부 호출 |
| 0xF2 | CALLCODE | 700+ | 코드 호출 (deprecated) |
| 0xF3 | RETURN | 0 | 실행 결과 반환 |
| 0xF4 | DELEGATECALL | 700+ | 위임 호출 |
| 0xF5 | CREATE2 | 32000+ | 결정적 주소 컨트랙트 생성 |
| 0xFA | STATICCALL | 700+ | 읽기 전용 호출 |
| 0xFD | REVERT | 0 | 실행 되돌리기 |
| 0xFF | SELFDESTRUCT | 5000+ | 계정 자기 파괴 |

#### EIP-8024 새 연산 (실험적)
| 옵코드 | 이름 | 설명 |
|--------|------|------|
| 0xE6 | DUPN | 임의 위치 복제 |
| 0xE7 | SWAPN | 임의 위치 교환 |
| 0xE8 | EXCHANGE | 임의 두 위치 교환 |

### 3.2 옵코드 디스패치 전략

- **상수 배열 룩업 테이블**: 256개 항목의 컴파일 타임 배열
- **O(1) 룩업**: 바이트 값으로 직접 인덱싱
- **godbolt.org 벤치마크 기반 최적화**

## 4. 콜 프레임 관리

### 4.1 스택 구조

```rust
Stack {
    values: Box<[U256; 1024]>,  // 고정 1024 항목 버퍼
    offset: usize,               // 아래로 성장 (0=가득, 1024=비어있음)
}
```

**연산**:
- `pop<const N: usize>()`: 컴파일 타임 바운드 체크로 N개 항목 팝
- `pop1()`: 빠른 단일 항목 팝
- `push(value)`: 오버플로우 감지와 함께 푸시

### 4.2 메모리 구조

```rust
Memory {
    buffer: Rc<RefCell<Vec<u8>>>,  // 콜 프레임 간 공유
    len: usize,                     // 현재 프레임의 할당 크기
    current_base: usize,            // 현재 프레임 메모리 시작점
}
```

**특징**:
- **저비용 클론**: `Rc<RefCell>`로 중첩 호출 간 공유
- **워드 정렬**: 32바이트 청크로 확장
- **효율적 확장**: 64바이트 단위로 할당
- **프레임별 격리**: 각 콜 프레임이 고유한 `current_base` 오프셋 보유
- `next_memory()`: 부모 할당 이후 시작하는 자식 프레임 메모리 생성
- `clean_from_base()`: 반환 시 프레임 메모리 정리

### 4.3 메모리 가스 비용

```
정적 비용: 3 gas (MLOAD/MSTORE마다)
동적 확장 비용: (size² / 512) + (size * 3)
최적화: 새 확장에 대해서만 요금 부과 (재접근은 무료)
```

## 5. 실행 환경 (Environment)

### 5.1 블록 및 트랜잭션 컨텍스트

```rust
Environment {
    // 트랜잭션 정보
    origin: Address,                         // 외부 트랜잭션 발신자
    gas_limit: u64,                          // 트랜잭션 가스 리밋
    gas_price: U256,                         // 유효 가스 가격
    tx_nonce: u64,                           // 트랜잭션 논스
    tx_max_priority_fee_per_gas: Option<U256>, // EIP-1559 팁
    tx_max_fee_per_gas: Option<U256>,        // EIP-1559 최대 수수료
    tx_max_fee_per_blob_gas: Option<U256>,   // EIP-4844 블롭 수수료
    tx_blob_hashes: Vec<H256>,               // 버전드 해시

    // 블록 정보
    block_number: U256,
    coinbase: Address,                       // 블록 보상 수신자
    timestamp: U256,
    prev_randao: Option<H256>,               // RANDAO 값
    difficulty: U256,
    slot_number: U256,                       // Amsterdam+: 비콘 슬롯
    chain_id: U256,
    base_fee_per_gas: U256,                  // EIP-1559
    base_blob_fee_per_gas: U256,             // EIP-4844
    block_gas_limit: u64,
    block_excess_blob_gas: Option<U256>,
    block_blob_gas_used: Option<U256>,

    // EVM 설정
    config: EVMConfig,                       // 포크별 규칙

    // L2 전용
    is_privileged: bool,                     // 특권 트랜잭션 여부
    fee_token: Option<Address>,              // L2 수수료 토큰
}
```

### 5.2 블롭 스케줄링

| 포크 | 블록당 최대 블롭 수 |
|------|-------------------|
| Cancun | 6 |
| Prague/Electra | 8 |
| 커스텀 | EIP-7840 설정에 따름 |

## 6. 프리컴파일 (Precompiled Contracts)

### 6.1 프리컴파일 목록

#### Pre-Cancun (9개)

| 주소 | 이름 | 설명 | 가스 비용 |
|------|------|------|-----------|
| 0x01 | ECRECOVER | ECDSA 서명 복구 | 3,000 |
| 0x02 | SHA256 | SHA-256 해시 | 60 + 12×(워드) |
| 0x03 | RIPEMD160 | RIPEMD-160 해시 | 600 + 120×(워드) |
| 0x04 | IDENTITY | 메모리 복사 (항등 함수) | 15 + 3×(워드) |
| 0x05 | MODEXP | 모듈러 거듭제곱 | 동적 |
| 0x06 | ECADD | 타원곡선 덧셈 (BN254) | 150 |
| 0x07 | ECMUL | 타원곡선 곱셈 (BN254) | 6,000 |
| 0x08 | ECPAIRING | BN254 페어링 검사 | 45,000 + 34,000×(쌍) |
| 0x09 | BLAKE2F | BLAKE2b 압축 함수 | 라운드 수 |

#### Cancun (1개 추가)

| 주소 | 이름 | 설명 | 가스 비용 |
|------|------|------|-----------|
| 0x0A | KZG Point Evaluation | 블롭 검증 (EIP-4844) | 50,000 |

#### Prague (7개 추가, 총 17개)
- BLS12-381 곡선 연산 관련 프리컴파일 추가

### 6.2 구현 세부사항

- **암호화 라이브러리**: bls12_381, ark_bn254, k256, sha2, blake2b
- **포인트 인코딩**: 64바이트 (G1) 및 256바이트 (G2)
- **에러 처리**: 호출자에게 revert 이유 반환

## 7. 가스 비용 시스템

### 7.1 주요 가스 비용

#### 산술 연산
| 연산 | 정적 가스 | 동적 가스 |
|------|-----------|-----------|
| ADD/SUB | 3 | - |
| MUL/DIV | 5 | - |
| ADDMOD/MULMOD | 8 | - |
| EXP | 10 | 50 × (바이트 크기) |

#### 메모리 연산
| 연산 | 정적 가스 | 동적 가스 |
|------|-----------|-----------|
| MLOAD/MSTORE | 3 | 확장 비용 |
| MCOPY | 3 | 3 × (워드 수) |
| 메모리 확장 | - | (size²/512) + (size×3) |

#### 스토리지 연산 (EIP-2929/2930)
| 연산 | 콜드 (Cold) | 워밍 (Warm) |
|------|------------|------------|
| SLOAD | 2,600 | 100 |
| SSTORE (신규) | 20,000 | - |
| SSTORE (업데이트) | 5,000 | - |
| SSTORE (환불) | 2,900 | - |
| TLOAD/TSTORE | 100 | 100 |

#### 시스템 연산
| 연산 | 기본 가스 | 추가 비용 |
|------|-----------|-----------|
| CALL/DELEGATECALL | 700 | 접근 비용 + 전송 비용 |
| CREATE | 32,000 | initcode 비용 |
| SELFDESTRUCT | 5,000 | 계정 생성 시 +25,000 |

#### 로깅
```
LOG{N}: 375 (정적) + 375 × N(토픽 수) + 8 × (데이터 바이트 수)
```

### 7.2 EIP-7778 가스 추적 (Amsterdam+)

- **gas_used**: 환불 전 가스 (블록 회계용)
- **gas_spent**: 환불 후 가스 (영수증용)

## 8. 시스템 컨트랙트 (Prague 포크)

### 8.1 시스템 컨트랙트 목록

| 주소 | 이름 | 기능 |
|------|------|------|
| BEACON_ROOTS | 비콘 루트 저장소 | 부모 비콘 블록 루트 저장 (EIP-4788) |
| HISTORY_STORAGE | 히스토리 저장소 | 블록 해시 이력 관리 (EIP-2935) |
| DEPOSIT_CONTRACT | 입금 컨트랙트 | 스테이킹 입금 (EIP-7251) |
| WITHDRAWAL_REQUEST | 출금 요청 | 밸리데이터 탈퇴 큐 |
| CONSOLIDATION_REQUEST | 통합 요청 | 밸리데이터 통합 |

### 8.2 시스템 호출 특성

- 가스 리밋: 30M + 고유 가스 기본 비용
- 블록 가스 리밋 제약 없음
- 실패 시 전체 블록 무효화

## 9. 실행 결과

### 9.1 실행 결과 타입

```rust
enum ExecutionResult {
    Success {
        gas_used: u64,        // 사용된 가스
        gas_refunded: u64,    // 환불된 가스
        logs: Vec<Log>,       // 이벤트 로그
        output: Bytes,        // 출력 데이터
    },
    Revert {
        gas_used: u64,        // 사용된 가스
        output: Bytes,        // REVERT 데이터
    },
    Halt {
        reason: String,       // 중단 이유
        gas_used: u64,        // 모든 가스 소진
    },
}
```

## 10. L2 실행 차이점

L2 모드에서의 LEVM 실행은 L1과 다음 차이가 있다:

| 항목 | L1 | L2 |
|------|----|----|
| 시스템 컨트랙트 | 비콘 루트, 블록 해시 이력 등 | 없음 |
| 수수료 | 기본 수수료만 | 기본 + 오퍼레이터 + L1 수수료 |
| 출금 처리 | 있음 (Shanghai+) | 없음 |
| 요청 추출 | 있음 (Prague+) | 없음 |
| 수수료 토큰 | ETH만 | ETH 또는 ERC20 토큰 |

## 11. 성능 최적화

| 최적화 | 설명 |
|--------|------|
| 점프 대상 사전 계산 | `jump_targets` 벡터로 JUMPDEST 위치 O(1) 검증 |
| 옵코드 룩업 테이블 | 256개 항목 상수 배열로 O(1) 디스패치 |
| SIMD 최적화 | 지원되는 플랫폼에서 SIMD 명령어 활용 |
| 프리컴파일 캐싱 | 프리컴파일 결과 캐싱 |
| 객체 풀링 | 할당 재사용으로 메모리 압박 감소 |
| 핫 옵코드 인라이닝 | 자주 사용되는 옵코드 인라인 처리 |
| arkworks 라이브러리 | BN254 페어링에 2배 속도 향상 |

## 12. VM 데이터베이스 인터페이스

```rust
trait VmDatabase: Send + Sync + DynClone {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256>;
    fn get_chain_config(&self) -> Result<ChainConfig>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata>;
}
```

**구현체**:
- `DynVmDatabase`: 임의의 스토리지 백엔드 래핑 (RocksDB, 인메모리)
- `DatabaseLogger`: 접근된 상태 추적 (디버깅/프로파일링)
- `GeneralizedDatabase`: 캐싱을 포함한 래퍼, L1/L2 지원
