#!/usr/bin/env bash
set -e

KERNEL_PID=""
JAVA_PID=""

cleanup() {
    echo "Cleaning up..."
    [ -n "$KERNEL_PID" ] && kill "$KERNEL_PID" 2>/dev/null || true
    [ -n "$JAVA_PID" ] && kill "$JAVA_PID" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "=== Building Rust kernel ==="
cd /agos-kernel
cargo build --quiet

echo "=== Starting agOS Kernel on :7400 ==="
cargo run &
KERNEL_PID=$!
sleep 4

echo "=== Verifying kernel health ==="
curl -sf http://127.0.0.1:7400/health -o /dev/null && echo "OK" || (echo "FAILED"; exit 1)

echo "=== Installing .aapp packages ==="
curl -sf -X POST http://127.0.0.1:7400/sandbox/install \
    -H 'Content-Type: application/json' \
    -d '{"name":"financial_auditor","aapps_path":"aapps"}' -o /dev/null && echo "  financial_auditor: OK"
curl -sf -X POST http://127.0.0.1:7400/sandbox/install \
    -H 'Content-Type: application/json' \
    -d '{"name":"accountant","aapps_path":"aapps"}' -o /dev/null && echo "  accountant: OK"

echo "=== Testing kernel panic (async fix) ==="
curl -sf -X POST http://127.0.0.1:7400/kernel/panic \
    -H 'Content-Type: application/json' \
    -d '{"pid":"test-pid","reason":"test panic"}' | python3 -m json.tool

echo ""
echo "=== Starting Spring Boot Demo on :8080 ==="
cd /agos-demo
mvn spring-boot:run -q &
JAVA_PID=$!
sleep 10

echo "=== Verifying Java app ==="
curl -sf http://127.0.0.1:8080/ -o /dev/null && echo "OK" || (echo "FAILED"; exit 1)

echo "=== Running full demo orchestration ==="
RESULT=$(curl -s -X POST http://127.0.0.1:8080/api/demo/run)
echo "$RESULT" | python3 -m json.tool

sleep 2

echo "=== Detailed results ==="
curl -s http://127.0.0.1:8080/api/demo/status | python3 -c "
import json, sys
data = json.load(sys.stdin)
for s in data['steps']:
    mark = '✓' if s['success'] else '✗'
    print(f'{mark} [{s[\"phase\"]}] {s[\"title\"]} ({s[\"elapsedMs\"]}ms)')
    if not s['success']:
        print(f'   └─ {s[\"detail\"]}')
print(f'\nTotal: {data[\"total\"]}, Passed: {data[\"passed\"]}, Failed: {data[\"failed\"]}')
"

echo ""
echo "=== Demo complete ==="
