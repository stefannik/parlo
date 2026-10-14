#!/usr/bin/env bash
# Generate test fixtures (OS-aware). Usage: scripts/gen-fixtures.sh [--only synth,formats,edge,text,fleurs] [--long2h]
#   synth/formats/edge/text: espeak-ng (Linux) or say (macOS) + ffmpeg; output fixtures/generated (gitignored) + fixtures/text (committed)
#   fleurs: pinned FLEURS subset (network, dev time only; ~7 min, extracted via range reads)
set -euo pipefail
exec python3 -I "$(dirname "$0")/gen_fixtures.py" "$@"
