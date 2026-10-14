# Parlo — STATUS

_Last updated: 2026-10-14 (cloud session 1, in progress)_

## Needs owner
_(nothing blocking so far. All needed domains are reachable: HF (incl. LFS/xethub), download.pytorch.org, GitHub assets, crates/npm/PyPI.)_

## Current phase
**Phase 5 (UI) in progress; Phases 0–4 portable parts done; TTS parity + STT bench running in the background.**

- [x] **Phase 0** a–c, f (skeleton, manifest + fetch, fixtures, Mac-only spikes drafted)
- [ ] d. TTS parity gate — reference + Candle main set generated; Candle long texts generating; ASR + metrics + report + `listen.html` pending
- [ ] e. STT selection — pipeline fixed (see DECISIONS C-009), bench re-running from scratch (`spike/stt-bench/run_all.sh`)
- [x] **Phase 1** audio core (decode, resample, crash-safe WAV, recorder, export) — tests incl. 3 h soak and `kill -9`
- [x] **Phase 2** STT pipeline (VAD windows, chunker, language vote, post-filters, checkpoints/resume), whisper-rs engine, `parlo-cli transcribe|lang`
- [x] **Phase 3** (code) TTS pipeline + PocketTTS (Candle) engine behind `TtsEngine`, text import (txt/md/docx/epub/pdf-cleanup), `parlo-cli speak|bench` — real-engine tests pass; parity gate decision pending
- [x] **Phase 4** (portable) model lifecycle, job manager, settings/workspace/retention, Tauri shell (`src-tauri`: typed IPC via tauri-specta, window lifecycle, menu, ticker) — compiles; Xvfb smoke run pending
- [ ] **Phase 5** UI: views/stores/mock backend/settings/player/shortcuts/drag-drop done, 60+ vitest tests; screenshots (light+dark) in review; typecheck/lint pending bindings export

### Setup-script additions (installs made mid-session that don't persist)
- `npx playwright install --with-deps webkit` (+ apt: libgtk-4-1, libgraphene, libevent, gstreamer plugins, libavif — pulled in by `--with-deps`). Add to the cloud setup script if screenshots should work without the agent re-installing.

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
