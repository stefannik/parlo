#!/usr/bin/env bash
# Real microphone path inside the BUNDLED, signed app (MAC_HANDOFF A4/B4/B7): TCC prompt → CoreAudio capture → power
# assertion → frame count vs wall clock → not silent. Launched through LaunchServices (`open`) so the app itself — not
# Terminal — is the process macOS asks the user about.   scripts/mac-mic-test.sh [seconds=8] [app=dist/Parlo.app]
set -euo pipefail
cd "$(dirname "$0")/.."
SECS=${1:-8}; APP=${2:-dist/Parlo.app}
OUT=$(mktemp -d)/mic.out; : > "$OUT"
echo "Launching $APP — click Allow if macOS asks for microphone access, then speak for $SECS s…"
open -W -n "$APP" --env PARLO_SELFTEST=mic --env PARLO_SELFTEST_SECONDS="$SECS" --env PARLO_HOME="$(dirname "$OUT")/home" --stdout "$OUT" --stderr "$OUT.err"
cat "$OUT"; grep -q PARLO_SELFTEST_OK "$OUT"
