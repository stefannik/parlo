#!/usr/bin/env bash
# Idle physical-footprint measurement of the bundled app (BRIEF §12.1/§12.2, MAC_HANDOFF A3/D3).  macOS only.
#   scripts/measure-idle-memory.sh [--app dist/Parlo.app] [--shell-baseline]
# Default (scripted) run: launches the app with PARLO_SELFTEST=lifecycle, which closes its own window, recreates it, runs a
# TTS + STT job and then idles; the script samples `footprint` for the app + its WebKit helpers at each phase:
#   open      window open, no models                   target ≤ 150 MB total, main ≤ 50 MB
#   closed    window destroyed (no WebContent)         target ≤ 60 MB total
#   reopened  window recreated (baseline for the job)
#   job-done  models loaded (informational)
#   after-job 66 s later (idle timeout 60 s + 5 s)     target: back within +30 MB of `reopened`
# --shell-baseline: only the `open` row, for the empty-shell build (BRIEF §6.3).
# Attribution of WebKit helpers: they are XPC services launched by launchd whose responsible PID is the app, but reading
# that (`launchctl procinfo`) needs root on macOS 26, so we diff the WebKit helper PIDs before/after launch (run on a
# machine without other WebKit apps starting up during the run; helpers that already existed are excluded).
set -euo pipefail
cd "$(dirname "$0")/.."
APP="dist/Parlo.app"; SHELL_ONLY=0; SETTLE=15
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app) APP="$2"; shift 2;;
    --shell-baseline) SHELL_ONLY=1; shift;;
    --settle) SETTLE="$2"; shift 2;;
    -h|--help) sed -n 2,16p "$0"; exit 0;;
    *) echo "unknown arg $1" >&2; exit 2;;
  esac
done
[[ "$(uname)" == "Darwin" ]] || { echo "macOS only" >&2; exit 2; }
command -v footprint >/dev/null || { echo "footprint(1) not found" >&2; exit 2; }
APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
[[ -d "$APP" ]] || { echo "app not found: $APP" >&2; exit 2; }
BIN="$APP/Contents/MacOS/$(defaults read "$APP/Contents/Info" CFBundleExecutable)"

helpers() { pgrep -f 'com\.apple\.WebKit\.(WebContent|Networking|GPU)' | sort -n || true; }
BEFORE=" $(helpers | tr '\n' ' ') "
mb() {  # physical footprint in MB for a pid (footprint prints "Footprint: 28 MB" / "512 KB" / "1.2 GB")
  footprint -p "$1" 2>/dev/null | awk '/Footprint: / {for(i=1;i<NF;i++) if($i=="Footprint:"){v=$(i+1); u=$(i+2)}
    if (u ~ /^KB/) v/=1024; else if (u ~ /^GB/) v*=1024; else if (u ~ /^B/) v/=1048576; printf "%.1f", v; exit}'
}
sample() {  # sample LABEL → prints per-process rows, sets MAIN_MB, TOTAL_MB
  local label="$1" total=0 n=0
  MAIN_MB="$(mb "$APP_PID")"; total="${MAIN_MB:-0}"
  printf '[%s]\n  %-8s %-26s %8s MB\n' "$label" "$APP_PID" main "${MAIN_MB:-?}"
  for pid in $(helpers); do
    [[ "$BEFORE" == *" $pid "* ]] && continue
    m="$(mb "$pid")"; [[ -z "$m" ]] && continue
    name="$(ps -o comm= -p "$pid" | xargs basename)"; n=$((n+1))
    printf '  %-8s %-26s %8s MB\n' "$pid" "$name" "$m"
    total="$(awk -v a="$total" -v b="$m" 'BEGIN{print a+b}')"
  done
  TOTAL_MB="$total"
  printf '  TOTAL %s MB (%d WebKit helper processes)\n' "$TOTAL_MB" "$n"
  ROWS+=("| $(date +%F) | $label | $MAIN_MB | $TOTAL_MB | $n | $(git rev-parse --short HEAD 2>/dev/null || echo -) |")
}
ROWS=()
T="$(mktemp -d)"; trap 'kill "$APP_PID" 2>/dev/null || true; rm -rf "$T"' EXIT

if [[ $SHELL_ONLY == 1 ]]; then
  "$BIN" >/dev/null 2>&1 & APP_PID=$!
  sleep "$SETTLE"; sample open
else
  PARLO_HOME="$T/home" PARLO_SELFTEST=lifecycle "$BIN" >"$T/out" 2>"$T/err" & APP_PID=$!
  seen=0
  wait_phase() {  # block until the app prints "PHASE $1"
    until grep -q "^PHASE $1\$" "$T/out"; do
      kill -0 "$APP_PID" 2>/dev/null || { echo "app exited early:"; cat "$T/out" "$T/err"; exit 1; }; sleep 0.5; done
  }
  wait_phase open;      sleep 17; sample open
  wait_phase closed;    sleep 22; sample closed
  wait_phase reopened;  sleep 12; sample reopened; BASE="$TOTAL_MB"
  wait_phase job-done;  sleep 1;  sample job-done
  wait_phase after-job; sleep 66; sample after-job
  echo "after-job − reopened = $(awk -v a="$TOTAL_MB" -v b="$BASE" 'BEGIN{printf "%+.1f", a-b}') MB (target ≤ +30)"
fi
echo; echo "| date | phase | main MB | total MB | helpers | commit |"; echo "|---|---|---|---|---|---|"
printf '%s\n' "${ROWS[@]}"
