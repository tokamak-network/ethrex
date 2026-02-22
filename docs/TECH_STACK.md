---
생성일: 2026-02-22
아이디어: Geth에서 ethrex로 안전하게 마이그레이션하는 CLI MVP를 설계하고 싶습니다. 핵심은 데이터 변환, 무결성 검증, 롤백 지원입니다.
버전: 1.0
프로젝트 유형: cli
---

```markdown
# Geth → Ethrex 마이그레이션 CLI MVP 기술 스택 분석 및 추천

## 1. 카테고리별 기술 선택

| 카테고리 | 선택 기술 | 버전 권장사항 |
|----------|-----------|----------------|
| **프로그래밍 언어** | Python 3.11+ | 3.11.6+ |
| **CLI 프레임워크** | Typer + Click | Typer 0.11.0+, Click 8.1.7+ |
| **이더리움 데이터 처리** | Web3.py | 6.15.0+ |
| **데이터 변환/직렬화** | Pydantic | 2.7.0+ |
| **데이터 저장/상태 관리** | SQLite3 (내장) | Python 내장 |
| **검증/해시** | hashlib, eth-hash | hashlib (내장), eth-hash 0.4.0+ |
| **로깅** | structlog | 24.4.0+ |
| **테스트** | pytest, pytest-cov | pytest 8.2.0+, pytest-cov 5.0.0+ |
| **패키지 관리** | Poetry | 1.8.0+ |
| **CI/CD** | GitHub Actions | 최신 안정 버전 |
| **도커** | Docker (빌드/배포) | 26.0+ |
| **형태 검사/정렬** | ruff, black, mypy | ruff 0.5.0+, black 24.4.2+, mypy 1.10.0+ |

---

## 2. 각 선택의 근거

### ✅ **Python 3.11+**
- **근거**:  
  - 이더리움 관련 라이브러리(Web3.py, eth-hash 등)의 공식 지원 언어.  
  - 3.11은 성능 향상(예: faster exception handling)과 타입 힌트 강화로 CLI 도구에 최적화.  
  - EOL이 2027년으로 장기 지원 가능.  
  - `typing` 및 `dataclass` 기반 구조가 데이터 변환 로직을 명확하게 표현 가능.

### ✅ **Typer + Click**
- **근거**:  
  - Typer는 Click 위에 구축된 현대적 CLI 프레임워크로, **타입 힌트 기반 자동 파싱**과 **자동 문서 생성** 기능 제공.  
  - `@app.command()` 구조로 명령어가 직관적이고 테스트 가능.  
  - Click은 안정성과 커뮤니티 지원이 뛰어나며, Typer는 이를 상속하여 개발 생산성 극대화.  
  - `--help`, 자동 완성, 색상 출력 등 CLI 표준 기능을 자연스럽게 지원.

### ✅ **Web3.py**
- **근거**:  
  - Geth RPC 인터페이스(JSON-RPC)와의 통신을 위한 **공식 파이썬 라이브러리**.  
  - 블록체인 데이터(계정, 트랜잭션, 상태) 조회, 트랜잭션 해시 생성, 블록 해시 검증 등 모든 기능 제공.  
  - `eth-account` 모듈을 통해 개인키 기반 서명 및 계정 생성 지원 → Ethrex 마이그레이션 시 계정 복사에 필수.  
  - 활발한 유지보수 및 문서화.

### ✅ **Pydantic**
- **근거**:  
  - 데이터 변환 과정에서 **구조화된 스키마 정의**와 **자동 검증**이 핵심.  
  - Geth의 `geth dump` 또는 `etherscan export` 형식 → Ethrex의 `ethrex import` 형식으로의 변환을 타입 안전하게 처리.  
  - `BaseModel`을 이용해 `Account`, `Transaction`, `Block` 등의 모델을 정의하고, 유효성 검사 + 커스텀 변환 로직 결합 가능.  
  - JSON/YAML 직렬화/역직렬화 지원 → 상태 저장 및 롤백용 시점 복원에 유용.

### ✅ **SQLite3 (내장)**
- **근거**:  
  - CLI 도구이므로 외부 DB 서버 불필요.  
  - **롤백 지원**을 위한 "마이그레이션 포인트" 저장에 최적.  
    - 예: `migration_state` 테이블에 `checkpoint_id`, `source_block_hash`, `target_block_hash`, `status`, `timestamp` 저장.  
  - 트랜잭션 지원 → `BEGIN TRANSACTION`으로 데이터 무결성 보장.  
  - 파일 기반 → 단일 실행 파일로 배포 가능.  
  - 성능: 100만 건 이하의 계정/트랜잭션 변환에는 충분.

### ✅ **hashlib + eth-hash**
- **근거**:  
  - `hashlib`은 SHA3-256, keccak256 등 이더리움에서 사용하는 해시 함수를 직접 구현.  
  - `eth-hash`는 이더리움 표준 keccak256 해시를 안전하게 계산하는 라이브러리로, Web3.py와 호환.  
  - **무결성 검증** 시:  
    - Geth 블록 해시 → 변환 후 Ethrex 블록 해시와 비교.  
    - 트랜잭션 RLP 해시 비교.  
  - 보안상 **암호화 해시 사용 필수** → MD5/SHA1 금지.

### ✅ **structlog**
- **근거**:  
  - CLI 도구에서 **디버깅 및 로그 추적**이 핵심.  
  - JSON 로그 형식 지원 → CI/CD 파이프라인에서 자동 분석 가능.  
  - 구조화된 로그: `{event: "migration_step", step: "convert_accounts", count: 12345, duration_ms: 450}`  
  - 색상 출력 + 로그 레벨 컨트롤 가능 → 사용자 경험 향상.

### ✅ **pytest + pytest-cov**
- **근거**:  
  - CLI 테스트는 함수 단위보다 **명령어 실행 흐름** 테스트가 중요.  
  - `pytest` + `cli_runner`로 CLI 명령어를 실제처럼 호출하여 테스트 가능.  
  - `pytest-cov`로 커버리지 95% 이상 유지 → 무결성 검증 로직의 신뢰성 확보.  
  - 테스트 데이터는 `fixtures`로 SQLite 임시 DB 생성 → 실제 환경과 유사한 테스트 가능.

### ✅ **Poetry**
- **근거**:  
  - Python 패키지 관리에서 **의존성 해결**과 **가상 환경 관리**가 핵심.  
  - `pyproject.toml`로 일관된 설정 → 팀 협업 및 CI/CD 통합 용이.  
  - `poetry lock`로 정확한 버전 고정 → 재현성 보장.  
  - `poetry build`로 단일 `.whl` 패키지 생성 → 배포 간편.

### ✅ **ruff + black + mypy**
- **근거**:  
  - `ruff`: 매우 빠른 형식 검사 및 linting (Black + Flake8 + isort 통합).  
  - `black`: 코드 스타일 강제 → 팀 협업 시 일관성 보장.  
  - `mypy`: 정적 타입 검사 → `Pydantic` + `Web3.py`와 결합해 **데이터 변환 오류를 컴파일 시점에 잡음**.  
  - 3개 모두 `pre-commit`에 통합 가능 → 개발자가 코드 커밋 전 자동 검사.

### ✅ **Docker + GitHub Actions**
- **근거**:  
  - CLI 도구이지만, **환경 차이로 인한 오류 방지**가 중요.  
  - Docker로 실행 환경을 고정 → 모든 사용자에게 동일한 환경 제공.  
  - GitHub Actions로:  
    - PR마다 테스트 실행  
    - `mypy`/`ruff`/`pytest` 자동 실행  
    - Docker 이미지 빌드 및 GitHub Packages에 푸시  
  - 배포용 이미지: `python:3.11-slim` 기반으로 최소화.

---

## 3. 대안 비교

| 카테고리 | 대안 | 비교 평가 | 선택 이유 |
|----------|------|-----------|-----------|
| **언어** | Go | 성능 우수, 단일 바이너리 배포 가능. 하지만 이더리움 라이브러리 생태계 부족, Web3.py 대체품(ethgo)은 미성숙. | Python 생태계 완성도, 라이브러리 지원, 개발 속도가 우선. |
| **CLI 프레임워크** | Fire, Argparse | Argparse는 기본적이지만 확장성 낮음. Fire는 자동화는 좋지만 타입 힌트 미지원. | Typer는 타입 힌트 + 문서화 + 테스트 가능성이 압도적. |
| **데이터 저장** | PostgreSQL | 롤백/트랜잭션 지원은 하지만 과도한 복잡성. CLI 도구에 불필요. | SQLite는 단일 파일, 설치 없이 실행 가능 → MVP 핵심 가치. |
| **해시 라이브러리** | pysha3 | keccak256 지원. 하지만 eth-hash가 Web3.py와 정식 통합되어 있어 호환성 우수. | eth-hash는 Web3.py의 내부 의존성과 일치 → 충돌 방지. |
| **패키지 관리** | pip + requirements.txt | 간단하지만 의존성 충돌, 버전 고정 불안정. | Poetry는 lock 파일로 재현성 보장 → 프로덕션 CLI에 필수. |
| **테스트** | unittest | Python 기본. 하지만 CLI 테스트에 불편함. | pytest의 `cli_runner`와 fixture가 훨씬 직관적. |

---

## 4. 버전 권장사항 (정확한 고정)

| 패키지 | 버전 |
|--------|------|
| python | 3.11.6 |
| typer | 0.11.0 |
| click | 8.1.7 |
| web3 | 6.15.0 |
| pydantic | 2.7.0 |
| eth-hash | 0.4.0 |
| structlog | 24.4.0 |
| pytest | 8.2.0 |
| pytest-cov | 5.0.0 |
| poetry | 1.8.0 |
| ruff | 0.5.0 |
| black | 24.4.2 |
| mypy | 1.10.0 |
| docker | 26.1.4 (빌드 환경) |

> 💡 **주의**: 모든 버전은 `poetry.lock`에 고정되어야 하며, CI/CD에서는 `poetry install --only-root`로 재현성 보장.

---

## 5. 패키지/라이브러리 목록 (`pyproject.toml` 기준)

```toml
[tool.poetry]
name = "geth-to-ethrex-migrator"
version = "0.1.0"
description = "CLI tool to safely migrate data from Geth to Ethrex"
authors = ["Your Name <you@example.com>"]
readme = "README.md"

