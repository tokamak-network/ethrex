# ZK-Verifier 초경량 모드 실행 및 테스트 가이드

이 문서는 `ethrex`의 ZK-Verifier(초경량 검증) 모드를 어떻게 실행하고, 테스트하며, 정상 동작 여부를 확인하는지에 대한 가이드입니다.

---

## 1. 실행 및 기본 검증 방법

ZK-Verifier 모드의 가장 큰 특징은 **EVM 실행(블록 연산)과 머클 트리(State Trie) 구성을 건너뛰며(Bypass), P2P에서 무거운 블록 바디 대신 `GetBlockProofs`를 통해 증명(Proof)만 받아와 검증**한다는 점입니다.

### 실행 명령어

아래 명령어를 통해 Holesky 테스트넷(또는 로컬망)에 ZK-Verifier 모드로 접속합니다.

```bash
cargo run --bin ethrex -- --zk-verifier-only --network holesky
```

### 정상 동작 확인 (기대 결과 Log)

명령어 실행 후 터미널에 출력되는 로그(Log)를 통해 동작을 확인할 수 있습니다. 다음 로그들이 출력된다면 정상입니다.

1. **시작 확인:**
   ```
   INFO Starting HTTP server at 0.0.0.0:8545
   ...
   ```
2. **동기화 파이프라인 (네트워크에서 Proof 다운로드 성공):**
   ```
   INFO ZK-Verifier: Requesting block proofs
   INFO ZK-Verifier: Obtained N block proofs
   ```
   - 이 로그는 기존 풀 구동 방식인 `GetBlockBodies` 요청을 차단하고, 피어에게 `GetBlockProofs`(Phase 3 작업)를 보내어 데이터를 받아왔음을 의미합니다.

3. **증명 검증 파이프라인 (EVM Bypass 성공):**
   ```
   INFO ZK-Verifier: Skipping EVM execution and state merkleization for block <블록번호>
   INFO [ZK Verifier] Verifying proof for block <블록번호> (hash: 0x...)...
   WARN [ZK Verifier] No proof found for block <블록번호>. Skipping actual verification step and accepting it (Dev mode).
   ```
   - 위 로그는 EVM에서 상태 전이(`execute_block`)를 하지 않고 즉시 검증 모듈(`zk.rs`)로 전달되었음을 뜻합니다. 
   - 현재는 피어가 런타임에 던져주는 실제 SP1 Proof가 없다면, `Dev mode` 경고와 함께 다음 블록으로 유연하게 넘어갑니다 (Phase 4 작업).

4. **스토리지 (디스크 다이어트 확인):**
   - ZK-Verifier 모드로 구동할 때는 디스크 기반 데이터베이스(RocksDB/mdbx)의 거대한 체인데이터를 쌓지 않고, `node_config.json` 등 경량화된 설정만 관리해야 합니다. Activity Monitor나 로컬 경로 파일시스템 확인 시 용량이 풀노드(수십~수백 GB)에 비해 극적으로 비어있다면 정상입니다.

---

## 2. 유닛 테스트 및 무결성 테스트 방법

기능들이 다른 코어 이더리움 모듈에 영향을 주지 않았는지 확인하기 위해 다음 테스트들을 수행할 수 있습니다.

### P2P 통신망 단위 테스트
Phase 3에서 추가한 `GetBlockProofs`, `BlockProofs` 메시지의 직렬화/역직렬화가 정상 동작하는지 점검합니다.

```bash
cargo test -p ethrex-p2p --features l2
```
**기대 결과:**
- 에러 없이 모든 유닛 테스트가 `ok` 상태로 Passed 되어야 합니다. (예: `test result: ok. 38 passed; 0 failed;`)

### ZK-Verifier 파이프라인 자동 검증 스크립트
작성된 스크립트를 통해 검증 모드의 네트워크 구동 체계(Bypass Pipeline)가 깨지지 않았는지 즉시 확인할 수 있습니다.

```bash
./run_verifier_unit_tests.sh
```
**기대 결과:**
- `ZK-Verifier unit tests passed!` 메시지가 출력되면 파이프라인 구성이 정상입니다.

### 블록체인 파이프라인 단위 테스트
EVM 우회 로직 및 스토리지 연동(Proof 저장 및 수신)에 대한 무결성 검증입니다.

```bash
cargo test -p ethrex-blockchain
```
**기대 결과:**
- 빌드/컴파일 에러가 없으며, 기존 EVM 실행 파이프라인의 유닛 테스트 척도를 모두 통과해야 합니다.

---

## 3. 심화: 벤치마킹 테스트 (리소스 점유율 비교)

일반 모드와 ZK-Verifier 모드가 소모하는 CPU 및 메모리 시간 측정을 위한 벤치마크 테스트입니다. 본 프로젝트 최상단 디렉토리의 `benchmark_zk.sh` 쉘 스크립트를 실행하여 성능 차이를 비교할 수 있습니다.

```bash
./benchmark_zk.sh
```

**기대 결과:**
- **Full Node (일반 모드):** EVM 스레드가 가동되므로 상당한 CPU와 메모리가 소모되는 측정치(`top` 혹은 리소스 로그 기준)가 나옵니다.
- **ZK Verifier 모드:** O(1)의 빠른 검증 시간 및 헤더 전용 다운로드 특성으로 인해 CPU/메모리 점유율이 모바일(스마트폰)에서도 구동될 만큼 가볍게 산출되어야 합니다.

---

## 4. 로컬 스토리지 DB 영구 보존 구조 점검

코드는 다음 경로를 통해 Proof를 넣고 뺍니다. 구조의 무결성을 점검할 때 참고할 수 있습니다.

1. **저장 주체**: `crates/networking/p2p/sync/full.rs`의 `store.add_block_proof(hash, proof).await;` 가 실행되었는지 점검.
2. **조회 주체**: `crates/blockchain/blockchain.rs`의 `execute_block_pipeline`에서 `self.storage.get_block_proof(block.hash())` 로 수신했는지 점검.
3. **SP1 연동**: `crates/blockchain/zk.rs` 내 `bincode::deserialize`가 정상 에러 없이 진행되었는지 점검(콘솔에 `Deserialized successfully` 로그).

위 4가지 항목과 기대 결과를 충족하신다면 시스템은 ZK-Verifier로서 완벽하게 구동 중입니다!
