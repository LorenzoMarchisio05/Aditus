#!/usr/bin/env bash
# Starts vite + the test server, runs the debug app's self-test, prints logs, asserts. Usage: selftest-run.sh <binary>
bin=$1
export ADITUS_SERVER_LOG=server.log; rm -f server.log
(npm run dev > vite.log 2>&1 &)
python3 scripts/selftest-server.py 8765 &
sleep 8
ADITUS_DATA_DIR="${RUNNER_TEMP:-/tmp}/aditus-data" ADITUS_SELFTEST_PORT=8765 ADITUS_SELFTEST_EXIT=1 timeout 240 "$bin" > app.log 2>&1
echo "exit=$?"
cat app.log
echo ---- server; cat server.log || true
scripts/selftest-check.sh app.log server.log
