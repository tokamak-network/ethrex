# Tools 컨테이너 격리 분석 (Multi-Deployment)

## 현상

로컬 L2와 세폴리아 L2를 동시에 배포했을 때, 나중에 시작한 배포의 대시보드(Blockscout + Bridge UI)가
기존 배포의 대시보드를 덮어씌워서 이전 대시보드가 내려감.

## 근본 원인 (3가지)

### 1. 이전 서버 프로세스에 `-p` 플래그 미반영 (해결됨)

**원인**: Tauri 앱이 시작한 `node server.js` 프로세스가 코드 수정 후에도 재시작되지 않아,
이전 버전 코드(프로젝트명 없이 tools 시작)로 실행됨.

**증거**: 서버 재시작 후 `-p tokamak-639f587e-tools` 정상 전달 확인.
컨테이너명이 `l2-*` → `tokamak-639f587e-tools-*`로 정상 생성됨.

**해결**: 서버 재시작 시 자동으로 해결. 코드상 `-p projectName`은 이미 올바르게 구현됨.

### 2. Tools 포트 범위 겹침 (미해결 - 핵심 문제)

**현상**: 로컬 배포의 L1 explorer 포트와 세폴리아 배포의 L2 explorer 포트가 충돌.

| 항목 | 로컬 (1269ad5e) | 세폴리아 (639f587e) |
|---|---|---|
| L2 Explorer | 8083 | **8084** |
| L1 Explorer | **8084** | 8085 |
| Bridge UI | 3010 | 3011 |
| DB | 7433 | 7434 |

로컬의 `tools_l1_explorer_port=8084`와 세폴리아의 `tools_l2_explorer_port=8084`가 충돌.

**원인**: `getNextAvailablePorts()`에서 L1 explorer와 L2 explorer를 **독립적인 시퀀스**로 할당:
```javascript
// L1: MAX(tools_l1_explorer_port) 기반 → 8084, 8085, 8086...
// L2: MAX(tools_l2_explorer_port) 기반 → 8083, 8084, 8085...
```
두 시퀀스가 겹치는 구간이 발생. `findFreePort()`가 실제 OS 포트 점유를 체크하지만,
**같은 트랜잭션 내에서 할당하는 다른 포트와의 충돌은 체크하지 않음**.

**해결 방안**: 모든 tools 포트를 하나의 통합된 범위에서 순차 할당하거나,
L1과 L2 explorer 포트 간 최소 간격을 보장.

### 3. blockscout-db Named Volume 공유 (잠재적 문제)

Docker Compose named volume `blockscout-db`는 프로젝트별로 분리됨:
- `tokamak-1269ad5e-tools_blockscout-db`
- `tokamak-639f587e-tools_blockscout-db`

프로젝트명이 다르면 자동으로 격리되므로, `-p` 수정 후에는 문제 없음.

### 4. `stopTools()` 프로젝트명 누락 (해결됨)

`routes/deployments.js:543`에서 `stopTools()`에 프로젝트명 미전달 → 기본 `l2` 프로젝트 삭제.
이미 수정 완료: `stopTools(\`${deployment.docker_project}-tools\`)`.

---

## 해결 계획

### Phase 1: 포트 할당 로직 수정 (필수)

파일: `crates/desktop-app/local-server/db/deployments.js`

```javascript
async function getNextAvailablePorts() {
  // 기존: L1/L2 explorer를 각각의 MAX에서 +1
  // 수정: 전체 tools explorer 포트를 하나의 풀에서 할당

  const result = db.prepare(`
    SELECT MAX(
      CASE WHEN tools_l1_explorer_port > tools_l2_explorer_port
           THEN tools_l1_explorer_port
           ELSE tools_l2_explorer_port END
    ) as max_explorer_port,
    ...
  `).get();

  const baseExplorer = (result.max_explorer_port || 8082) + 1;
  const toolsL2ExplorerPort = await findFreePort(baseExplorer);
  const toolsL1ExplorerPort = await findFreePort(toolsL2ExplorerPort + 1);
  // L2를 먼저 할당하고, L1은 그 다음 포트부터 검색
}
```

핵심: **모든 explorer 포트를 하나의 증가 시퀀스로 할당**하여 겹침 방지.

### Phase 2: Proxy 포트 통합 (선택)

현재 proxy 컨테이너가 L1/L2 explorer 두 포트를 모두 바인딩.
세폴리아(external L1)의 경우 `proxy-l2-only` 사용으로 L1 포트 불필요.

포트 절약 방안: 세폴리아 배포에서 `tools_l1_explorer_port`를 null로 설정.

### Phase 3: 기존 배포 포트 마이그레이션

이미 생성된 배포의 포트가 겹치는 경우, DB를 직접 수정하거나
restart-tools 시 동적으로 포트를 재할당하는 로직 추가.

---

## 현재 상태

| 항목 | 상태 |
|---|---|
| `-p` 프로젝트명 전달 | 해결됨 (코드 정상, 서버 재시작 필요) |
| `stopTools()` 프로젝트명 | 해결됨 (커밋 완료) |
| 포트 충돌 (L1/L2 explorer) | **미해결** — 코드 수정 필요 |
| Named volume 격리 | 해결됨 (프로젝트명 분리로 자동 격리) |
