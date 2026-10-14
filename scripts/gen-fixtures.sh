#!/usr/bin/env bash
# Generate test fixtures (OS-aware). Usage: scripts/gen-fixtures.sh [--only synth,formats,edge,text,fleurs] [--long2h]
#   synth/formats/edge/text: espeak-ng (Linux) or say (macOS) + ffmpeg; output fixtures/generated (gitignored) + fixtures/text (committed)
#   fleurs: pinned FLEURS subset (network, dev time only; ~7 min, extracted via range reads)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# Prefer the repo-local venv (python-docx, reportlab); on macOS also prefer the full ffmpeg (libvorbis/libopus/libvpx).
py=python3; [ -x "$here/../.venv/bin/python" ] && py="$here/../.venv/bin/python"
[ -d /opt/homebrew/opt/ffmpeg-full/bin ] && export PATH="/opt/homebrew/opt/ffmpeg-full/bin:$PATH"
exec "$py" -I "$here/gen_fixtures.py" "$@"
