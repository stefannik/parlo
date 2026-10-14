# Parlo

A fully local macOS app: **speech → text** (audio/video files and long microphone recordings, multilingual) and **text → speech** (English, Kyutai PocketTTS). Tauri v2 (Rust) + Vue 3. After the models are installed it never touches the network.

![Parlo, Transcribe tab](docs/screenshots/transcribe-result.light.png)

## Where things are
| | |
|---|---|
| `docs/BRIEF.md` | requirements, architecture, targets, phases (source of truth) |
| `docs/STATUS.md` | current state, measurements, what's next / needs the owner |
| `docs/DECISIONS.md` | every meaningful decision (context · alternatives · decision · why) |
| `docs/MAC_HANDOFF.md` | what still needs macOS (checklist with verification steps) |
| `docs/QA-CHECKLIST.md` | manual QA (recording interruptions, UI review, acceptance scenarios) |
| `crates/parlo-core` | audio, recorder, STT/TTS pipelines, text import, jobs, model lifecycle (portable Rust) |
| `crates/parlo-cli` | headless access to all of it (tests, benchmarks): `decode · record · transcribe · lang · speak · bench · recover` |
| `crates/parlo-macos` | macOS platform code (objc2): PDFKit, AVFoundation fallback decoder, AAC encoder, power/sleep/memory, mic permission |
| `crates/pocket-tts` | vendored + patched Candle port of PocketTTS (see `PATCHES.md`) |
| `src-tauri` | the app shell: typed IPC (`tauri-specta`), window lifecycle, menu, notifications |
| `ui` | Vue 3 + Pinia UI; runs in a browser with a mock backend (`VITE_MOCK_BACKEND=1`) |
| `spike/` | Phase-0 evidence: TTS parity gate, STT model bench |

Commands are listed in [`CLAUDE.md`](CLAUDE.md#commands).

## Quick start (macOS, Apple silicon)
```bash
brew install cmake                       # whisper.cpp and LAME build from source
scripts/fetch-models.sh                  # pinned + SHA-256-verified models (build time only)
scripts/build-app.sh                     # → dist/Parlo.app (Metal enabled)
```
Development in the cloud or on Linux: see `docs/CLOUD_SETUP.md`; `cargo test --workspace` and `npm --prefix ui test` need no Mac.

## Keeping the microphone permission across rebuilds
macOS ties the microphone permission (TCC) to the app's code signature. An ad-hoc signature (the default of `build-app.sh`) changes on every build, so macOS asks again after each rebuild. To avoid that, create a self-signed code-signing certificate once:

1. *Keychain Access → Certificate Assistant → Create a Certificate…*
2. Name: **Parlo Local Signing**, Identity Type: *Self Signed Root*, Certificate Type: *Code Signing*.
3. `export PARLO_SIGN_IDENTITY="Parlo Local Signing"` before running `scripts/build-app.sh`.

## Licenses
Parlo: MIT. Models: Whisper (MIT), Silero VAD (MIT), Kyutai PocketTTS (code MIT; weights CC-BY-4.0 — attribution: Kyutai). LAME (LGPL) is statically linked for MP3 export: fine for personal use, flag it before any distribution. See `models/manifest.toml` and the app's About pane.
