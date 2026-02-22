# Task Checklist (2026-02-22)

## Goal

Produce a decision-ready project analysis deliverable that combines:

- current architecture/dependency documentation
- prioritized risk register and improvement roadmap

## Plan

- [x] Gather repository facts from workspace, crate manifests, entrypoints, and workflows.
- [x] Map L1/L2 runtime initialization and feature-gated dependency boundaries.
- [x] Identify high/medium/low architectural and operational risks with code evidence.
- [x] Author a single analysis document for developers.
- [x] Wire the new document into developer docs navigation.
- [x] Record review notes and lessons to improve future iterations.

## Step Notes

1. Repository grounding
- Inspected workspace roots and package manifests (`Cargo.toml`, `cmd/ethrex/Cargo.toml`).
- Confirmed default member and feature bundles.

2. Runtime path mapping
- Traced startup paths from `cmd/ethrex/ethrex.rs` into `init_l1` and L2 command initializers.
- Mapped boundary adapters for RPC/P2P/storage.

3. Risk extraction
- Collected line-level evidence for temporary key usage, API leakage, and feature-doc drift.
- Cross-checked local/CI testing asymmetry from `Makefile` and `.github/workflows`.

4. Deliverable production
- Added `docs/developers/project-analysis.md`.
- Added navigation links in `docs/SUMMARY.md` and `docs/developers/README.md`.

## Review

### What changed

- Added: `docs/developers/project-analysis.md`
- Updated: `docs/SUMMARY.md`
- Updated: `docs/developers/README.md`
- Added: `docs/todo.md`
- Added: `docs/lessons.md`

### Verification run

- Executed static checks and evidence gathering with:
  - `cargo metadata --no-deps --format-version 1`
  - targeted `sed`/`nl`/`rg` inspections across crates and workflows

### Verification not run

- `mdbook build` was not executed in this task.
- Full `cargo check` across all feature sets was not completed here due prior toolchain/cache lock contention during exploratory checks.

---

## Task Checklist (2026-02-22) - Idea-Based Evaluation Update

### Goal

Evaluate current `ethrex` against three product ideas and rewrite `docs/developers/project-analysis.md` as a decision-ready strategy document:

- Rollup-as-a-Service Client
- Native L2 Integration Client
- Latency-Optimized Routing Client

### Plan

- [x] Collect line-level evidence for each idea from code and docs.
- [x] Score each idea with current-fit assessment and identify concrete gaps.
- [x] Rewrite `docs/developers/project-analysis.md` in Korean around the three ideas.
- [x] Record this task review and update lessons.

### Step Notes

1. Evidence mapping
- Re-validated L2 launch/deploy flow (`ethrex l2 --dev`, `ethrex l2 deploy`) and deployment modes.
- Re-validated integrated sequencer components (watcher/committer/proof coordinator/admin API).
- Re-validated P2P peer selection logic and networking metrics limitations for latency-aware routing.

2. Scoring and gap analysis
- Rated idea readiness on a 5-point scale.
- Added explicit gap sections for missing productization layers and latency-aware policy/data.

3. Deliverable update
- Replaced the previous architecture-risk-centric content in `docs/developers/project-analysis.md` with idea-based evaluation content.

### Review

#### What changed

