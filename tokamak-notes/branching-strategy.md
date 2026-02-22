# Tokamak ethrex 브랜치 전략

## 개요

lambdaclass/ethrex를 포크한 tokamak-network/ethrex의 브랜치 관리 전략.
upstream 동기화를 유지하면서 L1, L2, ZK, 새 모듈 전반에 걸친 Tokamak 특화 개발을 진행한다.

**핵심 원칙: `main`은 upstream의 깨끗한 미러로 유지하고, Tokamak 코드는 `tokamak` 브랜치 계열에서 관리한다.**

이렇게 하면:
- upstream과 Tokamak 코드가 명확히 분리됨
- upstream 동기화가 단순해짐 (`main`에서 바로 pull)
- 나중에 완전 독립 시 `tokamak` → 새 `main`으로 전환하면 끝
- upstream에 기여(PR)할 때 `main` 기반으로 깔끔하게 가능

## 브랜치 구조

```
upstream/main (lambdaclass)
    │
    ▼ (주기적 fast-forward)
main ─────────────────────────────────── upstream 미러 (순수 upstream 코드)
    │
    ▼ (주기적 머지)
tokamak ──────────────────────────────── Tokamak 안정 브랜치 (배포 가능 상태)
    │
    ├── tokamak-dev ──────────────────── Tokamak 통합 개발 브랜치
    │       │
    │       ├── feat/l1/xxx ─────────── L1 기능 개발
    │       ├── feat/l2/xxx ─────────── L2 기능 개발
    │       ├── feat/zk/xxx ─────────── ZK 관련 개발
    │       ├── feat/mod/xxx ────────── 새 모듈 개발
    │       │
    │       ├── fix/l1/xxx ──────────── L1 버그 수정
    │       ├── fix/l2/xxx ──────────── L2 버그 수정
    │       │
    │       ├── refactor/xxx ────────── 리팩토링
    │       ├── test/xxx ────────────── 테스트 추가/수정
    │       └── docs/xxx ────────────── 문서 작업
    │
    ├── release/vX.Y.Z ──────────────── 릴리스 준비
    └── hotfix/xxx ───────────────────── 긴급 수정 (tokamak에서 분기)
```

## 브랜치 상세

### 영구 브랜치

| 브랜치 | 용도 | 보호 규칙 |
|--------|------|-----------|
| `main` | upstream 미러. 순수 lambdaclass 코드만 유지 | direct push 금지, upstream sync만 허용 |
| `tokamak` | Tokamak 안정 버전, 배포 가능 상태 | PR 필수, 리뷰 2명 이상, CI 통과 필수 |
| `tokamak-dev` | 통합 개발 브랜치, feature 브랜치가 여기로 머지 | PR 필수, 리뷰 1명 이상, CI 통과 필수 |

### main 브랜치 규칙

`main`은 **upstream 전용**이다:
- Tokamak 코드를 직접 커밋하지 않는다
- upstream sync 외에는 변경하지 않는다
- 이렇게 유지하면 `git diff main..tokamak`로 **Tokamak이 변경한 모든 것**을 한눈에 볼 수 있다

### 작업 브랜치 네이밍

```
<type>/<scope>/<short-description>
```

**type:**
- `feat` : 새 기능
- `fix` : 버그 수정
- `refactor` : 리팩토링
- `test` : 테스트
- `docs` : 문서
- `chore` : 빌드, CI, 설정 등

**scope:**
- `l1` : L1 (실행 클라이언트) 관련
- `l2` : L2 (롤업, 시퀀서, 프로포저 등) 관련
- `zk` : ZK 프루버/검증 관련
- `mod` : 새 모듈/크레이트 추가
- `infra` : CI/CD, Docker, 인프라
- `common` : 공통 라이브러리, 유틸리티
- scope가 명확하지 않으면 생략 가능

**예시:**
```
feat/l2/custom-sequencer-logic
fix/zk/prover-memory-leak
feat/mod/tokamak-bridge
refactor/l1/storage-optimization
chore/infra/ci-docker-cache
```

### 특수 브랜치

| 브랜치 | 분기점 | 머지 대상 | 용도 |
|--------|--------|-----------|------|
| `release/vX.Y.Z` | `tokamak-dev` | `tokamak` + `tokamak-dev` | 릴리스 준비, QA, 버전 태깅 |
| `hotfix/xxx` | `tokamak` | `tokamak` + `tokamak-dev` | 프로덕션 긴급 수정 |
| `upstream-contrib/xxx` | `main` | upstream PR 전용 | upstream에 기여할 때 사용 |

## Upstream 동기화 전략

### 동기화 흐름

```
upstream/main
    │
    ▼ fast-forward
main (항상 upstream과 동일)
    │
    ▼ merge into tokamak-dev (충돌 해결)
tokamak-dev
    │
    ▼ 안정 확인 후
tokamak
```

### 동기화 절차

