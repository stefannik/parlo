#!/usr/bin/env bash
# Fetch and verify pinned build-time tools (currently: yt-dlp for "Transcribe from URL"). See tools/manifest.toml.
#   scripts/fetch-tools.sh                         # yt-dlp for this machine (onedir variant)
#   scripts/fetch-tools.sh --variant onefile       # the single-file build instead
#   scripts/fetch-tools.sh --platform macos        # fetch another platform's build (to inspect; build-app.sh does this itself)
#   scripts/fetch-tools.sh --verify-only
# Build/dev time only: the shipped app never downloads anything by itself.
set -euo pipefail
exec python3 "$(dirname "$0")/fetch_tools.py" "$@"
