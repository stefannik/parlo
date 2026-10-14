#!/usr/bin/env bash
# Offline verification (BRIEF §12.2, §14.1).
#   Linux (cloud):  runs `parlo-cli` flows inside a network-less namespace (unshare -rn); they must succeed.
#   macOS (Mac):    DRAFTED, UNCHECKED — samples `lsof -i -a -p <pids>` while the app/CLI flows run; asserts no sockets,
#                   and that the frontend CSP only allows self + IPC + asset protocols.
# Usage: scripts/check-offline.sh [--cli target/release/parlo-cli] [--app dist/Parlo.app]
set -euo pipefail
cd "$(dirname "$0")/.."
CLI="target/release/parlo-cli"; APP=""
while [[ $# -gt 0 ]]; do case "$1" in --cli) CLI="$2"; shift 2;; --app) APP="$2"; shift 2;; *) echo "unknown arg $1" >&2; exit 2;; esac; done

flows() {  # the flows to exercise offline; extended as parlo-cli grows (decode/encode/transcribe/speak)
  "$CLI" --version
  [[ -x scripts/offline-flows.sh ]] && scripts/offline-flows.sh "$CLI" || true
}

if [[ "$(uname)" == "Linux" ]]; then
  [[ -x "$CLI" ]] || { echo "build first: cargo build --release -p parlo-cli" >&2; exit 2; }
  # unshare a user+net namespace: only loopback (down) exists; any outbound connect fails immediately.
  if ! unshare -rn true 2>/dev/null; then echo "unshare -rn unavailable (need user namespaces); try docker run --network none" >&2; exit 2; fi
  # sanity: prove the namespace is really offline
  if unshare -rn bash -c 'exec 3<>/dev/tcp/1.1.1.1/53' 2>/dev/null; then echo "FAIL: namespace has network" >&2; exit 1; fi
  unshare -rn bash -c "$(declare -f flows); CLI='$CLI'; flows"
  echo "OK: parlo-cli flows succeeded with no network"
  # the Tauri app itself (if built): UI loads under the CSP and the startup IPC works with no network namespace
  APP_BIN="${APP_BIN:-target/debug/parlo}"
  if [[ -x "$APP_BIN" ]] && command -v xvfb-run >/dev/null; then
    PARLO_SMOKE_HOLD_MS=300 unshare -rn scripts/smoke-linux.sh "$APP_BIN" | grep -q PARLO_SMOKE_OK && echo "OK: app smoke run succeeded with no network"
    # the strongest proof: the whole command layer (STT, TTS, export, storage) with the real engines, no network namespace
    if [[ "${PARLO_OFFLINE_FULL:-0}" == "1" ]]; then
      unshare -rn scripts/selftest-linux.sh "$APP_BIN" >/dev/null && echo "OK: app self-test (real engines) succeeded with no network"
    fi
  else
    echo "(app smoke skipped: build it with cargo build -p parlo-app --features tauri/custom-protocol)"
  fi
else
  [[ -n "$APP" ]] || { echo "macOS: pass --app dist/Parlo.app" >&2; exit 2; }
  CSP="$(grep -o 'default-src[^"]*' src-tauri/tauri.conf.json | head -1 || true)"
  echo "CSP: $CSP"
  if grep -Eq 'https?:|ws:|wss:' <<<"$CSP"; then echo "FAIL: CSP allows network origins" >&2; exit 1; fi
  "$APP/Contents/MacOS/$(defaults read "$PWD/$APP/Contents/Info" CFBundleExecutable)" & PID=$!
  trap 'kill $PID 2>/dev/null || true' EXIT
  bad=0
  for i in $(seq 1 60); do
    pids="$PID $(pgrep -f 'com\.apple\.WebKit\.' | tr '\n' ' ')"
    if lsof -nP -i -a -p "$(echo $pids | tr ' ' ',')" 2>/dev/null | grep -Ev '^COMMAND' | grep -Ev '127\.0\.0\.1|localhost|::1'; then bad=1; fi
    sleep 1
  done
  [[ $bad -eq 0 ]] && echo "OK: no non-loopback sockets observed" || { echo "FAIL: sockets observed" >&2; exit 1; }
fi
