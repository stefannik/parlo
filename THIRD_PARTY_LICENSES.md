# Third-party licenses

Parlo itself is MIT-licensed ([LICENSE](LICENSE)). The app bundles the models, voices and programs below. Each keeps
its own license. Exact files, revisions and SHA-256 checksums are in [`models/manifest.toml`](models/manifest.toml) and
[`tools/manifest.toml`](tools/manifest.toml).

## Models

| What | Source | License |
|---|---|---|
| Whisper large-v3-turbo (speech recognition, GGML q8_0) | OpenAI, converted by [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) | MIT |
| Silero VAD v5.1.2 (voice activity detection, GGML) | [snakers4/silero-vad](https://github.com/snakers4/silero-vad), via [ggml-org/whisper-vad](https://huggingface.co/ggml-org/whisper-vad) | MIT |
| PocketTTS English (`english_2026-09`) and Spanish models | Kyutai, [kyutai/pocket-tts-without-voice-cloning](https://huggingface.co/kyutai/pocket-tts-without-voice-cloning) | CC-BY-4.0, attribution: Kyutai |

## Voices

Kyutai provides the voice states for PocketTTS. The recordings they are made from carry their own licenses (details:
[kyutai/tts-voices](https://huggingface.co/kyutai/tts-voices)).

| Voices | Recordings from | License |
|---|---|---|
| Anna, Azelma, Charles, Eponine, Eve, Fantine, George, Jane, Mary, Michael, Paul, Vera | [VCTK corpus](https://datashare.ed.ac.uk/handle/10283/3443), University of Edinburgh | CC-BY-4.0 |
| Alba | Alba MacKenna | CC-BY-4.0 |
| Bill Boerst, Caro Davy, Peter Yearsley, Stuart Bell | [Voice-Zero](https://github.com/OwenTyme/voice-zero) (LibriVox readers) | CC0 |
| Javert, Marius | Kyutai's Unmute Voice Donation Project | CC0 |
| Lola (Spanish) | Mozilla Common Voice | CC0 |
| Enrique (Spanish) | Kyutai's Unmute Voice Donation Project (`Enrique_(Spanish)`); voice state made for Parlo with `scripts/export-voices.py` | CC0 recording; state derived with the CC-BY-4.0 model |

## Programs and libraries

| What | Use in Parlo | License |
|---|---|---|
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) 2026.08.19 (official macOS build, with its embedded Python runtime) | "Transcribe a link" downloader, runs as a separate process | Unlicense; the embedded Python and libraries keep their own licenses (PSF and others) |
| [LAME](https://lame.sourceforge.io) via `mp3lame-sys` | MP3 export | LGPL-2.0-or-later. LAME is linked into the app. Parlo's complete source is in this repository, so you can rebuild the app with a modified LAME (LGPL §6). LAME's own source: https://lame.sourceforge.io |
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp) via `whisper-rs` | speech recognition engine | MIT |
| [Candle](https://github.com/huggingface/candle) and a patched PocketTTS port (`crates/pocket-tts`, from babybirdprd/pocket-tts) | speech synthesis engine | MIT OR Apache-2.0 |
| [Symphonia](https://github.com/pdeljanov/Symphonia) | audio decoding | MPL-2.0 |
| [Tauri](https://tauri.app) | app shell | MIT OR Apache-2.0 |
| [Vue](https://vuejs.org), [Pinia](https://pinia.vuejs.org) | user interface | MIT |

The remaining Rust and JavaScript dependencies are listed in `Cargo.lock` and `ui/package-lock.json`, each with its own
license (see each package's metadata on crates.io and npm).
