#!/usr/bin/env bash
# Build dist/Parlo.app (macOS, arm64). DRAFTED IN THE CLOUD — never run on a Mac yet (docs/MAC_HANDOFF.md section D1).
#   scripts/build-app.sh [--dmg] [--no-metal] [--skip-models]
# Steps: fetch/verify models + yt-dlp → build UI → `tauri build` → place models + the bundled yt-dlp (Contents/Helpers) →
# sign (PARLO_SIGN_IDENTITY, else ad-hoc; the helper gets its own entitlements) → dist/Parlo.app
# PARLO_YTDLP_VARIANT=onedir|onefile picks the yt-dlp build (default onedir; DECISIONS C-022).
# Prerequisites: Xcode Command Line Tools, Rust (aarch64-apple-darwin), Node ≥ 22, `brew install cmake`, `cargo install tauri-cli --locked --version '^2'`.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD

DMG=0 METAL=1 FETCH=1
for a in "$@"; do
  case "$a" in
    --dmg) DMG=1 ;;
    --no-metal) METAL=0 ;;
    --skip-models) FETCH=0 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done
[[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] || { echo "build-app.sh must run on an Apple-silicon Mac" >&2; exit 1; }

# 1 · models (pinned + SHA-256 verified; skips what is already verified)
if [[ $FETCH == 1 ]]; then scripts/fetch-models.sh --only pockettts,whisper,vad; fi
# 1b · the downloader for "Transcribe from URL" (pinned + SHA-256 verified; never committed). Bundled so the feature works
# out of the box; it runs as a child process — Parlo itself never opens a socket (DECISIONS C-021).
YTDLP_VARIANT="${PARLO_YTDLP_VARIANT:-onedir}"
scripts/fetch-tools.sh --platform macos --variant "$YTDLP_VARIANT"
[[ -x tools/yt-dlp/yt-dlp ]] || { echo "missing tools/yt-dlp/yt-dlp — run scripts/fetch-tools.sh" >&2; exit 1; }
WHISPER_MODEL="ggml-large-v3-turbo-q8_0.bin"   # keep in sync with DEFAULT_WHISPER_MODEL in src-tauri/src/state.rs
for f in "models/whisper/$WHISPER_MODEL" "models/vad/ggml-silero-v5.1.2.bin" "models/pockettts"; do
  [[ -e "$f" ]] || { echo "missing $f — run scripts/fetch-models.sh" >&2; exit 1; }
done

# 2 · UI
( cd ui && npm ci && npm run build )

# 3 · Tauri bundle (Metal for whisper.cpp and Candle unless --no-metal)
FEATURES=()
[[ $METAL == 1 ]] && FEATURES=(--features "whisper-metal,tts-accelerate")
( cd src-tauri && cargo tauri build --target aarch64-apple-darwin --bundles app "${FEATURES[@]}" )
APP_SRC="target/aarch64-apple-darwin/release/bundle/macos/Parlo.app"
[[ -d "$APP_SRC" ]] || { echo "tauri build did not produce $APP_SRC" >&2; exit 1; }

# 4 · assemble dist/Parlo.app and place models (never committed; copied from the verified local cache)
rm -rf dist && mkdir -p dist
cp -R "$APP_SRC" dist/Parlo.app
RES="dist/Parlo.app/Contents/Resources"
mkdir -p "$RES/models/whisper" "$RES/models/vad" "$RES/resources"
cp "models/whisper/$WHISPER_MODEL" "$RES/models/whisper/"
cp "models/vad/ggml-silero-v5.1.2.bin" "$RES/models/vad/"
cp -R "models/pockettts" "$RES/models/pockettts"
[[ -d resources/voice-previews ]] && cp -R resources/voice-previews "$RES/resources/voice-previews"
# the bundled downloader: Contents/Helpers/yt-dlp/yt-dlp (+ _internal/ for onedir) — found by parlo_core::download::discover
mkdir -p dist/Parlo.app/Contents/Helpers
cp -R tools/yt-dlp dist/Parlo.app/Contents/Helpers/yt-dlp
rm -f dist/Parlo.app/Contents/Helpers/yt-dlp/.pinned

# 5 · sign: hardened runtime + audio-input entitlement. An ad-hoc signature changes on every build, so macOS (TCC)
# forgets the microphone permission after each rebuild — create a self-signed "Parlo Local Signing" certificate once
# (Keychain Access → Certificate Assistant → Create a Certificate… → Code Signing) and export PARLO_SIGN_IDENTITY="Parlo Local Signing".
IDENTITY="${PARLO_SIGN_IDENTITY:--}"
# 5a · the bundled yt-dlp first (inside-out): every Mach-O under Helpers/yt-dlp (the launcher, Python, the .so/.dylib files
# of _internal/) gets the hardened runtime and src-tauri/entitlements-ytdlp.plist (PyInstaller needs library validation off
# and executable memory; scoped to this helper only — the app itself keeps its own, smaller set). `--deep` is NOT used for
# the app below, because it would re-sign the helper with the app's entitlements.
HELPER=dist/Parlo.app/Contents/Helpers/yt-dlp
while IFS= read -r f; do
  if file -b "$f" | grep -q 'Mach-O'; then
    codesign --force --options runtime --entitlements src-tauri/entitlements-ytdlp.plist --sign "$IDENTITY" "$f"
  fi
done < <(find "$HELPER" -type f | awk '{ print length, $0 }' | sort -rn | cut -d' ' -f2-)
# 5b · the app
codesign --force --options runtime --entitlements src-tauri/entitlements.plist --sign "$IDENTITY" dist/Parlo.app
codesign --verify --deep --strict --verbose=2 dist/Parlo.app
# the bundled downloader must start from inside the signed bundle (hardened runtime, Gatekeeper quarantine not set here)
"$HELPER/yt-dlp" --version | sed 's/^/bundled yt-dlp: /'
du -sh "$HELPER" | sed 's/^/bundled yt-dlp size: /'
echo "signed with: ${PARLO_SIGN_IDENTITY:-ad-hoc (mic permission will be re-requested after every rebuild)}"

# 6 · optional DMG
if [[ $DMG == 1 ]]; then
  hdiutil create -volname Parlo -srcfolder dist/Parlo.app -ov -format UDZO dist/Parlo.dmg
fi
du -sh dist/Parlo.app
echo "built dist/Parlo.app"
