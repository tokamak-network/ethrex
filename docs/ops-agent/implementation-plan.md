# Ethrex L1/L2 자율 운영 AI Agent 구현 계획

## 1. 목표

Ethrex L1/L2 운영 중 발생하는 이슈를 에이전트가 **탐지 → 판단 → 조치 → 검증**까지 수행하도록 하되,  
고위험 조치는 사람 승인(ChatOps)을 거치는 안전한 자율 운영 체계를 구축한다.

초기 구현 원칙은 다음과 같다.

- 배포 형태: **사이드카 서비스**
- 자동화 수준: **제한적 자동조치**
- 운영 채널: **텔레그램 봇/디스코드 알림·승인**
- 적용 범위: **동기화·정체, 시퀀서/커미터, 인프라 자원·로그**

---

## 2. 범위

### 2.1 In Scope (MVP)

1. L1/L2 노드 건강 상태 수집 및 이상 탐지
2. 규칙 기반 판단 + 상태머신 기반 진단
3. 저위험 자동 조치 실행
4. 고위험 조치 승인 요청/승인 후 실행
5. 조치 결과 검증 및 실패 시 롤백/에스컬레이션
6. 전 단계 감사 로그 저장

### 2.2 Out of Scope (MVP)

1. DB 스키마 변경/자동 마이그레이션
2. 체인 파라미터 자동 변경
3. 완전 무인(고위험 포함) 자동화
4. 멀티 리전 오케스트레이션

---

## 3. 기존 Ethrex 자산 활용 계획

1. 모니터링/로그
- `metrics/`의 Prometheus/Grafana/Loki 구성 재사용
- 기존 Alert 룰(`metrics/provisioning/grafana/alerting/alerts.json`)을 탐지 신호로 통합

2. 운영 API
- L2 Admin API(`docs/l2/admin.md`) 활용:
  - `/health`
  - `/committer/start`
  - `/committer/stop`

3. 운영 자동화 패턴
- `tooling/sync/docker_monitor.py`의 상태머신 접근을 일반화
  - waiting/syncing/block_processing/failed/success 개념을 incident 상태로 확장

4. RPC/실행 컨텍스트
- L1/L2 JSON-RPC, 로컬 프로세스/컨테이너 상태, 시스템 리소스 정보 결합

---

## 4. 목표 아키텍처

## 4.1 컴포넌트

1. `collector`
- 데이터 수집기
- 소스: Prometheus API, JSON-RPC, 프로세스 상태, 로그 시그널

2. `diagnoser`
- 룰 엔진 + 상태머신으로 이상 판단
- incident 생성/갱신

3. `planner`
- incident별 가능한 조치(playbook) 후보 생성
- 위험도/신뢰도 점수 산정

4. `actuator`
- 조치 실행기
- 안전 가드(allowlist, rate limit, max retry, timeout) 적용

5. `approval-gateway`
- 텔레그램 봇/디스코드 승인/거절 처리
- 승인 토큰 발행/만료/검증

6. `auditor`
- 판단 근거, 액션 실행, 결과, 롤백 기록 영속화

### 4.2 저장소

MVP: SQLite  
확장: Postgres로 전환 가능한 Repository 인터페이스

---

## 5. 도메인 모델

1. `Incident`
- `id`
- `domain`: `sync | sequencer | infra`
- `severity`: `low | medium | high | critical`
- `confidence`: `0..1`
- `status`: `open | investigating | action_pending | recovering | resolved | escalated`
- `evidence`: metric/log/rpc 증거 집합

2. `ActionPlan`
- `id`
- `incident_id`
- `action_type`
- `risk_level`
- `requires_approval`
- `prechecks`
- `rollback_plan`

3. `ExecutionRecord`
- `id`
- `action_plan_id`
- `started_at`, `finished_at`
- `outcome`: `success | failed | rolled_back | partial`
- `artifacts`: command output, rpc response, metrics snapshot

4. `ApprovalToken`
- `token`
- `action_plan_id`
- `requested_at`, `expires_at`
- `approved_by`
- `status`

---

## 6. 조치 정책

### 6.1 위험도 기준

1. Low
- 관찰 주기 조정, 재시도, 비파괴 진단 실행
- 자동 실행

2. Medium
- 단일 컴포넌트 제한 재시작 등
- 자동 실행 + 즉시 보고

3. High
- 노드 재기동, committer 제어
- 텔레그램 봇/디스코드 승인 필요

4. Critical
- 자동 조치 금지
- 즉시 에스컬레이션 및 수동 런북 안내

### 6.2 금지 조치

1. 데이터 삭제성 명령
2. DB 파일 직접 변경
3. 체인 설정 변경
4. 승인 없는 고위험 액션

---

## 7. Playbook 카탈로그 (초안)

### 7.1 동기화·정체

1. `sync_stall_soft_recheck` (Low)
- 조건: block 증가 정체가 임계치 미만
- 조치: 관찰 주기 단축 + 추가 진단

2. `sync_stall_restart_consensus` (High)
- 조건: consensus endpoint 비정상 지속
- 조치: consensus 재시작(승인 필요)

3. `sync_stall_restart_execution` (High)
- 조건: execution node unresponsive 지속
- 조치: ethrex 재시작(승인 필요)

### 7.2 시퀀서/커미터/프로버

1. `committer_health_recover` (Medium)
- 조건: committer 중지 감지 + 복구 가능
- 조치: `/committer/start/{delay}` 호출

