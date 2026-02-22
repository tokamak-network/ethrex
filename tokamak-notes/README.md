# Tokamak Notes

> Tokamak Network의 ethrex 포크 내부 문서

## 구조

```
tokamak-notes/
├── branching-strategy.md          # 브랜치 전략
├── zk-optimization-plan.md        # ZK Prover 최적화 + 병렬 실행 계획
└── analysis-doc-kr/               # ethrex 코드 분석 (한국어)
    ├── 00-개요.md
    ├── 01-아키텍처.md
    ├── 02-EVM-실행엔진.md
    ├── 03-스토리지.md
    ├── 04-네트워킹.md
    ├── 05-L2-롤업.md
    ├── 06-블록체인-코어.md
    ├── 07-빌드-및-설정.md
    ├── 08-L1-L2-시스템-구성.md
    ├── 09-브릿지-및-출금-파이널리티.md
    ├── 10-데이터-가용성.md
    └── 11-프루버-경제모델-및-수수료.md
```

## 문서 목록

### 프로젝트 관리
- [branching-strategy.md](./branching-strategy.md) — upstream 동기화 및 Tokamak 특화 브랜치 관리 규칙

### 개발 계획
- [zk-optimization-plan.md](./zk-optimization-plan.md) — ZK Prover 최적화, 병렬 블록 실행(#6209), 병렬 상태 루트(#6210), 시뇨리지 마이닝

### ethrex 코드 분석

| # | 문서 | 설명 |
|---|------|------|
| 00 | [프로젝트 개요](./analysis-doc-kr/00-개요.md) | 프로젝트 소개, 설계 철학, 워크스페이스 구조, 주요 의존성 |
| 01 | [전체 아키텍처](./analysis-doc-kr/01-아키텍처.md) | 시스템 아키텍처, 진입점, 블록 실행 파이프라인, 동기화 메커니즘 |
| 02 | [EVM 실행 엔진](./analysis-doc-kr/02-EVM-실행엔진.md) | LEVM 구현, 옵코드, 콜 프레임, 프리컴파일, 가스 비용 |
| 03 | [스토리지 레이어](./analysis-doc-kr/03-스토리지.md) | RocksDB/InMemory 백엔드, 테이블 구조, Merkle Patricia Trie |
| 04 | [네트워킹 레이어](./analysis-doc-kr/04-네트워킹.md) | P2P (DevP2P, RLPx), 스냅 싱크, JSON-RPC, Engine API |
| 05 | [L2 ZK-Rollup](./analysis-doc-kr/05-L2-롤업.md) | 시퀀서, 증명 시스템, 스마트 컨트랙트, 입출금 |
| 06 | [블록체인 코어](./analysis-doc-kr/06-블록체인-코어.md) | 블록 검증, 포크 초이스, 멤풀, 페이로드 빌딩 |
| 07 | [빌드 및 설정](./analysis-doc-kr/07-빌드-및-설정.md) | 빌드 시스템, CLI, Docker, CI/CD, 테스트 |
| 08 | [L1+L2 시스템 구성](./analysis-doc-kr/08-L1-L2-시스템-구성.md) | 전체 시스템 구성도, Docker 서비스, 프루버, 배처 |
| 09 | [브릿지 및 출금](./analysis-doc-kr/09-브릿지-및-출금-파이널리티.md) | 입금/출금 흐름, Merkle Proof, 파이널리티 |
| 10 | [데이터 가용성](./analysis-doc-kr/10-데이터-가용성.md) | EIP-4844 블롭, 롤업/발리디움 모드, KZG 커밋먼트 |
| 11 | [프루버 경제모델](./analysis-doc-kr/11-프루버-경제모델-및-수수료.md) | 프루버 인센티브, L2 수수료, 서버 운영비 추정 |
