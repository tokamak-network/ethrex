# Plan A: ZK-Proof Verifier 개조 작업 진행 결과 (Phase 2)

**작성일**: 2026-02-25
**현재 단계**: Phase 2 (ZK 검증 모듈 연동 및 파이프라인 우회 완료)

## 1. 개요
이더리움 블록을 수신했을 때, 무거운 연산을 담당하는 EVM(Ethereum Virtual Machine) 계층(Execution Layer)을 완전히 배제하고, 수학적 증명 검증(Zero-Knowledge Proof Verification)으로 대체하는 구조를 성공적으로 삽입했습니다. 이로써 ZK-Verifier 파이프라인 우회로가 완성되었습니다.

## 2. 주요 구현 내용

### 2.1 블록체인 검증 모듈 (`crates/blockchain/zk.rs`) 신규 개발
- 향후 완전한 ZK-SNARK 배포 체계(SP1, RISC Zero, OP-Succinct 등)의 Verifier SDK를 부착할 수 있는 전용 모듈(`zk::verify_proof_for_block`)을 생성했습니다.
- 현재는 시뮬레이션 및 벤치마킹을 위해 O(1) 시간 복잡도를 모사하는 15ms의 대기 시간(Delay)과 더미 성공(Result::Ok)을 반환하도록 설계되었습니다.

### 2.2 메인 블록체인 파이프라인 인터셉트 (`crates/blockchain/blockchain.rs`)
- 기존에 `LEVM` 등 무거운 EVM 실행 엔진을 생성하고 상태 머클 트리(State Merkleization)를 업데이트하던 핵심 함수인 `execute_block_pipeline`에 진입 권한 분기점(Bypass)을 추가했습니다.
- `--zk-verifier-only` 플래그가 활성화되어 있을 경우, 블록 바디나 트랜잭션 수량에 관계 없이 트랜잭션 처리를 강제 생략하고, 즉각적으로 `zk::verify_proof_for_block()` 함수를 호출하게 됩니다.
- 검증 결과가 성공이면 풀 노드가 검증한 것과 동일하게 `BlockExecutionResult` (영수증, 가스 사용량 등)의 빈 껍데기를 리턴하여, 상위 함수가 이를 정상 블록 처리한 것으로 오인(합법적 우회)하도록 설계했습니다.

## 3. 중간 시뮬레이션 및 테스트
- `cargo check` 및 전역 컴파일에서 에러나 참조 문제가 발생하지 않고 L2 프로버 모듈 연계에도 이상이 없음을 확인했습니다.

## 4. 넥스트 스텝 (Phase 3 & 4)
- **로컬 벤치마킹 (Benchmarking)**: 방금 구현된 우회로를 검증하기 위해, ZK 모드가 인에이블 된 노드와 기존 풀(Full) 노드를 대상으로 다량의 블록 동기화 성능 비교(CPU 점유율, 속도)를 수행할 쉘 스크립트를 작성합니다.
- **P2P 네트워킹 및 RPC 개조**: 블록 본문(Body) 대신 증명(Proof Bytes)을 수신하기 위한 통신 레이어를 손봐, Phase 2의 더미 검증 함수가 실제 Bytes 배열을 입력받아 동작할 수 있는 환경을 만듭니다.
