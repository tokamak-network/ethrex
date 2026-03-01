# Group-IB High-Tech Crime Trends 2026 — Idea Extraction

> Source: Dmitry Volkov (CEO, Group-IB) — [Cyber Predictions 2026](https://www.group-ib.com/blog/cyber-predictions-2026/)
> Date: 2026-02

## Report 핵심 위협 3가지

1. **Supply Chain 공격의 산업화** — npm/PyPI 패키지, OAuth 토큰, 브라우저 확장, CI/CD 파이프라인을 통한 대규모 침투
2. **AI Agentic 공격** — 자율 AI가 취약점 발견→침투→횡이동→탈취 전체 kill chain을 관리
3. **Crypto/Stablecoin 범죄 경제** — DeFi 조작, 스마트 컨트랙트 익스플로잇, AI 기반 자금세탁

## 파생 제품 아이디어 5가지

### 1. Package Provenance Chain
- 모든 패키지 빌드에 "이 소스코드에서 이 바이너리가 나왔다"를 암호학적으로 증명
- Sigstore가 시작했지만 빌드 재현성 + 런타임 행동 모니터링이 없음
- 시장: 모든 소프트웨어 회사 (npm 주당 ~300억 다운로드)

### 2. Autonomous Red Team Agent
- 고객 인프라를 24/7 자율적으로 공격하는 AI 에이전트
- 기존 BAS와 다른 점: LLM이 맥락을 이해하고 새로운 공격 경로를 창의적으로 탐색
- 시장: SOC 팀이 있는 중대형 기업

### 3. DeFi Circuit Breaker
- 온체인 이상 탐지 + 자동 일시정지 미들웨어
- Forta가 탐지는 하지만 자동 대응(pause)까지 하지 않음
- 시장: TVL $10M 이상 DeFi 프로토콜 (~500개)

### 4. AI-Generated Backdoor Scanner
- PR/커밋 단위로 "이 코드 변경이 의도적 백도어일 확률"을 점수화
- "에러 핸들링처럼 보이지만 실제로는 인증 우회" 같은 의미적 분석
- 기존 SAST는 패턴 매칭이라 교묘한 백도어를 못 잡음
- 시장: 오픈소스 재단, 금융/방산 기업

### 5. Stablecoin AML Intelligence
- 스테이블코인 거래의 실시간 AML 스코어링 + 크로스체인 자금 추적
- 전통 은행이 crypto rails를 도입하면서 기존 AML 시스템이 온체인 추적 못 함
- 시장: 규제 대상 금융기관

## 평가

| Idea | 기술 난이도 | 시장 크기 | 타이밍 | 경쟁 |
|---|---|---|---|---|
| Package Provenance | 중 | 거대 | 지금 | Sigstore (약함) |
| Autonomous Red Team | 높음 | 큼 | 지금 | Pentera, HackerOne AI |
| DeFi Circuit Breaker | 중 | 중 | 지금 | Forta, OZ Defender |
| Backdoor Scanner | 매우 높음 | 큼 | 1-2년 | 거의 없음 |
| Stablecoin AML | 중 | 거대 | 지금 | Chainalysis (강함) |
