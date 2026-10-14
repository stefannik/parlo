#!/usr/bin/env bash
# End-to-end self-test of the Tauri command layer with the REAL engines, inside the real app process under Xvfb
# (src-tauri/src/selftest.rs). Needs models (scripts/fetch-models.sh) and fixtures (scripts/gen-fixtures.sh).
#   scripts/selftest-linux.sh [path/to/parlo-binary]
# Build first: cargo build -p parlo-app --features tauri/custom-protocol   (run `npm --prefix ui run build` before)
# PARLO_WHISPER_MODEL picks the speech model for the run (default here: ggml-small-q8_0.bin, to keep it quick).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${1:-target/debug/parlo}
[[ -x "$BIN" ]] || { echo "binary $BIN not found — build it first (see header)" >&2; exit 2; }
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
export PARLO_HOME="$TMP/home" PARLO_SELFTEST=1 PARLO_WHISPER_MODEL=${PARLO_WHISPER_MODEL:-ggml-small-q8_0.bin}
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11
export BIN
xvfb-run -a -s "-screen 0 1280x860x24" dbus-run-session -- bash -c '"$BIN" 2>/dev/null' | tee "$TMP/out.txt"
grep -q "PARLO_SELFTEST_OK" "$TMP/out.txt"
