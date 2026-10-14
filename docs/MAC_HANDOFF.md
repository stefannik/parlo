# Parlo — Mac handoff checklist

Executed top to bottom by the local Mac session (`docs/MAC_FINISH_PROMPT.md`). Everything below was built and tested on
Linux x86_64 (4 vCPU, no GPU); anything that needs macOS is listed here with a status and a way to verify.

**Status values:** *not started* · *drafted, unchecked* (code exists, never compiled for/run on macOS) · *type-checked*
(`cargo check --target aarch64-apple-darwin` passes) · *verified on Mac* (filled in by the Mac session).

**macOS code status:** `cargo check --target aarch64-apple-darwin -p parlo-macos` passes with no warnings (run from Linux; objc2 0.3 APIs). `parlo-core` with all features cannot be cross-checked from Linux (C dependencies — whisper.cpp, LAME, oniguruma — need the macOS SDK), so a first full `cargo build --release --features whisper-metal,tts-metal` on the Mac may surface platform issues outside `parlo-macos`.

**Quick start on the Mac**
```bash
git pull origin main
brew install cmake                       # whisper.cpp (whisper-rs-sys) and LAME (mp3lame-sys) need it; ffmpeg only for fixtures
scripts/fetch-models.sh                  # pockettts + whisper + vad  (add --only eval for the speaker-embedding model)
scripts/gen-fixtures.sh                  # uses `say` + ffmpeg on macOS; add --only fleurs for the real-speech subset
cargo test --workspace                   # the portable suite must pass on the Mac too (see "Platform-gated tests" below)
```

## A. Gates to confirm first (BRIEF §6)

| # | Item | Where | Status | How to verify | § |
|---|---|---|---|---|---|
| A1 | **TTS runtime on Apple Silicon.** Cloud decision: Candle PocketTTS **passed the parity gate** (DECISIONS C-012, `spike/tts-parity/report.md`; cloud CPU numbers: warm RTF 0.97 vs reference 1.26, TTFA 307 ms, cold load 0.54 s, RSS 641 MB). Confirm on the M4 Pro: warm RTF ≤ the Python reference (re-run it there: `spike/tts-parity/run_ref.sh`, `speed_bench.py`), **TTFA ≤ 500 ms** warm, cold load (model + voice) ≤ 3 s, peak RSS ≤ 800 MB on a long text; then try `--features tts-metal` (Candle Metal) and `Accelerate` and record CPU vs Metal. If the engine loses on the M4 Pro, revisit with the Mac numbers (DECISIONS) — it sits behind `TtsEngine`; the Python sidecar protocol in BRIEF §6.1 is the fallback. Also: listen to `spike/tts-parity/listen.html` and record the verdict. | `crates/pocket-tts`, `crates/parlo-core/src/tts/pocket.rs` | not started | `scripts/tts-parity.sh` / `parlo-cli bench tts` + `spike/tts-parity/` scripts | 6.1, 9.5 |
| A2 | **STT on Metal.** ≥ 15× real time with large-v3-turbo (1 h audio ≤ 4 min), peak RAM ≤ 1.5 GB for any length, model load ≤ 2 s warm. Build with `--features parlo-core/whisper-metal`; confirm Metal shaders compile at runtime (no Xcode, Command Line Tools only). Re-check the q5_0/q8_0/f16 choice for speed vs accuracy on the Mac. | `crates/parlo-core/src/stt/whisper.rs` | not started | `parlo-cli transcribe <1h.mp3> --json` (prints `rtf`), `/usr/bin/time -l` for RSS | 6.2, 7.4 |
| A3 | **Shell idle baseline** (empty Tauri + Vue shell bundled, no models): measure with the draft script; if the shell alone exceeds §12 targets, re-baseline to *shell + 20 MB* and log it. | `scripts/measure-idle-memory.sh` | drafted, unchecked | `scripts/measure-idle-memory.sh --app dist/Parlo.app --shell-baseline` (the footprint/WebKit-helper attribution via `launchctl procinfo` is unverified — fix the script if needed and document the method) | 6.3, 12 |
| A4 | **Real recorder** inside the *bundled* app: cpal/CoreAudio, TCC permission prompt (`NSMicrophoneUsageDescription`, `com.apple.security.device.audio-input`), AirPods in/out, **cpal's default-device auto-reroute vs our 1 Hz default-device poll** (cpal 0.18 reports `ErrorKind::DeviceChanged` and keeps the stream; make sure the two mechanisms don't fight and the file's sample-rate handling stays right when the new device has another rate), stream errors, sleep/wake, 3 h soak with the real device. | `crates/parlo-core/src/platform/cpal_input.rs` (type-checks for aarch64-apple-darwin), `recorder/mod.rs` | type-checked | QA checklist rows + `parlo-cli record --dir /tmp/r --seconds 20` then `parlo-cli recover` after `kill -9` | 6.4, 8 |

