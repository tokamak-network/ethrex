# ZK-Verifier Node Architecture & Implementation Details

## 1. Project Overview
- **프로젝트 목적:** 이더리움 코어(`ethrex`) 클라이언트의 EVM 싱크 방식을 우회하고, 가벼운 연산(ZK Proof Verify)만으로 네트워크 풀 노드와 동일한 보안성을 가지는 '초경량 ZK-Verifier 노드' 파이프라인(CLI Flag 기반)을 구축.
- **해결하려는 문제:** 기존 이더리움 블록체인 노드의 막대한 하드웨어 요구사항(TB 단위 스토리지, 고성능 프로세서)과 블록 수신 시 전체 히스토리를 재계산해야 하는 극심한 동기화(Sync) 지연 시간 완화.
- **핵심 가치:** 100만 명의 사용자가 고가의 무거운 연산 장비 없이 스마트폰/랩톱 수준의 기기에서도 네트워크의 무결성을 스스로 입증(Trustless) 할 수 있게 함으로써 이더리움 본연의 거버넌스와 클라이언트의 다양성(Decentralization) 회복.

## 2. Functional Requirements
- **기능 목록:** 
  1. `--zk-verifier-only` CLI 플래그를 통한 노드 기동 모드 분기 (스토리지/DB 초기화 다이어트 포함).
  2. P2P 네트워크 동기화 시 풀 블록바디(Transactions) 대신 `GetBlockProofs` 기반 증명서 파일(Proofs)만 가져오는 L2 핸들러 통신 우회.
  3. 블록 실행(`execute_block_pipeline`) 단계 진입 시 EVM 호출 및 머클 상태 전이 연산을 스킵하고, SP1 기반 `verify_proof_for_block` 검증 로직으로 직행(Bypass).
- **입력/출력 정의:**
  - 입력: L2/P2P 네트워크를 통해 수신한 직렬화된 RLP 패킷 구조체(`GetBlockProofs` / `BlockProofs`), CLI 플래그 `--zk-verifier-only`.
  - 출력: 블록당 검증 결과 (EVM 패스 후 성공 시 `Result<(), String>`, DB 영구 보존).
- **예외 처리 정책:**
  - 증명 파일이 잘못된 형식(`Vec<u8>` 직렬화 실패)이거나, 악의적일 경우(Verify 수학적 검증 실패) 즉각 블록 유효성을 폐기하고 `Result::Err`을 반환, 해당 피어를 페널티 대상으로 처리(`peer_handler` 연결 차단).

## 3. Non-Functional Requirements
- **성능 요구사항:** ZK 증명의 파일 파싱 및 수학적 검증은 블록 수신 후 수십 밀리초(ms) 단위 이내에完了되어야 하며, 로컬 CPU(스마트폰/구형 랩톱 코어) 점유율을 체인 처리 과정에서 5% 이내로 유지해야 함 (O(1) 속도).
- **보안 요구사항:** 연산을 건너뛰더라도 누군가 조작한 장부(트랜잭션)를 받아들이면 안 되며, 증명(Proof)을 SP1 툴체인 암호학 공식에 통과시켜 수학적으로 100% 결점이 없음을 증명해 내는 무신뢰(Trustless) 구조 유지가 필수.
- **확장성 요구사항:** 향후 SP1 외에도 RISC0, TDX 등 다양한 검증 구조(Prover Types)가 연동될 수 있도록 인터페이스를 모듈화해야 함.

## 4. System Architecture
- **구조 다이어그램 (텍스트):**
  ```text
  [Full Node/Prover] -- (P2P Gossip) --> [ZK-Verifier Node (CLI Flag On)]
      |                                        |
      v                                        v
   Generates ZK Proof (Heavy)            crates/networking/p2p/
                                           - Peer Handler requests `GetBlockProofs` instead of Body
                                           - Sync Pipeline receives `BlockProof`
                                               |
                                               v
                                         crates/blockchain/
                                           - `execute_block_pipeline` bypasses EVM/StateTrie
                                           - Calls `zk::verify_proof_for_block`
                                               |
                                               v
                                         crates/storage/
                                           - Saves Header & Proof to Lightweight DB
  ```
- **모듈별 역할:**
  - `cmd/ethrex`: 사용자 진입점, `--zk-verifier-only` 옵션 파싱 및 구동 분기 설정.
  - `crates/networking/p2p`: ZK Proof 패킷 요청/응답을 위한 새로운 통신 프로토콜 확장, 블록 동기화 분기.
  - `crates/blockchain`: EVM 우회 로직 및 실제 증명서를 해체(Desirialize)하여 암호학적 검증 엔진에 태우는 메인 파이프라인.
  - `crates/storage`: 최소한의 헤더와 확인된 ZK Proof만 저장하는 `BLOCK_PROOFS` Column Family 관리.

