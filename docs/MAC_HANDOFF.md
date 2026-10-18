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
| A1 | **TTS runtime on Apple Silicon.** Cloud decision: Candle PocketTTS **passed the parity gate** (DECISIONS C-012, `spike/tts-parity/report.md`; cloud CPU numbers: warm RTF 0.97 vs reference 1.26, TTFA 307 ms, cold load 0.54 s, RSS 641 MB). Confirm on the M4 Pro: warm RTF ≤ the Python reference (re-run it there: `spike/tts-parity/run_ref.sh`, `speed_bench.py`), **TTFA ≤ 500 ms** warm, cold load (model + voice) ≤ 3 s, peak RSS ≤ 800 MB on a long text; then try `--features tts-metal` (Candle Metal) and `Accelerate` and record CPU vs Metal. If the engine loses on the M4 Pro, revisit with the Mac numbers (DECISIONS) — it sits behind `TtsEngine`; the Python sidecar protocol in BRIEF §6.1 is the fallback. Also: listen to `spike/tts-parity/listen.html` and record the verdict. | `crates/pocket-tts`, `crates/parlo-core/src/tts/pocket.rs` | verified on Mac (C-015; listening verdict pending) | `scripts/tts-parity.sh` / `parlo-cli bench tts` + `spike/tts-parity/` scripts | 6.1, 9.5 |
| A2 | **STT on Metal.** ≥ 15× real time with large-v3-turbo (1 h audio ≤ 4 min), peak RAM ≤ 1.5 GB for any length, model load ≤ 2 s warm. Build with `--features parlo-core/whisper-metal`; confirm Metal shaders compile at runtime (no Xcode, Command Line Tools only). Re-check the q5_0/q8_0/f16 choice for speed vs accuracy on the Mac. | `crates/parlo-core/src/stt/whisper.rs` | verified on Mac (31× RT, 1.13 GB) | `parlo-cli transcribe <1h.mp3> --json` (prints `rtf`), `/usr/bin/time -l` for RSS | 6.2, 7.4 |
| A3 | **Shell idle baseline** (empty Tauri + Vue shell bundled, no models): measure with the draft script; if the shell alone exceeds §12 targets, re-baseline to *shell + 20 MB* and log it. | `scripts/measure-idle-memory.sh` | verified on Mac (shell 70 MB, UI 78 MB) | `scripts/measure-idle-memory.sh --app dist/Parlo.app --shell-baseline` (the footprint/WebKit-helper attribution via `launchctl procinfo` is unverified — fix the script if needed and document the method) | 6.3, 12 |
| A4 | **Real recorder** inside the *bundled* app: cpal/CoreAudio, TCC permission prompt (`NSMicrophoneUsageDescription`, `com.apple.security.device.audio-input`), AirPods in/out, **cpal's default-device auto-reroute vs our 1 Hz default-device poll** (cpal 0.18 reports `ErrorKind::DeviceChanged` and keeps the stream; make sure the two mechanisms don't fight and the file's sample-rate handling stays right when the new device has another rate), stream errors, sleep/wake, 3 h soak with the real device. | `crates/parlo-core/src/platform/cpal_input.rs` (type-checks for aarch64-apple-darwin), `recorder/mod.rs` | partly: device list ok; real recording pending owner (scripts/mac-mic-test.sh) | QA checklist rows + `parlo-cli record --dir /tmp/r --seconds 20` then `parlo-cli recover` after `kill -9` | 6.4, 8 |

## B. macOS platform implementations (BRIEF §5.7) — traits exist in `parlo-core::platform`, Linux has stubs

