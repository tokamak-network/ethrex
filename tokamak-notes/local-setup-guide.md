# 로컬 프루버 벤치마크 환경 구축 가이드

## 사전 요구사항

### Docker 환경 (권장)

| 도구 | 버전 | 설치 |
|------|------|------|
| Docker | 28.x+ | [Docker Desktop](https://www.docker.com/products/docker-desktop/) |
| Docker Compose | v2.x+ | Docker Desktop에 포함 |
| NVIDIA Container Toolkit | 최신 | GPU 사용 시 필수, [설치 가이드](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html) |

### GPU (SP1 가속)

- NVIDIA GPU + CUDA: SP1 증명 생성 시 GPU 가속
- GPU 없으면 `--no-gpu` 플래그로 CPU 모드 사용 (느리지만 동일하게 동작)

---

## Docker 환경 구축 (SP1 ZK-DEX)

모든 서비스(L1, 컨트랙트 배포, L2, SP1 프루버)가 Docker 컨테이너로 실행된다.
SP1 실제 ZK 증명을 생성하며, NVIDIA GPU가 있으면 자동으로 GPU 가속을 사용한다.

### Quick Start

```bash
cd crates/l2

# 전체 환경 기동 (SP1 + GPU 가속)
make zk-dex-docker

# GPU 없이 (CPU-only SP1 증명)
make zk-dex-docker-no-gpu

# 프루버 없이 (앱/프론트엔드 테스트용)
make zk-dex-docker-no-prover
```

또는 스크립트 직접 실행:

```bash
./scripts/zk-dex-docker.sh start              # SP1 + GPU
./scripts/zk-dex-docker.sh start --no-gpu     # SP1 CPU-only
./scripts/zk-dex-docker.sh start --no-prover  # 프루버 없이
./scripts/zk-dex-docker.sh stop               # 전체 중지
./scripts/zk-dex-docker.sh status             # 상태 확인
./scripts/zk-dex-docker.sh logs [l1|l2|prover|deploy]  # 로그
./scripts/zk-dex-docker.sh clean              # 이미지/볼륨 정리
```

### 동작 과정

스크립트가 자동으로 다음 단계를 순차 실행한다:

1. **Docker 이미지 빌드** — `Dockerfile.sp1`로 SP1 툴체인 + ZK-DEX 게스트 프로그램 포함 이미지 빌드 (최초 10-20분, 이후 캐시)
2. **L1 기동** — ethrex `--dev` 모드, 포트 8545
3. **컨트랙트 배포** — OnChainProposer, Bridge, SP1 Verifier 배포 + ZK-DEX 게스트 프로그램 등록
4. **L2 기동** — ZK-DEX 게스트 프로그램으로 L2 시퀀서, 포트 1729
5. **SP1 프루버 기동** — 실제 ZK 증명 생성, GPU 가속 (가능 시)

각 단계마다 health check를 거쳐 다음 단계로 진행한다.

### Proving 시간 참고

Docker 환경에서 실측한 SP1 ZK-DEX 증명 시간 (Apple M4 Max, CPU-only):

| 구간 | 시간 |
|------|------|
| 배치 수신 → STARK execution | ~1초 |
| STARK core + recursive compression | ~183초 |
| Groth16 wrapping (Docker gnark) | ~19초 |
| Groth16 verification | ~0.2초 |
| **총 proving 시간** | **203초 (3분 23초)** |

환경별 비교:

| 환경 | Proving 시간 | 아키텍처 | 총 사이클 |
|------|-------------|---------|----------|
| **Docker (SP1 ZK-DEX)** | **203초 (3m 23s)** | linux/arm64 native | 357,761 |
| 네이티브 Rosetta 2 (SP1 ZK-DEX) | 206초 (3m 26s) | x86_64 emulated | 357,761 |
| 네이티브 Rosetta 2 (ZK-DEX 패치 전) | 305초 (5m 05s) | x86_64 emulated | 11,449,345 |
| 네이티브 Rosetta 2 (EVM L2 baseline) | 1,664초 (27m 44s) | x86_64 emulated | 65,360,896 |

> Docker와 Rosetta 2 네이티브가 거의 동일한 이유: 전체 시간의 대부분이
> recursive compression + Groth16 wrapping **고정 오버헤드**(~3분)이므로,
> STARK execution이 빨라져도 전체 시간에 미치는 영향이 미미하다.
> 대규모 배치(100+ tx)에서 STARK proving 비례 구간이 커지면 native ARM의 이점이 드러날 것.
>
> 상세 프로파일링은 [sp1-zk-dex-vs-baseline.md](./sp1-zk-dex-vs-baseline.md) 참조.

### 확인

```bash
# L1
curl -s -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# L2
curl -s -X POST http://localhost:1729 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

### Docker Compose 구성

| 파일 | 역할 |
|------|------|
| `docker-compose.yaml` | 기본 서비스 정의 (L1, deployer, L2, prover) |
| `docker-compose-zk-dex.overrides.yaml` | ZK-DEX + SP1 + GPU 설정 |
| `docker-compose-zk-dex-gpu.overrides.yaml` | GPU 가속 opt-in |
| `Dockerfile.sp1` (repo root) | SP1 툴체인 포함 Docker 이미지 |

### Make 명령어

| 명령 | 설명 |
|------|------|
| `make zk-dex-docker` | 전체 기동 (SP1 + GPU) |
| `make zk-dex-docker-no-gpu` | SP1 CPU-only 모드 |
| `make zk-dex-docker-no-prover` | 프루버 제외 |
| `make zk-dex-docker-stop` | 전체 중지 |
| `make zk-dex-docker-status` | 컨테이너 상태 |
| `make zk-dex-docker-logs` | 로그 확인 |
| `make zk-dex-docker-clean` | 이미지/볼륨 완전 정리 |

---

## 엔드포인트

| 서비스 | URL |
|--------|-----|
| L1 RPC | `http://localhost:8545` |
| L2 RPC | `http://localhost:1729` |
| Proof Coordinator | `tcp://127.0.0.1:3900` |
| Prometheus Metrics | `http://localhost:3702` |

---

## Load Test (트랜잭션 생성)

Docker 환경이 실행 중인 상태에서 호스트에서 실행:

```bash
# 프로젝트 루트에서
make load-test

# 커스텀
LOAD_TEST_TX_AMOUNT=50 LOAD_TEST_RPC_URL=http://localhost:1729 make load-test

# 무한 반복
LOAD_TEST_ENDLESS=true make load-test
```

---

## 벤치마크 결과 수집

### 프루버 로그 확인

```bash
# 실시간 로그
./scripts/zk-dex-docker.sh logs prover

# 로그 파일로 저장
docker logs -f ethrex_prover 2>&1 | tee prover-sp1.log
```

### 증명 시간 집계

```bash
# crates/l2/ 디렉토리에서
../../scripts/bench_metrics.sh prover-sp1.log
# → bench_results.md 생성 (배치별 proving_time, gas, tx count, blocks)
```

### 사이클 프로파일링

```bash
# 전체 사이클 카운트
grep -E "└╴[0-9,]+ cycles" prover-sp1.log

# 주요 함수별 사이클
grep -E "(execute_block|apply_account_updates|validate_receipts_root|get_final_state_root|get_state_transitions)" prover-sp1.log | grep "cycles"

# 배치별 총 증명 시간
grep "proving_time_ms" prover-sp1.log
```

---

## 트러블슈팅

### Docker 이미지 빌드 실패

SP1 툴체인 설치가 Docker 빌드 중 실패할 경우:

```bash
# 캐시 없이 재빌드
docker build --no-cache -f Dockerfile.sp1 -t ethrex:sp1 .
```

### GPU 미감지

```bash
# NVIDIA 드라이버 확인
nvidia-smi

# NVIDIA Container Toolkit 확인
docker run --rm --gpus all nvidia/cuda:12.0-base nvidia-smi
```

GPU가 없으면 자동으로 CPU 모드로 전환된다.

### SP1 Groth16 Docker 이미지 접근 거부

SP1 Groth16 증명 단계에서 Docker 이미지 접근이 필요:

```bash
# GitHub Container Registry 인증
gh auth token | docker login ghcr.io -u <GITHUB_USERNAME> --password-stdin
docker pull ghcr.io/succinctlabs/sp1-gnark:v5.0.0
```

### 컨트랙트 배포 실패

```bash
# 배포 로그 확인
./scripts/zk-dex-docker.sh logs deploy

# 전체 정리 후 재시작
make zk-dex-docker-clean
make zk-dex-docker
```

### L2 시작 실패

```bash
# L2 로그 확인
./scripts/zk-dex-docker.sh logs l2

# L1이 정상인지 확인
curl -s -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

### 포트 충돌

이미 사용 중인 포트가 있으면:

```bash
# 포트 사용 확인
lsof -i :8545
lsof -i :1729

# 기존 컨테이너 정리
docker compose down
make zk-dex-docker-clean
```

### Docker 디스크 공간 부족

```bash
docker system prune -f
docker builder prune -f
```

---

## 네이티브 빌드 환경 (참고)

Docker 없이 직접 빌드하려면 기존 네이티브 스크립트를 사용할 수 있다.
정확한 벤치마크 측정이 필요한 경우 네이티브 환경이 더 적합할 수 있다.

### 추가 사전 요구사항

| 도구 | 버전 | 설치 |
|------|------|------|
| Rust | 1.90.0+ | `rustup update` |
| solc | **=0.8.31** (정확히) | 아래 참조 |
| git-lfs | 3.x+ | `brew install git-lfs && git lfs install` |
| Foundry (forge) | 최신 | `curl -L https://foundry.paradigm.xyz \| bash && foundryup` |
| SP1 toolchain | 최신 | `curl -L https://sp1.succinct.xyz \| bash && sp1up` |

### solc 0.8.31 설치

```bash
curl -L "https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-macos" \
  -o /usr/local/bin/solc
chmod +x /usr/local/bin/solc
solc --version
```

### 네이티브 실행

```bash
cd crates/l2

# 네이티브 ZK-DEX 전체 환경
make zk-dex-localnet

# 프루버 없이
make zk-dex-localnet-no-prover

# 개별 실행 (터미널 4개)
make init-l1-docker          # Terminal 1: L1
make deploy-l1-sp1-zk-dex   # Terminal 1: 배포
ETHREX_NO_MONITOR=true make init-l2-zk-dex  # Terminal 2: L2
PROVER_CLIENT_TIMED=true make init-prover-sp1-zk-dex  # Terminal 3: 프루버
```

---

## 환경 변수 정리

| 변수 | 기본값 | 설명 |
|------|--------|------|
| `PROVER_CLIENT_TIMED` | false | 배치별 증명 시간 로깅 |
| `ETHREX_GUEST_PROGRAM_ID` | evm-l2 | 게스트 프로그램 ID |
| `GUEST_PROGRAMS` | evm-l2 | 빌드할 게스트 프로그램 목록 (comma-separated) |
| `ETHREX_REGISTER_GUEST_PROGRAMS` | (없음) | 배포 시 등록할 게스트 프로그램 |
| `ETHREX_L2_SP1` | false | SP1 검증자 배포 여부 |
| `LOAD_TEST_TX_AMOUNT` | 1000 | 계정당 트랜잭션 수 |
| `LOAD_TEST_ENDLESS` | false | 무한 반복 모드 |
| `LOAD_TEST_RPC_URL` | http://localhost:8545 | L2는 http://localhost:1729 사용 |

---

## 파일 구조

```
ethrex/
  Dockerfile.sp1                              # SP1 Docker 이미지
  crates/l2/
    docker-compose.yaml                       # 기본 서비스 정의
    docker-compose-zk-dex.overrides.yaml      # ZK-DEX + SP1 + GPU
    docker-compose-zk-dex-gpu.overrides.yaml   # GPU 가속 opt-in
    programs-zk-dex.toml                      # ZK-DEX 프루버 설정
    Makefile                                  # make 명령어
    scripts/
      zk-dex-docker.sh                        # Docker 기반 스크립트
      zk-dex-localnet.sh                      # 네이티브 기반 스크립트
```

---

## 참고 문서

- [docs/l2/prover-benchmarking.md](../../docs/l2/prover-benchmarking.md) — upstream 프루버 벤치마킹 가이드
- [scripts/bench_metrics.sh](../../scripts/bench_metrics.sh) — 배치별 proving_time 집계 스크립트
- [tokamak-notes/sp1-profiling-baseline.md](./sp1-profiling-baseline.md) — SP1 사이클 프로파일링 분석
- [crates/l2/scripts/ZK-DEX-LOCALNET.md](../crates/l2/scripts/ZK-DEX-LOCALNET.md) — 네이티브 로컬넷 Quick Start
