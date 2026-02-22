---
생성일: 2026-02-22
아이디어: Geth에서 ethrex로 안전하게 마이그레이션하는 CLI MVP를 설계하고 싶습니다. 핵심은 데이터 변환, 무결성 검증, 롤백 지원입니다.
버전: 1.0
프로젝트 유형: cli
---

```markdown
# Geth → Ethrex 마이그레이션 CLI MVP 프로젝트 디렉토리 구조

## 전체 트리 구조

```
ethrex-migration-cli/
├── bin/
│   └── ethrex-migrate           # 실행 가능한 CLI 진입점
├── src/
│   ├── core/
│   │   ├── migration_engine.py  # 마이그레이션 로직 중심 제어
│   │   ├── validator.py         # 데이터 무결성 검증 로직
│   │   ├── rollback_manager.py  # 롤백 상태 관리 및 실행
│   │   └── transformer.py       # Geth 데이터 → Ethrex 형식 변환
│   │
│   ├── data/
│   │   ├── geth/
│   │   │   ├── chaindata_parser.py  # Geth chaindata 파싱 (leveldb/rocksdb)
│   │   │   ├── state_parser.py      # Geth 상태 트리 파싱
│   │   │   └── config_loader.py     # Geth genesis 및 node 설정 로드
│   │   │
│   │   └── ethrex/
│   │       ├── chaindata_builder.py # Ethrex 형식의 chaindata 생성
│   │       ├── state_builder.py     # Ethrex 상태 트리 구성
│   │       └── genesis_builder.py   # Ethrex genesis.json 생성
│   │
│   ├── storage/
│   │   ├── snapshot_manager.py    # 마이그레이션 중간 상태 스냅샷 저장/복원
│   │   ├── checkpoint_store.py    # 체크포인트 메타데이터 저장 (JSON/SQLite)
│   │   └── backup_manager.py      # 원본 Geth 데이터 백업 (읽기 전용)
│   │
│   ├── utils/
│   │   ├── logger.py              # 통일된 로깅 시스템
│   │   ├── cli_formatter.py       # CLI 출력 포맷팅 (progress bar, color, table)
│   │   ├── crypto.py              # Keccak256, RLP, ECDSA 등 암호학 유틸
│   │   └── config.py              # CLI 설정 및 환경 변수 관리
│   │
│   ├── errors/
│   │   ├── exceptions.py          # 사용자 정의 예외 정의
│   │   └── error_codes.py         # 에러 코드 및 메시지 매핑
│   │
│   └── main.py                    # 앱 진입점 (CLI 인자 파싱 및 core 호출)
│
├── tests/
│   ├── unit/
│   │   ├── test_transformer.py
│   │   ├── test_validator.py
│   │   ├── test_rollback_manager.py
│   │   └── test_chaindata_parser.py
│   │
│   ├── integration/
│   │   ├── test_migration_flow.py     # 실제 Geth 데이터 샘플로 통합 테스트
│   │   └── test_rollback_safety.py
│   │
│   └── fixtures/
│       ├── sample_geth_chaindata/     # 테스트용 Geth chaindata 샘플
│       └── expected_ethrex_output/    # 기대되는 Ethrex 출력 구조
│
├── config/
│   ├── migration.yaml               # 마이그레이션 설정 (변환 규칙, 스케일, 제외 주소 등)
│   └── logging.yaml                 # 로그 레벨 및 포맷 설정
│
├── docs/
│   ├── architecture.md              # 아키텍처 다이어그램 및 설계 원칙
│   ├── migration_protocol.md        # 변환 프로토콜 상세 사양 (RLP, Trie, Account)
│   └── usage_guide.md               # CLI 사용법 및 예시
│
├── requirements.txt                 # Python 의존성 목록
├── pyproject.toml                   # 빌드 및 패키징 설정 (Poetry 기준)
├── README.md                        # 프로젝트 개요, 설치, 실행 가이드
├── LICENSE                          # 라이선스
└── .gitignore                       # Git 무시 파일 목록
```

---

## 각 디렉토리/파일의 역할 설명

### `bin/`
- **`ethrex-migrate`**: 실행 가능한 CLI 스크립트. `python -m src.main`을 실행하거나, `setuptools`로 패키징 후 `entry_points`로 바인딩됨. 사용자는 이 파일만으로 CLI를 실행.

### `src/core/`
- **`migration_engine.py`**: 마이그레이션의 전체 흐름을 제어하는 오케스트레이터. 단계별 실행(백업 → 파싱 → 변환 → 검증 → 저장 → 롤백 가능 상태 저장)을 관리.
- **`validator.py`**: 변환된 Ethrex 데이터가 원본 Geth 데이터와 동일한 상태를 유지하는지 검증. Merkle Proof, 상태 해시 비교, 계정 수, 잔액, nonce 일치 확인.
- **`rollback_manager.py`**: 마이그레이션 중간에 실패 시, 이전 상태로 복원. 스냅샷과 체크포인트를 기반으로 롤백 실행. 롤백 가능 여부를 체크포인트에 기록.
- **`transformer.py`**: Geth의 `chaindata`와 `state`를 Ethrex 형식으로 변환. RLP 인코딩, Trie 구조 재구성, 계정 형식 변환(예: Geth의 `stateRoot` → Ethrex의 `stateRootV2`) 처리.

### `src/data/geth/`
- **`chaindata_parser.py`**: Geth의 LevelDB/ RocksDB 내부 `chaindata/` 디렉토리에서 블록, 트랜잭션, 수신 상태를 파싱. `geth` 전용 데이터 구조 해석.
- **`state_parser.py`**: Geth 상태 트리(Merkle Patricia Trie)를 읽어 계정, 코드, 스토리지 항목을 추출. `state/` 디렉토리의 키-값 쌍 해석.
- **`config_loader.py`**: `geth/genesis.json`, `nodekey`, `chainid` 등을 로드하여 변환 규칙에 반영.

### `src/data/ethrex/`
- **`chaindata_builder.py`**: 변환된 블록 및 트랜잭션을 Ethrex가 이해하는 형식으로 저장. `chaindata/` 구조 생성.
- **`state_builder.py`**: Ethrex용 상태 트리를 재구성. Geth와 다른 Trie 구조(예: Solidity 0.8+ 형식)에 맞춰 빌드.
- **`genesis_builder.py`**: Ethrex 전용 `genesis.json` 생성. Geth genesis을 기반으로 `config`, `alloc`, `difficulty`, `homesteadBlock` 등을 Ethrex 사양에 맞게 조정.

### `src/storage/`
- **`snapshot_manager.py`**: 마이그레이션 중간 상태(예: 1000블록까지 변환 완료)를 압축된 바이너리 스냅샷으로 저장. 롤백 시 복원.
- **`checkpoint_store.py`**: 마이그레이션 진행 상태(현재 블록 번호, 검증 결과, 롤백 가능 여부)를 SQLite 또는 JSON 파일로 기록.
- **`backup_manager.py`**: 원본 Geth 데이터를 읽기 전용으로 백업(복사). 변환 중 원본 손상 방지.

### `src/utils/`
- **`logger.py`**: `logging` 모듈을 감싸서 CLI용 형식(진행률, 색상, 에러 아이콘)으로 로그 출력.
- **`cli_formatter.py`**: `rich` 또는 `tqdm` 기반의 프로그레스 바, 테이블, 상태 메시지 렌더링.
- **`crypto.py`**: 암호학 유틸리티. Keccak256 해시, RLP 인코딩/디코딩, 주소 생성, 서명 검증 등. Geth와 Ethrex 간 호환성 확보.
- **`config.py`**: CLI 인자, 환경 변수, `config/migration.yaml`을 통합하여 설정 객체로 제공.

### `src/errors/`
- **`exceptions.py`**: `MigrationFailedError`, `IntegrityViolationError`, `RollbackNotPossibleError` 등 사용자 정의 예외.
- **`error_codes.py`**: 에러 코드 매핑 (`E001`, `E002` 등)과 사용자 친화적 메시지. 로그 및 CLI 출력에 사용.

### `tests/`
- **unit/**: 단위 테스트. 각 모듈의 독립적 기능 검증.
- **integration/**: 실제 Geth 데이터 샘플을 사용해 전체 마이그레이션 흐름을 테스트. 롤백 시나리오 포함.
- **fixtures/**: 테스트용 Geth 데이터 및 기대 출력. CI/CD에서 재현성 확보.

### `config/`
- **`migration.yaml`**: 마이그레이션 설정. 예: `skip_accounts: ["0x..."]`, `max_blocks: 100000`, `enable_rollback: true`.
- **`logging.yaml`**: 로그 레벨, 파일 출력 여부, 포맷 정의. 개발/프로덕션 환경 분리 가능.

### `docs/`
- **`architecture.md`**: 시스템 아키텍처 다이어그램(모듈 간 의존성), 마이그레이션 흐름 다이어그램 포함.
- **`migration_protocol.md`**: Geth ↔ Ethrex 데이터 형식 차이점, 변환 규칙의 수학적/기술적 정의.
- **`usage_guide.md`**: `ethrex-migrate --source=/path/to/geth --target=/path/to/ethrex --dry-run` 같은 CLI 사용법.

### 기타
- **`requirements.txt`**: `web3`, `pyrlp`, `plyvel`, `rich`, `pyyaml`, `pytest` 등 의존성.
- **`pyproject.toml`**: Poetry 기반 패키징. CLI 진입점 등록.
- **`README.md`**: 설치, 실행, 테스트, 기여 가이드. 최초 사용자 경험 중심.
- **`LICENSE`**: Apache 2.0 또는 MIT로 설정 권장.
- **`.gitignore`**: `__pycache__`, `.env`, `*.log`, `*.snapshot`, `venv/` 등 무시.

---

## 핵심 파일의 모듈 책임

| 파일 | 책임 | 핵심 로직 |
|------|------|-----------|
| **`src/core/migration_engine.py`** | 마이그레이션 오케스트레이션 | 1. 백업 시작 → 2. Geth 파싱 → 3. 변환 → 4. 검증 → 5. Ethrex 저장 → 6. 체크포인트 기록 → 7. 성공 시 롤백 가능 상태 설정. 실패 시 롤백 요청 처리. |
| **`src/core/transformer.py`** | 데이터 형식 변환 | Geth의 `Account` 구조(`nonce`, `balance`, `storageRoot`, `codeHash`) → Ethrex의 `AccountV2` 구조로 매핑. Trie 노드 재구성 및 RLP 인코딩 방식 조정. |
| **`src/core/validator.py`** | 무결성 검증 | - 상태 트리 루트 해시 비교<br>- 계정 수, 총 잔액, 트랜잭션 수 일치 검증<br>- Merkle Proof 검증을 통한 특정 계정 상태 검증 |
| **`src/core/rollback_manager.py`** | 롤백 지원 | - 스냅샷과 체크포인트 로드<br>- Ethrex 생성된 데이터 삭제<br>- Geth 백업 복원<br>- 롤백 가능 여부는 `checkpoint_store`에 `rollback_enabled: true`로 기록 |
| **`src/data/geth/chaindata_parser.py`** | Geth 데이터 파싱 | LevelDB에서 `0x` 키로 저장된 블록/트랜잭션을 읽고, `blockNumber` 기준 정렬. `Block` 객체 생성. |
| **`src/data/ethrex/chaindata_builder.py`** | Ethrex 데이터 생성 | 변환된 블록을 Ethrex가 요구하는 디렉토리 구조(`blocks/`, `receipts/`)에 저장. DB 쓰기 최적화. |
| **`src/storage/snapshot_manager.py`** | 상태 스냅샷 | `pickle` 또는 `msgpack`로 변환 중간 상태 저장. 압축 후 `snapshots/` 디렉토리에 저장. 복원 시 복원 지점 재개 가능. |
| **`src/utils/crypto.py`** | 암호학 유틸 | `keccak256`, `rlp.encode/decode`, `address_from_pubkey`, `hash_transaction` 등 Geth와 Ethrex 간 호환성 보장. |
| **`src/main.py`** | CLI 진입점 | `argparse`로 인자 파싱 → `config.py` 로드 → `migration_engine` 실행 → 결과 출력 및 종료 코드 반환. |

---

> ✅ **설계 철학**:  
> - **안전성**: 모든 변환 전 백업, 롤백 가능 상태 유지.  
> - **모듈화**: 각 컴포넌트는 독립 테스트 가능.  
> - **확장성**: Ethrex 형식이 변경되어도 `transformer.py`와 `ethrex/` 디렉토리만 수정.  
> - **사용자 친화성**: CLI는 진행률, 에러 코드, 로그, 롤백 옵션을 명확히 제공.  
> - **재현성**: 테스트용 fixtures로 CI/CD 파이프라인 구축 가능.
```

이 구조는 **MVP 단계**에서부터 **프로덕션 배포**까지 확장 가능하도록 설계되었으며, Geth와 Ethrex 간의 복잡한 데이터 변환을 안전하게 처리할 수 있는 기반을 제공합니다.