| # | Item | Trait / where | Status | How to verify | § |
|---|---|---|---|---|---|
| B1 | AVFoundation fallback decoder (`AVAssetReader` via `objc2-av-foundation`) for files symphonia can't decode (e.g. FairPlay-free AAC oddities, some MOV) | `FallbackDecoder` → `crates/parlo-macos/src/decode.rs`; hook = `audio::prepare::prepare_audio_with` (used by the STT pipeline via `SttJob.fallback_decoder`; tested on Linux with a fake decoder) | verified on Mac (tests/platform.rs) | add fixtures symphonia fails on; the §7.1 format tests must still pass | 7.1 |
| B2 | AudioToolbox **AAC/M4A encoder** (`ExtAudioFile`, ~96 kbps mono) | `AacEncoder` → `crates/parlo-macos/src/aac.rs` (hand-declared AudioToolbox `ExtAudioFile` FFI; verify the converter bit-rate trick); `audio::encode::export_m4a` already concatenates segments and calls it | verified on Mac (+bit-rate clamp, iTunSMPB) | `tests/encode_export.rs` has the Linux "unavailable" case; add a macOS-gated test: M4A decodable, duration ±100 ms | 9.4 |
| B3 | **PDFKit** text extraction (`PDFDocument.string` per page via `objc2-pdf-kit`) → `text::pdf::clean_pdf_pages` (portable, snapshot-tested) | `PdfTextExtractor` → `crates/parlo-macos/src/pdf.rs`; real PDFs `fixtures/text/headers_hyphens.pdf` + `scanned.pdf` | verified on Mac (own PDFKit snapshot) | extraction of `headers_hyphens.pdf` must clean to `fixtures/text/expected/pdf_headers_hyphens.txt`; `scanned.pdf` → "no selectable text" message | 9.2 |
| B4 | `PowerGuard`: `NSProcessInfo beginActivity` (user-initiated + idle-system-sleep-disabled) while recording or any job runs; App Nap off | `PowerGuard` → `crates/parlo-macos/src/power.rs` (used by `recorder::start` and `jobs::JobManager`) | verified on Mac (pmset); in-app check via mic test pending | `pmset -g assertions` shows the assertion during a recording; released after stop | 8.2, 10 |
| B5 | `SleepWakeEvents` (`NSWorkspace` willSleep/didWake) | `SleepWakeEvents` → `crates/parlo-macos/src/sleepwake.rs` (recorder consumes it; `ManualSleepWake` is the test double) | type-checked | lid-close test in QA checklist; `tests/recorder.rs::sleep_and_wake_*` covers the logic | 8.2 |
| B6 | `MemoryPressure` (dispatch source `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`) and `malloc_zone_pressure_relief` (already coded in `MemoryRelief`, `cfg(target_os="macos")`, type-checked) | `MemoryPressure`; `models::ModelHost::watch_memory_pressure` | `MemoryRelief` + `MemoryPressure` → `crates/parlo-macos/src/memory.rs` | type-checked | `memory_pressure -S -l warn` (or `sudo memory_pressure -l critical`) unloads idle models; RSS returns within +30 MB of baseline | 10 |
| B7 | Mic permission API: `AVCaptureDevice authorizationStatus(for: .audio)` / request, and "open System Settings → Privacy → Microphone" | Tauri commands `mic_permission_*`, `open_privacy_settings`; macOS side `crates/parlo-macos/src/mic.rs` | type-checked | deny, then grant, in System Settings with the bundled app | 8.2 |

## C. Tauri shell on macOS (BRIEF §10, §11, §13)

| # | Item | Status | How to verify |
|---|---|---|---|
| C1 | Window lifecycle: closing the window **destroys the webview**, app stays in the Dock, recorder/jobs continue; Dock click recreates the window and rehydrates from the core; recorder reattaches | window destroy/recreate verified (WebContent exits) | record, close the window, wait, reopen — one continuous file; `Activity Monitor` shows no WebContent process while closed |
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
| D1 | `scripts/build-app.sh`: fetch/verify models → build UI → `tauri build` (arm64) → copy models into `Contents/Resources/models/` → sign (`PARLO_SIGN_IDENTITY`, else ad-hoc) → `dist/Parlo.app` (+ optional DMG) | verified on Mac | runs from a clean checkout |
| D2 | Self-signed "Parlo Local Signing" certificate note in the README (ad-hoc signatures change per build → TCC forgets the mic permission) | not started | rebuild twice, permission survives with the certificate |
| D3 | Idle memory: window open ≤ 150 MB total (main ≤ 50 MB), window closed ≤ 60 MB, return to baseline +30 MB within timeout+5 s after a job | window open/closed verified; after-job FAILS (see STATUS) | `scripts/measure-idle-memory.sh --mode open|closed|after-job` |
| D4 | Offline check with Wi-Fi off + `lsof`/Little Snitch: zero sockets in every §14.2 scenario | verified on Mac | `scripts/check-offline.sh --app dist/Parlo.app` |
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

