# Parlo — Project Brief

> **Status:** v1.1 brief, 2026-10-14 (revised for a cloud-first build). Source of truth for the initial build.
> **Owner:** Stefan Nikolaev. **Builder:** an autonomous coding agent working mostly hands-off: first in Claude Code cloud sessions (Linux), then in a local session on the owner's Mac (§5.6).
> **Reference material:** the owner's earlier app *ctric* isn't accessible to the builder. Everything needed from it is in this brief (§3) and in the copied files under `docs/reference/ctric/`.

---

## 0. TL;DR

Parlo is a small, fully local macOS app that does two things well:

1. **Speech → Text.** Drop an audio/video file *or* record from the microphone (minutes to multiple hours), and get an accurate transcript in the spoken language (auto-detected, multilingual).
2. **Text → Speech.** Paste text *or* import a document (.txt/.md/.docx/.pdf/.epub, up to book length), pick a voice, and get natural English speech you can play and export (WAV / M4A / MP3).

Offline; the only network use is a URL download you start; nothing is ever uploaded. No cloud models, no accounts. (Amended 2026-10-18, DECISIONS C-021: the original wording was "no network at runtime".) It must feel like a polished native Mac utility, use almost no RAM when idle, and **never lose a recording**.

**Stack (decided):** Tauri v2 (Rust) + Vue 3 (Composition API, `<script setup>`) + TypeScript. Inference is Rust-native: **whisper.cpp** (via `whisper-rs`, Metal) for STT and **Kyutai PocketTTS** for TTS through a Rust runtime. Python is used only for dev tooling. It ships in the product only if the TTS parity gate in §6 fails, and then only as an on-demand TTS sidecar.

**Build setup (decided):** most of the work happens in **Claude Code cloud sessions** (Ubuntu 24.04 x86_64, no macOS, no GPU), working in a GitHub repo. That covers the portable core, engines, CLI, UI and tests. macOS-only integration, Metal, real-device testing, measurements and packaging happen afterwards in a **local session on the owner's Mac**. The split is in §5.6.

---

## 1. Goals, non-goals, success criteria

### 1.1 Goals
- **G1 — Reliable capture.** Mic recording that survives app UI crashes, webview reloads, window closing, device switches (AirPods in/out), screen lock, idle sleep and app crashes. At most ~2 s of audio may be lost on a hard crash.
- **G2 — Accurate multilingual transcription** of files and recordings of any length. RAM stays bounded no matter how long the input is.
- **G3 — Same TTS experience as ctric.** Same model family, same voices, same parameters, at least the same speed. Long documents are chunked and can be listened to while they generate.
- **G4 — Idle is (nearly) free.** No models in memory, no timers, no polling, no background work when the user isn't doing anything.
- **G5 — Simple, modern, delightful UX.** Two clear modes, obvious primary actions, honest progress, helpful errors.
- **G6 — Offline; the only network use is a URL download you start; nothing is ever uploaded** ("Transcribe from URL", DECISIONS C-021/C-022). No telemetry, no updater, no background requests. The Parlo process itself opens **zero sockets** — a download runs in a child process (yt-dlp, or curl for a plain audio file) — verified by `scripts/check-offline.sh`.

### 1.2 Non-goals (v1)
Live/streaming transcription, history library, menu-bar mode and global hotkey, timestamps/SRT/VTT export, system-audio capture, voice cloning and custom voices, non-English TTS, translation, speaker diarization, OCR for scanned PDFs, Intel Macs, notarized public distribution, Windows/Linux. See §17 "Later".

### 1.3 Success criteria (v1 is done when…)
All acceptance criteria in §14 pass, the performance targets in §12 are met (or re-baselined with measured evidence and logged), and `Parlo.app` built by a single script runs from `/Applications` on the owner's Mac (Apple M4 Pro, 48 GB, macOS 26.4).

---

## 2. Decisions already made (do not re-litigate)

| # | Decision | Notes |
|---|---|---|
| D1 | **Tauri v2 + Rust core**, no Python backend in the product | Overrides the original "Python backend" idea for performance and idle RAM. Python is allowed for dev tooling, and as a fallback TTS sidecar only per §6. |
| D2 | **Vue 3 Composition API (`<script setup>`) + TypeScript + Vite + Pinia** | TS gives typed IPC bindings. No heavy UI kit. |
| D3 | **STT: multilingual with auto-detect.** whisper.cpp via `whisper-rs` with Metal. Default model **Whisper large-v3-turbo** (quantization chosen in §6.2) + **Silero VAD** | A big upgrade over ctric's Whisper *small*. |
| D4 | **TTS: English only, PocketTTS**, same model/voices/params as ctric, Rust runtime subject to a **parity gate** (§6.1) | Fallback: on-demand Python sidecar running the official `pocket-tts==2.1.0`. |
| D5 | **Recording: microphone only, up to multiple hours**, captured natively in Rust (not by the webview) and streamed to disk | |
| D6 | **v1 extra: import TTS text from files** (.txt, .md, .docx, .pdf, .epub) with chunking for book-length input | Other extras are deferred. |
| D7 | **TTS speed = playback speed only** (pitch-preserving, in the in-app player). Exported files are always 1.0× | Same behavior as ctric. |
| D8 | **Distribution: personal use on one Mac.** Apple Silicon only, ad-hoc or self-signed, no notarization | Keep scripts ready for Developer ID later. |
| D9 | **Name: Parlo.** A GitHub repo (cloned into cloud sessions; on the Mac it lives at `~/Desktop/Projects/parlo/`). Bundle id `com.parlo.app` (agent may adjust) | |
| D10 | Work directly on the `main` branch and **push after every checkpoint** (cloud VMs can be reclaimed). **Do not create git worktrees** | Owner preference. |
| D11 | **Cloud-first, Mac-finish** build (§5.6). Portable code is built and tested on Linux; macOS-specific code sits behind platform traits and is verified on the Mac | Uses cloud credits for most of the work. |

---

## 3. Prior implementation (ctric) — reference material

Parlo's features were first built inside **ctric**, the owner's local LLM app (FastAPI + Python backend, Vue 3 + JavaScript frontend). **The builder can't access ctric.** Don't look for it, and don't ask for it. Everything relevant is in this section and in `docs/reference/ctric/` (see its README for what each file teaches). Treat those files as knowledge, not code to port.

### 3.1 What ctric did
| Area | ctric implementation | Reference file |
|---|---|---|
| STT | faster-whisper (CTranslate2), CPU int8, beam 5, **language hard-coded "en"**. The model folder was named "base" but held **Whisper small** (12 encoder layers, 483 MB fp16) | `backend/service_transcribe.py` |
| TTS engine | Kyutai `pocket-tts` 2.1.0 (PyTorch, CPU) with the exact parameters in §3.2 | `backend/pockettts_provider.py`, `backend/pocket_tts_2.1.0_*` |
| TTS service | Content-addressed WAV cache, job + progress snapshots, cache cleanup | `backend/service_tts.py`, `backend/tts_base.py`, `backend/api_services.py` |
| Recording | Webview `MediaRecorder` (WebM) held in memory → decoded to 16 kHz WAV in the browser → uploaded | `frontend/ChatEditor.recording-excerpt.vue.txt`, `frontend/audioUtils.js` |
| Conversation mode | Segmented recording with overlap, low-confidence transcript filtering, playback speed | `frontend/ChatConversationMode.vue` |
| Read-aloud | `<audio>` with `playbackRate` + `preservesPitch`, speed 0.25–2.0, estimated progress | `frontend/ChatFeed.read-aloud-excerpt.vue.txt` |

### 3.2 The exact TTS configuration ctric uses (parity target)
- Package: `pocket-tts` **2.1.0** (Kyutai, PyTorch, ~100M params, CPU). PyPI; supports Python 3.10–3.14 and needs torch ≥ 2.5 (the CPU build is enough).
- Model: Hugging Face repo **`kyutai/pocket-tts-without-voice-cloning`** (ungated, CC-BY-4.0), language config **`english`**. Not `english_2026-01`, `english_2026-04` or `english_2026-09`, which are newer weights for "Later".
- **Pinned files** (download only these, at these revisions, and verify SHA-256):