```bash
# 1. upstream 최신화 → main 반영
git fetch upstream
git checkout main
git merge upstream/main        # fast-forward (충돌 없어야 정상)
git push origin main

# 2. tokamak-dev에 main 머지
git checkout tokamak-dev
git merge main                 # 여기서 충돌 해결
# 충돌 해결 후 커밋

# 3. PR 생성: tokamak-dev → tokamak (안정 확인 후)
```

### 동기화 주기
- **권장**: 2주에 1회 (또는 upstream에 중요 변경이 있을 때)
- **담당**: 로테이션 또는 지정 담당자
- **주의**: `main`은 항상 fast-forward만. 충돌 해결은 `tokamak-dev`에서.

### Upstream에 기여할 때

```bash
# main (= 순수 upstream) 에서 브랜치 생성
git checkout main
git checkout -b upstream-contrib/fix-block-validation

# 작업 후 upstream에 PR 생성
# Tokamak 코드가 섞이지 않으므로 깔끔한 PR 가능
```

## 나중에 완전 분리할 때

```bash
# tokamak 브랜치가 곧 새로운 main
git branch -m main upstream-archive   # 기존 main 보관
git branch -m tokamak main            # tokamak → main 승격
git remote remove upstream            # upstream 연결 해제
```

`git diff upstream-archive..main`으로 Tokamak이 변경한 전체 내역을 확인 가능.

## 워크플로우

### 일반 기능 개발

```
1. tokamak-dev에서 feature 브랜치 생성
   git checkout tokamak-dev
   git checkout -b feat/l2/custom-sequencer

2. 작업 후 커밋 (Conventional Commits)
   git commit -m "feat(l2): add custom sequencer logic"

3. PR 생성 → tokamak-dev
   - 리뷰어 지정 (해당 영역 담당자)
   - CI 통과 확인

4. 리뷰 승인 후 Squash Merge
```

### 릴리스

```
1. tokamak-dev에서 release 브랜치 생성
   git checkout tokamak-dev
   git checkout -b release/v0.1.0

2. 버전 번호 업데이트, 최종 QA

3. PR → tokamak (리뷰 2명)
4. tokamak에 태그: v0.1.0
5. release 브랜치를 tokamak-dev에도 머지 (버전 변경 반영)
```

### 긴급 수정

```
1. tokamak에서 hotfix 브랜치 생성
   git checkout tokamak
   git checkout -b hotfix/critical-crash-fix

2. 수정 후 PR → tokamak + tokamak-dev
```

## 커밋 메시지 컨벤션

[Conventional Commits](https://www.conventionalcommits.org/) 준수:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**예시:**
```
feat(l2): add Tokamak custom deposit handling
fix(zk): resolve prover OOM on large batches
refactor(l1): simplify block validation pipeline
docs(common): update API documentation for bridge module
chore(infra): add prover benchmark CI job
```

## PR 규칙

- **tokamak-dev로의 PR**: 리뷰어 최소 1명, CI 통과 필수
- **tokamak으로의 PR**: 리뷰어 최소 2명, CI 통과 필수, tokamak-dev에서만 머지
- **main**: 직접 PR 금지. upstream sync만 허용
- **PR 제목**: 커밋 컨벤션과 동일한 형식
- **PR 본문**: 변경 사항 요약, 관련 이슈 링크, 테스트 계획 포함

## 영역별 코드 오너십 (참고)

| 영역 | 디렉토리 (예상) | 담당 |
|------|-----------------|------|
| L1 실행 클라이언트 | `crates/blockchain/`, `crates/networking/` | TBD |
| L2 롤업 | `crates/l2/` | TBD |
| ZK 프루버 | `crates/l2/prover/` | TBD |
| 새 모듈 | `crates/tokamak-*` (신규) | TBD |
| 인프라/CI | `.github/`, `docker/`, `scripts/` | TBD |

> CODEOWNERS 파일을 설정하면 PR 시 자동으로 리뷰어가 지정된다.

## 브랜치 생명주기

- **feature/fix 브랜치**: 머지 후 삭제
- **release 브랜치**: 릴리스 완료 후 삭제
- **hotfix 브랜치**: 머지 후 삭제
- **upstream-contrib 브랜치**: upstream PR 완료 후 삭제
- **main, tokamak, tokamak-dev**: 영구 유지

## 요약 비교

| | 이전 구조 | 현재 구조 |
|---|---|---|
| `main` | Tokamak + upstream 혼합 | upstream 미러 (순수) |
| Tokamak 안정 | `main` | `tokamak` |
| 개발 통합 | `develop` | `tokamak-dev` |
| upstream 동기화 | sync 브랜치 → develop → main | main fast-forward → tokamak-dev 머지 |
| 분리 시 | 어려움 (코드 분리 필요) | 쉬움 (tokamak → main 이름 변경) |
| upstream 기여 | 어려움 (커스텀 코드 섞임) | 쉬움 (main 기반으로 깨끗한 PR) |
