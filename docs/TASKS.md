---
생성일: 2026-02-22
아이디어: Geth에서 ethrex로 안전하게 마이그레이션하는 CLI MVP를 설계하고 싶습니다. 핵심은 데이터 변환, 무결성 검증, 롤백 지원입니다.
버전: 1.0
프로젝트 유형: cli
---

### T-001: Geth 데이터 디렉토리 구조 분석 및 스키마 문서화
- **의존성**: 없음
- **예상 시간**: 8시간
- **입력**: Geth 데이터 디렉토리 (예: `~/.ethereum/chaindata`, `~/.ethereum/keystore`, `~/.ethereum/geth.ipc`)
- **출력**: `docs/geth-data-schema.md`, `docs/geth-storage-layout.json`
- **수락 기준**:
  - [ ] Geth의 `chaindata` 내 LevelDB 키/값 구조를 100% 분석하여 테이블별 데이터 형식 기록
  - [ ] keystore 파일 형식 (UTC-JSON) 및 암호화 방식(Scrypt)을 명세화
  - [ ] IPC 소켓 및 로그 파일의 역할을 포함한 전체 아키텍처 다이어그램 포함
- **구현 가이드**: 
  - `geth` 실행 중 `debug.dumpBlock(n)` 및 `admin.nodeInfo`로 동작 확인
  - `leveldb`를 `go-leveldb` 라이브러리로 읽어 키 패턴 분석 (예: `0x00...` → 블록, `0x01...` → 상태)
  - `chaindata/LOG` 파일 분석하여 로그 구조 파악
  - 결과를 Markdown과 JSON으로 출력, JSON은 다음 구조: `{ "tables": [{ "name": "blocks", "keys": ["prefix:00", "suffix:01"], "fields": ["number", "hash", "parentHash", ...] }] }`

---

### T-002: Ethrex 데이터 스키마 정의 및 변환 매핑 테이블 생성
- **의존성**: T-001
- **예상 시간**: 6시간
- **입력**: `docs/geth-data-schema.md`, `docs/geth-storage-layout.json`
- **출력**: `docs/ethrex-data-mapping.md`, `config/mapping-rules.yaml`
- **수락 기준**:
  - [ ] Geth의 각 테이블이 Ethrex의 어떤 스토리지 엔티티(예: `blocks`, `states`, `receipts`)에 매핑되는지 명시
  - [ ] 데이터 타입 변환 규칙 (예: Geth의 `big.Int` → Ethrex의 `u256`) 포함
  - [ ] 비호환 필드(예: Geth의 `extraData`)에 대한 처리 전략 기록
