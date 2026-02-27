#!/bin/bash
# ==============================================================================
# Local Devnet Simulation (Test 3) - Full Node vs ZK-Verifier Node
# ==============================================================================
# This script spins up a local devnet with two nodes:
# 1. A Producer (Full Node) that mines/processes transactions.
# 2. A ZK-Verifier Node that connects to the Producer via P2P and syncs 
#    by requesting Block Proofs instead of executing bodies.

BINARY="./target/release/ethrex"
PRODUCER_DIR="/tmp/ethrex_producer"
VERIFIER_DIR="/tmp/ethrex_verifier"
NETWORK="dev"

echo "Building release binary..."
cargo build --release --features l2 -q

echo "============================================================"
echo "Cleaning up old data directories..."
rm -rf $PRODUCER_DIR $VERIFIER_DIR
rm -f /tmp/ethrex_producer.log /tmp/ethrex_verifier.log

echo "============================================================"
echo "[Step 1] Initializing Producer (Full Node)..."
$BINARY --dev --datadir $PRODUCER_DIR --http.port 8545 --p2p.port 30303 --discovery.port 30303 --authrpc.port 8551 --metrics.port 9090 > /tmp/ethrex_producer.log 2>&1 &
PRODUCER_PID=$!

sleep 3

# Wait for producer to write node record to get its enode URL
sleep 3
ENODE=$(grep "Local node initialized enode=" /tmp/ethrex_producer.log | awk -F'enode=' '{print $2}' | tr -d '\n')
if [ -z "$ENODE" ]; then
    echo "Warning: Producer might have failed to start or write enode. Check /tmp/ethrex_producer.log"
    exit 1
fi
echo "Producer ENODE discovered: $ENODE"
echo "Producer is running (PID: $PRODUCER_PID)."

echo "============================================================"
echo "[Step 2] Initializing ZK-Verifier Node..."
# Notice the addition of --zk-verifier-only and --bootnodes
$BINARY --dev --bootnodes "$ENODE" --datadir $VERIFIER_DIR --zk-verifier-only --http.port 8546 --p2p.port 30304 --discovery.port 30304 --authrpc.port 8552 --metrics.port 9091 > /tmp/ethrex_verifier.log 2>&1 &
VERIFIER_PID=$!

echo "Verifier is running (PID: $VERIFIER_PID)."
echo "Both nodes are running. The Verifier should now connect to the Producer exclusively, request Headers, and issue 'GetBlockProofs' requests instead of 'GetBlockBodies'."

echo "============================================================"
echo "Waiting for 25 seconds to allow P2P discovery and sync simulation..."
sleep 25

echo "Shutting down nodes..."
kill $PRODUCER_PID 2>/dev/null || true
kill $VERIFIER_PID 2>/dev/null || true

echo "============================================================"
echo "Simulation Finished! Check the following logs to verify behavior:"
echo "1. Producer: tail -n 20 /tmp/ethrex_producer.log"
echo "2. Verifier: tail -n 20 /tmp/ethrex_verifier.log"
echo "   -> Look for: 'INFO ZK-Verifier: Requesting block proofs'"
echo "   -> Look for: 'Warning: ... SP1 proof deserialized successfully ...' or 'Skipping EVM execution'"
echo "============================================================"
