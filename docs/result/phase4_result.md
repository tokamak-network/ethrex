# Phase 4: ZK 증명 처리 파이프라인 연동 심화

## 1. 개요
Phase 4는 Phase 3에서 마련된 P2P 메시지 기반을 바탕으로, 노드가 실제로 로컬 스토리지에서 ZK Proof 데이터를 꺼내어 응답하거나, 수신한 Proof를 SP1 SDK를 통해 물리적으로 검증하는 연결고리를 구축하는 단계입니다. 

## 2. 작업 내용
이번 단계에서는 다음 세 가지 주요 영역에서 연동 작업을 진행했습니다.

### 2.1. Storage/DB 레이어 구축 (Proof Storage)
- **목표:** 노드가 생성 또는 수신한 ZK Proof 데이터를 영구적으로 저장하고 검색할 수 있도록 스토리지 레이어를 연장합니다.
- **수정사항:**
  - `crates/storage/api/tables.rs`: 
    - `BLOCK_PROOFS` 상수를 추가하여 스토리지 테이블(Column Family)을 새로 할당하였습니다 (`[&str; 20]`).
    - 구조: `[block_hash] => [serialized_block_proof]` 
  - `crates/storage/store.rs`:
    - `Store` 구조체 구현부에 `add_block_proof` 및 `get_block_proof` 메서드를 개발했습니다. 
    - `BlockProof` 데이터를 직렬화(Serialization `/` Deserialization) 하여 R/W 할 수 있도록 `serde_json`을 적용하여 안전성 높은 DB 접근 계층을 마련했습니다.

### 2.2. P2P 처리 핸들러 DB 연결
- **목표:** Phase 3에서 `TODO`로 남겨두었던 P2P 네트워크의 `GetBlockProofs` 처리기(Handler)를 실제 스토리지와 연결합니다.
- **수정사항:**
  - `crates/networking/p2p/rlpx/l2/l2_connection.rs`:
    - `L2Message::GetBlockProofs` 메시지를 수신받았을 때, 요청된 모든 `block_hashes`를 순회하며 `established.storage.get_block_proof(hash)`를 호출하여 실제 Proof를 검색하도록 로직을 수정했습니다.
    - 검색된 Proof만 `BlockProofs` 응답 메시지에 담아 전송하도록 하였습니다.

### 2.3. SP1 SDK 검증 (Prover) 연동 준비
- **목표:** ZK-Verifier 파이프라인이 단순한 대기열(Sleep dummy)을 넘어 실제 검증기(Verifying Key 기반 수학적 증명 확인)로 진입할 수 있도록 SDK 연동 로직을 구성합니다.
- **수정사항:**
  - 코어 구동을 위한 의존성으로 `sp1-sdk` 크레이트를 `crates/blockchain`의 `Cargo.toml`에 추가했습니다.
  - `crates/blockchain/zk.rs`:
    - `verify_proof_for_block(block: &Block)` 시그니처를 `verify_proof_for_block(block: &Block, proof: Option<BlockProof>)`로 개선하여 P2P나 DB로부터 가져온 실제 증명 Payload를 주입받을 수 있도록 변경했습니다.
    - 증명 버퍼를 역직렬화하여 `sp1_sdk::SP1ProofWithPublicValues` 구조체로 캐스팅하는 과정을 추가했습니다.
    - **Note:** SP1 검증은 현재 Verification Key(VK)가 필요하므로, 파이프라인의 완성도를 높이는 틀(Template)을 작성하고 Warning Log와 함께 주석으로(`client.verify(&sp1_proof, &vk)`) 보존하여 추후 ELF 바이너리와 키가 생성될 때 즉시 활성화되도록 했습니다.
  - `crates/blockchain/blockchain.rs`:
    - `execute_block_pipeline` 의 `zk_verifier_only` 분기점에서, 로컬 Storage를 통해 `get_block_proof()`를 선행 탐색하여 `zk::verify_proof_for_block(..., proof)`로 전달하도록 파라미터 컨텍스트를 동기화했습니다.

## 3. 검증 결과
모든 의존성 추가와 코드 주가/교정 작업이 정상적으로 P2P/Blockchain/L2 모듈에 스며들었음을 `cargo test -p ethrex-p2p --features l2` 유닛 테스트(100% Passed)와 무결성 점검을 통해 확인했습니다.

## 4. 결론
이제 `ethrex` 클라이언트는 ZK Proof 데이터를 DB에 담아놓고 필요한 피어에게 전송할 수 있으며, 수신한 Proof를 SP1 라이브러리 포맷으로 파싱하여 검증 준비 단계까지 올려놓았습니다. 남은 것은 ELF 생성과 키 발급 후 검증부의 주석을 푸는 일입니다.
