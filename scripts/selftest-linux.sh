#!/usr/bin/env bash
# End-to-end self-test of the Tauri command layer with the REAL engines, inside the real app process under Xvfb
# (src-tauri/src/selftest.rs). Needs models (scripts/fetch-models.sh) and fixtures (scripts/gen-fixtures.sh).
#   scripts/selftest-linux.sh [path/to/parlo-binary]
# Build first: cargo build -p parlo-app --features tauri/custom-protocol   (run `npm --prefix ui run build` before)
# PARLO_WHISPER_MODEL picks the speech model for the run (default here: ggml-small-q8_0.bin, to keep it quick).
# Also runs the "Transcribe from URL" steps (src-tauri/src/selftest.rs url_script) against local HTTP servers
# (scripts/fixture-server.py, serving fixtures/generated/url) with the pinned yt-dlp (scripts/fetch-tools.sh):
# download → pipeline, export, re-transcribe, replace, cancel, errors, off switch, orphan sweep.
#   PARLO_SELFTEST=url scripts/selftest-linux.sh   runs only those URL steps (≈ 30 s, no TTS model needed).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${1:-target/debug/parlo}
[[ -x "$BIN" ]] || { echo "binary $BIN not found — build it first (see header)" >&2; exit 2; }
TMP=$(mktemp -d)
PIDS=()
trap 'kill "${PIDS[@]}" 2>/dev/null || true; rm -rf "$TMP"' EXIT
[[ -x tools/yt-dlp/yt-dlp ]] || { echo "tools/yt-dlp missing — run scripts/fetch-tools.sh" >&2; exit 2; }
[[ -f fixtures/generated/url/podcast_ep.mp3 ]] || { echo "URL fixtures missing — run scripts/gen-fixtures.sh --only synth,url" >&2; exit 2; }
# two local servers: fast, and throttled (so the cancel step has something to interrupt)
start_server() {  # name [extra args…] → port in $TMP/port-<name> (not a command substitution: $! must reach PIDS)
  local name=$1; shift
  python3 -I scripts/fixture-server.py fixtures/generated/url "$@" >"$TMP/srv-$name.txt" &
  PIDS+=($!)
  for _ in $(seq 1 50); do grep -q '^PORT ' "$TMP/srv-$name.txt" 2>/dev/null && break; sleep 0.1; done
  grep '^PORT ' "$TMP/srv-$name.txt" | cut -d' ' -f2 >"$TMP/port-$name"
}
if [[ -z "${PARLO_SELFTEST_NO_URL:-}" ]]; then   # PARLO_SELFTEST_NO_URL=1: the classic run without the URL steps
  start_server fast; start_server slow --bps 15000
  export PARLO_SELFTEST_URL_BASE="http://127.0.0.1:$(cat "$TMP/port-fast")"
  export PARLO_SELFTEST_URL_SLOW="http://127.0.0.1:$(cat "$TMP/port-slow")"
fi
export PARLO_HOME="$TMP/home" PARLO_SELFTEST=${PARLO_SELFTEST:-1} PARLO_WHISPER_MODEL=${PARLO_WHISPER_MODEL:-ggml-small-q8_0.bin}
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11
# PARLO_SAMPLE_SOCKETS=<report.json>: sample the app's sockets while it runs (scripts/sample-sockets.py; see check-offline.sh)
export BIN SAMPLE="${PARLO_SAMPLE_SOCKETS:-}" ROOT="$PWD"
xvfb-run -a -s "-screen 0 1280x860x24" dbus-run-session -- bash -c '
  "$BIN" 2>/dev/null &
  app=$!
  if [[ -n "$SAMPLE" ]]; then python3 -I "$ROOT/scripts/sample-sockets.py" --pid $app --out "$SAMPLE" 2>/dev/null & sampler=$!; fi
  wait $app; rc=$?
  [[ -n "${sampler:-}" ]] && wait $sampler
  exit $rc' | tee "$TMP/out.txt"
grep -q "PARLO_SELFTEST_OK" "$TMP/out.txt"
