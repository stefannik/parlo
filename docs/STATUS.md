# Parlo — STATUS

_Last updated: 2026-10-15 (Mac session 1 — in progress, stopped for the night)_

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

---

# Update 2026-10-15 (evening) — new TTS weights, 21 English voices + Spanish
Shipped in `dist/Parlo.app` / `/Applications`: English `english_2026-09` (the weights the owner chose in the blind test; NOT 09_2, which truncates digits — DECISIONS C-020), 21 English voices + Spanish (Lola), temperature 0.3, quality Standard = 1 sampler step / "Higher" = 4. Parity vs pocket-tts 3.3.0 (same files, Whisper-turbo round trip): English WER 5.08 % (Python) vs 5.53 % (Rust) over 28 clips (24 plain clips 0.06 % vs 0.17 %), digit-heavy text n=21 voices 46.2 % vs 48.5 %, duration ratio 1.02–1.05; Spanish n=3 10.0 % vs 11.7 %. TTS speed/memory unchanged (RTF ≈ 0.1, peak ≈ 660 MB; after-job +25 MB, window open 75 MB / closed 43 MB). Bundle 1.4 GB. Owner fixes shipped earlier the same day (C-019).

# Mac session 1 (2026-10-15, M4 Pro 48 GB / macOS 26.4.1) — nearly done; UI/AirPods/lid/pressure items need the owner

Commit at last update: see `git log` (final docs commit). Hardware: Apple M4 Pro, 48 GB.

## Measured vs target (BRIEF §12 / §6–§9)
| Metric | Target | Measured | Status |
|---|---|---|---|
| STT speed, turbo q8_0 + Metal, 1 h MP3 (4 threads) | ≥ 15× real time (≤ 4 min) | 115 s = **31×** (`parlo-cli transcribe`) | ✅ |
| STT peak RSS, 1 h file | ≤ 1.5 GB | **1.13 GB** | ✅ |
| STT model load, warm | ≤ 2 s | **0.29 s** (first ever 6.6 s: cold disk + runtime shader compile) | ✅ |
| TTS warm RTF vs Python reference (same machine, 4 threads, 22 jobs) | ≤ reference | **0.103 vs 0.105** (Candle+Accelerate); plain Candle CPU 0.153 ✗; Candle Metal broken → C-015 | ✅ |
| TTS time to first audio, warm | ≤ 500 ms | **28 ms** (CLI end-to-end first segment 0.3 s) | ✅ |
| TTS cold load (model + voice) | ≤ 3 s | **58 ms** | ✅ |
| TTS peak RSS (long text) | ≤ 800 MB | **650 MB** (reference 1090 MB) | ✅ |
| Idle, window open, no models | ≤ 150 MB total, main ≤ 50 | **76 MB**, main 26 MB (empty Tauri shell: 70 MB) | ✅ |
| Idle, window closed | ≤ 60 MB, no WebContent | **44 MB**, WebContent exits (GPU + Networking helpers stay) | ✅ |
| Back to baseline after a job (idle timeout + 5 s) | ≤ +30 MB | **+20 MB** (66 s after job; was +593 MB before C-017) | ✅ |
| Models loaded during a job (STT+TTS both) | informational | main 1.4 GB, WebKit GPU helper 133 MB, back to 41 MB / 18 MB | info |
| Network | zero sockets | CLI flows pass under a `deny network*` sandbox; app self-test with `lsof` sampling: none; CSP = self/ipc/asset only | ✅ |
| `kill -9` mid-recording (real mic, bundled app) | ≤ ~2 s lost | **1.0 s** (sync interval 1 s, C-018; was 2.05 s with 2 s) | ✅ |
| Real mic in the bundled app | works, TCC prompt once | recorded, power assertion held/released, permission survived a rebuild | ✅ |
| `cargo test --workspace` (arm64, final tree) | pass | **all pass** (incl. 9 `parlo-macos` platform tests, STT with the shipped turbo model, 2 h file, 3 h soak) | ✅ |
| Cold launch → interactive | ≤ 1 s | not measured yet | ⏳ |
| 3 h soak with the real device | 0 dropped samples | not run yet (`PARLO_SELFTEST_SECONDS=10800 scripts/mac-mic-test.sh 10800`, occupies the mic) | ⏳ |

## Needs owner (all optional-to-batch; I have done everything else)
1. **Accessibility for Claude.app** (System Settings → Privacy & Security → Accessibility, then relaunch Claude): needed for me to click through the UI (window close/reopen via the red button, menus, ⌘Q dialog, each UI state, light/dark). Screen Recording is already working (I can capture Parlo's window: `docs/screenshots-mac/`). Without it, do the UI rows of `docs/QA-CHECKLIST.md` by hand.
2. `scripts/mac-memory-pressure-test.sh` — asks for sudo once (simulated memory warning; no memory used). Expect "memory pressure unloaded both engines".
3. Scenario 3: AirPods in/out during a 10-min recording, lock screen 2 min, one lid close. Tell me when ready, or run: open Parlo, press ⌘R, do it, ⌘R to stop, transcribe.
4. Optional: listen to `spike/tts-parity/listen.html` and add the verdict to DECISIONS C-012.

## Not done / known issues
- UI review in WKWebView (light+dark), keyboard-only and VoiceOver passes, ⌘Q-while-recording dialog, menus, notifications, drag-drop, `<audio>` seeking/speed in WKWebView (C2–C9) — need UI driving (item 1).
- Scenarios 1 (partly: idle numbers), 2 (1 h MP3 transcribed via CLI, not by dropping into the window), 3, 5 (300-page EPUB through the UI), 6, 7 (idle return verified by script), 8 (CLI+self-test verified offline; UI flows pending).
- D5 App Sandbox evaluation (Could): not attempted.
- PDFKit page text has no blank lines, so paragraph breaks inside a page are lost vs the synthetic fixture (own snapshot `pdf_headers_hyphens_pdfkit.txt`).
- `tts-metal` feature is kept but unusable (wrong audio, 8.5 s shader compile); Accelerate is the shipped path.
- Placeholder app icon (generated); replace before distribution. The bundle id is `com.parlo.desktop`.
- Homebrew `ffmpeg` got a broken x265 link after installing `ffmpeg-full`; fixtures use the full build via PATH (cosmetic).

## Helper scripts added on the Mac
`scripts/measure-idle-memory.sh` (scripted window/job phases), `scripts/check-offline.sh` (macOS branch), `scripts/mac-mic-test.sh`, `scripts/mac-kill9-test.sh`, `scripts/mac-memory-pressure-test.sh`; app self-test modes `PARLO_SELFTEST=1|mic|kill9|recover|pressure|lifecycle`; `crates/parlo-core/examples/memtest.rs`.