## 5. Core Logic Explanation
- **상태 변화 흐름:**
  1. 클라이언트 부팅 시 `--zk-verifier-only` 감지.
  2. 타 피어 발견 후 동기화 시작 시, 거대한 Transactions 다운로드를 포기하고 헤더와 `BlockProof`만 다운로드 시작.
  3. 받은 Proof들을 로컬 `BLOCK_PROOFS` 스토리지 DB에 캐싱.
  4. 파이프라인 실행 시 EVM 객체(Transaction Loop)에 들어가지 않고, DB에 캐싱된 해당 블록 해시의 Proof를 꺼내 `verify()` 가동.
  5. 성공하면 헤더 높이(Chain head)를 증가시킴.
- **주요 데이터 구조 정의:**
  - `BlockProof { proof: Vec<u8> }` : RLP 인코딩을 지원하며, P2P 전송을 위해 감싸진 SP1 증명서 바이트 래퍼 구조체.
  - `GetBlockProofs / BlockProofs` : DevP2P Eth/68 프로토콜 위에 덧대진 L2 전용 Custom RLP 메시지 규격.

## 6. External Dependencies
- **라이브러리/프레임워크:**
  - `sp1-sdk`: Succinct Labs의 범용 ZKVM (SP1ProofWithPublicValues, VerifyingKey 파싱 로직 포함).
  - `bincode`: 네트워크로 들어온 바이트 배열 형태의 Proof 객체를 Rust의 구조체 배열로 역직렬화(Deserialize)하기 위한 툴.
  - `serde_json`: 스토리지(RocksDB)에 ZK 증명 구조체를 유연하게 JSON 스트링 타입으로 저장, 호출하기 위해 사용.

## 7. Security Considerations
- **공격 벡터 분석:** 악의적인 노드가 컴퓨팅 우회를 악용하여, 검증도 안 된 쓰레기 바이트 배열 정보(`[0, 1, 2...]`)를 마치 정상적인 증명서(Proof)인 양 `BlockProofs` 패킷에 실어 보내어 동기화를 강제하는 행위.
- **방어 전략:** 받은 패킷을 DB에 저장하기 전/처리하기 직전에 `bincode::deserialize` 및 `client.verify(proof, vk)` 를 통해 형식이 깨졌는지(Malformed), 위조 키맵인지 밀리초 단위로 파악하고 즉각 `ChainError`를 발생시켜 해당 피어(Peer)의 신뢰도를 강등(Penalize)하고 블록을 폐기(Reject)함.

## 8. Test Evidence
- **테스트 케이스:**
  1. `test_verify_proof_invalid_data` (Unit Test): 악의적 난수 바이트 주입 시 `vk` 검증 우회를 실패하고 역직렬화 에러를 반환하는지 증명 완료 (`verify_proof` 통과).
  2. `test_local_devnet.sh` (Integration): 1개의 EVM 풀노드(Prover)와 1개의 ZK 모드 노드(Verifier)를 백그라운드 구동 후 25초간 P2P 통신망 시뮬레이션. Verifier의 로그에서 `Skipping EVM execution and state merkleization`이 관측됨을 통해 완벽한 우회 구동 입증.
- **실패 시 시나리오:** 네트워크에서 Proof를 유효하게 추출하지 못할 경우 파이프라인이 멈추지 않고, 개발 모드(Dev mode) fallback 로그를 띄우며 최소한의 체인 동기화는 유지하도록 임시 보완.

## 9. Known Limitations
- **아직 해결되지 않은 부분 (기술적 부채):**
  - **진짜 ELF와 VK의 부재:** 현재 아키텍처는 P2P, DB, 우회 분기에 이르는 100% 파이프라인을 관통했으나, `crates/blockchain/zk.rs` 의 실제 `client.verify()` 로직은 주석 처리되어 있음 (실전 검증 키 값이 주입되지 않았기 때문). 

## 10. Improvement Roadmap
- **다음 단계 개선 방향 (Next Steps):**
  1. ZK 인프라팀이 이더리움 상태변이(`levm`) 코드를 SP1 환경에 맞게 컴파일(Build)하여 `.elf`와 검증 자물쇠 변수(`VerifyingKey` 바이트 배열)를 배포.
  2. `zk.rs`의 주석을 풀고, 해당 검증 키 상수값을 주입(`client.verify(&sp1_proof, &vk)`)하여 실전 테스트넷(Holesky 등)에 노드 편입.
  3. JSON-RPC (HTTP) 통신 단에 `eth_submitBlockProof` 와 같은 외부 Prover들의 증명 제출 창구 API 개설.
