# Cloud setup (for the owner)

How to run Parlo's build in Claude Code cloud sessions, and when to switch to the Mac. Facts about the cloud VM come from the Claude Code docs as of 2026-10-14: Ubuntu 24.04 x86_64, 4 vCPU, 16 GB RAM, 30 GB disk, no GPU, no macOS. The setup script runs as root and is cached when it finishes in under ~5 min. The VM is reclaimed after inactivity; reopening a session restores the conversation but not running processes.

## 1. Repository
1. Create an **empty private** GitHub repo, e.g. `parlo`.
2. Put this folder's contents in it (`CLAUDE.md`, `docs/`) on `main`:
   ```bash
   cd ~/Desktop/Projects/parlo
   git init -b main
   git add CLAUDE.md docs
   git commit -m "docs: project brief, kickoff prompts, ctric reference"
   git remote add origin git@github.com:<you>/parlo.git
   git push -u origin main
   ```
3. Make sure cloud sessions can reach the repo: install the Claude GitHub App on it during claude.ai/code onboarding, or use `/web-setup`.

## 2. Cloud environment (claude.ai/code → environment selector → new environment, e.g. "parlo")

**Network access:** **Custom**, with **"Also include default list of common package managers" checked**. Allowed domains:
```
huggingface.co
*.huggingface.co
hf.co
*.hf.co
*.xethub.hf.co
download.pytorch.org
download-r2.pytorch.org
cdn.playwright.dev
playwright.download.prss.microsoft.com
playwright.azureedge.net
codeberg.org
docs.rs
```
Why:
- **Hugging Face:** model weights and the FLEURS test-speech subset (large files are served from `cdn-lfs*.huggingface.co` and `*.xethub.hf.co`).
- **PyTorch:** CPU-only torch wheels for the reference TTS generator.
- **Playwright:** WebKit browser for UI screenshots.
- **Codeberg:** whisper-rs source and issues.
- **docs.rs:** crate documentation.

GitHub, crates.io, npm, PyPI and Ubuntu apt are already in the default list.

**yt-dlp ("Transcribe from URL", `scripts/fetch-tools.sh`):** the pinned release is downloaded from `github.com/yt-dlp/yt-dlp/releases/download/<version>/…`, which redirects to `release-assets.githubusercontent.com`. Both were reachable with the default list on 2026-10-18; if a session reports them blocked, add `release-assets.githubusercontent.com` (and `objects.githubusercontent.com`). The feature's tests deliberately use **local** HTTP servers (`scripts/fixture-server.py`); YouTube and other sites are not (and need not be) on the allowlist — real-site checks are in `docs/MAC_HANDOFF.md` section H.

One-time WebKit setup for `scripts/screenshots`: `cd scripts/screenshots && npm ci && npx playwright install webkit && npx playwright install-deps webkit` (the last one needs root/apt; the setup script can run it).

**Environment variables:**
```
BASH_DEFAULT_TIMEOUT_MS=600000
BASH_MAX_TIMEOUT_MS=1800000
CARGO_TERM_COLOR=never
HF_HUB_DISABLE_TELEMETRY=1
```

**Setup script** (keep it under ~5 minutes so it gets cached):
```bash
#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential pkg-config libssl-dev file curl wget \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev \
  libasound2-dev libopus-dev \
  ffmpeg espeak-ng \
  xvfb xauth
if command -v rustup >/dev/null 2>&1; then
  rustup component add clippy rustfmt || true
  rustup target add aarch64-apple-darwin || true
fi
```
The agent installs Playwright WebKit itself during the session (`npx playwright install --with-deps webkit`), because doing it here could push the setup past the cache limit.

## 3. Starting and steering the build
- Start a cloud session on the repo with the **parlo** environment and paste `docs/KICKOFF_PROMPT.md` (everything below its line).
- The agent works through the cloud phases (0–5 in BRIEF §15) without stopping for check-ins. It pushes to `main` frequently and keeps `docs/STATUS.md` current.
- **Check in occasionally:** read the top of `docs/STATUS.md` on GitHub, especially "Needs owner". If it lists blocked domains, add them to the environment's allowlist.
- **If a session stops or expires:** reopen it and say "continue". Or start a new session with the same kickoff prompt; it resumes from `STATUS.md`.
- Usage: cloud sessions count toward your plan's normal usage limits. There's no separate VM charge.

## 4. Switching to the Mac
When `STATUS.md` says the cloud phases are done (or only Mac-dependent work remains):
```bash
cd ~/Desktop/Projects/parlo
git pull origin main
```
Then open a **local** Claude Code session in that folder and paste `docs/MAC_FINISH_PROMPT.md`. You'll be asked to do a few things only you can do: approve the microphone permission prompt, switch AirPods, close the lid, and listen to the A/B voice page.
