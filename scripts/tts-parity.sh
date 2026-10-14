#!/usr/bin/env bash
# Re-runnable TTS parity gate (BRIEF §6.1): Python reference (pocket-tts 2.1.0, torch CPU) vs the Rust/Candle engine —
# generation, ASR round trip (WER), speaker similarity, artifacts, RTF, RSS — and the blind A/B page.
#   scripts/tts-parity.sh [--groups main,long] [--skip-ref] [--skip-cand] [--candidate candle]
# Everything is resumable (existing outputs are kept; delete spike/tts-parity/out/<engine>/ to redo) and long: on 4 vCPU
# the full set takes many hours, so run it in the background (nohup … &). Also launched by `parlo-cli bench tts --parity`.
# Needs: uv, models (scripts/fetch-models.sh --only pockettts,whisper,eval), `cargo`, ffmpeg (listen.html clips).
set -euo pipefail
cd "$(dirname "$0")/../spike/tts-parity"
GROUPS_=main; SKIP_REF=0; SKIP_CAND=0; CAND=candle
while [[ $# -gt 0 ]]; do case "$1" in
  --groups) GROUPS_="$2"; shift 2;; --skip-ref) SKIP_REF=1; shift;; --skip-cand) SKIP_CAND=1; shift;;
  --candidate) CAND="$2"; shift 2;; *) echo "unknown arg $1" >&2; exit 2;; esac; done
IFS=, read -ra G <<<"$GROUPS_"

# 1 · Python reference environment (CPU-only torch) — recreated from requirements.lock
if [[ ! -d .venv ]]; then
  command -v uv >/dev/null || { echo "uv is required (https://docs.astral.sh/uv/)" >&2; exit 2; }
  uv venv --python 3.12 .venv
  if [[ "$(uname -s)" == Darwin ]]; then   # PyPI's macOS torch wheel has no +cpu local version
    sed 's/+cpu//' requirements.lock | uv pip install --python .venv/bin/python -r /dev/stdin
  else
    uv pip install --python .venv/bin/python --extra-index-url https://download.pytorch.org/whl/cpu -r requirements.lock
  fi
fi
python3 ../../scripts/make-pockettts-config.py >/dev/null            # models/pockettts/english.local.yaml
[[ -f jobs-ref-main.jsonl ]] || { .venv/bin/python make_jobs.py ref; .venv/bin/python make_jobs.py "$CAND"; }

# 2 · Rust candidate binary
[[ -x ../tts-rust/target/release/tts-candle ]] || (cd ../tts-rust && cargo build --release)
[[ -x ../stt-rust/target/release/stt-spike ]] || (cd ../stt-rust && cargo build --release)

# 3 · generation (reference, then candidate) and ASR round trips with whisper small
for g in "${G[@]}"; do
  [[ $SKIP_REF == 1 ]]  || ./run_ref.sh "$g"
  [[ $SKIP_CAND == 1 ]] || ./run_candle.sh "$g"
done
./run_asr.sh ref
./run_asr.sh "$CAND"

# 4 · metrics + report + listening page
.venv/bin/python metrics.py score ref "$CAND" --asr asr-small | tee out/report-"$CAND".md
.venv/bin/python make_listen.py "$CAND"
echo "see spike/tts-parity/report.md (hand-written summary) and out/report-$CAND.md (generated tables)"
