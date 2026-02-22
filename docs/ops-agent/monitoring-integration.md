# Ethrex Ops Agent 모니터링 통합 전략

## 1. 질문에 대한 결론

`Ethrex L1/L2 자율 운영 AI Agent`에 모니터링 스택을 통합하는 방식은,  
**논리적으로는 강하게 통합하고 물리적으로는 분리**하는 것이 가장 적합하다.

- 권장: `ethrex` + `metrics/`(Prometheus/Grafana/Loki) + `ops-agent`를 분리 배포
- 비권장: agent 내부에 Prometheus/Grafana/Loki를 내장

즉, 에이전트는 모니터링 스택의 **소비자/오케스트레이터**로 동작하고,  
모니터링 스택은 기존처럼 독립 서비스로 운영한다.

---

## 2. 왜 분리형 통합이 적합한가

## 2.1 운영 안정성

1. 장애 격리
- agent 장애가 모니터링 스택 전체 장애로 전파되지 않는다.
- 모니터링 스택 장애가 있어도 agent는 RPC/프로세스 체크 기반 최소 진단이 가능하다.

2. 업그레이드 독립성
- `ethrex`, `metrics/`, `ops-agent` 버전 업그레이드 주기를 분리할 수 있다.

3. 확장성
- 단일 노드부터 다중 노드까지, agent와 모니터링 스택을 독립 스케일링할 수 있다.

## 2.2 재사용성

1. 기존 `metrics/` 구성 재사용
- L1: Prometheus/Grafana/Loki/Promtail + exporter 구성을 그대로 사용 가능
- L2: Prometheus/Grafana 기본 구성을 유지하고 필요 시 Loki 확장

2. 기존 alert 룰 자산 재사용
- Grafana alerting 룰을 incident 신호로 연결 가능

---

## 3. 권장 통합 토폴로지

## 3.1 단일 노드 기준

1. Ethrex 실행
- L1 또는 L2 노드 실행 (`--metrics`, 필요 시 `--log.dir`)

2. 모니터링 스택 실행
- `metrics/` compose로 Prometheus/Grafana(/Loki) 구동

3. Ops Agent 실행
- 데이터 입력:
  - Prometheus Query API
  - Ethrex JSON-RPC / L2 Admin API
  - 로그 신호(Loki query 또는 파일 tail)
- 데이터 출력:
  - Slack incident/승인 메시지
  - 액션 실행 및 감사 로그

## 3.2 다중 노드 기준

1. 중앙 Prometheus/Grafana/Loki
2. 노드별 agent(사이드카) 또는 중앙 agent(읽기+원격조치) 택1
3. 초기에는 노드별 agent를 권장(권한 경계 단순)

---

## 4. “Ethrex 실행 후 Agent 즉시 운영자동화”의 효용 (현 코드 대비)

현재 코드베이스는 모니터링·관찰·부분 자동화를 제공하지만,  
운영자가 최종 판단/실행을 담당하는 비중이 높다.

Agent를 붙이면 아래 효용이 생긴다.

1. 탐지에서 조치까지 폐루프 전환
- 현재: 대시보드/알림 확인 후 수동 대응
- 도입 후: 탐지 → 판단 → 실행 → 사후검증 자동 루프

2. 반복 이슈 MTTR 단축
- sync stall, node unresponsive, committer 이상 등 반복 패턴 대응 시간 단축

3. 대응 일관성 확보
- 담당자 경험 차이 대신 정책/플레이북 기반 동일 대응

4. 승인 기반 안전 자동화
- 고위험 조치는 ChatOps 승인 후 실행해 리스크 제어

5. 감사 가능성 강화
- 판단 근거, 실행 액션, 결과, 롤백 이력 자동 기록

---

## 5. 정량 기대효과(초기 목표값)

아래 수치는 초기 운영 목표값으로 사용한다.

1. MTTR 40% 이상 단축
2. 저위험 반복 이슈 자동해결률 70% 이상
3. 오탐으로 인한 불필요 조치율 5% 이하
4. 조치 이력 감사 로그 기록률 100%
5. 승인 필요 조치의 승인-실행 리드타임 5분 이내

---

## 6. 즉시 자동화 도입 시 권장 가드레일

“바로 자동화”는 가능하지만, 다음 순서를 지켜야 안전하다.

1. Day 0-7: 관찰 전용 모드
- incident 생성/분류만 수행
- 실제 액션은 제안만

2. Day 8-14: 저위험 자동조치 활성
- 재시도, 경량 복구, 비파괴 진단만 자동 실행

3. Day 15+: 승인 기반 고위험 조치 활성
- 재시작/committer 제어는 Slack 승인 후 실행

4. 금지 조치 유지
- 데이터 삭제성 조치, DB 직접 변경, 무승인 고위험 조치 금지

---

## 7. 현재 Ethrex 기능과 Agent 역할 분리

1. Ethrex
- 체인 실행, RPC/API, 메트릭 노출, L2 admin endpoint 제공

2. metrics 스택
- 시계열 저장, 대시보드, 로그 집계, alerting

3. Ops Agent
- 다중 신호 상관판단
- 정책 기반 액션 결정/실행
- 승인/감사/에스컬레이션

이 분리를 유지하면 결합도는 낮고 운영 자동화 효과는 높다.

---

## 8. 구현 우선순위

1. P0
- Prometheus + RPC + Admin API를 한 번에 읽는 collector
- sync/sequencer/infra 3개 도메인 incident 모델

2. P1
- 저위험 자동조치 playbook
- 실행 후 health re-check 및 실패 시 rollback/escalation

3. P2
- Slack ChatOps 승인 흐름
- 고위험 조치 실행 제어

4. P3
- 주간 성과 리포트(MTTR, 자동해결률, 오탐률)

---

## 9. 최종 권고

`ethrex` 운영 자동화의 현실적인 최적점은 다음이다.

1. 모니터링 스택은 독립 서비스로 운영한다.
2. Ops Agent가 해당 스택과 강하게 연동해 자율 판단/조치를 수행한다.
3. 초기에는 저위험 자동화부터 시작하고, 고위험은 승인 기반으로 점진 확대한다.

이 방식이 현재 코드 자산을 가장 많이 재사용하면서도 운영 효용을 가장 빠르게 만든다.
