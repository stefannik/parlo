#!/usr/bin/env bash
# BRIEF §14.2 scenario 4 in the bundled app: record from the real mic, `kill -9` the app, relaunch → the recording is
# recoverable, playable and ≤ 2 s were lost.    scripts/mac-kill9-test.sh [seconds=10] [app=dist/Parlo.app]
set -euo pipefail
cd "$(dirname "$0")/.."
SECS=${1:-10}; APP=${2:-dist/Parlo.app}
D=$(mktemp -d); : > "$D/k.out"; : > "$D/r.out"
open -W -n "$APP" --env PARLO_SELFTEST=kill9 --env PARLO_SELFTEST_SECONDS="$SECS" --env PARLO_HOME="$D/home" --stdout "$D/k.out" --stderr "$D/k.err" || true
cat "$D/k.out"
EXPECT=$(sed -n 's/^KILL elapsed_ms=//p' "$D/k.out" | tail -1); [[ -n "$EXPECT" ]] || { echo "the app never reached the kill step"; cat "$D/k.err"; exit 1; }
open -W -n "$APP" --env PARLO_SELFTEST=recover --env PARLO_EXPECT_MS="$EXPECT" --env PARLO_HOME="$D/home" --stdout "$D/r.out" --stderr "$D/r.err"
cat "$D/r.out"; grep -q PARLO_SELFTEST_OK "$D/r.out"