## H. Transcribe from URL (added 2026-10-18 in the cloud — DECISIONS C-021 … C-024)

Everything below the line "type-checked / Linux-tested" was built and tested on Linux with the real pinned yt-dlp (2026.08.19) and
curl against a local HTTP server, plus a fake yt-dlp (`scripts/fake-yt-dlp.py`). YouTube and any real site are **blocked from the
cloud**, so no real-world link has ever been downloaded. The Mac pass checks, in this order:

| # | Item | Status | How to verify |
|---|---|---|---|
| H1 | **A real YouTube video** end to end (paste with ⌘V on the empty screen → Downloading → transcript → player → re-transcribe in another language → Save audio "As downloaded") | not started | `brew install deno` is *not* needed first: try with the bundled yt-dlp alone and note what happens (no deno → possible "Couldn't download this link"); then `brew install yt-dlp deno` and retry — the UI must say "Bundled downloader failed, retried with system yt-dlp" if the bundled one still fails |
| H2 | **A real podcast episode** (RSS enclosure `.mp3`/`.m4a`, ideally one behind a redirect/tracking host such as `traffic.megaphone.fm`/`chrt.fm`), with and without yt-dlp reachable (rename the helper to force the curl path) | not started | the title falls back to `file name (host)`; check the Waiting 5–15 s state shows for the generic extractor |
| H3 | **Bundled yt-dlp signing / hardened runtime / Gatekeeper**: `scripts/build-app.sh` now fetches the pinned macOS build into `tools/yt-dlp`, copies it to `Contents/Helpers/yt-dlp/` and signs every Mach-O inside it with `src-tauri/entitlements-ytdlp.plist` (`disable-library-validation`, `allow-unsigned-executable-memory`), then the app **without `--deep`** (so the helper keeps its entitlements). Drafted blind. Verify: `codesign --verify --deep --strict dist/Parlo.app`, `codesign -d --entitlements - dist/Parlo.app/Contents/Helpers/yt-dlp/yt-dlp`, the helper starts *from inside the bundle* (`build-app.sh` runs `--version`), a **quarantined** copy of the app (`xattr -w com.apple.quarantine …`) still downloads. Then try removing each helper entitlement to find the minimal set (curl_cffi / libffi closures are the suspect for executable memory). | drafted, unchecked | `Console.app` for `Library not loaded` / code-signature kills when a download starts |
| H4 | **Startup time, onefile vs onedir** (decision C-022 is provisional on Linux numbers: 1.09 s vs 0.45 s per launch, ~4 launches per job) | not started | `time dist/Parlo.app/Contents/Helpers/yt-dlp/yt-dlp --version` ×5 for `PARLO_YTDLP_VARIANT=onedir` and `onefile` builds (cold = first launch after a reboot/quarantine is the interesting one — Gatekeeper/XProtect scan the first run); flip the default in `build-app.sh` if onefile wins, and record both numbers in C-022 |
| H5 | **Bundle size added** | not started | build-app.sh prints `bundled yt-dlp size:` (Linux onedir unpacks to 92 MB, macOS zip is 53.9 MB / onefile 37.1 MB) — record the delta of `du -sh dist/Parlo.app` |
| H6 | **deno detection** — GUI app has no shell PATH: `Tools::discover` looks in `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin` (Settings → Downloads shows "deno at …" or "deno not found") | type-checked / Linux-tested with fake dirs | install deno with Homebrew, relaunch from Finder, confirm Settings shows the path and a YouTube download passes `--js-runtimes deno:/opt/homebrew/bin/deno` (the log line / `ps` while downloading) |
| H7 | **Fallback to Homebrew yt-dlp** and the Settings path override | Linux-tested with two fakes | break the bundled copy (`chmod -x`) → system yt-dlp is used silently (no "retried" notice when the bundled one is simply missing); keep it working but make it fail (a link only the newer yt-dlp can read) → the notice appears; type a wrong override path → "Nothing runnable at that path." |
| H8 | **Cookies from Chrome (Keychain) and Safari (Full Disk Access)** — Settings → Use browser cookies lists only installed browsers that have a profile. Expect: Chrome-family browsers pop a **Keychain access prompt** ("Chrome Safe Storage") the first time; Safari fails until Parlo (the *app*, because the child inherits its TCC identity — check whether the helper or Parlo needs the grant) has **Full Disk Access**. Both failure texts must be understandable; log what yt-dlp prints. | not started | private/members-only video, then an age-restricted one; also confirm cookies are NOT used when the toggle is off |
| H9 | **Impersonation availability in both binaries** — the bundled build lists Chrome targets (curl_cffi) on Linux; Homebrew's build may lack curl_cffi → `--impersonate` must be skipped with a log warning, never fail | Linux-tested (fake with "(unavailable)" rows) | `yt-dlp --list-impersonate-targets` for both copies; Settings shows "No Chrome impersonation in this build." for the one without |
| H10 | **Offline check with the real app on macOS** — `scripts/check-offline.sh --app dist/Parlo.app` now also runs the URL self-test against a local server and asserts *Parlo's own PID* (not its children) has no inet sockets (drafted blind, see the macOS branch) | drafted, unchecked | read the script output; the deny-network sandbox part of the macOS flow still covers the non-URL CLI flows |
| H11 | **Drop a link dragged from Safari/Chrome onto the window; ⌘V on the empty screen** — the UI listens for DOM `drop` (`text/uri-list`/`text/plain`) and `paste`; whether wry's native drag-drop handler (needed for file drops) lets non-file drags through to the webview in WKWebView is **unverified**. If it doesn't, ⌘V and the field still work; the fix would be an `onDragDropEvent` text payload or a drop-zone element | not started | drag the URL from the address bar and a link from a page; Edit → Paste with a link on the clipboard |
| H12 | **Quit / crash during a download** — quit kills the child's process group (`RunEvent::Exit` → `kill_all_children`); on Linux `PR_SET_PDEATHSIG` also covers `kill -9`, **macOS has no equivalent** → after `kill -9` of Parlo a download child may keep running until its transfer ends (it writes only into `downloads/<id>/.dl`, swept at the next launch) | not started | start a download, ⌘Q (no confirm dialog is shown for downloads — decide whether it should), relaunch: the link is back in the field, `~/Library/Application Support/Parlo/downloads/` is empty; repeat with `kill -9` |
| H13 | **yt-dlp spawns `ffmpeg` (and the socket sampler saw a `file`) on Linux when they're on `PATH`.** Checked with a logging wrapper: yt-dlp only runs `ffmpeg -bsfs` (feature detection) and does no post-processing/fixups for these audio-only downloads (itag-140 m4a, webm/opus, mp3 all untouched and decodable). A Finder-launched app has no Homebrew in `PATH`, so on the Mac nothing should spawn at all — confirm, and confirm downloads decode with and without a Homebrew ffmpeg visible | Linux-verified | `ps` during a download |
| H14 | **Rate-limit sleep and the real site** — 5–15 s before every download feels long for a 3-second podcast clip; owner may want the default off for direct audio links | owner call | — |

## G. TTS Python sidecar (only if the parity gate had failed)

See DECISIONS (TTS runtime). If the Candle runtime was adopted, this section is **not applicable**.
