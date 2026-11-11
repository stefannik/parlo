# Parlo

Fully local macOS app: speech→text (files + long mic recordings, multilingual) and text→speech (English + Spanish, PocketTTS). Tauri v2 (Rust) + Vue 3 (Composition API, TypeScript). Offline: the only network use is a URL download you start; nothing is ever uploaded (DECISIONS C-021).

## Read first
1. `docs/STATUS.md` — current state, next steps, "Needs owner" (if it exists, you are resuming).
2. `docs/BRIEF.md` — the source of truth for requirements, architecture, targets and phases.
3. On the Mac: `docs/MAC_HANDOFF.md` — what still needs macOS to implement or verify.

Local-only (not in the public repo, gitignored; owner's archive: `parlo-internal-files-2026-11-11.tar.gz` in the project files): `docs/reference/ctric/` (prior-art code from the owner's earlier app, read-only knowledge), `spike/` (Phase 0 evidence and tooling), `docs/KICKOFF_PROMPT.md`, `docs/MAC_FINISH_PROMPT.md`. Scripts that use them (`scripts/tts-parity.sh`, `scripts/make-pockettts-config.py`) only work where those copies exist.

## Rules
- Work on `main`. Commit **and push** at every meaningful checkpoint. No git worktrees, no force-push.
- Never commit model weights, large audio, `.venv/`, `target/`, `node_modules/`. Models come from `scripts/fetch-models.sh` (pinned + SHA-256).
- The shipped app is offline; the only network use is a URL download you start; nothing is ever uploaded. The Parlo process itself never opens a socket (downloads run in a yt-dlp/curl child process; no HTTP-client crate). Build-time downloads only from pinned sources (models, yt-dlp).
- Log meaningful decisions in `docs/DECISIONS.md` (context · alternatives · decision · why).
- Report measured numbers honestly. Never weaken tests or targets to pass.
- Keep `docs/STATUS.md` handoff-ready at all times.

## Commands
```bash
# --- Rust (workspace: crates/parlo-core, crates/parlo-cli; spike/* are standalone crates, excluded)
cargo build -p parlo-cli                      # CLI (decode | info | record | recover | devices | transcribe | speak | download <url>)
cargo test --workspace                        # needs fixtures (scripts/gen-fixtures.sh), models (fetch-models.sh) and tools (fetch-tools.sh) — tests fail loudly if missing
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
cargo check --target aarch64-apple-darwin -p parlo-core --no-default-features --features cpal-input   # macOS type-check from Linux (C deps — LAME, whisper.cpp — need a real macOS toolchain, so they are off)

# --- Tauri app + UI (src-tauri = crate parlo-app, binary `parlo`; ui/ = Vue 3 + Pinia + Vite)
npm --prefix ui ci && npm --prefix ui run build            # vue-tsc + vite build (ui/dist)
npm --prefix ui test                                       # vitest: stores, mock-backend contract, app shell (94 tests)
npm --prefix ui run lint && npm --prefix ui run typecheck && npm --prefix ui run check:contrast
VITE_MOCK_BACKEND=1 npm --prefix ui run dev                # UI in a plain browser with fake engines/recorder (http://localhost:5173)
PARLO_UPDATE_BINDINGS=1 cargo test -p parlo-app export_bindings   # regenerate ui/src/bindings.ts after changing src-tauri/src/ipc.rs (plain run = drift check)
cargo build -p parlo-app --features tauri/custom-protocol && scripts/smoke-linux.sh   # app smoke run under Xvfb (software rendering)
scripts/selftest-linux.sh                                  # E2E of the command layer with the REAL engines inside the real app (needs models + fixtures + tools; ~4 min); also runs the URL job steps against local servers
PARLO_SELFTEST=url scripts/selftest-linux.sh               # only the "Transcribe from URL" steps (~30 s)
scripts/fixture-server.py DIR [--bps N] [--no-head]        # tiny local HTTP server (Range, redirects, /status/<code>, throttle) for the URL tests
cargo test -p parlo-core --test download_fake --test download_real   # URL flow with scripts/fake-yt-dlp.py / with the real yt-dlp + curl on localhost
(cd scripts/screenshots && npm ci && node shoot.mjs ../../docs/screenshots)          # Playwright-WebKit screenshots, every state, light+dark
scripts/build-app.sh [--dmg]                               # macOS only (drafted): dist/Parlo.app
cargo check --target aarch64-apple-darwin -p parlo-macos   # type-check the macOS platform code from Linux

# --- Models & fixtures (build/dev time only)
scripts/fetch-models.sh [--only pockettts,whisper,vad,eval] [--match q8_0] [--verify-only]
scripts/gen-fixtures.sh [--only synth,formats,edge,text,url,fleurs] [--long2h]   # espeak-ng/say + ffmpeg; url = podcast/DASH-shaped audio for the URL tests; fleurs ≈ 7 min, network
scripts/fetch-tools.sh [--variant onedir|onefile] [--verify-only]            # pinned yt-dlp → tools/yt-dlp (URL downloader; build/dev time only; tests, self-test and build-app.sh need it)
python3 scripts/make-pockettts-config.py                                     # models/pockettts/english.local.yaml (local-path config)

# --- Spikes (Phase 0; local-only copies, see above)
spike/tts-parity/run_ref.sh main|long         # Python reference generation (resumable, background it)
(cd spike/tts-rust && cargo build --release)  # Candle PocketTTS candidate
(cd spike/stt-rust && cargo build --release)  # whisper-rs bench tool
spike/tts-parity/metrics.py prep|score ...    # parity metrics (uses spike/tts-parity/.venv)

# --- Offline check / Mac measurements
scripts/check-offline.sh --cli target/release/parlo-cli   # Linux: CLI flows in a network-less namespace; PARLO_OFFLINE_FULL=1 adds the sampled app self-test; a URL job must show 0 sockets in Parlo's PID, sockets only in the downloader child
scripts/measure-idle-memory.sh --app dist/Parlo.app       # macOS only (drafted)
```