| File (repo path) | Revision | Size (bytes) | SHA-256 |
|---|---|---|---|
| `languages/english/model.safetensors` | `d29db7978e464fb90cb3359ee0c69a273b9142cc` | 219 029 196 | `be9c6b4876d3f30740a8225dfcaa2e43dc4aeb753c15272735bee16bbb4abb0a` |
| `languages/english/tokenizer.model` | `d29db7978e464fb90cb3359ee0c69a273b9142cc` | 59 339 | `d461765ae179566678c93091c5fa6f2984c31bbe990bf1aa62d92c64d91bc3f6` |
| `languages/english/embeddings/alba.safetensors` | `e041936c75475d350b405bc870bcf7c22da4e9e6` | 6 194 424 | `69c32db63ca56843d994f81f343f62e0bf2d73f7e4c9bc73e44bb1110b1d8845` |
| `languages/english/embeddings/anna.safetensors` | `e041936c…` | 7 816 440 | `5ea82f78db006c9fd34e32ddd5aae82674b5b32646097977436458d00af80dfa` |
| `languages/english/embeddings/charles.safetensors` | `e041936c…` | 6 194 424 | `299edc20182eeccfbf94e308626f259da4fbf339daa8d5905218f2b1774639b8` |
| `languages/english/embeddings/cosette.safetensors` | `e041936c…` | 6 194 424 | `c4fdc15f5a3a20c44dd0064a37e87d15d25562936e8dbad7e07b9832015a545d` |
| `languages/english/embeddings/george.safetensors` | `e041936c…` | 6 243 576 | `0c1c6c57c55a98d81254b33728150c7776f40647fe95258d1a6c1a02780b5d02` |
| `languages/english/embeddings/jane.safetensors` | `e041936c…` | 7 374 072 | `37386227ca8ec5bf1b8e516c13d132ce5ff5437a304fe90129a1c62f41d9a008` |
| `languages/english/embeddings/mary.safetensors` | `e041936c…` | 6 194 424 | `a8f2adf260cab966fe0a113d6b549d6efdeaa79de544ae0ff34b5b6a41445a59` |
| `languages/english/embeddings/michael.safetensors` | `e041936c…` | 7 275 768 | `8937f724ac4719b9aa51ea0ba1f18f9de0af7a663ad6263558266c1a53c9722d` |
| `languages/english/embeddings/paul.safetensors` | `e041936c…` | 6 980 856 | `ed7a019168f94dfe77009f1b0de59387abc6fbb0db954d38ce722ecb77da61aa` |
| `languages/english/embeddings/vera.safetensors` | `e041936c…` | 6 735 096 | `4bf50ddd957b5d218b264fdcf18efbbc7384d12da3eca98ca19b9e8dd6976acc` |

  Use the full embeddings revision `e041936c75475d350b405bc870bcf7c22da4e9e6` (abbreviated above). Download URL pattern: `https://huggingface.co/kyutai/pocket-tts-without-voice-cloning/resolve/<revision>/<path>`. If a pinned revision no longer resolves, find the file whose SHA-256 matches in the repo history, and log it. Never substitute a different file silently.
- **Voices (10):** alba, anna, charles, cosette, george, jane, mary, michael, paul, vera. ctric's UI also listed bill_boerst, peter_yearsley, stuart_bell and caro_davy, which weren't installed locally. **Don't** carry that bug over. Newer repo revisions contain up to 27 English voices; adding them is "Later" (after parity).
- Parameters: `temperature=0.7`, **`lsd_decode_steps=4`** (the library default is 1; ctric deliberately uses 4 for quality), `max_tokens=50` per internal text chunk, `eos_threshold=-4.0` (library default), `quantize=False`. The language is `english`, and voice states load from the embedding files by name.
- Output: mono, **24 000 Hz**, written as PCM16 WAV.
- Voice states are cached per voice after first use. Text is whitespace-normalized before synthesis.
- Offline env for the Python lib: `HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`, `HF_HOME` pointed at a local HF-cache-layout directory holding the pinned files.
- The owner's experience on Apple Silicon: the fastest engine and the best speech quality of those ctric implemented (Kokoro and Chatterbox were also implemented there; PocketTTS became the default).
- Owner's current default voice: **George**, speed **1.0**.

### 3.3 Lessons from ctric (keep the good, fix the bad)
1. **Recording lived entirely in browser memory** (`MediaRecorder` chunks array, no timeslice) until Stop. A crash or webview hiccup lost everything, and errors surfaced via `alert()`. → **Native capture streamed to disk** (§8).
2. **Whole-file decode in the browser** (`decodeAudioData` of the full WebM) uses O(duration) memory. A 3 h recording would need ~700 MB of float samples. → Stream decode/resample in Rust with bounded buffers.
3. **STT language hard-coded to `en`**, and the model folder name lied (base vs small). → Auto-detect plus a **model manifest** with explicit names, revisions and SHA-256.
4. **Voice list didn't match the files on disk.** → Build the voice list from the manifest and verify it at startup.
5. **Models were loaded on first use and never unloaded** (singletons in a resident Python + torch process). → Explicit model lifecycle with idle unload (§10).
6. **Non-blocking TTS lock returned "busy" (HTTP 409).** → A real job queue with visible state.
7. **Good idea to keep:** content-addressed TTS cache key = sha256(version + engine + voice + params + normalized text). Re-generating identical input is instant.
8. **Good idea to keep:** progress reporting from the synthesis loop, and a cache cleanup policy.
9. **Good idea to keep:** low-confidence transcript filtering (RMS + voiced-ratio + known hallucination phrases) from conversation mode. Adapt it as a post-filter for silent or near-silent chunks.
10. **Whole-WAV generation before playback** made long texts slow to start. → Segment-wise progressive output (§9.3).
11. Playback speed via `audio.playbackRate` + `preservesPitch = true` works well. Keep it (D7).

---

## 4. Users and core flows

Single user, power user, on a busy Mac with many apps competing for RAM. The two modes have equal weight.

### 4.1 Flow A — Transcribe a file
Drop a file on the window (or ⌘O / "Choose file…") → Parlo auto-switches to *Transcribe* and starts immediately. The UI shows the stages (Preparing audio → Detecting language → Transcribing x% · ETA). Partial transcript paragraphs appear as chunks finish. Then: an editable transcript with Copy and Export (.txt/.md), a detected-language badge (click → choose another language → re-transcribe), and a mini player for the source.

### 4.2 Flow B — Dictate / record
Click **Record** (⌘R) → the mic permission check runs (with a guided fix if denied) → recording starts: big timer, live level meter, device name, a "Saved continuously" indicator, Pause/Resume, **Stop & Transcribe** (primary), and Discard (with confirm). On Stop, the file is finalized and goes straight into Flow A. Recordings are kept per the retention setting and can be saved via "Save audio…".

### 4.3 Flow C — Speak text
Type or paste text, or drop/open a document (Parlo auto-switches to *Speak*). Pick a voice (with an instant preview sample) and press **Generate** (⌘↩). The progress bar shows ETA. Playback can start as soon as the first segment is ready ("listen while it generates"). Finish with a player (play/pause, scrub, speed 0.5–2.0×) and Export ▾ (WAV / M4A / MP3, always 1.0×).

### 4.4 Flow D — Recover
On launch, if a recording or job was interrupted (crash, power loss), a banner offers **Transcribe / Save audio… / Discard** for the recording, or **Resume / Discard** for an unfinished TTS or STT job.

---

## 5. Architecture

### 5.1 Overview
```
┌───────────────────────────── Parlo.app ─────────────────────────────┐
│  WKWebView (Vue 3 SPA)                                              │
│   views: Transcribe | Speak | Settings     stores: Pinia            │
│        ▲  typed commands (invoke)    ▲  events / ipc::Channel       │
│────────┼─────────────────────────────┼──────────────────────────────│
│  Tauri v2 shell (src-tauri): commands, window lifecycle, menus,     │
│  drag-drop, dialogs, notifications, macOS integration (objc2)       │
│────────────────────────────────────────────────────────────────────│
│  parlo-core (Rust lib)                                              │
│   audio/   capture (cpal) · ring buffer · crash-safe WAV writer     │
│            decode (symphonia + AVFoundation fallback) · resample    │
│            encode (WAV/hound, AAC/AudioToolbox, MP3/LAME)           │
│   stt/     SttEngine trait · whisper-rs impl · VAD · chunker        │
│   tts/     TtsEngine trait · PocketTTS impl (winner of §6.1)        │
│   text/    import (txt/md/docx/pdf/epub) · normalize · segment      │
│   jobs/    queue · cancel · progress · checkpoints · resume         │
│   models/  manifest · lazy load · idle unload · memory pressure     │
│   store/   settings.json · workspace.json · retention/cleanup       │
│────────────────────────────────────────────────────────────────────│
│  parlo-cli (Rust bin): headless access to all of parlo-core,        │
│  used for tests, benchmarks and the agent's own verification        │
└────────────────────────────────────────────────────────────────────┘
       Resources/models/: whisper (ggml) · silero-vad · pockettts · voices
```

