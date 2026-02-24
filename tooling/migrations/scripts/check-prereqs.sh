#!/usr/bin/env bash
set -euo pipefail

missing=0

if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  echo "[missing] cargo not found in PATH (or ~/.cargo/bin/cargo)"
  missing=1
else
  echo "[ok] cargo found"
fi

if command -v rustc >/dev/null 2>&1 || [ -x "$HOME/.cargo/bin/rustc" ]; then
  echo "[ok] rustc found"
else
  echo "[missing] rustc not found in PATH (or ~/.cargo/bin/rustc)"
  missing=1
fi

clang_glob=( -name 'libclang.so' -o -name 'libclang-*.so' -o -name 'libclang.so.*' -o -name 'libclang-*.so.*' )

if [ -n "${LIBCLANG_PATH:-}" ]; then
  libclang_candidate="$(find "$LIBCLANG_PATH" -maxdepth 3 -type f \( "${clang_glob[@]}" \) 2>/dev/null | head -n 1 || true)"
  if [ -n "$libclang_candidate" ]; then
    echo "[ok] libclang found via LIBCLANG_PATH: $libclang_candidate"
  else
    echo "[missing] LIBCLANG_PATH is set but no libclang shared library was found there"
    missing=1
  fi
else
  search_dirs=(
    /usr/lib
    /usr/lib64
    /usr/local/lib
    /lib
    /lib64
    /usr/lib/llvm-14/lib
    /usr/lib/llvm-15/lib
    /usr/lib/llvm-16/lib
    /usr/lib/llvm-17/lib
    /usr/lib/llvm-18/lib
  )

  libclang_candidate="$(find "${search_dirs[@]}" -type f \( "${clang_glob[@]}" \) 2>/dev/null | head -n 1 || true)"
  if [ -n "$libclang_candidate" ]; then
    echo "[ok] libclang shared library found: $libclang_candidate"
  else
    echo "[missing] libclang shared library not found"
    echo "          install libclang-dev/clang (or equivalent), or set LIBCLANG_PATH"
    missing=1
  fi
fi

if [ "$missing" -ne 0 ]; then
  echo "\nPrerequisite check failed."
  exit 1
fi

echo "\nAll prerequisite checks passed."