- Updated: `docs/developers/project-analysis.md`
- Updated: `docs/SUMMARY.md`
- Updated: `docs/developers/README.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- Executed targeted static evidence checks with:
  - `rg` (file discovery and code locations)
  - `nl -ba` + `sed` (line-anchored evidence extraction)
  - shell-based LOC aggregation for rough shared-code estimate

#### Verification not run

- No runtime benchmark or network latency experiment was executed.
- `mdbook build` was attempted, but failed because `mdbook` is not installed in the current environment.

---

## Task Checklist (2026-02-22) - Ops Agent 구현 계획 문서화

### Goal

`ethrex` L1/L2 운영 이슈를 자율적으로 탐지/판단/조치하는 AI agent에 대해, 코드 구현 없이 실행 가능한 수준의 구현 계획 문서를 작성한다.

### Plan

- [x] 기존 운영 툴링(모니터링, sync 자동화, admin API) 기반 범위와 제약을 반영한 계획 구조 정의
- [x] 단계별 구현 로드맵(Phase 0~4), 도메인 모델, 인터페이스, 테스트/수용 기준 명시
- [x] `docs/ops-agent/implementation-plan.md` 신규 작성
- [x] 작업 리뷰/학습 기록 반영

### Step Notes

1. 계획 문서 스코프 고정
- 배포 형태(사이드카), 자동화 수준(제한적 자동조치), 승인 방식(ChatOps), 운영 도메인(동기화/시퀀서/인프라)을 고정 값으로 문서화했다.

2. 실행 가능성 중심 상세화
- 아키텍처 컴포넌트(collector/diagnoser/planner/actuator/approval/auditor), 도메인 모델, API 초안, playbook 카탈로그를 구현 단위로 분해했다.

3. 검증 기준 명시
- 단위/통합/장애 시뮬레이션/수용 테스트 기준과 SLI/SLO를 포함해 구현 이후 평가 가능한 기준으로 정리했다.

### Review

#### What changed

- Added: `docs/ops-agent/implementation-plan.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- 문서 품질 점검을 위해 계획 항목 간 정합성(범위, 단계, 테스트, 수용 기준) 수동 검토 수행

#### Verification not run

- `mdbook build` 미실행
- 코드/테스트 실행 없음(요청 범위가 문서화만 포함)

---

## Task Checklist (2026-02-22) - Ops Agent 모니터링 통합 전략 문서화

### Goal

`Ethrex L1/L2 자율 운영 AI Agent`에 모니터링 스택을 어떻게 통합할지, 그리고 즉시 운영 자동화 도입 시 현재 코드 대비 효용을 문서로 정리한다.

### Plan

- [x] 통합 방식(내장 vs 분리형 연동) 기술적 권고안 명시
- [x] 권장 토폴로지 및 역할 경계(ethrex/metrics/agent) 정리
- [x] 현재 대비 효용과 정량 목표(KPI) 문서화
- [x] 안전 도입 가드레일 및 단계적 활성화 전략 명시
- [x] 문서 링크를 `docs/SUMMARY.md`, `docs/developers/README.md`에 반영

### Step Notes

1. 통합 방식 결론 고정
- 물리적 분리(독립 서비스) + 논리적 통합(강한 연동) 방식을 권고안으로 확정했다.

2. 효용을 운영 관점으로 재구성
- 탐지/판단/조치/검증 폐루프 전환, MTTR 단축, 대응 일관성, 감사성 향상으로 정리했다.

3. 즉시 적용 시 리스크 제어 포함
- 관찰 전용 → 저위험 자동조치 → 승인 기반 고위험 조치의 단계적 활성화 전략을 포함했다.

### Review

#### What changed

- Added: `docs/ops-agent/monitoring-integration.md`
- Updated: `docs/SUMMARY.md`
- Updated: `docs/developers/README.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- 문서 간 링크 확인:
  - `docs/SUMMARY.md` 신규 링크 추가
  - `docs/developers/README.md` 신규 링크 추가

#### Verification not run

- `mdbook build` 미실행
- 코드/테스트 실행 없음(문서화 요청 범위)

---

## Task Checklist (2026-02-22) - Ops Agent 6개월 로드맵 문서화

### Goal

기능별 agent 분할과 agent 간 상호작용을 전제로, 완전 자율 운영을 지향하는 6개월 구현 로드맵을 문서화한다.

### Plan

- [x] 멀티-agent 타겟 구조(역할/상호작용) 정의
- [x] Month 1~6 월별 마일스톤 및 완료 기준 작성
- [x] 인터페이스 변경 로드맵과 검증/품질 게이트 포함
- [x] 리스크 대응 및 운영 프로세스 제안 포함
- [x] 문서 링크를 `docs/SUMMARY.md`, `docs/developers/README.md`에 반영

### Step Notes

1. 로드맵 관점 정렬
- “완전 자율 운영”을 바로 목표로 두지 않고, 6개월 내 달성 가능한 “준자율 운영”을 단계 목표로 설정했다.

2. 월별 실행 가능성 강화
- 각 월에 목표/구현 항목/완료 기준을 명시해 실행 책임과 검증 기준을 분리했다.

3. 멀티-agent 협업 핵심 포함
- 이벤트 계약, 공통 컨텍스트 저장소, agent 간 합의 규칙을 명시했다.

### Review

#### What changed

- Added: `docs/ops-agent/roadmap-6months.md`
- Updated: `docs/SUMMARY.md`
- Updated: `docs/developers/README.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- 문서 링크 및 참조 경로 수동 검토

