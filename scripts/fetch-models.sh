#!/usr/bin/env bash
# Fetch and verify pinned models (build/dev time only). See models/manifest.toml.
#   scripts/fetch-models.sh                       # pockettts + whisper + vad
#   scripts/fetch-models.sh --only pockettts      # just TTS (~290 MB)
#   scripts/fetch-models.sh --only whisper --match q8_0
#   scripts/fetch-models.sh --verify-only
# Fixtures (FLEURS subset, synthetic audio) come from scripts/gen-fixtures.sh.
set -euo pipefail
exec python3 "$(dirname "$0")/fetch_models.py" "$@"
