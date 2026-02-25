#!/bin/bash
# ==============================================================================
# Ethrex ZK-Proof Verifier vs Full Node Benchmark Script
# ==============================================================================
# This script runs a quick simulation of the node under two different configurations:
# 1. Full Node (Regular Execution)
# 2. Ultra-Light ZK-Verifier Node (No Execution, Memory Only)
#
# It measures CPU and Memory footprints during block synchronization.

BINARY="./target/release/ethrex"
DATADIR_FULL="/tmp/ethrex_bench_full"
DATADIR_ZK="/tmp/ethrex_bench_zk"
NETWORK="holesky"
RUNTIME_SEC=15

# Ensure binary is built in release mode for accurate benchmarking
echo "Building ethrex in release mode..."
cargo build --release -q

echo ""
echo "============================================================"
echo "▶ Running FULL NODE Benchmark..."
echo "============================================================"
# Clean up previous runs
rm -rf $DATADIR_FULL
rm -f /tmp/ethrex_full_stats.txt
rm -f /tmp/ethrex_full_node.log

echo "[1/2] Starting Full Node on network: $NETWORK for $RUNTIME_SEC seconds..."
$BINARY --network $NETWORK --datadir $DATADIR_FULL --syncmode full > /tmp/ethrex_full_node.log 2>&1 &
FULL_PID=$!

# Monitor resources for RUNTIME_SEC
echo "Monitoring resources for PID $FULL_PID..."
for i in $(seq 1 $RUNTIME_SEC); do
    # Log memory and cpu footprint roughly
    ps -p $FULL_PID -o %cpu,%mem,rss >> /tmp/ethrex_full_stats.txt 2>/dev/null
    sleep 1
done

# Kill node
kill $FULL_PID 2>/dev/null || true
echo "Full Node Benchmark Complete."

echo ""
echo "============================================================"
echo "▶ Running ZK-VERIFIER NODE Benchmark..."
echo "============================================================"
rm -rf $DATADIR_ZK
rm -f /tmp/ethrex_zk_stats.txt
rm -f /tmp/ethrex_zk_node.log

echo "[2/2] Starting ZK Verifier Node on network: $NETWORK for $RUNTIME_SEC seconds..."
$BINARY --network $NETWORK --datadir $DATADIR_ZK --syncmode full --zk-verifier-only --http.port 8546 --p2p.port 30304 --discovery.port 30304 --authrpc.port 8552 --metrics.port 9091 > /tmp/ethrex_zk_node.log 2>&1 &
ZK_PID=$!

# Monitor resources
echo "Monitoring resources for PID $ZK_PID..."
for i in $(seq 1 $RUNTIME_SEC); do
    ps -p $ZK_PID -o %cpu,%mem,rss >> /tmp/ethrex_zk_stats.txt 2>/dev/null
    sleep 1
done

# Kill node
kill $ZK_PID 2>/dev/null || true
echo "ZK Verifier Benchmark Complete."

echo ""
echo "============================================================"
echo "📊 BENCHMARK RESULTS (Averages)"
echo "============================================================"
echo "[FULL NODE]"
if [ -f "/tmp/ethrex_full_stats.txt" ]; then
    awk 'NR>1 {cpu+=$1; mem+=$2; rss+=$3; count++} END {if(count>0) printf "Avg CPU: %.2f%%\nAvg RAM Usage: %.2f MB\n", cpu/count, (rss/count)/1024}' /tmp/ethrex_full_stats.txt
    echo "Directory Size:" $(du -sh $DATADIR_FULL/database 2>/dev/null | awk '{print $1}')
else
    echo "Failed to get stats."
fi

echo ""
echo "[ZK-VERIFIER NODE]"
if [ -f "/tmp/ethrex_zk_stats.txt" ]; then
    awk 'NR>1 {cpu+=$1; mem+=$2; rss+=$3; count++} END {if(count>0) printf "Avg CPU: %.2f%%\nAvg RAM Usage: %.2f MB\n", cpu/count, (rss/count)/1024}' /tmp/ethrex_zk_stats.txt
    echo "Directory Size: 0B (Memory Only)"
else
    echo "Failed to get stats."
fi
echo "============================================================"