## B. macOS platform implementations (BRIEF §5.7) — traits exist in `parlo-core::platform`, Linux has stubs

| # | Item | Trait / where | Status | How to verify | § |
|---|---|---|---|---|---|
| B1 | AVFoundation fallback decoder (`AVAssetReader` via `objc2-av-foundation`) for files symphonia can't decode (e.g. FairPlay-free AAC oddities, some MOV) | `FallbackDecoder` → `crates/parlo-macos/src/decode.rs`; hook = `audio::prepare::prepare_audio_with` (used by the STT pipeline via `SttJob.fallback_decoder`; tested on Linux with a fake decoder) | type-checked | add fixtures symphonia fails on; the §7.1 format tests must still pass | 7.1 |
| B2 | AudioToolbox **AAC/M4A encoder** (`ExtAudioFile`, ~96 kbps mono) | `AacEncoder` → `crates/parlo-macos/src/aac.rs` (hand-declared AudioToolbox `ExtAudioFile` FFI; verify the converter bit-rate trick); `audio::encode::export_m4a` already concatenates segments and calls it | type-checked | `tests/encode_export.rs` has the Linux "unavailable" case; add a macOS-gated test: M4A decodable, duration ±100 ms | 9.4 |
| B3 | **PDFKit** text extraction (`PDFDocument.string` per page via `objc2-pdf-kit`) → `text::pdf::clean_pdf_pages` (portable, snapshot-tested) | `PdfTextExtractor` → `crates/parlo-macos/src/pdf.rs`; real PDFs `fixtures/text/headers_hyphens.pdf` + `scanned.pdf` | type-checked | extraction of `headers_hyphens.pdf` must clean to `fixtures/text/expected/pdf_headers_hyphens.txt`; `scanned.pdf` → "no selectable text" message | 9.2 |
| B4 | `PowerGuard`: `NSProcessInfo beginActivity` (user-initiated + idle-system-sleep-disabled) while recording or any job runs; App Nap off | `PowerGuard` → `crates/parlo-macos/src/power.rs` (used by `recorder::start` and `jobs::JobManager`) | type-checked | `pmset -g assertions` shows the assertion during a recording; released after stop | 8.2, 10 |
| B5 | `SleepWakeEvents` (`NSWorkspace` willSleep/didWake) | `SleepWakeEvents` → `crates/parlo-macos/src/sleepwake.rs` (recorder consumes it; `ManualSleepWake` is the test double) | type-checked | lid-close test in QA checklist; `tests/recorder.rs::sleep_and_wake_*` covers the logic | 8.2 |
| B6 | `MemoryPressure` (dispatch source `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`) and `malloc_zone_pressure_relief` (already coded in `MemoryRelief`, `cfg(target_os="macos")`, type-checked) | `MemoryPressure`; `models::ModelHost::watch_memory_pressure` | `MemoryRelief` + `MemoryPressure` → `crates/parlo-macos/src/memory.rs` | type-checked | `memory_pressure -S -l warn` (or `sudo memory_pressure -l critical`) unloads idle models; RSS returns within +30 MB of baseline | 10 |
| B7 | Mic permission API: `AVCaptureDevice authorizationStatus(for: .audio)` / request, and "open System Settings → Privacy → Microphone" | Tauri commands `mic_permission_*`, `open_privacy_settings`; macOS side `crates/parlo-macos/src/mic.rs` | type-checked | deny, then grant, in System Settings with the bundled app | 8.2 |

## C. Tauri shell on macOS (BRIEF §10, §11, §13)

