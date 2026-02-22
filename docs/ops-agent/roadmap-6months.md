# Ethrex Ops Agent 6개월 구현 로드맵

## 1. 목표

장기 목표는 기능별 agent가 상호작용하여 L1/L2 운영을 자율적으로 수행하는 체계다.  
6개월 내 달성 목표는 **“제한적 자동화 → 다중 agent 협업 자동화 → 승인 기반 준자율 운영”**으로 정의한다.

핵심 원칙은 다음과 같다.

1. 물리적 배포는 분리(ethrex / metrics / ops-agent)
2. 논리적 통합은 강화(공통 사건 모델 + 공통 정책 + 공통 감사)
3. 자동화는 단계적 확대(관찰 전용 → 저위험 자동조치 → 승인 기반 고위험 조치)

---

## 2. 타겟 멀티-Agent 구조

## 2.1 Agent 분할

1. `telemetry-agent`
- Prometheus/RPC/Loki/프로세스 상태 수집
- 표준화된 `TelemetrySnapshot` 발행

2. `diagnosis-agent`
- 룰 + 상태머신 + 상관분석으로 incident 생성

3. `planning-agent`
- playbook 후보 생성, 위험도/신뢰도 계산, 액션 계획 수립

4. `execution-agent`
- 액션 실행, 사후 검증, 롤백 시도

5. `approval-agent`
- Slack ChatOps 승인 요청/토큰/만료/감사 처리

6. `governance-agent`
- 정책 버전 관리, 금지 조치 가드, 변경 이력 통제

## 2.2 상호작용 패턴

1. 이벤트 버스 기반 비동기 상호작용
- `snapshot.collected`
- `incident.detected`
- `action.planned`
- `approval.requested`
- `action.executed`
- `recovery.verified`

2. 공통 컨텍스트 저장소
- Incident, ActionPlan, ExecutionRecord, ApprovalToken 공유

3. 의사결정 순서
- 진단 agent 단독 판단 금지
- 최소 2개 agent 증거 합의(진단 + 계획) 후 액션 단계로 진입

---

## 3. 6개월 마일스톤

## Month 1: 기반 플랫폼 구축

### 목표

멀티 agent를 얹을 수 있는 공통 운영 플랫폼을 먼저 만든다.

### 구현 항목

1. 공통 도메인 모델 고정
- Incident/ActionPlan/ExecutionRecord/ApprovalToken 스키마 확정

2. 공통 이벤트 계약 정의
- 메시지 타입, idempotency key, trace id 표준화

3. 단일 실행체 MVP
- 우선은 모놀리식 `ops-agent`로 구현하고 내부 모듈 경계를 agent 단위로 나눔

4. 공통 감사 저장소 구축
- SQLite 시작, Postgres 전환 가능한 repository 인터페이스 적용

### 완료 기준

1. end-to-end 파이프라인(수집→탐지→계획→실행 시뮬레이션) 동작
2. 감사 로그 누락 0건

---

## Month 2: 기능별 Agent 분리 1차

### 목표

`telemetry-agent`와 `diagnosis-agent`를 독립 실행 단위로 분리한다.

### 구현 항목

1. telemetry-agent 분리
- Prometheus/RPC/Loki 수집 기능 독립 프로세스화

2. diagnosis-agent 분리
- sync/sequencer/infra 도메인 룰 엔진 독립화

3. 이벤트 버스 도입
- NATS 또는 Redis Streams 중 1개 선택해 비동기 이벤트 연결

4. 사건 중복 제거(dedup) + 플래핑 완화
- cooldown/hysteresis/merge rule 추가

### 완료 기준

1. 분리 후에도 incident 탐지 정확도 유지(기존 대비 -5%p 이내)
2. 동일 incident 중복 생성률 10% 이하

---

## Month 3: Planning/Execution Agent 도입

### 목표

진단 이후의 판단·실행을 분리해 “협업 자동화”를 시작한다.

### 구현 항목

1. planning-agent 도입
- playbook 매핑, risk scoring, confidence scoring

2. execution-agent 도입
- 저위험 자동조치 실행 + post-check + rollback

3. 정책 가드 적용
- allowlist, max retry, timeout, concurrency limit

4. 공통 상태 전이 규칙 강제
- `open -> action_pending -> recovering -> resolved/escalated`

### 완료 기준

1. 저위험 액션 자동 성공률 85% 이상
2. 실패 액션의 rollback/escalation 누락 0건

---

## Month 4: Approval/Governance Agent 도입

### 목표

고위험 조치를 안전하게 허용하기 위한 승인·정책 계층을 완성한다.

### 구현 항목

1. approval-agent 도입
- Slack 승인 요청/승인/거절/만료 처리

2. governance-agent 도입
- 정책 파일 버전 관리
- 금지 조치 실행 차단
- 정책 변경 이력 감사

3. 고위험 액션 워크플로우 활성화
- 승인 없이는 실행 불가