[tool.poetry.dependencies]
python = "^3.11"
typer = "^0.11.0"
web3 = "^6.15.0"
pydantic = "^2.7.0"
eth-hash = "^0.4.0"
structlog = "^24.4.0"

[tool.poetry.group.dev.dependencies]
pytest = "^8.2.0"
pytest-cov = "^5.0.0"
ruff = "^0.5.0"
black = "^24.4.2"
mypy = "^1.10.0"
docker = "^7.1.0"  # 테스트용 도커 컨테이너 생성 시 필요 (선택)

[build-system]
requires = ["poetry-core"]
build-backend = "poetry.core.masonry.api"

[tool.ruff]
line-length = 88
target-version = "py311"

[tool.ruff.lint]
select = ["E", "W", "F", "I", "C", "R", "UP", "ANN", "B", "PT", "Q"]
ignore = ["E501"]  # black이 처리하므로

[tool.black]
line-length = 88
target-version = ["py311"]

[tool.mypy]
python_version = "3.11"
strict = true
warn_unused_configs = true
```

> ✅ **`requirements.txt` 대신 `pyproject.toml` 사용 권장**  
> Poetry는 현대 Python 프로젝트의 표준이며, `poetry export -f requirements.txt > requirements.txt`로 필요 시 생성 가능.

---

## ✅ 종합 결론

이 프로젝트는 **데이터 무결성**, **롤백 가능성**, **CLI 사용자 경험**이 핵심입니다.  
Python은 이더리움 생태계와의 호환성, 타입 안전성, 라이브러리 풍부함에서 압도적 우위를 점합니다.  
Typer + Pydantic + SQLite + structlog 조합은 **MVP 개발 속도와 신뢰성**을 동시에 달성할 수 있는 최적의 조합입니다.  
Docker + CI/CD로 배포 안정성을 확보하면, 이 CLI는 팀 내부는 물론 **이더리움 커뮤니티에 공개 가능한 프로덕션급 도구**가 될 수 있습니다.
```