# Plan A: ZK-Proof 기반 초경량 검증 노드 (Ultra-Light ZK Verifier)

## 1. 개요
스마트폰이나 저사양 미니 PC에서도 스테이킹 및 검증을 수행할 수 있도록, 이더리움 블록의 방대한 연산을 직접 수행하지 않고 외부(Prover Network)에서 생성된 수학적 증명(Zero-Knowledge Proof, 예를 들어 SP1 또는 RISC Zero 기반)만을 확인하는 초경량 클라이언트 개조 계획입니다.

## 2. 개발 목표 및 차별점
- **무연산(Zero-Execution):** 스마트 컨트랙트 실행 과정(EVM 구동)을 생략하여 CPU 점유율을 극도로 낮춥니다.
- **초저사양 최적화:** 백그라운드 구동 시 배터리 소모와 발열을 최소화하여 스마트폰에서도 상시 가동이 가능합니다.
- **ethrex 구조 활용:** 이미 `crates/guest-program`과 방대한 ZKVM(SP1, RISC Zero, TEE 등) 호환성을 염두에 둔 ethrex의 장점을 극대화합니다.

## 3. 핵심 아키텍처 및 수정 범위

### 3.1. `crates/vm` (EVM 계층)
- EVM 실행 로직(`levm`)을 완전히 우회(Bypass)하는 새로운 실행 파이프라인 구축.
- 블록 데이터 수신 시 "EVM Execution" 대신 "Proof Verification" 함수로 분기하도록 구조 변경.

### 3.2. `crates/storage` (스토리지 계층)
- State Trie 전체와 히스토리컬 트랜잭션 데이터를 디스크에 저장하는 기능(libmdbx, parity-db 등) 비활성화.
- 오직 최신 블록 헤더(Header), 최신 State Root, 그리고 합의에 필요한 최소한의 메타데이터만 1GB 미만으로 관리하는 초경량 DB 어댑터 구현.

### 3.3. `crates/networking` & `crates/l2/prover` (네트워크 및 ZK 연동)
- 거래(Transaction) 전파 P2P 통신망 대역폭 최소화 (Full block 전파 대신 Header + Proof 전파 기반 P2P 수용).
- 외부 노드나 Prover로부터 해당하는 블록의 ZK Proof 데이터를 가져오기 위한 RPC 통신 또는 프로토콜 구현.

## 4. 단계별 마일스톤 (Milestones)

### Phase 1: 기반 세팅 및 스토리지 다이어트
- `--zk-verifier-only` CLI 실행 플래그 추가.
- 해당 플래그로 실행 시 방대한 데이터베이스 초기화를 건너뛰고, 메모리와 최소 디스크만 사용하는 경량 스토리지 모드 구축.

### Phase 2: ZK 검증 모듈 연동
- ethrex 내부에 내장된 SP1 / RISC Zero Verifier를 가져와서, 일반 L1/L2 블록이 들어왔을 때 증명의 정합성만 검증하는 코어로 교체.
- 블록 연산(EVM) 로직을 무력화하고 Proof 검증 결과에 따라 체인 상태를 갱신하도록 State Transition 로직 변경.

### Phase 3: 네트워크 동기화 개조
- 무거운 State Sync 및 Full Block Sync 로직 차단.
- 헤더 선점 후 백그라운드에서 Proof를 획득하여 빠르게 노드 상태를 최신화하는 ZK Sync 로직 구현.

### Phase 4: 최적화 및 디바이스 테스트
- 스마트폰(ARM) 및 미니 PC (라즈베리파이 등) 환경에서의 크로스 컴파일.
- 장기간 구동 시 메모리 누수 방지 및 배터리 소모율 벤치마킹.
