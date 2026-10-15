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

# Mac session 1 (2026-10-15, M4 Pro / macOS 26.4.1) — IN PROGRESS, resume here

## Resume checklist (do these first tomorrow)
1. **Idle-after-job miss (open bug):** after a TTS+STT job and the 60 s idle unload, the main process stays at ≈ 580–614 MB footprint (baseline 28 MB; target +30). Engines DO unload (`LOADED end stt=false tts=false`); `vmmap` shows `MALLOC_LARGE (empty)` ≈ 482 MB still dirty/resident — libmalloc keeps freed large blocks cached and `malloc_zone_pressure_relief(NULL,0)` (in `MemoryRelief::relieve`, platform/system.rs) does not return them. Ideas: call relief on every zone from `malloc_get_all_zones`; `MallocLargeCacheSize=0`-style tuning (set env early in `main`, or `malloc_zone_*` API); or a global allocator that `munmap`s large blocks (e.g. tikv-jemallocator/mimalloc with decay 0); also the WebKit GPU helper jumps to 132 MB during a job and returns to 19 MB. Re-measure with `scripts/measure-idle-memory.sh` (≈ 3 min, prints a markdown table).
2. **Owner steps are pending** (see "Needs owner" below). Then run: `scripts/mac-mic-test.sh 12` (real mic in the bundled app).
3. Still to do from MAC_HANDOFF: A4 real recorder (AirPods, sleep/wake, 3 h soak), B6 real memory-pressure unload, C2–C9 + WKWebView UI review (light+dark), D2/D5 (signing note in README, optional sandbox eval), E scenarios 1–8, QA checklist, README (build-from-scratch, install to /Applications, signing note, data locations, tests/benchmarks, architecture), `cargo test --workspace` re-run on final tree, final report.

## Done and verified on this Mac
| What | Result |
|---|---|
| Toolchain | Rust 1.99, cmake, tauri-cli 2.12.3 (`cargo install tauri-cli --locked --version '^2'`), no `metal` compiler (CLT only) — whisper.cpp embeds shaders and compiles at runtime: **works** |
| Fixtures on macOS | needed `brew install espeak-ng` (Bulgarian has no `say` voice), `brew install ffmpeg-full` (std ffmpeg lacks libvorbis/libopus/libvpx; script puts it first in PATH), `.venv` with python-docx+reportlab (script prefers it). FLEURS via `--only fleurs`; 2 h file via `--only edge --long2h` |
| Baseline `cargo test --workspace` (arm64, CPU, default features) | all pass except 4 targets, all explained: `stt_fleurs` (my FLEURS download raced the test), `stt_pipeline::two_hour…` (needed `--long2h`), `stt_real` ×2 (small model vs `say` voices → C-014), `stt_vad` (real bug: span past EOF, fixed). After fixes `stt_real`, `stt_vad`, `stt_pipeline`, `stt_fleurs` pass (release + `whisper-metal`). **Re-run the full workspace once more on the final tree.** |
| **A2 STT Metal** (turbo q8_0, 1 h MP3, 4 threads) | **115 s = 31× real time** (target ≥ 15×), peak RSS **1.13 GB** (≤ 1.5 GB), warm model load **0.29 s** (≤ 2 s); first-ever load 6.6 s (cold disk + shader compile). q5_0/f16 not re-measured. |
| **A1 TTS** (see C-015) | Candle+Accelerate warm RTF **0.103** vs Python reference **0.105**, TTFA **28 ms**, load **58 ms**, peak RSS **650 MB**. Metal rejected. Listening verdict (`spike/tts-parity/listen.html`) still pending (owner). |
| Bundle | `PARLO_SIGN_IDENTITY="Parlo Local Signing" scripts/build-app.sh [--skip-models]` → `dist/Parlo.app` 1.1 GB, arm64, min macOS 14, hardened runtime, audio-input entitlement, designated requirement pinned to the certificate leaf (stable across rebuilds). The cert shows `CSSMERR_TP_NOT_TRUSTED` in `find-identity -v` (normal for self-signed) but codesign works. |
| App self-test in the bundle (`PARLO_SELFTEST=1`, real engines, PDFKit, M4A) | **PASS**, clean exit code 0 |
| `parlo-macos` native tests (`cargo test -p parlo-macos`) | 9 pass: PDFKit snapshot + scanned + damaged, AudioToolbox M4A decodable ±100 ms, AVFoundation fallback (mov, m4a, caf, mp3), fallback error path, PowerGuard (`pmset -g assertions`), mic status, sleep/wake + memory-pressure sources created. (B1,B2,B3,B4 → verified on Mac; B5/B6/B7 partly, see MAC_HANDOFF) |
| Offline (D4) `scripts/check-offline.sh --cli target/release/parlo-cli --app dist/Parlo.app` | **PASS**: CSP has only self/ipc/asset origins; CLI flows succeed inside a `sandbox-exec … (deny network*)` profile; app self-test with 21 `lsof` samples → no non-loopback sockets. NB: build the CLI with `--features parlo-core/whisper-metal,parlo-core/tts-accelerate` first. |
| Idle memory (A3/D3), `scripts/measure-idle-memory.sh` | empty Tauri shell: **70 MB**; real UI window open **78 MB** (main 28 MB) ≤ 150 ✓; window closed **45.8 MB** ≤ 60 ✓ (WebContent exits; GPU+Networking helpers stay); window recreate works. (One first-launch reading of 222 MB had a 137 MB GPU helper; not reproduced in 4 later runs.) After-job: **FAIL, see checklist item 1.** |

## Needs owner (batch — do when convenient, ~10 min)
1. **Microphone (A4/B7):** in a Terminal run `cd ~/Projects/parlo && scripts/mac-mic-test.sh 12` — click **Allow** on the macOS prompt for “Parlo”, and talk for ~12 s. Expect `PARLO_SELFTEST_OK`. Afterwards run it again to confirm the permission survives (same signing identity).
2. **Accessibility + Screen Recording for the app that hosts Claude/Terminal** (System Settings → Privacy & Security): lets me script window clicks and take WKWebView screenshots for the UI review. (Optional but saves a lot of manual checking; otherwise do the QA-CHECKLIST UI pass by hand.)
3. **AirPods test (scenario 3):** tell me when you're ready; I'll start a 10-min recording via the app, you switch AirPods out/in, lock the screen 2 min, then close the lid once (sleep/wake).
4. Optional: open `spike/tts-parity/listen.html` and note which voices/engine you prefer in DECISIONS C-012.

## Uncommitted/local-only notes
- Scratch: `/tmp/target-cpu`, `/tmp/target-acc` (benchmark builds), `/tmp/shell` (empty-shell app) — disposable.
- `spike/tts-parity/.venv` (uv, py3.12, torch CPU) exists locally (gitignored).