4. 운영자 개입 UX 개선
- incident context bundle 자동 첨부(지표/로그/RPC 상태)

### 완료 기준

1. 승인 플로우 성공률 99% 이상
2. 무승인 고위험 액션 실행 0건

---

## Month 5: Agent 협업 고도화

### 목표

에이전트 간 상호작용을 규칙형에서 정책형으로 고도화한다.

### 구현 항목

1. 협업 정책 엔진
- multi-signal consensus rule(예: 진단+계획 동의 + 신뢰도 임계치)

2. conflict resolution
- 상충하는 액션 제안이 나오면 governance-agent가 우선순위 결정

3. LLM 보조 판단 도입(선택적)
- 원인 요약과 조치 설명에 한정
- 실행 승인 로직은 규칙 엔진 우선

4. 학습 루프
- 실패 incident 재학습 데이터셋 구성
- 룰 튜닝 자동 제안

### 완료 기준

1. 오탐 조치율 5% 이하
2. 같은 유형 incident 재발 시 MTTR 20% 추가 단축

---

## Month 6: 준자율 운영 전환

### 목표

운영에 적용 가능한 준자율 모드(고위험 승인 기반)를 안정화한다.

### 구현 항목

1. 운영 모드 3단계 정식화
- Observe only
- Low-risk autonomous
- Approval-gated high-risk

2. 운영 KPI 대시보드 제공
- MTTR
- 자동해결률
- 승인 리드타임
- 오탐률
- rollback 비율

3. 카나리 → 점진 확장
- 1개 노드 카나리
- L1/L2 한 쌍
- 전체 운영군

4. 런북/온콜 핸드오버 문서 완성
- 실패 시 수동 전환 절차
- incident 레벨별 대응 SOP

### 완료 기준

1. 카나리 2주 동안 심각 장애 유발 0건
2. 목표 KPI 충족:
- MTTR 40% 단축
- 자동해결률 70% 이상
- 감사 로그 기록률 100%

---

## 4. 분기별 Deliverable

## Q1 (Month 1~3)

1. 다중 agent 전환 가능한 플랫폼
2. telemetry/diagnosis/planning/execution agent 기본 동작
3. 저위험 자동조치 운영 시작

## Q2 (Month 4~6)

1. approval/governance agent 완성
2. 승인 기반 고위험 조치 운영
3. 준자율 운영 모드 정식 전환

---

## 5. 인터페이스 변경 로드맵

## Month 1~2

1. `POST /v1/incidents/evaluate`
2. `GET /v1/incidents/{id}`
3. `POST /v1/incidents/{id}/act`

## Month 3~4

1. `POST /v1/approvals/request`
2. `POST /v1/approvals/{token}/confirm`
3. `GET /v1/executions/{id}`

## Month 5~6

1. `GET /v1/agent-health`
2. `GET /v1/policy/version`
3. `POST /v1/policy/validate` (dry-run)

---

## 6. 검증 계획

## 6.1 테스트 트랙

1. 단위 테스트
- 룰/상태머신/정책 가드

2. 통합 테스트
- agent 간 이벤트 계약
- 승인 워크플로우

3. 시뮬레이션 테스트
- sync stall
- committer stuck
- node unresponsive
- mempool burst

4. 카오스 테스트
- 메시지 중복/유실
- telemetry source 부분 장애

## 6.2 품질 게이트

1. release 후보는 회귀 시뮬레이션 100% 통과
2. 정책 변경은 dry-run 결과 첨부 필수
3. 고위험 액션 정책은 2인 승인 원칙(운영 정책)

---

## 7. 주요 리스크와 대응

1. Agent 간 결정 충돌
- 대응: governance-agent 우선순위 중재 규칙

2. 자동화 과잉(aggressive actions)
- 대응: risk cap, rate cap, cooldown, mandatory approval

3. 모니터링 데이터 품질 저하
- 대응: source health score + confidence 하향 + action 억제

4. 정책 드리프트
- 대응: 정책 버전 고정 + 변경 감사 + 주간 리뷰

---

## 8. 운영 조직/프로세스 제안

1. 주간 Ops-Agent 리뷰
- 오탐/미탐/실패 액션 점검

2. 월간 정책 릴리즈
- 정책 버전 업그레이드와 KPI 리포트 동시 배포

3. 온콜 협업 프로세스
- 승인 SLA
- 에스컬레이션 SLA

---

## 9. 최종 상태(6개월 후)

6개월 종료 시점의 목표 상태는 다음과 같다.

1. 기능별 agent가 독립 실행 단위로 운영된다.
2. 공통 이벤트 계약과 공통 정책 계층으로 상호작용한다.
3. 저위험 이슈는 완전 자동, 고위험 이슈는 승인 기반 자동조치가 가능하다.
4. 운영 성과가 KPI로 계량되고, 정책 개선 루프가 정착된다.

이 로드맵은 “완전 자율 운영”으로 가기 위한 실제 구현 가능한 중간 단계(준자율 운영) 달성을 목표로 한다.
