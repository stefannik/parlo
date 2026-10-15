#!/usr/bin/env bash
# Real memory-pressure unload in the bundled app (MAC_HANDOFF B6): the app loads both models (15 min idle timeout), then
# `sudo memory_pressure -S -l warn` *simulates* a warning (no memory is consumed; needs your password) and the app must
# unload the idle engines at once.      scripts/mac-memory-pressure-test.sh [app=dist/Parlo.app]
set -euo pipefail
cd "$(dirname "$0")/.."
APP=${1:-dist/Parlo.app}
sudo -v                                   # ask for the password up front
D=$(mktemp -d); OUT=$D/p.out
PARLO_HOME=$D/home PARLO_SELFTEST=pressure "$APP/Contents/MacOS/$(defaults read "$PWD/$APP/Contents/Info" CFBundleExecutable)" >"$OUT" 2>&1 & PID=$!
SP=""; trap 'kill $PID $SP 2>/dev/null || true; sudo pkill -f "memory_pressure -S" 2>/dev/null || true' EXIT
until grep -q '^READY' "$OUT" 2>/dev/null; do kill -0 $PID 2>/dev/null || { cat "$OUT"; exit 1; }; sleep 0.5; done
echo "engines loaded; footprint before: $(footprint -p $PID | grep -m1 Footprint:)"
sudo memory_pressure -S -l warn >/dev/null 2>&1 & SP=$!
wait $PID || true
sudo pkill -f "memory_pressure -S" 2>/dev/null || true
cat "$OUT"; grep -q PARLO_SELFTEST_OK "$OUT"