### 5.2 Principles (load-bearing; log any departure in `docs/DECISIONS.md`)
1. **Never lose user data.** Audio and text are persisted *before* being shown, and every destructive action is confirmed or undoable.
2. **Idle is free.** Nothing loaded, scheduled, animating or polling when idle. Models are loaded lazily and unloaded on a timer or on memory pressure.
3. **Offline by construction.** Offline; the only network use is a URL download you start; nothing is ever uploaded. No HTTP client in the product's dependency graph (a download is made by a child process: yt-dlp, or `/usr/bin/curl` for a plain audio file), a strict CSP that the webview's network access never needs, no updater, no telemetry. "Allow downloads from URLs" (Settings, default on) switches even that off.
4. **Native where it matters.** Audio capture, decoding, encoding, PDF text and power/sleep handling live in Rust/macOS APIs. The webview is UI only.
5. **Engines behind traits.** STT/TTS backends are swappable without touching UI or job code.
6. **Bounded memory.** Every pipeline streams through disk with fixed-size buffers. RAM must not scale with input length.
7. **Measure, don't guess.** Performance and RAM claims are backed by scripts in `scripts/`, with numbers recorded in `docs/STATUS.md`.
8. **Simple surface, deep internals.** One obvious path per task. Advanced options live in Settings.

### 5.3 Repository layout
```
parlo/
  Cargo.toml                 # workspace
  crates/
    parlo-core/              # all logic, no Tauri dependency
    parlo-cli/               # headless CLI over parlo-core
  src-tauri/                 # Tauri app (thin: commands, events, window, macOS glue)
  ui/                        # Vue 3 + TS + Vite app
  models/                    # gitignored; populated by scripts/fetch-models.sh
    manifest.toml            # committed: names, sources, revisions, sha256, licenses
  scripts/
    fetch-models.sh          # build-time only; verifies sha256
    gen-fixtures.sh          # test fixtures: espeak-ng (Linux) / say+afconvert (macOS), ffmpeg, pinned FLEURS subset (dev only)
    build-app.sh             # one command → dist/Parlo.app
    measure-idle-memory.sh   # §12 measurement
    check-offline.sh         # §12 no-network verification
  spike/                     # Phase 0 artifacts (benchmarks, reports, A/B page); large audio gitignored
  docs/
    BRIEF.md                 # this file
    KICKOFF_PROMPT.md        # cloud build prompt
    CLOUD_SETUP.md           # owner's guide: repo, cloud environment, switching to the Mac
    MAC_FINISH_PROMPT.md     # local Mac phase prompt
    reference/ctric/         # copied reference code from ctric (read-only knowledge)
    DECISIONS.md             # decision log (alternatives, choice, why)
    STATUS.md                # living status, measurements, handoff notes
    MAC_HANDOFF.md           # everything the Mac phase must verify or implement (§5.6)
    QA-CHECKLIST.md          # manual test checklist (§14.3)
  CLAUDE.md                  # short agent orientation: read BRIEF, STATUS; rules from §16
```