- **구현 가이드**:
  - Ethrex 공식 문서 및 소스 코드(https://github.com/ethrex/ethrex)에서 스토리지 스키마 추출
  - 매핑 규칙을 YAML로 정의:  
    ```yaml
    - from: "chaindata:00" # Geth 블록 테이블
      to: "ethrex.blocks"
      transform: "decodeRLP -> serializeJson"
      fields:
        number: "number"
        hash: "hash"
        parentHash: "parent_hash"
        extraData: "null" # Ethrex에서 제거
    ```
  - 비호환 필드는 `mapping-rules.yaml`에 `fallback: discard` 또는 `fallback: legacy`로 명시

---

### T-003: Geth → Ethrex 데이터 변환 파이프라인 구현 (핵심 변환기)
- **의존성**: T-002
- **예상 시간**: 20시간
- **입력**: Geth 데이터 디렉토리 (`~/.ethereum/chaindata`, `keystore`)
- **출력**: 변환된 Ethrex 데이터 디렉토리 (`/var/lib/ethrex/data/chaindata`, `/var/lib/ethrex/keystore`)
- **수락 기준**:
  - [ ] 블록, 상태, 수신, 트랜잭션 데이터가 100% 변환되어 Ethrex 형식으로 저장
  - [ ] keystore 파일은 암호화 방식 유지하되, Ethrex 호환 키 스토어 구조로 복사
  - [ ] 변환 중 발생하는 모든 비정상 키는 `logs/failed-keys.log`에 기록
- **구현 가이드**:
  - Go로 구현: `cmd/convert/main.go`
  - LevelDB 읽기: `github.com/syndtr/goleveldb/leveldb`
  - 변환 로직: 각 테이블에 대해 `mapping-rules.yaml`을 로드하고 `transformer` 함수 적용
  - 예: `00` 키 → 블록 RLP 디코딩 → JSON으로 직렬화 → Ethrex의 `blockstore`에 저장
  - keystore: `crypto/keystore` 패키지 사용해 파일 복사 + 헤더 업데이트 (Ethrex는 `UTC--` 접두사 동일)
  - 실패한 키는 `failed-keys.log`에 `(key, reason, timestamp)`로 기록

---

### T-004: 마이그레이션 무결성 검증 모듈 구현
- **의존성**: T-003
- **예상 시간**: 10시간
- **입력**: 원본 Geth 데이터 디렉토리, 변환된 Ethrex 데이터 디렉토리
- **출력**: `reports/integrity-check-report.json`, `reports/integrity-check-summary.txt`
- **수락 기준**:
  - [ ] 블록 수, 총 트랜잭션 수, 상태 루트 해시가 원본과 일치 여부 검증
  - [ ] 블록 해시 체인의 연속성 검증 (parentHash 연결)
  - [ ] 최종 상태 루트(merkle root)가 일치하는지 검증 (Ethrex에서 `stateRoot` 추출 가능)
- **구현 가이드**:
  - Geth에서 `eth.blockNumber`, `eth.getBalance`, `eth.getStorageAt`로 샘플 검증
  - Ethrex에서 동일한 API 호출 가능하도록 임시 노드 실행 (`ethrex --datadir /var/lib/ethrex --rpc`)
  - 검증 항목:  
    - 블록 개수 차이 < 1  
    - 마지막 블록 해시 일치  
    - 10개 무작위 주소의 잔액 일치  
    - 상태 루트 일치 (Ethrex의 `stateRoot` vs Geth의 `root` in block header)
  - 결과를 JSON으로 출력: `{ "passed": true, "checks": [...], "mismatches": [] }`

---

### T-005: 롤백 기능 구현 (원본 복구 및 상태 되돌리기)
- **의존성**: T-003
- **예상 시간**: 8시간
- **입력**: 변환 전 Geth 데이터 디렉토리, 변환 중 생성된 백업 파일
- **출력**: 원본 Geth 데이터 디렉토리 복구, `logs/rollback-<timestamp>.log`
- **수락 기준**:
  - [ ] 마이그레이션 실패 시, Geth 데이터 디렉토리가 변환 전 상태로 완전 복구
  - [ ] 백업은 `~/.ethereum/backup/` 하위에 압축된 tar.gz 형식으로 생성
  - [ ] 롤백 명령어는 `ethrex-migrate --rollback`으로 실행 가능
- **구현 가이드**:
  - 변환 시작 전, `tar -czf ~/.ethereum/backup/chaindata-<timestamp>.tar.gz ~/.ethereum/chaindata`
  - 동일하게 keystore도 백업
  - `--rollback` 옵션 시:  
    1. Ethrex 데이터 디렉토리 삭제  
    2. 백업 압축 파일 해제  
    3. `~/.ethereum/chaindata` 및 `keystore` 복원  
    4. `logs/rollback-<timestamp>.log`에 시간, 실행자, 원인 기록
  - 백업은 최대 3개만 유지 (LRU)

---

### T-006: 진행 상황 로깅 및 CLI 인터페이스 구현
- **의존성**: T-003, T-004, T-005
- **예상 시간**: 8시간
- **입력**: 사용자 명령어 (`ethrex-migrate --from /path/to/geth --to /path/to/ethrex`)
- **출력**: CLI 콘솔 로그, `logs/migration-<timestamp>.log`
- **수락 기준**:
  - [ ] 진행률 바(percentage) 및 예상 남은 시간 실시간 출력
  - [ ] 각 단계(변환, 검증, 롤백)의 시작/종료 시간 기록
  - [ ] 로그 파일은 JSONL 형식으로 저장 (각 행은 { "ts": "...", "level": "INFO", "msg": "...", "stage": "convert" })
- **구현 가이드**:
  - Go의 `github.com/sirupsen/logrus` 사용
  - CLI 프레임워크: `github.com/spf13/cobra`
  - 진행률: 변환 단계에서 처리된 키 수 / 총 키 수 → `fmt.Fprintf`로 실시간 업데이트
  - 로그 형식 예:  
    ```json
    {"ts":"2025-04-05T10:00:00Z","level":"INFO","msg":"Converted 125000 blocks","stage":"convert","progress":0.62}
    ```
  - `--verbose` 옵션 시 디버그 로그 출력

---

### T-007: 에러 처리 및 재시도 메커니즘 구현
- **의존성**: T-003, T-006
- **예상 시간**: 10시간
- **입력**: 변환 중 발생한 에러 (파일 읽기 실패, LevelDB 손상, 네트워크 장애 등)
- **출력**: `logs/errors.json`, 재시도 후 성공/실패 상태
- **수락 기준**:
  - [ ] 임시 I/O 오류(예: EAGAIN, ENOSPC)는 최대 3회 재시도
  - [ ] 영구 오류(예: corrupt LevelDB, invalid key)는 중단 후 `errors.json`에 기록
  - [ ] 재시도 간 대기 시간은 지수 백오프 (1s, 2s, 4s)
- **구현 가이드**:
  - 모든 I/O 및 DB 접근을 `retry.Do()` 함수로 감쌈  
    ```go
    retry.Do(func() error {
        return db.Get(key)
    }, retry.Attempts(3), retry.Delay(1*time.Second), retry.DelayType(retry.BackOffDelay))
    ```
  - 에러 유형 분류:  
    - `Transient`: `EAGAIN`, `ETIMEDOUT`, `ENOSPC` → 재시도  
    - `Fatal`: `corrupt data`, `invalid encoding`, `permission denied` → 중단
  - `errors.json` 구조:  
    ```json
    [
      {
        "timestamp": "...",
        "stage": "convert",
        "key": "0x000123...",
        "error": "leveldb: corrupted block",
        "type": "fatal"
      }
    ]
    ```
  - 재시도 실패 시 `--continue-on-error` 옵션 없으면 프로세스 종료

---

### T-008: SYSTEM_BASELINE 파싱 실패(degraded mode) 구현
- **의존성**: T-003, T-007
- **예상 시간**: 6시간
- **입력**: Geth 데이터 디렉토리 (일부 손상됨)
- **출력**: 부분 변환된 Ethrex 데이터, `logs/degraded-mode-<timestamp>.log`
- **수락 기준**:
  - [ ] LevelDB 손상으로 인해 일부 키를 읽지 못하더라도, 정상 키는 변환하여 Ethrex에 저장
  - [ ] 손상된 키는 `logs/degraded-mode-<timestamp>.log`에 기록하고, `--degraded` 모드로 실행됨을 명시
  - [ ] 전체 마이그레이션이 실패하지 않고, “degraded” 상태로 완료됨
- **구현 가이드**:
  - `geth`의 LevelDB 읽기 시 `leveldb.ErrCorrupted` 또는 `io.ErrUnexpectedEOF` 발생 시,  
    `retry` 대신 `continue`로 다음 키로 이동
  - `--degraded` 플래그가 설정되면, 모든 오류를 경고로 처리하고 프로세스 계속
  - `degraded-mode-<timestamp>.log`에:  
    `DEGRADED: skipped 12 keys due to corruption in chaindata/00`  
  - 최종 보고서에서 `status: degraded`로 표시
  - Ethrex 데이터는 일부 누락되더라도 실행 가능 상태여야 함 (예: 최신 블록은 정상)

---

### T-009: CLI 패키지 빌드 및 배포 구조 정의
- **의존성**: T-001~T-008
- **예상 시간**: 4시간
- **입력**: Go 소스 코드 전체
- **출력**: `dist/ethrex-migrate-v1.0.0-linux-amd64`, `dist/ethrex-migrate-v1.0.0-darwin-arm64`, `README.md`
- **수락 기준**:
  - [ ] Go 1.21+로 빌드된 정적 바이너리 2개 이상의 플랫폼 제공
  - [ ] `README.md`에 설치, 실행, 롤백, 오류 처리 예제 포함
  - [ ] `--help` 명령어로 모든 플래그 및 사용법 문서화
- **구현 가이드**:
  - `go build -ldflags="-s -w" -o dist/ethrex-migrate-$(GOOS)-$(GOARCH)`
  - `README.md` 예시:  
    ```bash
    ethrex-migrate --from ~/.ethereum --to /var/lib/ethrex --degraded
    ethrex-migrate --rollback
    ```
  - `cobra`로 자동 생성된 help 문서 포함
  - GitHub Actions로 CI/CD 설정 (선택사항이지만 권장)

---

### T-010: 통합 테스트 및 시나리오 검증
- **의존성**: T-009
- **예상 시간**: 12시간
- **입력**: 샘플 Geth 데이터 (1000 블록 규모), 테스트 스크립트
- **출력**: 테스트 리포트 `test/reports/integration-report.json`
- **수락 기준**:
  - [ ] 정상 마이그레이션: 성공률 100%
  - [ ] degraded 모드: 5% 손상 데이터 입력 시 95% 데이터 변환 성공
  - [ ] 롤백: 실패 후 롤백 실행 → 원본 복구 확인
  - [ ] 재시도: 네트워크 장애 시뮬레이션 → 3회 재시도 후 성공
- **구현 가이드**:
  - `test/fixtures/`에 작은 Geth 데이터셋 준비 (Docker로 geth 실행 후 `--datadir` 복사)
  - Go 테스트 파일: `cmd/ethrex-migrate/integration_test.go`
  - 테스트 시나리오:  
    1. 정상 변환 → 검증 → 삭제 → 롤백  
    2. 손상된 chaindata 생성 → `--degraded` 실행 → 실패 키 수 검증  
    3. `--rollback` 실행 → 원본 복원 확인  
    4. `mockfs`로 `ENOSPC` 에러 시뮬레이션 → 재시도 동작 검증
  - 결과는 JSON으로 출력: `{ "scenario": "normal", "passed": true, "duration": 45 }`

---

**총 예상 시간**: 92시간  
**최종 산출물**: CLI 바이너리, 문서, 테스트, 로그 구조, 배포 패키지  
**모든 태스크는 독립적으로 테스트 가능하며, CI/CD 파이프라인에 통합 가능**