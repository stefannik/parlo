#!/usr/bin/env bash
# Real flows exercised by scripts/check-offline.sh inside a network-less namespace: decode, language ID, transcribe,
# speak (PocketTTS), MP3 export. Needs fixtures (scripts/gen-fixtures.sh) and models (scripts/fetch-models.sh).
set -euo pipefail
CLI=${1:-target/release/parlo-cli}
cd "$(dirname "$0")/.."
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
export PARLO_HOME="$T/home"
WHISPER=${PARLO_OFFLINE_MODEL:-ggml-small-q8_0.bin}   # a small model keeps this check quick; the flow is identical

echo "== decode";      "$CLI" decode fixtures/generated/formats/flac.flac "$T/dec.wav" >/dev/null
echo "== lang";        "$CLI" lang fixtures/generated/synth/en_all.wav --model "$WHISPER" --threads 2
echo "== transcribe";  "$CLI" transcribe fixtures/generated/synth/en_all.wav --model "$WHISPER" --language en --threads 2 --job-dir "$T/job" --cache-dir "$T/cache" | head -c 300; echo
echo "== speak";       "$CLI" speak "Parlo works without any network connection." --voice george --out "$T/speak.wav"
echo "== speak mp3";   "$CLI" speak "A second, very short sentence." --voice alba --out "$T/speak.mp3"
for f in "$T/dec.wav" "$T/speak.wav" "$T/speak.mp3"; do [[ -s "$f" ]] || { echo "missing output $f" >&2; exit 1; }; done
echo "offline flows OK"
