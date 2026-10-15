#!/usr/bin/env bash
# Offline verification (BRIEF §12.2, §14.1).
#   Linux (cloud):  runs `parlo-cli` flows inside a network-less namespace (unshare -rn); they must succeed.
#   macOS (Mac):    deny-network sandbox for the CLI flows + `lsof` sampling of the app's full self-test; asserts the
#                   frontend CSP only allows self + IPC + asset protocols.
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
  # macOS: (1) CSP has no network origins; (2) the CLI flows (decode, language ID, transcribe, speak, MP3) succeed inside an
  # OS-enforced `deny network*` sandbox profile — any connect() would fail; (3) the real app runs its full self-test
  # (real engines, all command-layer flows) while `lsof -i` samples the app + its WebKit helpers: no non-loopback sockets.
  [[ -n "$APP" ]] || { echo "macOS: pass --app dist/Parlo.app" >&2; exit 2; }
  CSP="$(grep -o 'default-src[^"]*' src-tauri/tauri.conf.json | head -1 || true)"
  echo "CSP: $CSP"
  if grep -Eq 'https?:|ws:|wss:' <<<"$CSP" && ! grep -Eq 'connect-src[^;]*' <<<"$CSP"; then echo "FAIL: CSP allows network origins" >&2; exit 1; fi
  if grep -o 'connect-src[^;]*' <<<"$CSP" | grep -Eq 'https?://[a-z0-9.-]+' && grep -o 'connect-src[^;]*' <<<"$CSP" | grep -Evq 'ipc.localhost'; then echo "FAIL: connect-src allows a remote origin" >&2; exit 1; fi
  echo "OK: CSP allows only self, ipc and asset origins"
  [[ -x "$CLI" ]] || { echo "build first: cargo build --release -p parlo-cli" >&2; exit 2; }
  # sanity: the sandbox profile really blocks the network
  if sandbox-exec -p '(version 1)(allow default)(deny network*)' /usr/bin/curl -s -m 5 -o /dev/null https://example.com 2>/dev/null; then echo "FAIL: sandbox did not block the network" >&2; exit 1; fi
  sandbox-exec -p '(version 1)(allow default)(deny network*)' bash -c "$(declare -f flows); CLI='$CLI'; flows"
  echo "OK: parlo-cli flows succeeded under a deny-network sandbox"
  APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
  BIN="$APP/Contents/MacOS/$(defaults read "$APP/Contents/Info" CFBundleExecutable)"
  BEFORE="$(pgrep -f 'com\.apple\.WebKit\.' | sort -n | tr '\n' ' ')"
  T="$(mktemp -d)"
  PARLO_HOME="$T/home" PARLO_SELFTEST=1 "$BIN" >"$T/selftest.out" 2>"$T/selftest.err" & PID=$!
  trap 'kill $PID 2>/dev/null || true; rm -rf "$T"' EXIT
  bad=0; samples=0
  while kill -0 "$PID" 2>/dev/null; do
    pids="$PID"; for h in $(pgrep -f 'com\.apple\.WebKit\.'); do grep -qw "$h" <<<"$BEFORE" || pids="$pids,$h"; done
    out="$(lsof -nP -i -a -p "$pids" 2>/dev/null | grep -Ev '^COMMAND' || true)"
    if [[ -n "$out" ]] && grep -Ev '127\.0\.0\.1|localhost|\[::1\]|\*:' <<<"$out"; then bad=1; fi
    samples=$((samples+1)); sleep 0.3
  done
  wait "$PID" || true
  grep -q PARLO_SELFTEST_OK "$T/selftest.out" || { echo "FAIL: app self-test did not pass"; cat "$T/selftest.out"; exit 1; }
  [[ $bad -eq 0 ]] && echo "OK: app self-test (real engines) passed; $samples lsof samples, no non-loopback sockets" || { echo "FAIL: sockets observed" >&2; exit 1; }
fi
