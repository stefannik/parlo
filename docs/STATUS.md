# Parlo — STATUS

_Last updated: 2026-10-14 (cloud session 1 — finished; Mac session next)_

## Needs owner
_(nothing blocking so far. All needed domains are reachable: HF (incl. LFS/xethub), download.pytorch.org, GitHub assets, crates/npm/PyPI.)_

## Current phase
**Cloud phases 0–5 complete.** Next: the Mac session (`docs/MAC_FINISH_PROMPT.md` → `docs/MAC_HANDOFF.md`, top to bottom).

---

# Cloud report (session 1, 2026-10-14)

## What was built (all on `main`, all tested on Linux)
- **`parlo-core`** — audio decode (symphonia 0.6 + pure-Rust Opus, own MP4 edit-list reader), streaming resampler, crash-safe WAV, WAV/MP3 export (+ M4A via the platform trait); **recorder** (SPSC ring → writer → supervisor; gaps, stall rebuild, default-device follow, sleep/wake, disk guard, pause/resume, recovery); **STT pipeline** (Silero VAD windows, chunker, language vote, post-filters, checkpoints/resume, whisper-rs engine with our own abort trampoline); **TTS pipeline** (long-text chunking, streaming segments, early first segment, content-addressed cache, crash resume, PocketTTS engine behind `TtsEngine`); **text import** (txt/md/docx/epub/PDF-cleanup, encodings, normalization); model lifecycle (lazy load, single re-armable idle timer, memory-pressure hooks), job manager, settings/workspace/retention.
- **`parlo-cli`** — `decode · info · record · recover · devices · transcribe · lang · speak · bench`.
- **`crates/pocket-tts`** — vendored, patched Candle port (see `PATCHES.md`).
- **`src-tauri`** — Tauri v2 shell: ~33 typed commands + 6 events (`tauri-specta`, drift-tested `ui/src/bindings.ts`), window destroyed on close/app persists, menu + shortcuts, notifications, single instance, level ticker only while recording+visible, quit confirmation.
- **`ui`** — Vue 3 + Pinia: Transcribe (empty/recording/paused/processing/result/error), Speak (compose/imported/generating/result/stale/error), Settings (6 sections), recovery banner, drag-drop, player with playbackRate + preservesPitch, voice picker with previews, toasts, in-app dialogs (no `alert()`), light/dark tokens (contrast-checked), a11y roles/labels; **mock backend** with scripted engines.
- **`crates/parlo-macos`** — PDFKit, AVFoundation fallback decoder, AudioToolbox AAC encoder, power/sleep/memory, mic permission (objc2): **type-checks for `aarch64-apple-darwin`**, never run on a Mac.
- Scripts: fetch-models (pinned + SHA-256), gen-fixtures (21 formats, 8 languages, edge/text/FLEURS), check-offline, smoke-linux, selftest-linux, tts-parity, build-app (drafted), measure-idle-memory (drafted), screenshots.

## Decisions (details in DECISIONS.md)
- **TTS runtime: Candle port of PocketTTS** — parity gate **passed** (C-012): WER +0.39 pp vs reference, 10/10 voices within the similarity margin, warm RTF 0.97 vs 1.26 (−23 %), RSS 641 MB vs 956–1174 MB. Python sidecar not needed. `sherpa-onnx`/PocketTTS.cpp not evaluated (first passing candidate wins).
- **STT model: Whisper large-v3-turbo q8_0 + Silero VAD** (C-013): mean FLEURS WER 6.8 % (small: 19–21 %), 0 hallucinated characters on silence/noise/tone.
- Bench-found bug fixed (C-009, quiet speakers dropped); app-layer bug found by the self-test and fixed (cancelled jobs never sent a terminal event → UI could hang in "Cancelling…").

## Measurements (machine: cloud x86_64 Xeon 2.8 GHz AVX-512, 4 vCPU, 15 GB, no GPU — RELATIVE numbers, NOT Mac numbers)
| What | Result |
|---|---|
| TTS warm RTF, exclusive alternating run (33 jobs each) | Candle **0.972** (σ 0.027) vs Python reference **1.256** (σ 0.041) |
| TTS time to first audio, median / p90 | 307 / 366 ms vs 389 / 439 ms |
| TTS cold load (model + voice state) | 0.54 s vs 1.40 s |
| TTS peak RSS (≈ 1 500-word text) | 641 MB vs 1126–1174 MB |
| TTS WER round trip (Whisper small), n = 183 | 2.50 % vs 2.11 % (gate: ≤ +2 pp) |
| STT turbo q8_0 (105 s file, 4 threads, quiet) | RTF 1.25, RSS 1255 MB, model load 0.9 s (q5_0: 1.43 / 969 MB; f16: 1.26 / 1971 MB; small-q8: 0.30 / 570 MB) |
| 3 h accelerated recorder soak (48 kHz mono) | exact frame count, 0 overruns, +7.1 MB RSS, 16 s wall |
| `kill -9` mid-recording | 1.20 s lost (budget 2 s), header repaired |
| App smoke (Xvfb, software rendering) | UI loads under the CSP, 5 startup IPC commands arrive; also inside `unshare -rn` |
| App self-test (real engines, in the app process) | settings/workspace/import/TTS (cache hit 2 ms, cancel 3 ms)/exports/STT/storage all pass; also with no network |

Test totals: `cargo test --workspace` 146+ tests pass (incl. real-engine `stt_real`, `stt_fleurs`, `tts_pocket`, 3 h soak, 2 h STT run with bounded RSS, 50 k-word TTS); `npm --prefix ui test` 61 pass; clippy `-D warnings`, `cargo fmt --check`, `vue-tsc`, ESLint clean.

## Known issues / limits
- Absolute timings were taken on a shared 4-vCPU VM; only those marked "quiet"/"exclusive" are clean comparisons.
- Whisper turbo is slow on this CPU (RTF 1.25); the ≥ 15× target needs Metal (Mac phase).
- Linux builds have no microphone devices, overlay title bar, M4A export, PDF extraction (PDFKit) or AVFoundation fallback — all in `MAC_HANDOFF.md`.
- `English` WER on FLEURS ≈ 10 % for every model (reference-text style, not a model problem).
- Placeholder app icon (generated); replace before distribution.
- `cargo test --workspace` takes ≈ 25 min here; keep `CARGO_INCREMENTAL=0` (disk allowance is small).

## Needs owner
- Nothing blocking. Optional: listen to `spike/tts-parity/listen.html` (blind A/B) and note the verdict in DECISIONS C-012; create the "Parlo Local Signing" certificate (README) so the mic permission survives rebuilds.
- Cloud setup script: add `npx playwright install --with-deps webkit` (screenshots) if you want it pre-installed.

## Pointer
Everything that still needs macOS (A: gates incl. Metal & real recorder, B: platform implementations, C: window lifecycle/WKWebView, D: packaging/measurements, E: acceptance scenarios) is in **`docs/MAC_HANDOFF.md`** with a verification step per item.
