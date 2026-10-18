#!/usr/bin/env bash
# Pick the curated set from a full `node shoot.mjs <dir>` run and write small PNGs (960×700, palette-quantized) to docs/screenshots/.
#   ./curate.sh <shots-dir> [dest-dir]
set -euo pipefail
SRC=${1:?usage: curate.sh <shots-dir> [dest-dir]}
DEST=${2:-$(cd "$(dirname "$0")/../.." && pwd)/docs/screenshots}
mkdir -p "$DEST"
NAMES=(
  transcribe-empty transcribe-empty-mic-denied transcribe-recording transcribe-processing transcribe-result
  transcribe-error recovery-banner speak-empty speak-imported speak-voice-picker speak-generating speak-result
  speak-result-stale settings-general settings-storage settings-about
  transcribe-url-waiting transcribe-url-downloading transcribe-url-result transcribe-url-error-login settings-downloads
)
for n in "${NAMES[@]}"; do
  for theme in light dark; do
    f="$SRC/$n.$theme.png"
    [[ -f "$f" ]] || { echo "missing $f" >&2; exit 1; }
    convert "$f" -resize 50% -dither None -colors 160 -strip PNG8:"$DEST/$n.$theme.png"
  done
done
du -sh "$DEST"
ls "$DEST" | wc -l