| # | Item | Status | How to verify |
|---|---|---|---|
| C1 | Window lifecycle: closing the window **destroys the webview**, app stays in the Dock, recorder/jobs continue; Dock click recreates the window and rehydrates from the core; recorder reattaches | not started | record, close the window, wait, reopen — one continuous file; `Activity Monitor` shows no WebContent process while closed |
| C2 | WebContent process exit / webview reload while recording or generating | not started | `kill` the WebContent helper during a recording |
| C3 | Overlay title bar + traffic lights + draggable regions around the segmented control (WKWebView specifics), `tauri-plugin-window-state` | not started | visual |
| C4 | Native macOS menu bar (App, File, Edit, View, Window, Help) with the §11.5 shortcuts; Edit menu works in text fields | not started | keyboard-only pass |
| C5 | Notification when a job finishes while no window is open; single-instance focus | not started | finish a job with the window closed |
| C6 | ⌘Q while recording → "Stop and save / Cancel" dialog | not started | QA checklist |
| C7 | Asset protocol scope (`outputs/`, range requests for seeking) and CSP (`default-src 'self'; connect-src ipc: http://ipc.localhost`; no remote origins) in WKWebView | not started | `scripts/check-offline.sh --app dist/Parlo.app` asserts the CSP and that no non-loopback sockets open |
| C9 | Clipboard (`navigator.clipboard.writeText` with `execCommand` fallback, `ui/src/backend/real.ts`), file dialogs (`tauri-plugin-dialog`), drag-drop paths (`onDragDropEvent`), `<audio>` playback of `asset:` URLs with seeking and `playbackRate` + `preservesPitch` in WKWebView | not started | Copy transcript, drop a file, play + scrub + change speed on generated speech |
| C8 | `Info.plist` (microphone usage string, document types for audio/video/text), entitlements (hardened runtime + `com.apple.security.device.audio-input`), minimum macOS 14 | not started | `codesign -d --entitlements - dist/Parlo.app` |

## D. Packaging, signing, measurements (BRIEF §12, §13)

| # | Item | Status | How to verify |
|---|---|---|---|
| D1 | `scripts/build-app.sh`: fetch/verify models → build UI → `tauri build` (arm64) → copy models into `Contents/Resources/models/` → sign (`PARLO_SIGN_IDENTITY`, else ad-hoc) → `dist/Parlo.app` (+ optional DMG) | not started | runs from a clean checkout |
| D2 | Self-signed "Parlo Local Signing" certificate note in the README (ad-hoc signatures change per build → TCC forgets the mic permission) | not started | rebuild twice, permission survives with the certificate |
| D3 | Idle memory: window open ≤ 150 MB total (main ≤ 50 MB), window closed ≤ 60 MB, return to baseline +30 MB within timeout+5 s after a job | drafted, unchecked | `scripts/measure-idle-memory.sh --mode open|closed|after-job` |
| D4 | Offline check with Wi-Fi off + `lsof`/Little Snitch: zero sockets in every §14.2 scenario | drafted, unchecked | `scripts/check-offline.sh --app dist/Parlo.app` |
| D5 | Sandbox evaluation (time-boxed *Could*): App Sandbox **without** network entitlement as an OS-enforced offline guarantee | not started | log the result in DECISIONS |

## E. Acceptance scenarios (BRIEF §14.2) and QA

Run all eight scenarios in the bundled app and tick `docs/QA-CHECKLIST.md`. Notes: scenario 3 needs real AirPods and a
screen lock; scenario 4 uses `kill -9`; scenario 5 needs a 300-page EPUB (generate one with `scripts/gen-fixtures.sh`
extended, or use any public-domain book). Listen to `spike/tts-parity/listen.html` (blind A/B) and record the owner's
verdict in DECISIONS.

## F. Platform-gated tests

Linux-only stubs exist so everything else runs on both OSes. Tests that are `#[cfg(target_os = "macos")]` /
`#[ignore = "macos"]` (list is kept current; none exist yet beyond the shell items above):

- *(Mac session adds)* M4A export decodability; PDFKit extraction snapshot; AVFoundation fallback decode;
  `PowerGuard` assertion; `MemoryPressure` unload; mic permission state machine.

Portable suites that **must also pass on the Mac** (they use only POSIX APIs and `ffmpeg`/`say` fixtures):
`parlo-core` tests `decode_formats`, `recorder` (incl. the 3 h soak; RSS comes from `tests/common` `peak_rss_mb()` (`getrusage`) and
`current_rss_mb()` (`ps`) — portable, unverified on macOS), `encode_export`, `text_import`, `stt_*`,
`tts_pipeline`, `lifecycle`, and the `parlo-cli` `kill9` test.

## G. TTS Python sidecar (only if the parity gate had failed)

See DECISIONS (TTS runtime). If the Candle runtime was adopted, this section is **not applicable**.