### 5.4 IPC contract (sketch — the agent finalizes it, and types must be generated, not hand-duplicated)
Use `tauri-specta` or `ts-rs` (agent's choice, logged) so the TS types for commands and events are generated from Rust.

**Commands**
- `app_info()` → versions, engine ids, model manifest summary, data dir paths
- `settings_get()` / `settings_update(patch)`
- `mic_permission_status()` / `mic_permission_request()` / `open_privacy_settings(kind)`
- `input_devices_list()`
- `recording_start({ device_id? })` → `RecordingInfo`
- `recording_pause()` / `recording_resume()` / `recording_stop()` → `RecordingInfo` / `recording_discard(id)`
- `recoverables_list()` → interrupted recordings and jobs
- `transcribe_start({ source: { path } | { recording_id } | { url }, language: "auto" | iso639 }, on_event: Channel<SttEvent>)` → `JobId` (a `url` source first runs the **Downloading** stage — events `download_info`, `download_waiting`, `download_progress`, `downloader_fallback`, `downloaded` — then the unchanged pipeline; `downloads_info()` and `export_download(...)` support Settings → Downloads and "Save audio")
- `tts_start({ text | import_id, voice, quality }, on_event: Channel<TtsEvent>)` → `JobId`
- `job_cancel(job_id)` / `job_resume(job_id, on_event)`
- `text_import(path)` → `{ import_id, text_preview, stats, warnings }`
- `export_tts({ output_id, format: "wav"|"m4a"|"mp3", dest })` / `export_transcript({ text, format: "txt"|"md", dest })` / `export_recording({ recording_id, format: "wav"|"m4a", dest })`
- `workspace_get()` / `workspace_update(patch)` — last transcript and last TTS output survive restarts
- `voices_list()` → `[{ id, label, preview_src }]`
- `reveal_in_finder(path)`

**Events**
- `recorder:state` — `{ state, elapsed_ms, device, level: { rms, peak } }`, at ≤ 20 Hz **only while the window is visible**
- `recorder:warning` — `device_changed | resumed_after_sleep{gap_ms} | low_disk | silent_input | clipping`
- `models:state` — `{ engine, state: unloaded|loading|ready, unload_in_ms? }`
- `SttEvent` (channel) — `stage`, `progress{done_s,total_s,eta_ms}`, `language{code,prob}`, `paragraph{index,text}`, `done{transcript_id}`, `error{code,message,detail}`
- `TtsEvent` (channel) — `progress{done_chars,total_chars,eta_ms}`, `segment_ready{index,src,duration_ms}`, `done{output_id,duration_ms}`, `error{…}`

### 5.5 Data locations
- `~/Library/Application Support/Parlo/` — `settings.json`, `workspace.json`, `recordings/`, `outputs/tts/`, `jobs/<id>/` (checkpoints, manifests)
- `~/Library/Caches/Parlo/` — temporary decoded PCM, safe to delete
- `~/Library/Logs/Parlo/` — rotating local logs (size-capped, e.g. 5 × 2 MB)
- Model files: `Parlo.app/Contents/Resources/models/` in production; `parlo/models/` in dev (one `ModelPaths` resolver)
- On Linux (cloud dev/test only), use the XDG equivalents via the `directories` crate. The resolver is the only place that knows paths.

### 5.6 Build environments and work split
**Cloud sessions (Claude Code on the web):** Ubuntu 24.04, **x86_64**, 4 vCPU, 16 GB RAM, 30 GB disk, **no GPU, no macOS**.
- Preinstalled: Rust (rustc/cargo), Python 3 + uv, Node 22 (npm/pnpm; Bun exists but has proxy issues, so prefer npm/pnpm in the cloud), GCC/Clang/cmake/ninja, Docker, git/gh.
- The setup script runs as root (`apt install` works).
- Foreground commands time out after 2–10 min. Longer work must run in the background (up to ~30 more minutes per command) or be split.
- The VM is reclaimed after inactivity. Only **pushed** commits survive.
- Network goes through an allowlist: Hugging Face and PyTorch's CPU wheel index must be added by the owner (see `docs/CLOUD_SETUP.md`).

**Owner's Mac (local Claude Code session, later):** Apple M4 Pro, 48 GB, macOS 26.4, Rust 1.99, Node 23, Bun, uv, Python 3.13, Homebrew, ffmpeg, Command Line Tools only (no full Xcode, no cmake until installed).

| Work | Cloud (Linux) | Mac |
|---|---|---|
| Workspace, manifests, fetch scripts, CI-style checks | ✅ build + test | re-run |
| TTS parity gate (§6.1): reference vs Rust candidates, **relative** metrics on CPU | ✅ decide | confirm with Metal + absolute speed |
| STT model selection (§6.2): accuracy, language ID, hallucinations (CPU) | ✅ decide accuracy | confirm Metal speed and RAM |
| Recorder pipeline: ring buffer, crash-safe WAV writer, recovery, gap markers, device-swap logic, driven by `FileSource` and simulated errors | ✅ build + test | real CoreAudio via cpal, TCC permission, real devices, sleep/wake |
| Decoding (symphonia + Opus), resampling, WAV/MP3 encoding | ✅ | AVFoundation fallback decoder, AAC (AudioToolbox) encoder |
| STT pipeline (VAD, chunking, language, post-filters, checkpoints) | ✅ (CPU) | Metal build + targets |
| TTS pipeline (chunking, segments, cache, resume) | ✅ (CPU) | Metal or Accelerate if beneficial + targets |
| Text import: txt / md / docx / epub, plus PDF **page-text cleanup** (de-hyphenation, headers/footers) on page-text fixtures | ✅ | PDF text extraction via PDFKit |
| Tauri shell: commands, events, job manager, model lifecycle logic | ✅ compile + run on Linux (WebKitGTK, headless via Xvfb) | macOS behavior: window destroy/recreate, menus, power assertions, memory pressure, `malloc_zone_pressure_relief`, notifications |
| UI (all states), mock backend, component tests, screenshots | ✅ (Playwright **WebKit** for screenshots to approximate WKWebView) | real-app review in WKWebView |
| Idle memory, offline check, bundle, signing, entitlements, `/Applications` install | scripts written | ✅ run, measure, ship |

### 5.7 Platform abstraction
Keep everything that differs per OS behind small traits or modules in `parlo-core::platform`, with a `macos` implementation (the product) and a `linux` implementation that exists **only** so the core can be built and tested in the cloud. These include:
- `AudioInput` (cpal on both; FileSource for tests)
- `FallbackDecoder` (AVFoundation on macOS; none on Linux)
- `AacEncoder` (AudioToolbox; unavailable on Linux, so tests are cfg-gated)
- `PdfTextExtractor` (PDFKit; Linux tests use page-text fixtures)
- `PowerGuard` (NSProcessInfo activity; no-op on Linux)
- `SleepWakeEvents`, `MemoryPressure` (dispatch source; a Linux stub that can be triggered in tests)
- `MemoryRelief` (`malloc_zone_pressure_relief`; `malloc_trim` on Linux)

macOS implementations may be **drafted in the cloud** if they can at least be type-checked (e.g. `cargo check --target aarch64-apple-darwin` on the relevant crate or feature, after `rustup target add`). Anything not compiled or not run on macOS must be listed in `docs/MAC_HANDOFF.md` with its status: *not started* / *drafted, unchecked* / *type-checked* / *verified on Mac*.

---

## 6. Phase 0 — spikes and gates (do these first, record results in `spike/` + `DECISIONS.md`)

Phase 0 is split. **6.1 and 6.2 run in the cloud** and make the decisions using *relative* measurements: every candidate is compared on the same machine with the same thread count. **6.3 and 6.4, and the Metal/absolute confirmations of 6.1 and 6.2, run in the Mac phase.** Their pending status is tracked in `docs/MAC_HANDOFF.md`.

### 6.1 TTS runtime parity gate (cloud: decide · Mac: confirm)
**Goal:** pick the Rust runtime for PocketTTS that matches ctric's voices and quality at ≥ ctric's speed. If none passes, use the Python sidecar fallback.

**Reference generator (dev only).** Create `spike/tts-parity/.venv` with `uv` (Python 3.12 or 3.13) holding `pocket-tts==2.1.0` + `soundfile` + **CPU-only torch** (install torch from `https://download.pytorch.org/whl/cpu`, so pip doesn't pull the multi-GB CUDA build). Download the pinned files from §3.2 with `scripts/fetch-models.sh` into `parlo/models/pockettts/`, laid out as an HF cache (`hub/models--kyutai--pocket-tts-without-voice-cloning/snapshots/<rev>/…`) so the library resolves them offline. Point `HF_HOME` at it with `HF_HUB_OFFLINE=1`. Generate reference audio with the exact parameters in §3.2 (see `docs/reference/ctric/backend/pockettts_provider.py`).

**Candidates, in recommended evaluation order** (verify each one's current state; versions below are what was found on 2026-10-14):
1. **`pocket-tts` crate (Candle port, babybirdprd/pocket-tts, v0.6.x, `metal` feature).** It accepts `.safetensors` voice files and `.wav`, and has a streaming API (`generate_stream`). By default it loads models by variant id through `hf-hub`. Expect to **vendor and patch** it to load local files (`without-voice-cloning` weights + the 10 pinned voice embeddings) fully offline. Check the license before vendoring. Its CPU path is evaluated in the cloud; `metal` on the Mac.
2. **`sherpa-onnx` crate (official k2-fsa Rust bindings, v1.13.x; downloads a prebuilt static lib from GitHub releases at build time).** Its PocketTTS path is *voice cloning from reference audio* with int8/fp32 ONNX graphs (`lm_flow`, `lm_main`, `encoder`, `decoder`, `text_conditioner`), so it can't load the precomputed voice states directly. Voices would come from short reference WAVs; one option is to bootstrap them from the reference generator, which brings a risk of voice drift. Strong on robustness and maintenance.
3. **PocketTTS.cpp** (C++/ONNX Runtime, C FFI with streaming `ptt_stream_*`). Also reference-audio based.
4. Any newer port you find (e.g. from Kyutai or Laurent Mazare; an `lmz/pocket-tts-without-voice-cloning-q8` HF repo exists) — evaluate if it's credible.

**Test set:** 12 English texts. Include short (1 sentence), medium (1 paragraph), long (~1500 words), numbers/dates/abbreviations, quotes/dialogue, and a list-heavy markdown-derived text. Use all 10 voices on the short and medium texts, plus george/alba/michael on the long text. Generate **3 samples per (text, voice)**, because sampling is stochastic at temperature 0.7. On 4 cloud vCPUs, budget the run time and run it in the background in batches, so no single command exceeds the time limits.

**Metrics (scripted; dev tooling may use Python):**
- **Intelligibility:** transcribe every output with the STT model from §6.2 and compute WER against the input text. Pass: candidate mean WER ≤ reference mean WER + 2 pp.
- **Voice identity:** speaker-embedding cosine similarity (e.g., a sherpa-onnx speaker-embedding model from its GitHub releases, or an ECAPA model in dev tooling). Pass: for each voice, the mean similarity between candidate and reference ≥ (mean similarity between reference samples) − 0.05.
- **Speed (warm, relative, same machine and thread count):** Pass in the cloud: candidate RTF ≤ reference RTF. On the Mac: confirm RTF ≤ reference and **TTFA ≤ 500 ms** (absolute), plus Metal vs CPU.
- **Cold load:** indicative in the cloud. Pass on the Mac: model + voice-state load ≤ 3 s.
- **Memory:** peak RSS during the long text ≤ reference peak (same machine).
- **Artifacts:** no NaNs, clipping < 0.1 % of samples, no internal silence > 2 s, and duration per word within ±20 % of reference.

**Human check (non-blocking):** write `spike/tts-parity/listen.html`, a blind A/B page with randomized pairs per voice and text, so the owner can listen later. Commit the page plus a **small** curated set of compressed clips (e.g. 1 short text × 10 voices × reference/candidate as ~32 kbps Opus/M4A, a few MB total); everything else stays gitignored. Proceed on the metrics. If the owner's later listening disagrees, switching runtimes must be a one-line change, because the engine sits behind a trait.

**Fallback (only if no candidate passes):** a **Python TTS sidecar**. Use python-build-standalone 3.12 + a venv with `pocket-tts==2.1.0`, CPU-only torch and soundfile, bundled under `Contents/Resources/tts-sidecar/`. It is spawned **on demand** and talks JSON-lines over stdin/stdout:
- `{"op":"synthesize","id","text","voice","params","out_path"}` → streamed `{"id","progress"}` / `{"id","segment","path"}` / `{"id","done"}` / `{"id","error"}`
- It exits after the same idle timeout as models, and immediately when stdin closes (parent died). It runs with `HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`, `HF_HOME=<bundled models>`.
- It implements the same `TtsEngine` trait from the Rust side.
- The protocol, Rust client and sidecar script are built and tested in the cloud. The arm64 macOS bundle (python-build-standalone + wheels) is built in the Mac phase.

Log the outcome, with all numbers and the machine they came from, in `DECISIONS.md`.

### 6.2 STT model selection (cloud: decide accuracy · Mac: confirm speed and RAM)
Benchmark with `whisper-rs` (v0.16.x, now maintained on Codeberg; crates.io has releases; `metal` feature; exposes `WhisperVadContext`/`WhisperVadParams` and abort/progress callback types):
- Whisper **large-v3-turbo** in `q5_0`, `q8_0` and `f16` GGML from HF `ggerganov/whisper.cpp` (`ggml-large-v3-turbo-q5_0.bin`, `ggml-large-v3-turbo-q8_0.bin`, `ggml-large-v3-turbo.bin`); plus **small** (`ggml-small.bin` or `ggml-small-q8_0.bin`, ctric baseline). VAD: `ggml-org/whisper-vad` (`ggml-silero-v5.1.2.bin` or `ggml-silero-v6.2.0.bin`, whichever your whisper-rs/whisper.cpp version supports; log the choice). Pin revisions and SHA-256 in `models/manifest.toml`.
- **Fixtures:**
  - **Real multilingual speech:** a small pinned subset of Google **FLEURS** (CC-BY-4.0, on Hugging Face; e.g. 10 utterances each for English, German, French, Spanish, Italian, Bulgarian, Russian, Portuguese) with reference transcripts.
  - **Synthetic speech:** `espeak-ng` (apt) in the cloud; macOS `say` on the Mac. The fixtures script detects the OS.
  - **Edge files:** a ≥10-minute English file built by concatenating fixtures, a mixed-language file, and a file with long silences and background noise (ffmpeg).
  - Commit only small fixtures (< ~5 MB total). Large ones are regenerated by `scripts/gen-fixtures.sh`.
- Metrics: WER/CER per language, language-detection accuracy, hallucinations on silence/noise, and — relative only in the cloud — RTF, peak RSS and load time.
- **Default expectation:** large-v3-turbo `q8_0` + Silero VAD. Pick by measured accuracy and log it. The Mac phase confirms the §7.4 speed and RAM targets with Metal.
- Optional: compare NVIDIA Parakeet-TDT-0.6B-v3 via sherpa-onnx for European languages. Adopt it only if it's clearly better *and* covers the owner's languages; otherwise note it under "Later".

### 6.3 Shell idle baseline (Mac phase)
Build an empty Tauri v2 + Vue window, bundle it, and measure idle memory with `scripts/measure-idle-memory.sh` (§12.2). If the empty shell alone exceeds the §12 targets, re-baseline the targets to *shell + 20 MB*, and log it. The cloud writes the script; the Mac runs it.

### 6.4 Recorder spike (Mac phase)
Get `cpal` input working with mic permission inside the **bundled** app (permission prompt, `NSMicrophoneUsageDescription`, entitlement). Verify default-device switching and the stream-error callbacks with AirPods/built-in mic, and stream to a WAV on disk. In the cloud, build and test the full recorder pipeline against `FileSource` and simulated stream errors, so the Mac phase only has to plug in and verify the real device path.

**Cloud gate to proceed past Phase 0:** the TTS runtime is chosen (§6.1 relative metrics), the STT model is chosen (§6.2 accuracy), the skeleton and `fetch-models.sh` work, and `MAC_HANDOFF.md` lists the pending Mac confirmations.
**Mac gate (start of the Mac phase):** §6.1 and §6.2 confirmations, the §6.3 baseline measured, and the §6.4 bundled app records audio. If a Mac confirmation fails (e.g. the chosen TTS runtime is slower than the reference on Apple Silicon), revisit the decision with the Mac numbers and log it.

---

## 7. Speech-to-text — detailed requirements

### 7.1 Inputs
- **Audio:** WAV, MP3, M4A/AAC, ALAC, FLAC, OGG/Vorbis, Opus (OGG and WebM), AIFF, CAF.
- **Video (audio track):** MP4, MOV, M4V, MKV, WebM.
- **Links ("Transcribe from URL", DECISIONS C-021/C-022):** an `http(s)` link (YouTube, a podcast enclosure `.mp3`/`.m4a`, anything yt-dlp supports) → the *audio only* is downloaded by a child process into a per-job directory (bundled yt-dlp → system yt-dlp → curl for direct audio files), then handled exactly like a file. What arrives is whatever the site serves: AAC in (fragmented) MP4, WebM/Opus, MP3 — all decode with symphonia, no ffmpeg.
- Decoding: `symphonia` (all relevant features) first; fall back to **AVFoundation** (`AVAssetReader` via `objc2-av-foundation`) for anything symphonia can't handle. Add Opus via a libopus binding if neither covers it. **No bundled ffmpeg.** Every listed format must have a fixture and a passing test.
- Multi-channel → downmix to mono. Any sample rate → 16 kHz using a high-quality resampler (`rubato`). Stream the work: write 16 kHz mono PCM16 to a temp file in Caches. Never hold the whole file in RAM.
- Reject with a clear message: no audio track, zero duration, DRM-protected, unreadable.

### 7.2 Pipeline (bounded RAM, any length)
1. **Prepare:** decode → resample → temp PCM file. Progress counts toward the total.
2. **VAD:** run Silero VAD over the temp file in windows (e.g. 5 min, with overlap) → speech segments. Merge them into **chunks of ≤ ~30 s**, cutting at silences ≥ 300 ms. Hard-split overlong speech at the lowest-energy point.
3. **Language:** if set to Auto, detect on up to the first 3 speech chunks, take a majority vote, then lock the language for the file. Report `{code, probability}`. If the user picked a language, use it.
4. **Transcribe** chunk by chunk with whisper.cpp (Metal). Use temperature fallback, `suppress_blank`, `suppress_nst`, no-speech / log-prob / entropy thresholds, and `no_context` between chunks. The agent may tune these, including carrying the previous chunk's tail over as an initial prompt, based on the §6.2 benchmark. Log the chosen values.
5. **Post-filter:** drop chunks flagged as no-speech with low confidence, and drop known silence hallucinations ("Thank you for watching", "Subtitles by…", etc.) when the chunk's RMS and voiced-ratio are low (adapt ctric's conversation-mode heuristics in `docs/reference/ctric/frontend/ChatConversationMode.vue`). Collapse ≥ 3× consecutive repeated n-gram loops.
6. **Assemble:** join segments. Start a **new paragraph** when the gap between speech segments is ≥ 1.5 s (tunable). Keep segment timestamps internally for future SRT/VTT, even though v1 doesn't show them.
7. **Checkpoint:** append each chunk's result to `jobs/<id>/segments.jsonl`. A crash or quit can resume from the last completed chunk.
8. **Stream to UI:** emit finished paragraphs as they're ready.

### 7.3 Output and UX
- An editable transcript, auto-saved to the workspace.
- Copy (button + ⇧⌘C) and Export `.txt` / `.md` (the Markdown has a title, source, date, duration and language header).
- Detected-language badge → menu → "Re-transcribe as …".
- Metadata: source name, duration, word count, processing time.

### 7.4 Performance (targets on M4 Pro; calibrate in §6.2)
- Throughput ≥ 15× real-time (1 h audio in ≤ 4 min) with large-v3-turbo on Metal.
- Peak RAM ≤ 1.5 GB regardless of input length. Model load ≤ 2 s warm-disk.
- Cancel takes effect in ≤ 1 s (abort callback plus between-chunk checks).

---

## 8. Recording — "rock-solid" requirements

### 8.1 Capture design
- Capture natively with **`cpal`** (CoreAudio) on a dedicated thread. The audio callback **only** copies samples into a lock-free SPSC ring buffer (e.g. `rtrb`): no allocation, no locks, no I/O. A writer thread drains it to disk.
- **File format:** WAV, PCM 16-bit, mono (downmix by averaging), at the device's native sample rate. The file keeps one sample rate: if a device change brings a different rate, resample the new stream to the file's rate.
- **Crash safety:** patch the RIFF/data sizes in the header and `fsync` about every 2 s. On launch, scan `recordings/` for files whose sidecar JSON says `state: "recording"`, then repair the header from the file length and offer recovery (§4.4). Worst-case loss is ~2 s.
- **Sidecar** `rec-YYYYMMDD-HHMMSS.json`: state, device(s), sample rate, start time, pause intervals, gaps (sleep, device switch), and the final duration.
- Don't use VoiceProcessingIO (it ducks other audio and alters the signal). No AGC by default.

### 8.2 Interruption handling (each needs a test or QA item)
| Event | Required behavior |
|---|---|
| Window closed or webview reloads/crashes | Recording continues in the Rust core. Reopening the window reattaches the UI. |
| App quit (⌘Q) while recording | Confirm dialog: "Stop and save" / "Cancel". Never discard silently. |
| App crash / `kill -9` / power loss | ≤ 2 s lost. Recovery offered on next launch. |
| Default input device changes (AirPods connect/disconnect) | If following "System default": switch to the new default within ~1 s, keep the same file, insert a gap marker, show a non-blocking notice. If the user pinned a device that vanished: fall back to the system default and notify. |
| cpal stream error / stall (no callbacks for > 1 s while recording) | Rebuild the stream automatically with backoff. Log it, mark a gap, notify. |
| Idle system sleep | Prevented while recording (`NSProcessInfo beginActivity` with user-initiated + idle-system-sleep-disabled). Display sleep is allowed. |
| Screen lock / screensaver / fast user switch | Recording continues. |
| Forced sleep (lid closed without external display, low battery) | Flush and `fsync` on `willSleep`. On `didWake`, restart capture automatically, mark a "resumed after sleep" gap, notify. |
| App Nap | Disabled while recording or while any job runs (same activity token). |
| Disk nearly full | Pre-flight check before starting. Warn at < 2 GB free. At < 300 MB, stop gracefully and keep the file. |
| Mic permission denied or revoked | Clear guidance + button to open System Settings → Privacy & Security → Microphone. Keep any partial file. |
| Silent input (muted mic or wrong device) for > 10 s | Inline warning; recording continues. |
| Clipping | Subtle indicator on the level meter. |

### 8.3 Controls and UX
Start/Stop (⌘R), Pause/Resume (⌘P while recording), Discard (confirm), device picker (in Settings plus a compact switcher on the record card), timer HH:MM:SS, live level meter (animates only while recording and only while visible), and a "Saved continuously to disk ✓" indicator. Stop → finalize → transcription starts automatically. Bluetooth note in the device picker: "AirPods mics switch to call quality (16 kHz)".

### 8.4 Targets
- **Zero dropped samples** in a 3-hour soak. Verify by comparing the written frame count with the wall clock, and by counting overruns.
- Recording CPU ≤ 3 % of one core; RAM ≤ idle + 20 MB.
- Disk: ~345 MB/h at 48 kHz mono 16-bit (show estimated remaining capacity).

### 8.5 Retention
Recordings are kept after successful transcription for **7 days** by default (Settings: 1 day / 7 days / 30 days / Forever). **Never auto-delete** a recording that hasn't been successfully transcribed or explicitly discarded. "Save audio…" exports WAV or M4A.

---

## 9. Text-to-speech — detailed requirements

### 9.1 Engine
- PocketTTS, English, the 10 voices from §3.2, parameters from §3.2. Runtime per §6.1.
- **Quality setting** (in Settings → Speech): *Standard* = `lsd_decode_steps 4` (default, ctric parity) or *Fast* = `1`.
- Voices come from `models/manifest.toml` and are verified at startup. Missing voices are hidden and logged.
- **Voice previews:** pre-generate a ~3 s sample per voice (fixed sentence, Standard quality, chosen engine) as small MP3 files (~64 kbps, committed to the repo and bundled in Resources). Previews play instantly without loading the model. Regenerate them with `parlo-cli speak --previews` if the engine changes.

### 9.2 Text input and import
- Paste or type into the editor. Imported files: `.txt` (encoding detection: UTF-8/16 BOM, else `chardetng` + `encoding_rs`), `.md` (`pulldown-cmark` → plain text: headings become their own paragraphs, link text kept and URLs dropped, code blocks skipped by default, list items become sentences), `.docx` (zip + `quick-xml` over `word/document.xml` paragraphs, skipping headers/footers/comments), `.pdf` (**PDFKit** `PDFDocument.string` per page via `objc2-pdf-kit`, then de-hyphenate line breaks, remove repeated header/footer lines and bare page numbers; if there's no text layer → "This PDF has no selectable text (scanned). OCR isn't supported yet."), `.epub` (OPF spine order → XHTML → text, skipping nav/TOC).
- Show import stats and warnings (pages, characters, estimated listening time, anything skipped).
- **Large text:** if the text exceeds ~200 k characters, show a read-only, virtualized preview with an "Edit full text" escape hatch, so the editor stays responsive. Hard cap at ~5 M characters, with a clear message.
- **Normalization (testable, minimal):** Unicode NFC, smart quotes and dashes, collapsed whitespace, emoji and control characters removed, bare URLs replaced with "link". Don't rewrite numbers unless the parity tests show PocketTTS reading them badly. If they do, add targeted rules and log them.

### 9.3 Long-text pipeline
1. Split into paragraphs → sentences (Unicode sentence segmentation) → **job chunks** of whole paragraphs (≤ ~1000 chars; split long paragraphs at sentence boundaries). The engine does its own internal 50-token chunking.
2. For each chunk, stream synthesis → append PCM to the current **segment WAV** (24 kHz mono PCM16) on disk. Insert generated silence: ~400 ms between paragraphs, ~800 ms after headings. PocketTTS can't produce pauses from text.
3. **Rotate segments** about every 10 minutes of audio, at a paragraph boundary. Emit `segment_ready` so the player can **start playback while later segments generate**.
4. Checkpoint per chunk in `jobs/<id>/manifest.json`. Resume after crash or quit.
5. Output id = sha256(engine id + model revision + voice + quality + normalized text). An identical request is an instant cache hit.
6. Progress = characters done / total, with ETA from a rolling RTF.

### 9.4 Playback and export
- In-app player: `<audio>` over the Tauri asset protocol (scoped to `outputs/`; range requests for seeking) and a playlist across segments. Speed 0.5–2.0× (step 0.05; slider + numeric field like ctric), `preservesPitch = true`. **Speed affects playback only (D7).** The UI says "Exports are always 1×".
- Export ▾: **WAV** (hound; if > 4 GB, ask the user to choose M4A), **M4A/AAC** (AudioToolbox `ExtAudioFile`, ~96 kbps mono default), **MP3** (LAME via `mp3lame-encoder`, ~96 kbps CBR mono). Exports stream across segments, so memory stays bounded. Default file name: source file name or the first words of the text + voice + date.
- Outputs are kept for 7 days (same retention setting family), except the current workspace output.

### 9.5 Targets
- Time-to-first-audio ≤ 500 ms warm, ≤ 3 s cold (including model load).
- Throughput ≥ ctric reference (≈ 6× real-time on M-series CPU per Kyutai). Peak RAM ≤ 800 MB regardless of text length.

---

## 10. Model lifecycle and resource policy
- **Nothing loads at startup.** Engines load on first use (Generate / start of transcription). When the user presses Stop on a recording, start loading Whisper in parallel with file finalization.
- **Idle unload:** each engine unloads after `model_idle_timeout` (default **60 s** after its last job; Settings: Immediately / 60 s / 5 min / 15 min). Use a single re-armable timer, not polling.
- **Memory pressure:** subscribe to macOS memory-pressure notifications (dispatch source). On *warning*, unload idle engines immediately. On *critical*, also pause queued jobs.
- After unload, call `malloc_zone_pressure_relief` so freed memory goes back to the OS. Verify that RSS returns to within +30 MB of the pre-load baseline.
- **One job per engine at a time**, with a FIFO queue. STT and TTS may run concurrently.
- **Good-citizen compute:** threads run at user-initiated QoS during jobs. Settings → Performance: *Balanced* (default; caps CPU threads at about half the performance cores) / *Maximum*. Metal is preferred where the §6 benchmarks show it helps.
- **Window lifecycle:** closing the window (red button) **destroys the webview** to free WebKit memory. The app stays in the Dock, and recordings/jobs continue in the core. Clicking the Dock icon recreates the window and rehydrates state from the core. Show a notification when a job finishes while no window is open.
- **Single instance** (`tauri-plugin-single-instance`). A second launch focuses the existing window.

---

## 11. UI / UX specification

### 11.1 Visual direction
A calm, native-feeling macOS utility: think Voice Memos crossed with a minimal writing app.
- System font stack (`-apple-system`, SF Pro), 8 px spacing grid, 10–12 px radii, soft elevation, one accent color, and generous whitespace.
- Light/dark follows the system. Define all colors as CSS custom properties (tokens), with AA contrast in both themes.
- Overlay title bar with native traffic lights. A centered **segmented control "Transcribe | Speak"** sits in the title bar area (draggable regions around it), with a settings gear at the right.
- A subtle **status chip** at the bottom right shows engine state: "Idle · no models in memory" / "Speech model loaded · unloads in 42 s". It reassures a RAM-conscious user.
- Motion is 150–200 ms ease transitions. **No continuous animations while idle.** Respect `prefers-reduced-motion`.
- Window: default 960×700, minimum 720×560. Size and position are remembered (`tauri-plugin-window-state`).
- Lean dependencies: Vue, Pinia, a headless primitives library for accessible Select/Dialog/Slider/Menu if needed (e.g. Reka UI), and tree-shaken icons (e.g. `lucide-vue-next`). No CSS framework is required; the agent may use one if it stays small. No web fonts, no CDN, nothing remote.

### 11.2 Transcribe view states
1. **Empty:** three large cards side by side (two when downloads are switched off).
   - **Record** shows a big mic button, the current input device and the permission state.
   - **Transcribe a file** is a drop zone with "Choose file…" and the supported formats in small text.
   - **Transcribe a link** (DECISIONS C-021) has a URL field and a Transcribe button; ⌘V of a link on this screen and a link dragged from a browser start it too. The processing view gets a first stage, **Downloading** (percent, bytes, speed, ETA; "Waiting N s" during the rate-limit sleep; a notice when the system yt-dlp took over); everything after it is the normal file pipeline.
   - Under the cards: "Language: Auto-detect ▾".
2. **Recording:** a focused layout with a large timer, level meter, device, "Saved continuously ✓", Pause, **Stop & Transcribe**, and Discard. Warnings appear as inline banners (§8.2).
3. **Processing:** the source name and duration, then a stage list with the current stage highlighted, a progress bar with % and ETA, and Cancel. Paragraphs stream in below. "Loading speech model…" is shown when that's what's happening.
4. **Result:** a header (source, duration, language badge, word count), an editable transcript, and actions: Copy, Export ▾, Play source, and **New**. New asks for confirmation only if the transcript was edited and never copied or exported.
5. **Error:** a human message, Retry, and "Show details" (copyable). The source is preserved.

### 11.3 Speak view states
1. **Compose:** a large editor (placeholder: "Paste or type text — or drop a .txt, .md, .docx, .pdf or .epub"), Open file… and Clear. The footer shows words · characters · ~listening time at 1× (and at the current speed). After an import, a source chip shows the file name, page count and any warnings.
2. **Bottom bar:** a voice picker (name + ▶︎ preview per item; default George), a speed slider + numeric field (playback only), and **Generate** (⌘↩).
3. **Generating:** progress with ETA and Cancel. The player activates on the first `segment_ready`.
4. **Result:** the player (play/pause, scrub, elapsed/total, speed), Export ▾, Reveal in Finder, and Regenerate. If text or voice changed since generation, the output is marked "out of date".

### 11.4 Settings (modal sheet)
- **General:** appearance, model unload timeout, performance mode.
- **Recording:** input device (System default / specific), recording retention, open recordings folder.
- **Transcription:** default language (Auto / list).
- **Downloads:** "Allow downloads from URLs" (default on), the downloader in use (bundled / system path + version, deno found?), system yt-dlp path override, "Use browser cookies" (default off) with a picker of the installed browsers, browser impersonation (default on), rate-limit sleep 5–15 s (default on).
- **Speech:** default voice, default speed, quality (Standard/Fast).
- **Storage:** disk usage per category, clear outputs and caches.
- **About:** versions, engines, model names/revisions, licenses and attributions, "Parlo works offline; the only network use is a download you start; nothing is ever uploaded", and a "Copy diagnostics" button (versions + the last 200 log lines to the clipboard).

### 11.5 Global interactions
- Drag-and-drop anywhere in the window (use Tauri's native drag-drop events to get file paths). Audio/video goes to Transcribe; documents go to Speak (auto-switch).
- Keyboard shortcuts:
  - ⌘1 / ⌘2: switch tabs
  - ⌘R: record / stop
  - ⌘P: pause
  - ⌘O: open
  - ⌘↩: generate
  - Space: play / pause (when the editor isn't focused)
  - ⇧⌘C: copy transcript
  - ⌘E: export
  - ⌘,: settings
  - Esc: close dialogs
- Proper macOS menu bar (App, File, Edit, View, Window, Help) with the same shortcuts. Edit menu items must work in text fields.
- Accessibility: full keyboard navigation, visible focus rings, VoiceOver labels, `aria-live` for progress and status.
- Errors **never** use `alert()`. Use inline banners or toasts with clear next steps.

### 11.6 UI development mode
Provide a **mock IPC layer** (`VITE_MOCK_BACKEND=1`) so the UI runs in a normal browser with fake engines and recorder (scripted progress, fake levels, sample outputs). The agent uses it to iterate on and screenshot UI states quickly. Real-engine testing happens through the bundled app and `parlo-cli`. In the cloud, capture screenshots with **Playwright's WebKit** engine (closest to macOS WKWebView) for every state in light and dark mode, and review them yourself. Commit a small, curated set to `docs/screenshots/` for the owner.

---

## 12. Non-functional requirements and measurement

### 12.1 Targets (Apple M4 Pro, 48 GB, macOS 26.4)
| Metric | Target |
|---|---|
| Idle memory, window open, no models | ≤ **150 MB** total (Parlo process + its WebKit helper processes); Parlo main process ≤ 50 MB |
| Idle memory, window closed | ≤ **60 MB** total (no WebContent process alive) |
| Idle CPU | ~0 % (no wakeups beyond the OS baseline; check with `top`/Activity Monitor "Idle Wake Ups") |
| Cold launch → interactive | ≤ 1 s |
| Return to idle after a job | RAM back within +30 MB of baseline within timeout + 5 s |
| STT / TTS / recording | see §7.4, §9.5, §8.4 |
| App bundle size | informational (~1–1.3 GB with models); no hard limit |
| Network | **zero** sockets opened by the Parlo process; the only traffic is a user-started URL download, made by a child process |

If §6.3 shows the bare shell can't meet the idle targets, re-baseline to shell + 20 MB and document why.

### 12.2 Measurement scripts (required)
- `scripts/measure-idle-memory.sh`: launch the bundled app, wait for idle (and for model unload when testing after a job), then sum **physical footprint** (`footprint` CLI) for the Parlo process and every WebKit helper process (WebContent, Networking, GPU) attributable to it. The agent must find a reliable attribution method, e.g. the responsible-PID relationship or process-set diffing in a controlled run, and document it. Results go in `docs/STATUS.md` with date and commit.
- `scripts/check-offline.sh`: exercise every flow through `parlo-cli` and the app while sampling `lsof -i -a -p <pids>` (and/or `nettop`). Pass = no internet sockets in the Parlo process (for a URL job the *downloader child* may — and must — open its own; assert that Parlo's own PID did not). Also assert that the frontend CSP allows only self, IPC and asset protocols.
- `parlo-cli bench stt|tts|record` subcommands print RTF, TTFA, peak RSS and load times.

---

## 13. Packaging and platform
- `scripts/build-app.sh`: fetch/verify models → build UI → `tauri build` (arm64 only) → place models in `Contents/Resources/models/` → sign → output `dist/Parlo.app` (optionally a DMG).
- **Minimum macOS 14** (Sonoma) in the bundle config. Develop and test on macOS 26.4.
- `Info.plist`: `NSMicrophoneUsageDescription` ("Parlo records audio from your microphone to transcribe it on this Mac. Audio never leaves your computer."). Declare supported document types for audio/video/text, so "Open With → Parlo" works (Could-have; the routing is the same as drag-drop).
- **Signing:** use the hardened runtime with the `com.apple.security.device.audio-input` entitlement. Sign with `PARLO_SIGN_IDENTITY` if set, otherwise ad-hoc.
  - **Important:** an ad-hoc signature changes on every build, so macOS (TCC) forgets the microphone permission after each rebuild.
  - Document a one-time optional step in the README for the owner: create a self-signed "Parlo Local Signing" code-signing certificate in Keychain Access, so the mic permission survives rebuilds.
  - The app must handle the "not determined" state gracefully at all times.
- App Sandbox: **off** for v1 (personal use). Time-boxed *Could*: evaluate sandbox **without** the network entitlement as an OS-enforced offline guarantee (moot since C-021: a sandboxed downloader child would need the network entitlement). Adopt it only if drag-drop, dialogs, recordings, exports and WebKit all work. Log the result.
- Toolchains: see §5.6. On the Mac: Command Line Tools only (full Xcode **not** installed). Metal shaders must compile at runtime, which whisper.cpp/Candle do by default — verify. `cmake` may be installed via Homebrew. ffmpeg is for dev fixtures only, never bundled. In the cloud, apt packages come from the environment's setup script (`docs/CLOUD_SETUP.md`); the agent may also `apt install` mid-session, but those installs don't persist.
- **Models:** `scripts/fetch-models.sh` reads `models/manifest.toml` (name, source URL with pinned revision, sha256, size, license) and downloads at **build time only**. PocketTTS files come from Hugging Face at the pinned revisions and SHA-256s in §3.2. Whisper GGML comes from `ggerganov/whisper.cpp` on HF (e.g. `ggml-large-v3-turbo-q8_0.bin`), Silero VAD GGML from `ggml-org/whisper-vad`. Pin revisions, and record SHA-256 on first download. The script is idempotent: it skips verified files, resumes partial downloads, and can fetch a subset (e.g. `--only pockettts`) to save time in short cloud sessions. **Model files are never committed to git.**
- Licenses: record each model's and dependency's license in About and `models/manifest.toml`. PocketTTS code is MIT and its weights CC-BY-4.0 (check the voices' licenses); Whisper is MIT. LAME is LGPL, which is fine for personal use (flag it for any future distribution).

---

## 14. Testing and acceptance

### 14.1 Automated (must pass in `cargo test` / `bun test` or `vitest` / scripts)
- **Fixtures** via `scripts/gen-fixtures.sh`: espeak-ng (Linux) or `say` + `afconvert` (macOS) for synthetic speech, ffmpeg for formats and noise, and the pinned FLEURS subset (§6.2) for real multilingual speech. Downloads only from pinned sources, at dev time.
- **Decode:** every format in §7.1 decodes to the expected duration (±50 ms) and sample count.
- **Recorder:** a `FileSource` fake input (real-time and accelerated modes) feeds the real pipeline.
  - 3 h accelerated soak: bounded RAM, exact frame count, valid WAV.
  - Simulated stream errors and device swaps produce gap markers and continue.
  - `kill -9` mid-recording → the header repair yields a playable file with ≤ 2 s loss.
- **STT:** WER thresholds on FLEURS and synthetic fixtures per language (calibrated in §6.2). Language detection is correct on single-language fixtures.
  - A 2 h file transcribes with peak RAM ≤ the target.
  - Cancel and resume-from-checkpoint work.
  - A silence-only file yields an empty transcript, not hallucinations.
- **TTS:**
  - Parity suite (§6.1) re-runnable as `parlo-cli bench tts --parity`.
  - A 50 k-word text completes with bounded RAM and monotonic progress, emits `segment_ready` events, and resumes after a simulated crash.
  - A cache hit returns instantly.
- **Text import:** fixtures for each format (including a PDF with headers/footers and hyphenation, a scanned-PDF case, an EPUB with a TOC, and non-UTF-8 TXT) produce expected text snapshots.
- **Exports:** WAV/M4A/MP3 are decodable, with duration within ±100 ms (M4A on macOS only).
- **UI:** component and store tests for the state machines (Transcribe and Speak views), plus IPC mock contract tests.
- `scripts/measure-idle-memory.sh` and `scripts/check-offline.sh` pass (Mac). In the cloud, run the core offline check: execute the `parlo-cli` flows inside a network-less namespace (`unshare -rn` or `docker run --network none`); they must succeed.
- **Platform-gated tests** are marked with `#[cfg(target_os = "macos")]` or `#[ignore = "macos"]` and listed in `MAC_HANDOFF.md`, so nothing silently goes untested.

### 14.2 Acceptance scenarios (end-to-end, in the bundled app)
1. Fresh launch → no models loaded → idle memory within target.
2. Drop a 1 h MP3 interview → transcript with correct language, paragraphs, ≤ 4 min → export `.md`.
3. Record 10 min, switch AirPods in/out midway, lock the screen for 2 min → one continuous file with gap markers → auto-transcribed.
4. Start recording, `kill -9` Parlo, relaunch → recovery banner → transcribe the recovered file.
5. Import a 300-page EPUB → start Generate → playback starts within seconds → quit mid-way → relaunch → resume → export M4A.
6. Generate a short text with each of the 10 voices; previews play instantly.
7. After each scenario, wait for the idle timeout → models unloaded → memory back to baseline.
8. With Wi-Fi off and Little Snitch/`lsof` watching, every scenario works and no sockets open.

### 14.3 Manual QA checklist
Maintain `docs/QA-CHECKLIST.md`. It covers the §8.2 interruption table rows that can't be automated (real sleep/wake, real Bluetooth switching, lid close), the visual review of every UI state in light and dark mode, keyboard-only use, and VoiceOver basics.

---

## 15. Implementation plan (phases, each ends with a commit, a push, an updated `STATUS.md`, and passing tests)

| Phase | Where | Scope | Exit criteria |
|---|---|---|---|
| **0 — Spikes** | Cloud | Workspace skeleton, `CLAUDE.md`, `models/manifest.toml`, `fetch-models.sh`, `gen-fixtures.sh`, §6.1 and §6.2 decisions | Cloud gate in §6 met; decisions logged; `MAC_HANDOFF.md` started |
| **1 — Audio core** | Cloud | Recorder pipeline (ring buffer, crash-safe WAV, recovery, gap logic) with `FileSource`; cpal backend compiled; decoders (symphonia + Opus), resampler, WAV/MP3 encoders; platform traits (§5.7); `parlo-cli record/decode/encode` | Portable §14.1 decode, recorder and export tests pass |
| **2 — STT** | Cloud | `SttEngine` + whisper-rs impl, VAD chunker, language detect, post-filters, checkpoints, `parlo-cli transcribe` | §14.1 STT tests pass on CPU; relative perf recorded |
| **3 — TTS** | Cloud | `TtsEngine` + chosen runtime, text import (txt/md/docx/epub + PDF page-text cleanup), normalize/segment, long-text pipeline, cache, previews, `parlo-cli speak` | §14.1 TTS and import tests pass on CPU |
| **4 — App shell** | Cloud | Tauri commands/events, generated TS types, job manager, model lifecycle (timer + pressure hooks), single instance, menus; macOS-specific pieces drafted per §5.7 | IPC contract tests; app builds and runs on Linux (Xvfb smoke) |
| **5 — UI/UX** | Cloud | All views and states in §11, mock backend, tokens, light/dark, a11y, shortcuts | Every state screenshotted in Playwright WebKit (light + dark) and self-reviewed |
| **M — Mac integration & ship** | Mac | Everything in `MAC_HANDOFF.md`: §6.3–6.4, Metal builds and confirmations, CoreAudio/TCC, AVFoundation, AudioToolbox, PDFKit, power/sleep/memory-pressure, window lifecycle, `build-app.sh`, signing, entitlements, idle memory, offline check, §14.2 scenarios, QA checklist, README | §12 and §14 pass on the Mac; `dist/Parlo.app` runs from `/Applications` |

Within a phase, the agent orders the work itself. Across phases, keep the order unless a spike result forces a change (log it). The cloud phases may overlap when it helps, e.g. UI work in mock mode while long benchmarks run in the background.

---

## 16. Working agreement for the build agent
- **Autonomy:** make reasonable decisions within this brief's principles. Record each meaningful choice in `docs/DECISIONS.md`: context, alternatives, choice, why (one-liners are fine).
- **Escalate (stop and ask the owner)** only when:
  - (a) no TTS option passes §6.1, *including* the Python fallback;
  - (b) a requirement here is infeasible as written;
  - (c) an action would touch anything outside the repo and Parlo's own app/cache/log directories, except dev-tool installs (apt in the cloud, Homebrew on the Mac) and build-time downloads from the pinned sources;
  - (d) a network domain the work needs is blocked by the cloud allowlist. Write the exact domains to "Needs owner" in `STATUS.md`, then continue with other work.
- **Honesty:** report measured numbers, including misses. Don't mark targets as met without running the script. If a test is skipped, say so in `STATUS.md`.
- **Hygiene:** pin dependency versions in lockfiles; use the latest stable versions as of build time (Tauri 2.x stable — 2.12.x as of 2026-10; Vue 3.5+; current Vite). Keep the code warning-free: `cargo clippy -D warnings`, plus ESLint and `vue-tsc` clean.
- **Git:** work on `main` in the GitHub repo. Commit and **push** at every meaningful checkpoint, at least every ~30–60 minutes of work, because a reclaimed cloud VM loses anything unpushed. Use clear messages, no worktrees, no force-pushes. Never commit model weights, large audio, `.venv`, `target/` or `node_modules/`. Keep the repo lean (fixtures < ~5 MB total; everything else is regenerated by scripts).
- **Long-running work in the cloud:** foreground commands time out after 2–10 min. Run long builds, benchmarks and soaks in the background, writing logs to files, and check on them. Split big benchmark runs into resumable batches that write partial results to disk.
- **Don't:**
  - look for or depend on the original ctric repo (use `docs/reference/ctric/`);
  - add any runtime network feature other than the user-started URL download (updater, telemetry, crash upload, remote fonts/CDNs, background requests), or any HTTP client to the Parlo process;
  - use Electron;
  - use the webview `MediaRecorder` for recording;
  - load models at startup;
  - poll from the frontend;
  - hold whole audio files in memory;
  - use `alert()`;
  - bundle ffmpeg or Python, except the §6.1 fallback.

---

## 17. Later (explicitly out of v1, keep the architecture ready)
History library (SQLite: search, re-open, re-export) · menu-bar mode + global dictation hotkey with clipboard output · timestamps in the UI + SRT/VTT export (segments are already stored) · system-audio capture (ScreenCaptureKit) · multilingual TTS (PocketTTS FR/DE/ES/IT/PT packs, and newer `english_2026-04` weights, after parity testing) · voice cloning / custom voices · translation to English (Whisper translate task) · speaker diarization · OCR for scanned PDFs (Vision framework) · Finder Quick Action / Services menu · notarized DMG distribution · Parakeet or other faster STT engines.

---

## 18. Glossary
- **RTF** — real-time factor = processing time / audio duration (lower is faster; 0.1 = 10× real-time).
- **TTFA** — time to first audio chunk.
- **Voice state / embedding** — PocketTTS's precomputed conditioning (KV-cache) for a preset voice, stored as `.safetensors`.
- **Parity gate** — the measured comparison in §6.1 that a Rust TTS runtime must pass to replace ctric's Python runtime.
- **Workspace** — the current transcript and current TTS output, persisted across restarts (not a history library).
