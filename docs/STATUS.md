# Parlo — STATUS

_Last updated: 2026-10-14 (cloud session 1, in progress)_

## Needs owner
_(nothing blocking so far. All needed domains are reachable: HF (incl. LFS/xethub), download.pytorch.org, GitHub assets, crates/npm/PyPI.)_

### Setup-script additions (installs made mid-session that don't persist)
_(none so far — everything used is in the apt list of docs/CLOUD_SETUP.md; `uv` venvs and Playwright are re-created by scripts)_

## Current phase
**Phase 0 (spikes, cloud) — finishing; Phase 1 (audio core) done in parallel while benchmarks run in the background.**

- [x] a. Skeleton: workspace, docs, `.gitignore`, `CLAUDE.md`
- [x] b. `models/manifest.toml` + `scripts/fetch-models.sh` (pinned, SHA-256 verified; PocketTTS hashes confirmed identical to BRIEF §3.2; Whisper/VAD/eval hashes from HF LFS metadata)
- [x] c. `scripts/gen-fixtures.sh` (synthetic speech ×8 languages, 21 media formats incl. video containers, edge cases, text-import inputs, PDFs) + `scripts/fleurs_subset.py` (pinned FLEURS subset, 10 utt × 8 languages via range reads)
- [ ] d. TTS parity gate — reference (Python 2.1.0) generation running; Candle candidate built & works (see below); metrics script written
- [ ] e. STT selection — `whisper-rs` bench tool built; runs pending
- [x] f. Mac-only spikes drafted: `scripts/measure-idle-memory.sh`, `scripts/check-offline.sh`, cpal backend (type-checks for `aarch64-apple-darwin`)

### Phase 1 — audio core: DONE (portable parts)
`parlo-core::audio` (symphonia 0.6 + pure-Rust Opus, MP4 edit lists, streaming rubato resampler, crash-safe WAV, WAV/MP3 export, >4 GB guard), `parlo-core::recorder` (rtrb ring → writer → supervisor: gaps, stall rebuild, default-device follow, sleep/wake, disk guard, pause/resume, recovery), `parlo-core::platform` traits (§5.7), `parlo-cli decode|info|record|recover|devices`.
Tests (all on Linux): 27 decode tests (every §7.1 format ±50 ms, content-correlation), 16 recorder tests incl. **3 h accelerated soak** and 4 export tests, plus the `kill -9` test in parlo-cli.

## Measurements (machine: cloud x86_64 Xeon 2.8 GHz AVX-512, 4 vCPU, 15 GB — NOT Mac numbers)
| What | Result | Date |
|---|---|---|
| 3 h accelerated recorder soak (48 kHz mono) | exact frame count (518 400 000), 0 overruns, +7.1 MB RSS, 16 s wall | 2026-10-14 |
| `kill -9` mid-recording (real-time counter source) | 1.20 s lost (budget 2 s + slack), header repaired, content ordered | 2026-10-14 |
| TTS reference (pocket-tts 2.1.0, torch CPU) warm RTF | ≈ 1.2 (slower than real time on this VM) | 2026-10-14 |
| Candle port (patched) warm RTF, pipelined decode | ≈ 1.0 | 2026-10-14 |

## Known issues
- Whisper large-v3-turbo on 4 vCPU ≈ 36 s per 30 s window (RTF ≈ 1.2 even on full windows, ≈ 8 on short clips) → bulk TTS WER uses `small` (see DECISIONS).

## Next steps
1. Finish TTS parity run → metrics → decision (DECISIONS C-00x), report + listen.html.
2. STT selection bench (FLEURS + synthetic + edge) → `spike/stt-bench/report.md`.
3. Phase 2 (STT pipeline), then Phases 3–5.
