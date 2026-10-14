# Parlo

Fully local macOS app: speech→text (files + long mic recordings, multilingual) and text→speech (English, PocketTTS). Tauri v2 (Rust) + Vue 3 (Composition API, TypeScript). No network at runtime.

## Read first
1. `docs/STATUS.md` — current state, next steps, "Needs owner" (if it exists, you are resuming).
2. `docs/BRIEF.md` — the source of truth for requirements, architecture, targets and phases.
3. On the Mac: `docs/MAC_HANDOFF.md` — what still needs macOS to implement or verify.

Prior-art code from the owner's earlier app is in `docs/reference/ctric/` (read-only knowledge). The original ctric repo is not available.

## Rules
- Work on `main`. Commit **and push** at every meaningful checkpoint. No git worktrees, no force-push.
- Never commit model weights, large audio, `.venv/`, `target/`, `node_modules/`. Models come from `scripts/fetch-models.sh` (pinned + SHA-256).
- The shipped app makes zero network connections. Build-time downloads only from pinned sources.
- Log meaningful decisions in `docs/DECISIONS.md` (context · alternatives · decision · why).
- Report measured numbers honestly. Never weaken tests or targets to pass.
- Keep `docs/STATUS.md` handoff-ready at all times.

## Commands
```bash
# --- Rust (workspace: crates/parlo-core, crates/parlo-cli; spike/* are standalone crates, excluded)
cargo build -p parlo-cli                      # CLI (decode | info | record | recover | devices)
cargo test --workspace                        # needs fixtures: scripts/gen-fixtures.sh (tests fail loudly if missing)
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
cargo check --target aarch64-apple-darwin -p parlo-core --no-default-features --features cpal-input   # macOS type-check from Linux (mp3/LAME needs a real C toolchain, so it is off)

# --- Models & fixtures (build/dev time only)
scripts/fetch-models.sh [--only pockettts,whisper,vad,eval] [--match q8_0] [--verify-only]
scripts/gen-fixtures.sh [--only synth,formats,edge,text,fleurs] [--long2h]   # espeak-ng/say + ffmpeg; fleurs ≈ 7 min, network
python3 scripts/make-pockettts-config.py                                     # models/pockettts/english.local.yaml (local-path config)

# --- Spikes (Phase 0)
spike/tts-parity/run_ref.sh main|long         # Python reference generation (resumable, background it)
(cd spike/tts-rust && cargo build --release)  # Candle PocketTTS candidate
(cd spike/stt-rust && cargo build --release)  # whisper-rs bench tool
spike/tts-parity/metrics.py prep|score ...    # parity metrics (uses spike/tts-parity/.venv)

# --- Offline check / Mac measurements
scripts/check-offline.sh --cli target/release/parlo-cli   # Linux: runs CLI flows in a network-less namespace
scripts/measure-idle-memory.sh --app dist/Parlo.app       # macOS only (drafted)
```
