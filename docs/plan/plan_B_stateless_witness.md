# Plan B: Witness 기반 무상태 실행 노드 (Stateless Execution Node)

## 1. 개요
이더리움 로드맵 "The Verge"의 철학(Verkle Tree / Statelessness)에 완벽히 맞추어, 수백 GB의 상태 데이터베이스 없이 오로지 네트워크로부터 전달받은 상태 조각(Witness)만을 메모리에 올려 직접 블록 트랜잭션을 실행하고 탈중앙성과 검증의 완전성, 스스로 증명하는 자율성을 보장하는 진정한 무상태 클라이언트 노드 구현 계획입니다.

## 2. 개발 목표 및 차별점
- **네이티브 EVM 검증 (Decentralized execution):** Prover 네트워크 같은 외부 생태계에 대한 신뢰 없이 기기 자체에서 직접 코드를 검증하므로, 가장 이더리움스러운(L1) 검증 노드로 작동합니다.
- **초경량 스토리지 (No State DB):** `libmdbx`, `parity-db` 등 방대한 용량을 잡아먹는 로컬 스토리지를 전면 폐기하고, RAM에 올릴 수 있는 Witness 만을 바탕으로 검증합니다.
- **기존 인프라의 확장 (`witness_db.rs` 활용):** ethrex 초기 구조에 존재하는 `GuestProgramStateWrapper` 등 인메모리(Witness) DB 코드를 활용하여 EVM을 연동할 수 있는 튼튼한 토대가 마련되어 있습니다.

## 3. 핵심 아키텍처 및 수정 범위

### 3.1. `crates/vm` (EVM 계층)
- EVM 실행 파이프라인에서 기존의 영구 스토리지 DB 호출 방식을 차단.
- 대신, 메모리에 일시적으로 구성된 `witness_db.rs` (또는 개선된 Stateless DB 구조)와 연결.
- 실행 중 상태가 누락(Missing State)된 경우 처리에 대한 재요청 핸들링 로직 추가 구조 변경.

### 3.2. `crates/networking` & 블록 전파
- 블록 전파 시 블록 데이터 내부에 필요한 **상태 증빙 자료 (Witness - 주소의 nonce, 잔액, 스토리지 슬롯 데이터 등)**를 함께 구성하여 받아오는 프로토콜(`eth/68` 무상태 확장 등) 통신 규격 적용 및 파싱.
- Witness를 수신하면 이를 메모리에서 신뢰성 검증(Verkle Trie 또는 Merkle Trie Root 확인)을 거쳐 Stateless DB로 로드.

### 3.3. `crates/storage` (스토리지 계층)
- 디스크 접근 횟수 제로 목표 달성. 최신 256개의 블록 헤더와 현재 State Root만 갱신하며 유지.
- 무상태 노드이므로 히스토리 데이터나 복구(Archive) 데이터를 일절 저장하지 않는 구조로 경량화.

## 4. 단계별 마일스톤 (Milestones)

### Phase 1: 기반 세팅 및 State DB 비활성화
- `--stateless` CLI 실행 플래그 추가.
- State Sync 및 히스토리 스토리지 계층 접근 완전 차단. 오로지 블록 헤더만 저장하는 베어메탈 경량 상태 진입.

### Phase 2: 무상태 DB 메모리 로드 파이프라인 구현
- ethrex의 `crates/vm/witness_db.rs` 코드를 확장하여 완벽한 인메모리 상태 제어 DB 객체 개발.
- 하드코딩된 Witness 데이터를 수동 주입하여 EVM `levm` 모듈이 의존성 없이 정상적으로 실행되는지 유닛 테스트 구비.

### Phase 3: P2P 네트워크를 통한 Witness 수신
- 거래 네트워크 계층 개조: 새로운 블록이 들어올 때 이더리움 코어 네트워크나 전용 릴레이로부터 해당 트랜잭션 집합이 요구하는 상태 증빙(Witness)을 받아오도록 P2P/RPC 응답 구조 파싱.
- 수신한 Witness가 진짜인지 Root Hash 연산 및 검증 방어 로직.

### Phase 4: 최적화 및 스마트폰 환경 벤치마크
- 매 블록마다 네트워크 대역폭(Network Bandwidth) 사용량을 추적하고 최적의 메모리 캐시 로직 구현.
- ARM 모바일 칩 보드에서 원활한 블록(Execution) 타임을 맞출 수 있는지 벤치마크 측정.
