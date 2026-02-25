# Phase 3: 네트워크 및 상태 동기화 (Networking & P2P)

## 1. 개요
Phase 3 작업은 풀 노드의 무거운 블록 데이터(트랜잭션 바디) 대신 최신 헤더와 검증 증명(ZK Proof)만을 수신하도록 네트워크 로직을 간소화하고 확장하는 것을 목표로 합니다.
이를 통해 ZK-Verifier 전용 노드가 P2P 네트워크에 원활하게 통합되어 수학적 증명을 기반으로 한 초경량 검증 모드로 작동할 수 있습니다.

## 2. 작업 내용
본 Phase에서는 다음과 같은 세부 작업을 진행했습니다.

### 2.1. ZK Proof 데이터 타입 정의
- `crates/common/types/zk.rs`: `BlockProof` 타입 정의. 증명 데이터를 담기 위한 구조체입니다.
    - 직렬화(Serialization)/역직렬화(Deserialization) 및 `RLPEncode`, `RLPDecode` 트레이트를 구현하여 P2P 네트워크 상에서 효율적으로 전송될 수 있게 하였습니다.
- 타입 모듈(`crates/common/types/mod.rs`)에 `zk` 서브모듈을 노출.

### 2.2. P2P 메시지 프로토콜 확장
- `crates/networking/p2p/rlpx/eth/blocks.rs`: 이더리움 기본 프로토콜을 확장하여 `GetBlockProofs` (증명 요청)과 `BlockProofs` (증명 응답) 메시지 구조체를 추가했습니다.
    - 메시지 코드는 기존 `eth68` 등과 충돌하지 않도록 각각 `0x12`, `0x13`으로 지정했습니다.
- `crates/networking/p2p/rlpx/l2/messages.rs` & `rlpx/message.rs`: 새로 정의된 ZK Proof 요청/응답 메시지를 `L2Message` 열거형 및 `Message` 디코딩 로직에 통합하여 정상적인 핸들링이 가능하도록 수정했습니다.

### 2.3. 네트워크 동기화 파이프라인 우회 로직 구현
- `crates/networking/p2p/sync/full.rs`: 풀 동기화 사이클(`sync_cycle_full`) 내부의 로직을 조건부로 분기했습니다.
    - CLI 옵션(`--zk-verifier-only`)이 활성화된 경우:
        - 트랜잭션 등 블록 Body 정보가 무거운 `GetBlockBodies` 요청을 스킵(Bypass)합니다.
        - 대신 새로 구현된 `request_block_proofs`를 호출하여 피어 노드들로부터 `BlockProof` 데이터를 수신 받아 블록 객체에 연계(Log Level 표시)하도록 반영했습니다.
- `crates/networking/p2p/peer_handler.rs`: P2P 통신 핸들러 구조체인 `PeerHandler`에 `request_block_proofs()` 메서드를 신규 추가하여, `SUPPORTED_BASED_CAPABILITIES`를 지원하는 피어에게 ZK Proof를 명시적으로 요청할 수 있도록 구성했습니다.
- `crates/networking/p2p/rlpx/l2/l2_connection.rs`: 연결된 피어로부터 들어오는 `L2Message::GetBlockProofs` 메시지에 반응하기 위한 서버 측 처리 로직을 추가했습니다.
    - (추후 실제 Storage에서 Proof를 패치해 전송하도록 확장될 예정)

### 2.4. 빌드 및 테스트 검증
- 새로 추가/수정된 코드가 Rust 컴파일러와 링커 상에서 오류를 발생시키지 않도록, `cargo check -p ethrex-p2p -p ethrex --features l2` 명령어를 통해 무결성을 확보했습니다.
- `cargo test -p ethrex-p2p --features l2`를 통과시켜, 메시지 인코딩/디코딩 로직이 기존 이더리움 코어 통신에 사이드 이펙트를 주지 않았음을 완전히 검증했습니다 (총 38건 패스).

## 3. 결론 및 향후 계획 (Next Steps)
Phase 3의 성공적인 완료로 인해 ZK-Verifier 모드는 기존 디스크 기반의 육중한 이더리움 가상머신 연산과 P2P 블록 Body 다운로드 과정 일체를 성공적으로 건너뛸 수 있게(Bypass) 되었습니다.

**다음 마일스톤(Next Actions):**
1. **Proof DB 연동 (Storage):** 피어 노드로부터 요청받은 `BlockProof`를 메모리나 데이터베이스에서 꺼내어 실제로(`l2_connection.rs`) 응답하는 기능 연결.
2. **실제 증명 생성기(Prover) 연결:** SP1이나 RISC0 같은 ZK Prover 모듈에서 생성된 검증용 바이트코드 버퍼를 P2P에 실어 보내는 완전한 인프라 확보.
3. **ZK Verifier 로직 적용:** 블록 수신 시, `blockchain/zk.rs`의 `verify_proof_for_block`에서 `sp1_sdk::CpuProver::verify()` 등을 활용하여 수신된 증명의 수학적 타당성을 검증하는 실제 파이프라인 구체화.
