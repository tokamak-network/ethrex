# Ops-Agent Observe-Only MVP Status

## Implemented modules
- collector: Prometheus query API + execution RPC block number polling
- diagnoser: sync domain rules (3 scenarios)
- alerter: Telegram bot alerts with retry policy
- storage: SQLite incident persistence + false-positive labeling/rate

## Target scenarios
1. Block height stall (>=180s)
2. Execution RPC timeout rate (>30% consecutive)
3. CPU pressure (>90% for 3 consecutive checks)

## Runtime flow
collect snapshot -> evaluate rules -> store incident -> send Telegram alert

## False-positive measurement
- storage schema includes nullable `false_positive`
- helper CLI for labeling incidents (`incident-label`)
- computed false-positive rate from labeled rows

## Test coverage present in codebase
- diagnoser unit tests (3 scenarios)
- collector parsing/merge unit tests
- storage unit tests (insert, labeling rate, recent list)
- config unit tests (defaults and env overrides)
- service flow integration test (detect->store->alert path)

## Remaining blocker
- Cargo/Rust toolchain missing in current execution environment (`cargo: command not found`)
- therefore build/test execution cannot be validated in this environment

## Validation commands (to run in Rust-enabled environment)
```bash
cd tooling
cargo build -p ethrex-ops-agent
cargo test -p ethrex-ops-agent
```
