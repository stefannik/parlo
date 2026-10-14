#!/usr/bin/env bash
# macOS only — DRAFTED IN THE CLOUD, UNCHECKED (see docs/MAC_HANDOFF.md #3).
# Measures idle physical footprint of Parlo.app: the main process + every WebKit helper (WebContent, Networking, GPU)
# attributable to it. BRIEF §12.1 targets: window open <= 150 MB total (main <= 50 MB); window closed <= 60 MB.
#
# Usage: scripts/measure-idle-memory.sh [--app dist/Parlo.app] [--settle 20] [--mode open|closed|after-job]
#                                       [--shell-baseline]   (BRIEF §6.3: bundled *empty* shell, no models)
# Attribution (documented method): WebKit helpers are XPC services whose *responsible PID* is the app. We read
# `launchctl procinfo <pid>` ("responsible pid = N") for each com.apple.WebKit.* process. If that is unavailable
# (needs privileges on some macOS versions) we fall back to process-set diffing: helpers that appeared after launch
# and were absent before it (run on an otherwise WebKit-quiet machine; close Safari/Mail first).
# Output: one line per process + total, and a markdown row for docs/STATUS.md.
set -euo pipefail

APP="dist/Parlo.app"; SETTLE=20; MODE=open
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app) APP="$2"; shift 2;;
    --settle) SETTLE="$2"; shift 2;;
    --mode) MODE="$2"; shift 2;;
    --shell-baseline) MODE=open; shift;;
    -h|--help) sed -n 2,16p "$0"; exit 0;;
    *) echo "unknown arg $1" >&2; exit 2;;
  esac
done
[[ "$(uname)" == "Darwin" ]] || { echo "macOS only" >&2; exit 2; }
command -v footprint >/dev/null || { echo "footprint(1) not found" >&2; exit 2; }
[[ -d "$APP" ]] || { echo "app not found: $APP" >&2; exit 2; }

BIN="$APP/Contents/MacOS/$(defaults read "$PWD/$APP/Contents/Info" CFBundleExecutable)"
webkit_pids() { pgrep -f 'com\.apple\.WebKit\.(WebContent|Networking|GPU)' || true; }

BEFORE="$(webkit_pids | sort -n | tr '\n' ' ')"
"$BIN" >/dev/null 2>&1 &
APP_PID=$!
trap 'kill "$APP_PID" 2>/dev/null || true' EXIT
echo "launched pid $APP_PID, settling ${SETTLE}s (mode=$MODE)…"
sleep "$SETTLE"
if [[ "$MODE" == "closed" ]]; then
  echo "NOTE: close the window now (red button); waiting another ${SETTLE}s for the WebContent process to exit"
  sleep "$SETTLE"
elif [[ "$MODE" == "after-job" ]]; then
  echo "NOTE: run one transcription/TTS job now; waiting for model unload (default 60 s) + 5 s"
  sleep 65
fi

responsible_of() { launchctl procinfo "$1" 2>/dev/null | awk -F'= ' '/responsible pid/ {print $2; exit}'; }
ATTRIBUTED=()
for pid in $(webkit_pids); do
  r="$(responsible_of "$pid" || true)"
  if [[ -n "$r" && "$r" == "$APP_PID" ]]; then ATTRIBUTED+=("$pid"); fi
done
METHOD="responsible-pid"
if [[ ${#ATTRIBUTED[@]} -eq 0 && "$MODE" != "closed" ]]; then
  METHOD="set-diff"
  for pid in $(webkit_pids); do
    if ! grep -qw "$pid" <<<"$BEFORE"; then ATTRIBUTED+=("$pid"); fi
  done
fi

mb() {  # physical footprint in MB for a pid
  footprint -p "$1" 2>/dev/null | awk '/Footprint:/ {v=$2; u=$3; if (u ~ /^KB/) v/=1024; else if (u ~ /^GB/) v*=1024; printf "%.1f", v; exit}'
}
TOTAL=0
MAIN="$(mb "$APP_PID")"; TOTAL=$(awk -v a="$TOTAL" -v b="${MAIN:-0}" 'BEGIN{print a+b}')
printf '%-10s %-28s %8s MB\n' "$APP_PID" "main" "${MAIN:-?}"
for pid in "${ATTRIBUTED[@]:-}"; do
  [[ -z "$pid" ]] && continue
  m="$(mb "$pid")"; name="$(ps -o comm= -p "$pid" | xargs basename)"
  printf '%-10s %-28s %8s MB\n' "$pid" "$name" "${m:-?}"
  TOTAL=$(awk -v a="$TOTAL" -v b="${m:-0}" 'BEGIN{print a+b}')
done
echo "attribution: $METHOD; helpers: ${#ATTRIBUTED[@]}"
echo "TOTAL footprint: ${TOTAL} MB"
printf '| %s | %s | main %s MB | total %s MB | %s |\n' "$(date +%F)" "$MODE" "${MAIN:-?}" "$TOTAL" "$(git rev-parse --short HEAD 2>/dev/null || echo -)"