#### Verification not run

- `mdbook build` 미실행
- 코드/테스트 실행 없음(문서화 요청 범위)

---

## Task Checklist (2026-02-22) - Fork 작업 방식/자동 동기화 문서화

### Goal

`theo-learner/ethrex` 포크 기반 작업 방식과, push 시 `upstream/main` 최신 반영을 자동화하는 방법을 개발자 문서로 정리한다.

### Plan

- [x] 포크/업스트림 리모트 표준 구조 문서화
- [x] 일상 브랜치 작업 흐름 문서화
- [x] push 시 자동 동기화(`git push-theo` alias) 방법 문서화
- [x] 충돌 처리/force-with-lease/main 보호 규칙 명시
- [x] 개발자 문서 인덱스 링크 반영

### Step Notes

1. 실무형 자동화 선택
- pre-push 훅보다 동작 예측이 쉬운 `git push-theo` alias 방식을 권장안으로 제시했다.

2. 안전 규칙 포함
- `main`은 `ff-only`, feature branch rebase, 필요 시 `--force-with-lease`를 명시했다.

3. 접근성 개선
- `docs/SUMMARY.md`와 `docs/developers/README.md`에서 새 가이드 링크를 추가했다.

### Review

#### What changed

- Added: `docs/developers/fork-workflow.md`
- Updated: `docs/SUMMARY.md`
- Updated: `docs/developers/README.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- 문서 링크와 명령어 문법 수동 검토

#### Verification not run

- `mdbook build` 미실행
- 코드/테스트 실행 없음(문서화 요청 범위)

---

## Task Checklist (2026-02-23) - Ops Agent 운영 채널 변경

### Goal

`docs/ops-agent/implementation-plan.md`의 운영 채널을 Slack에서 텔레그램 봇/디스코드로 변경한다.

### Plan

- [x] 운영 채널 정책 문구 변경
- [x] 승인 게이트웨이/위험도 정책/단계별 구현 계획 내 Slack 의존 문구 치환
- [x] 테스트/리스크 항목의 채널 의존 문구 치환

### Review

#### What changed

- Updated: `docs/ops-agent/implementation-plan.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- `rg`로 Slack 잔존 문구 확인 후 채널 문구 반영 검증

#### Verification not run

- `mdbook build` 미실행

---

## Task Checklist (2026-02-23) - 자율 운영 목적/계획 전역 규칙화

### Goal

자율 운영 AI agent 구현 목적과 구현 계획을 프로젝트 전역 규칙으로 설정해, 이후 코드 변경 시 목적 드리프트를 방지한다.

### Plan

- [x] 루트 전역 규칙 파일(`AGENTS.md`) 추가
- [x] 구현 목적/소스 오브 트루스/변경 규칙/PR 체크리스트 명시
- [x] `CONTRIBUTING.md`에서 전역 규칙 참조 링크 추가
- [x] 작업/학습 기록 업데이트

### Review

#### What changed

- Added: `AGENTS.md`
- Updated: `CONTRIBUTING.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- 문서 경로/참조 링크 수동 확인 (`AGENTS.md` <-> `CONTRIBUTING.md`)

#### Verification not run

- `mdbook build` 미실행

---

## Task Checklist (2026-02-23) - AGENTS 규칙 브랜치 스코프 한정

### Goal

`AGENTS.md` 규칙을 프로젝트 전역이 아니라 `feat/ops-agent` 브랜치 한정 규칙으로 명확히 조정한다.

### Plan

- [x] `AGENTS.md`에 scope 섹션 추가 (`feat/ops-agent` only)
- [x] `Global` 표현을 `Branch-Scoped` 표현으로 변경
- [x] `CONTRIBUTING.md` 안내 문구를 브랜치 한정으로 수정

### Review

#### What changed

- Updated: `AGENTS.md`
- Updated: `CONTRIBUTING.md`
- Updated: `docs/todo.md`
- Updated: `docs/lessons.md`

#### Verification run

- 문서 내 스코프/용어 수동 검토

#### Verification not run

- `mdbook build` 미실행
