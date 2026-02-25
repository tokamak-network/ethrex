# Phase 5: P2P 동기화망의 Proof 영구 저장 파이프라인 마무리

## 1. 개요
Phase 5에서는 이전 단계(Phase 4)에서 구축한 Storage/DB의 `BlockProof` 저장 기능을 실제 네트워크 레이어의 동기화 파이프라인(Sync Cycle)과 매끄럽게 접합시켰습니다.
단순히 피어(Peer)로부터 Proof를 다운로드 받고 메모리에 로깅하는 것을 넘어, 로컬 DB(RocksDB, LMDB 등)에 영구 보존(Persistent Storage)하여 전체 블록체인 파이프라인이 자연스럽게 연동될 수 있도록 마무리했습니다.

## 2. 세부 작업 내용
이전 구성에서는 ZK-Verifier 모드일 경우 `GetBlockBodies` 통신을 건너뛰고 `request_block_proofs`를 호출해 증명 데이터를 수신하도록 구현되었지만, 실제로 이를 파싱해서 DB에 영구 기록하는 과정이 비어 있었습니다. 이번 작업으로 이 데이터 연결고리를 이어붙였습니다.

### 2.1. P2P Sync 파이프라인 내 Proof 영구 저장 구현
- **파일 경로:** `crates/networking/p2p/sync/full.rs`
- **목표:** 풀 노드(Full Node) 동기화 파이프라인 내에서, 블록의 헤더(Header)를 받은 직후 `Proof`를 수신했을 때 이를 DB에 저장하는 로직을 완전한 흐름으로 완성시킵니다.
- **수정사항:**
  - `peers.request_block_proofs(&block_hashes)`를 통해 받아온 증명(`proofs`) 리스트를 순회(`iter`)합니다.
  - 각각의 Proof를 `store.add_block_proof(hash, proof).await`를 호출하여 로컬 스토리지 엔진(Column Family: `BLOCK_PROOFS`)에 안전하게 저장합니다.
  - 이 과정이 정상적으로 수행되었을 때, 이전에 수정해 둔 `crates/blockchain/blockchain.rs`의 `execute_block_pipeline`에서 `self.storage.get_block_proof(block.hash())` 구문을 통해 방금 다운로드 된 Proof를 자연스럽게 로드할 수 있게 됩니다.

## 3. ZK Verifier 파이프라인 전체 흐름 요약 (Phase 1 ~ 5 완료 기준)
이로써 초경량 `ethrex` ZK-Verifier 클라이언트의 핵심 기능이 모두 한 길로 연결되었습니다. 내부 시스템 아키텍처 흐름은 다음과 같습니다.

1. **[네트워크 부팅]**: 노드가 구동되면 `--zk-verifier-only` 플래그를 인식하여 별도의 경량 모드로 전환됩니다 (Phase 1).
2. **[동기화(Sync)]**: 피어들과 통신을 시작할 때, 무거운 트랜잭션 바디(BlockBody)를 받지 않고 헤더(Header)만 수신한 뒤 `request_block_proofs`(Phase 3) 전용 L2 메시지를 날려 각 블록에 대한 검증 증명(ZK Proof) 버퍼를 받습니다.
3. **[스토리지 저장]**: 수신에 성공한 `ethrex_common::types::BlockProof` 데이터는 안전하게 직렬화되어 로컬 데이터베이스의 `BLOCK_PROOFS` 테이블에 보존됩니다 (Phase 4, Phase 5).
4. **[검증 실행 파이프라인 우회]**: 상태 트리(State Trie) 머클화 및 EVM 실행 구문(`execute_block`)에 진입하기 직전, 노드는 로컬에 방금 저장된 Proof를 꺼내옵니다. EVM 실행 쓰레드를 우회(Bypass)합니다 (Phase 2).
5. **[수학적 증명 검증]**: 꺼내온 Payload(Proof)를 `verify_proof_for_block`(Phase 4)으로 전달합니다. 여기서 `sp1_sdk::SP1ProofWithPublicValues` 구조체로 역직렬화한 뒤, SP1 ZKVM 코어 로직에 따라 참(True)인지 여부만 판단합니다.

## 4. 향후 남은 실무적 과제 (To-Do)
본 데모/엔진은 구조적으로 언제든지 SP1(혹은 다른 ZKVM)이 제공하는 `Verifying Key (VK)`가 주어지면 즉시 검증을 돌릴 수 있도록 설계되었습니다.
이후 블록체인 운용 시 다음 항목들이 병행되면 완벽한 프로덕션 노드가 됩니다.
- **ZK Verifying Key(VK) 배포체계 도입**: ZK Prover가 `ethrex` 코어 로직(ELF 바이너리)을 빌드했을 때 추출되는 `vk` 값을 노드의 Chain Config(예: `genesys.json` 등)나 하드코딩된 상수로 로드하여 `client.verify(...)` 함수를 활성화 시켜야 합니다.
- **Proof RPC 구현**: 로컬 P2P 망 구조 외에도, Cloud ZK Prover(분산 병렬 증명 서버들) 클러스터가 블록을 검증하고 생성된 증명을 노드에 밀어넣을 수 있도록(Push), 별도의 외부 JSON-RPC 엔드포인트(예: `eth_submitBlockProof`)를 열어주는 작업이 병행되면 유리합니다 (이 RPC API 내부에서는 단순히 `store.add_block_proof`를 호출하고 통신망에 브로드캐스팅해 주면 됩니다).