2. `committer_stuck_stop_start` (High)
- 조건: commit 지연 장기화
- 조치: `/committer/stop` 후 `/committer/start` (승인 필요)

3. `proof_sender_lag_escalate` (Critical)
- 조건: verification 지연 임계치 초과 + 반복 실패
- 조치: 자동 실행 없음, 즉시 에스컬레이션

### 7.3 인프라 자원·로그

1. `resource_pressure_throttle_checks` (Low)
- 조건: CPU/RAM 일시 급등
- 조치: 에이전트 내부 폴링 완화

2. `fd_leak_suspected_restart_target` (High)
- 조건: open fd 지속 상승 + 오류 연동
- 조치: 대상 프로세스 재시작(승인 필요)

3. `error_burst_attach_context` (Low)
- 조건: 특정 에러 로그 급증
- 조치: 관련 metrics/log bundle 생성 및 incident 첨부

---

## 8. 외부/내부 인터페이스 계획

## 8.1 내부 모듈 인터페이스

1. `CollectorPort`
- `collect_snapshot() -> TelemetrySnapshot`

2. `DiagnoserPort`
- `evaluate(snapshot) -> Vec<IncidentCandidate>`

3. `PlannerPort`
- `plan(incident) -> ActionPlan`

4. `ActuatorPort`
- `execute(plan) -> ExecutionRecord`

5. `ApprovalPort`
- `request(plan) -> ApprovalToken`
- `resolve(token, decision) -> ApprovalDecision`

## 8.2 서비스 API (MVP)

1. `GET /v1/health`
2. `POST /v1/incidents/evaluate`
3. `GET /v1/incidents/{id}`
4. `POST /v1/incidents/{id}/act`
5. `POST /v1/approvals/request`
6. `POST /v1/approvals/{token}/confirm`

---

## 9. 구현 단계

### Phase 0. 기준선 확정 (1주)

1. 운영 신호 목록 확정
2. 임계치/상태 전이 규칙 정의
3. 위험도 정책 및 승인 정책 문서화

**산출물**
- `playbooks.md` 초안
- `policies.md` 초안
- 기준 metric/rpc/log 매핑 표

### Phase 1. 탐지/진단 MVP (2주)

1. collector + diagnoser 구현
2. incident 저장/조회 구현
3. 텔레그램 봇/디스코드 알림 연동

**완료 기준**
- 3개 도메인에서 incident 생성/상태 전이 정상 동작

### Phase 2. 조치 엔진 MVP (2주)

1. actuator 구현
2. low/medium 자동조치 연결
3. 실행 후 검증 루프 구현

**완료 기준**
- 저위험 시나리오 자동 복구 성공률 90% 이상

### Phase 3. 승인 기반 고위험 조치 (1주)

1. approval-gateway 구현
2. 텔레그램 봇/디스코드 승인 토큰 흐름 구현
3. 고위험 조치 승인 후 실행 연결

**완료 기준**
- 승인 요청→승인→실행→감사 로그 전 과정 검증

### Phase 4. 안정화/운영 이관 (1주)

1. 카나리 운영
2. 오탐/미탐 튜닝
3. 런북 및 온콜 핸드오버

**완료 기준**
- 1주 카나리 동안 심각 장애 유발 0건

---

## 10. 테스트 전략

### 10.1 단위 테스트

1. 룰 엔진 판정
2. 위험도/신뢰도 산정
3. 승인 정책 분기
4. 금지 조치 가드

### 10.2 통합 테스트

1. Prometheus/RPC mock 기반 incident lifecycle
2. L2 Admin API 호출 성공/실패 분기
3. 텔레그램 봇/디스코드 승인 시나리오

### 10.3 장애 시뮬레이션

1. sync stall
2. committer 지연/중단
3. node unresponsive
4. mempool 급증
5. 리소스 압박

### 10.4 수용 테스트 기준

1. 탐지 정확도 95% 이상
2. 오탐 조치율 5% 이하
3. 조치 실패 시 롤백/에스컬레이션 누락 0건
4. 감사 로그 누락 0건

---

## 11. 운영 지표(SLI/SLO)

1. 탐지 리드타임
2. 조치 리드타임
3. 자동 복구 성공률
4. 인시던트 재발률
5. 승인 처리 지연
6. 조치 실패율

---

## 12. 보안/감사 요구사항

1. 최소 권한 실행 계정
2. 명령 allowlist 기반 실행
3. 민감정보 마스킹
4. 모든 액션의 입력/출력/근거 영속화
5. 정책 변경 이력 관리

---

## 13. 리스크와 대응

1. 오탐으로 인한 과잉 조치
- 대응: confidence 임계치, 쿨다운, 다중 증거 요구

2. 승인 지연으로 복구 지연
- 대응: 승인 타임아웃 후 자동 에스컬레이션

3. 외부 의존성 불안정(텔레그램/디스코드/Prometheus)
- 대응: 폴백 채널(로컬 로그/CLI), 재시도 백오프

4. 플레이북 드리프트
- 대응: 주간 리뷰 + 실패 사례 기반 룰 업데이트

---

## 14. 문서 산출물 계획

1. `docs/ops-agent/overview.md`
2. `docs/ops-agent/playbooks.md`
3. `docs/ops-agent/policies.md`
4. `docs/ops-agent/runbook.md`
5. `docs/ops-agent/acceptance.md`

본 문서는 구현 착수 기준이 되는 마스터 계획서로 사용한다.
