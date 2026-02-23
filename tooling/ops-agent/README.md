# ethrex-ops-agent (observe-only MVP)

## Scope
- Detect only (no automatic action)
- Persist incidents into SQLite
- Send Telegram alerts

## Covered scenarios
1. Block height stall (>= 180s)
2. Execution RPC timeout rate (> 30%, 2 consecutive checks)
3. CPU pressure (> 90%, 3 consecutive checks)

## Required environment variables
- `OPS_AGENT_PROMETHEUS_BASE_URL`
- `OPS_AGENT_EXECUTION_RPC_URL`
- `OPS_AGENT_TELEGRAM_BOT_TOKEN`
- `OPS_AGENT_TELEGRAM_CHAT_ID`

## Optional environment variables
- `OPS_AGENT_SQLITE_PATH` (default: `ops-agent.sqlite`)
- `OPS_AGENT_POLL_SECONDS` (default: `30`)
- `OPS_AGENT_TELEGRAM_RETRY_MAX` (default: `3`)
- `OPS_AGENT_TELEGRAM_RETRY_DELAY_MS` (default: `500`)

## Alert delivery behavior
Telegram alert sending uses retry-by-default:
- max retries: `3`
- retry delay: `500ms`

## False-positive measurement
Incidents are stored with nullable `false_positive`.
- `NULL`: unlabeled
- `1`: false positive
- `0`: true positive

Current repository methods:
- `mark_false_positive(incident_id, bool)`
- `false_positive_rate()`

## Labeling helper CLI
Use the included helper to label incidents and track false-positive rate.

```bash
# list recent incidents
OPS_AGENT_SQLITE_PATH=ops-agent.sqlite cargo run -p ethrex-ops-agent --bin incident-label -- list 20

# label incident as false-positive
OPS_AGENT_SQLITE_PATH=ops-agent.sqlite cargo run -p ethrex-ops-agent --bin incident-label -- label 42 fp

# label incident as true-positive
OPS_AGENT_SQLITE_PATH=ops-agent.sqlite cargo run -p ethrex-ops-agent --bin incident-label -- label 43 tp
```
