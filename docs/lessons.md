# Lessons Learned

## 2026-02-28 - geth-db-migrate TUI lifecycle

- If a TUI task is spawned for user-visible completion flow, keep its `JoinHandle` and await it explicitly at shutdown paths.
- Avoid sleep-based synchronization (`tokio::time::sleep`) for UI completion; prefer deterministic channel close + task join.
- Verify both success and error exits for spawned UI/background tasks.

## 2026-03-02 - Migration verifier runtime dependencies

- For operator-facing verification tools in this repository, prefer Rust examples/binaries over external scripting runtimes when users may not have extra toolchains installed.
- Keep verification phases explicit (`block`, `account`, `proof`, `asset`) with periodic progress lines so long-running checks are observable in real time.

## 2026-03-02 - Offline-first migration verification scope

- For DB migration verification, separate offline guarantees (canonical/hash/header/root parity) from online RPC guarantees (balances/calls/proofs) and avoid mixing them in one tool mode.
- If the requirement is offline-only, use direct store readers and keep optional checks explicit (e.g., `--skip-state-trie-check`).
