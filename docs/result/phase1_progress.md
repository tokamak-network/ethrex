# Plan A: ZK-Proof Verifier 개조 작업 진행 결과 (Phase 1)

**작성일**: 2026-02-25
**현재 단계**: Phase 1 (기반 세팅 및 스토리지 다이어트) & Phase 3 일부 (네트워크 동기화 경량화) 완료

## 1. 개요
Ethrex 클라이언트를 ZK-Proof 검증 전용 초경량 노드(Ultra-Light Verifier Node)로 개조하기 위한 기반 공사를 완료했습니다. 이 노드는 풀 노드가 전송하는 상태 루트(State Root)와 ZK 증명을 바탕으로 O(1)에 가까운 수학적 검증만 수행하도록 설계되었습니다.

## 2. 주요 구현 내용

### 2.1 CLI 옵션 확장 (`cmd/ethrex/cli.rs`)
- `--zk-verifier-only` 플래그를 추가하여 노드가 초경량 검증 모드로 구동될 수 있도록 제어 스위치를 마련했습니다.

### 2.2 디스크 스토리지 초기화 우회 (`cmd/ethrex/initializers.rs`)
- 기존에 뼈대로 사용되던 무거운 데이터베이스(RocksDB 등)의 초기화 프로세스를 우회하도록 `init_store`, `load_store`, `open_store` 함수를 수정했습니다.
- `--zk-verifier-only` 실행 시 하드디스크가 아닌 `EngineType::InMemory` 인메모리 스토리지나 빈 껍데기만 남겨 디스크 사용량(Storage Footprint)을 0에 가깝도록 대폭 경량화했습니다.

### 2.3 EVM 실행 엔진(Execution Engine) 연산 스킵 (`crates/blockchain/blockchain.rs`)
- 블록 실행 파이프라인인 `execute_block_pipeline` 함수 상단에 ZK Verifier를 위한 Bypass(우회로)를 신설했습니다.
- 블록이 수신될 때 EVM 트랜잭션 연산 스레드를 생성하지 않고, 헤더(Header)의 연산 결과만을 수용하며 빠져나오도록 더미 로직(Dummy Logic)을 1차적으로 연결했습니다. 
- (※ Phase 2에서 이 위치에 실제 ZK Proof `verify()` 함수가 연동될 예정입니다.)

### 2.4 P2P 네트워크 동기화 블록 본문(Body) 다운로드 차단 (`crates/networking/p2p/sync/full.rs`)
- 동기화 파이프라인에서 블록을 요청할 때 엄청난 대역폭을 소모하는 트랜잭션 본문(Body)의 `request_block_bodies` Network Fetch를 차단했습니다.
- 빈 Block 인스턴스로 넘겨 네트워크 대역폭 및 메모리 사용량을 최소화했습니다.

## 3. 테스트(시뮬레이션) 결과
- 벤치마킹을 위해 Holesky 테스트넷 설정과 `--zk-verifier-only` 플래그를 결합하여 구동한 결과, 
- `datadir` 에 `node_config.json` 등 필수 설정 파일만 생성되고, 수십 기가 이상을 차지하는 `database` 디렉토리가 전혀 생성되지 않음을 검증(Memory Storage Only 구동)했습니다.

## 4. 넥스트 스텝 (Phase 2 계획)
- **ZK Verifier 모듈 연동**: SP1 또는 Risc0 형태의 ZK Proof 검증 로직(`verify_proof()`)을 찾아, `execute_block_pipeline` 내부의 빈 우회로에 탑재합니다.
- 수신된 검증 페이로드가 유효하지 않을 경우 체인 동기화를 거부(Reject)하는 단단한 무결성 검증 환경을 구축합니다.
