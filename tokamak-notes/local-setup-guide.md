# 로컬 프루버 벤치마크 환경 구축 가이드

## 사전 요구사항

### 필수 도구

| 도구 | 버전 | 설치 |
|------|------|------|
| Rust | 1.90.0+ | `rustup update` |
| Docker | 28.x+ | [Docker Desktop](https://www.docker.com/products/docker-desktop/) |
| Docker Compose | v2.x+ | Docker Desktop에 포함 |
| solc | **=0.8.31** (정확히) | 아래 참조 |
| git-lfs | 3.x+ | `brew install git-lfs && git lfs install` |
| Foundry (forge) | 최신 | `curl -L https://foundry.paradigm.xyz | bash && foundryup` |

### solc 0.8.31 설치

ethrex 컨트랙트는 `pragma solidity =0.8.31` (정확한 버전)을 요구한다.
brew 기본 설치(`brew install solidity`)는 최신 버전이 설치되므로 **직접 다운로드**해야 한다.

```bash
# macOS (Apple Silicon / Intel)
curl -L "https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-macos" \
  -o /usr/local/bin/solc
chmod +x /usr/local/bin/solc

# 확인
solc --version
# solc, the solidity compiler commandline interface
# Version: 0.8.31+commit.fd3a2265.Darwin.appleclang
```

> **주의**: `solc-select`가 설치되어 있으면 PATH에서 solc를 가로챌 수 있다.
> `which solc`로 올바른 바이너리가 사용되는지 확인할 것.

### GPU (선택)

- NVIDIA GPU + CUDA: SP1/RISC0 GPU 가속 가능
- GPU 없어도 CPU 모드로 동작 (느리지만 기능적으로 동일)

---

## 환경 구축 단계

모든 명령은 `crates/l2/` 디렉토리에서 실행한다.

```bash
cd crates/l2
```

### Step 1: 기존 환경 정리

```bash
make down 2>/dev/null
# DB도 정리할 경우:
# make rm-db-l1
# make rm-db-l2
```

### Step 2: L1 Docker 기동

```bash
make init-l1-docker
```

확인:
```bash
curl -s -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
# {"jsonrpc":"2.0","id":1,"result":"0x..."}
```

### Step 3: L1 컨트랙트 배포

**백엔드에 맞는 배포 명령을 사용해야 한다:**

```bash
# exec 백엔드 (증명 없이 실행만)
make deploy-l1

# SP1 백엔드 → 반드시 deploy-l1-sp1 사용
make deploy-l1-sp1

# RISC0 백엔드 → deploy-l1-risc0 사용
make deploy-l1-risc0
```

> **주의**: 잘못된 배포 명령을 사용하면 프루버가 coordinator에서 거부된다.
> 예: `deploy-l1`로 배포 후 SP1 프루버를 실행하면 검증자 주소 불일치로 증명이 제출되지 않는다.
> 재배포가 필요하면 L1 Docker를 완전히 재시작해야 한다 (트러블슈팅 참조).

성공 시 마지막에 `Deployer binary finished successfully` 출력.
`.env` 파일이 `cmd/.env`에 생성된다 (브릿지/프로포저 주소 포함).

주소 충돌을 방지하려면:
```bash
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true make deploy-l1-sp1
```

### Step 4: L2 시퀀서 기동

```bash
ETHREX_NO_MONITOR=true make init-l2
```

> `ETHREX_NO_MONITOR=true`: TUI 모니터 비활성화, 일반 로그 출력

확인:
```bash
curl -s -X POST http://localhost:1729 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

### Step 5: 프루버 실행

**터미널을 새로 열어서** 실행:

#### SP1 사전 준비 (최초 1회)

```bash
# 1. SP1 succinct 툴체인 설치
curl -L https://sp1.succinct.xyz | bash
sp1up

# 2. Groth16 Docker 이미지 사전 풀 (GHCR 인증 필요)
gh auth token | docker login ghcr.io -u <GITHUB_USERNAME> --password-stdin
docker pull ghcr.io/succinctlabs/sp1-gnark:v5.0.0
```

> 이미지를 사전에 풀하지 않으면 증명 생성 마지막 단계에서 실패한다.

#### RISC0 사전 준비 (최초 1회)

```bash
# ARM 네이티브 터미널에서 실행 (arch → arm64 확인)
curl -L https://risczero.com/install | bash
rzup
```

#### 프루버 실행

```bash
cd crates/l2

# exec 백엔드 (증명 없이 실행만, 파이프라인 검증용)
PROVER_CLIENT_TIMED=true make init-prover-exec

# SP1 백엔드 (CPU 모드, 실제 ZK 증명 생성)
PROVER_CLIENT_TIMED=true make init-prover-sp1

# RISC0 백엔드 (CPU 모드, 실제 ZK 증명 생성)
PROVER_CLIENT_TIMED=true make init-prover-risc0

# GPU 가속 (NVIDIA GPU 있는 경우)
PROVER_CLIENT_TIMED=true GPU=true make init-prover-sp1
```

### Step 6: Load Test (트랜잭션 생성)

**터미널을 또 새로 열어서** 실행:

```bash
cd crates/l2

# 기본: 계정당 1000 tx
make load-test

# 커스텀
LOAD_TEST_TX_AMOUNT=50 LOAD_TEST_RPC_URL=http://localhost:1729 make load-test

# 무한 반복
LOAD_TEST_ENDLESS=true make load-test
```

### Step 7: 벤치마크 결과 수집

#### 7-1. 프루버 로그 저장

Step 5에서 로그를 파일로 저장한다:

```bash
PROVER_CLIENT_TIMED=true make init-prover-sp1 2>&1 | tee prover-sp1.log
```

#### 7-2. 증명 시간 집계 (bench_metrics.sh)

L2 시퀀서가 실행 중인 상태에서 실행한다 (Prometheus 메트릭 참조):

```bash
# crates/l2/ 디렉토리에서
../../scripts/bench_metrics.sh prover-sp1.log
# → bench_results.md 생성 (배치별 proving_time, gas, tx count, blocks)

# 커스텀 메트릭 URL
../../scripts/bench_metrics.sh prover-sp1.log http://localhost:3702/metrics
```

출력 예시:
```
| Batch | Time (s) | Time (ms) | Gas Used | Tx Count | Blocks |
|-------|----------|-----------|----------|----------|--------|
| 1     | 1664     | 1664553   | 21000    | 1        | 5      |
```

#### 7-3. 사이클 프로파일링 분석

`bench_metrics.sh`는 proving_time만 집계한다. 사이클 분석은 로그에서 직접 추출한다:

```bash
# 전체 사이클 카운트 추출 (named spans)
grep -E "└╴[0-9,]+ cycles" prover-sp1.log

# 주요 함수별 사이클 추출
grep -E "(execute_block|apply_account_updates|validate_receipts_root|get_final_state_root|get_state_transitions)" prover-sp1.log | grep "cycles"

# 배치별 총 증명 시간
grep "proving_time_ms" prover-sp1.log

# 전체 실행 사이클 (execution 블록 종료)
grep -E "^.*└╴[0-9,]+ cycles$" prover-sp1.log | tail -1

# STARK 증명 속도 (clk 진행)
grep "clk = " prover-sp1.log
```

사이클 분석 결과 정리 형식:

```
Section                          Cycles        % of Total
────────────────────────────────────────────────────────
read_input                       1,012,951     1.6%
execute_block                    29,363,722    45.6%
validate_receipts_root           4,619,876     7.2%
apply_account_updates            2,824,380     4.4%
get_final_state_root             1,974,096     3.1%
get_state_transitions            333,823       0.5%
...
Total execution                  64,345,179    100%
```

> 상세 분석 예시는 `tokamak-notes/sp1-profiling-baseline.md` 참조.

---

## 터미널 구성

```
Terminal 1: L1 Docker     (make init-l1-docker → 실행 중 유지)
Terminal 2: L2 Sequencer  (make init-l2 → 실행 중 유지)
Terminal 3: Prover        (make init-prover-xxx → 실행 중 유지)
Terminal 4: Load Test     (make load-test → 트랜잭션 생성)
```

---

## 벤치마크 비교 실행 방법

SP1과 RISC0를 비교하려면:

```bash
# 1. 환경 정리 후 동일 조건으로 재시작
make down && make rm-db-l2
make init-l1-docker && make deploy-l1

# 2. SP1 벤치마크
ETHREX_NO_MONITOR=true make init-l2 &
sleep 30
PROVER_CLIENT_TIMED=true make init-prover-sp1 2>&1 | tee prover-sp1.log &
sleep 10
LOAD_TEST_TX_AMOUNT=100 make load-test
# 배치 완료 대기...
../../scripts/bench_metrics.sh prover-sp1.log
mv bench_results.md bench_results_sp1.md

# 3. 환경 정리 후 RISC0로 재실행
# (동일 과정, sp1 → risc0로 변경)
```

---

## 트러블슈팅

### solc 버전 불일치

```
Error: Source file requires different compiler version
pragma solidity =0.8.31;
```

→ solc 0.8.31 정확한 버전이 필요. 위 설치 가이드 참조.

### solc-select PATH 충돌

```bash
which solc
# /Library/Frameworks/Python.framework/Versions/3.8/bin/solc  ← 잘못된 경로
```

Python `solc-select` 래퍼가 PATH 우선순위가 높을 경우:

```bash
sudo mv /Library/Frameworks/Python.framework/Versions/3.8/bin/solc \
  /Library/Frameworks/Python.framework/Versions/3.8/bin/solc-select-wrapper.bak
```

### git-lfs 미설치

```
git-lfs filter-process: git-lfs: command not found
```

→ `brew install git-lfs && git lfs install`

### L2 연결 실패

```
L2 not ready
```

→ L2 시퀀서 시작 후 30초 이상 대기 필요 (빌드 포함 시 수 분).
→ `curl http://localhost:1729`로 확인.

### SP1 succinct 툴체인 미설치

```
error: toolchain 'succinct' is not installed
```

→ SP1 빌드에 필요한 커스텀 Rust 툴체인:

```bash
curl -L https://sp1.succinct.xyz | bash
sp1up
```

### SP1 Groth16 Docker 이미지 접근 거부

```
Unable to find image 'ghcr.io/succinctlabs/sp1-gnark:v5.0.0' locally
docker: Error response from daemon: error from registry: denied
```

→ GitHub Container Registry 인증 필요:

```bash
# gh CLI 사용
gh auth token | docker login ghcr.io -u <GITHUB_USERNAME> --password-stdin

# 또는 PAT 사용
echo $GITHUB_TOKEN | docker login ghcr.io -u <GITHUB_USERNAME> --password-stdin
```

### SP1 빌드 "File exists" 에러

```
panicked at 'called `Result::unwrap()` on an `Err` value: Os { code: 17, kind: AlreadyExists }'
```

→ 빌드 캐시 정리:

```bash
rm -rf target/release/build/sp1-recursion-core-*
```

### L1 컨트랙트 재배포 실패

이미 배포된 상태에서 다시 배포하면 transaction receipt 에러 발생.
→ L1 Docker를 완전히 재시작:

```bash
make down
make rm-db-l1
make init-l1-docker
# 잠시 대기 후
make deploy-l1-sp1  # SP1 검증자 포함
```

### RISC0 rzup x86_64 macOS 미지원

Rosetta 2 환경(x86_64 emulation)에서는 `rzup`이 지원하지 않음:

```
Unsupported OS/Architecture: macos/amd64
```

→ ARM 네이티브 터미널에서 실행하거나, 소스에서 직접 빌드:

```bash
# ARM 네이티브 터미널 확인
arch  # arm64여야 함

# ARM 네이티브에서 설치
curl -L https://risczero.com/install | bash
rzup
```

### Docker 컨테이너 충돌

```bash
docker compose down
docker system prune -f
make init-l1-docker
```

---

## 환경 변수 정리

| 변수 | 기본값 | 설명 |
|------|--------|------|
| `PROVER_CLIENT_TIMED` | false | 배치별 증명 시간 로깅 |
| `PROVER_CLIENT_BACKEND` | exec | 프루버 백엔드 (exec/sp1/risc0) |
| `GPU` | (없음) | GPU 가속 활성화 |
| `ETHREX_NO_MONITOR` | (없음) | TUI 모니터 비활성화 |
| `LOAD_TEST_TX_AMOUNT` | 1000 | 계정당 트랜잭션 수 |
| `LOAD_TEST_ENDLESS` | false | 무한 반복 모드 |
| `LOAD_TEST_RPC_URL` | http://localhost:8545 | L2는 http://localhost:1729 사용 |
| `COMPILE_CONTRACTS` | (없음) | 설정 시 컨트랙트 재컴파일 |
| `ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT` | (없음) | 설정 시 컨트랙트 주소 랜덤화 |

---

## 참고 문서

- [docs/l2/prover-benchmarking.md](../../docs/l2/prover-benchmarking.md) — upstream 공식 프루버 벤치마킹 가이드
- [scripts/bench_metrics.sh](../../scripts/bench_metrics.sh) — 배치별 proving_time 집계 스크립트
- [tokamak-notes/sp1-profiling-baseline.md](./sp1-profiling-baseline.md) — SP1 사이클 프로파일링 분석 결과
