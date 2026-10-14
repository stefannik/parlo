#!/usr/bin/env bash
# Linux smoke run of the Tauri app under Xvfb (BRIEF §14 "Tauri app builds/smoke-runs on Linux").
#   scripts/smoke-linux.sh [path/to/parlo-binary] [screenshot.png]
# Build first:  cargo build -p parlo-app --features tauri/custom-protocol   (embeds ui/dist; run `npm --prefix ui run build` before)
# Passes when the bundled UI loaded under the app's CSP and called the startup commands over IPC (PARLO_SMOKE self-test in
# src-tauri/src/lib.rs). Uses an isolated data dir. Needs: xvfb, dbus-run-session, ImageMagick `import` (screenshot only).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${1:-target/debug/parlo}
SHOT=${2:-}
[[ -x "$BIN" ]] || { echo "binary $BIN not found — build it first (see header)" >&2; exit 2; }
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
export PARLO_HOME="$TMP/home" PARLO_SMOKE=1 PARLO_SMOKE_HOLD_MS=${PARLO_SMOKE_HOLD_MS:-5000}
# software rendering: WebKitGTK has no GPU/DRI3 under Xvfb
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11
export BIN SHOT TMP
xvfb-run -a -s "-screen 0 1280x860x24" dbus-run-session -- bash -c '
  "$BIN" >"$TMP/out.log" 2>"$TMP/err.log" &
  pid=$!
  for _ in $(seq 1 150); do
    grep -q "PARLO_SMOKE_OK" "$TMP/out.log" 2>/dev/null && break
    kill -0 $pid 2>/dev/null || break
    sleep 0.5
  done
  if [[ -n "$SHOT" ]] && grep -q "PARLO_SMOKE_OK" "$TMP/out.log" 2>/dev/null; then
    sleep 1.5; import -window root "$SHOT" || true
  fi
  wait $pid; rc=$?
  cat "$TMP/out.log"; tail -n 20 "$TMP/err.log" >&2
  exit $rc
'
