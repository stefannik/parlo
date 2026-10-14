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
<!-- The build agent maintains this section: build, test, lint, fetch models, benchmarks, run UI in mock mode. -->
