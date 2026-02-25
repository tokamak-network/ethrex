#!/bin/bash
# run_verifier_unit_tests.sh
# ZK-Verifier 모드 테스트 스크립트

echo "Running ZK-Verifier mode unit tests..."

# ZK Verifier 바이패스 파이프라인 전용 테스트가 있다면 지정 (ex. cargo test verifier )
cargo test zk_verifier --features l2

if [ $? -eq 0 ]; then
    echo "ZK-Verifier unit tests passed!"
else
    echo "ZK-Verifier unit tests failed!"
    exit 1
fi
