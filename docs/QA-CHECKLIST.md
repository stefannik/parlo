# Parlo — Manual QA checklist

Run on the Mac with the bundled app. Tick with date + commit.

## Recording interruptions (BRIEF §8.2)
- [x] Close window while recording → recording continues; reopen reattaches
- [x] ⌘Q while recording → "Stop and save / Cancel" dialog (owner: dialog + save OK; the saved recording was not offered on relaunch → fixed: it now appears in the recovery banner as "saved when you quit — not yet transcribed"; re-verify)
- [x] `kill -9` while recording → file recoverable and playable, 1.0 s lost (real mic, bundled app; `scripts/mac-kill9-test.sh`, 2026-10-15). Recovery *banner* look: UI pass pending
- [x] AirPods connect/disconnect (System default follows) → same file, gap marker, notice
- [x] Pinned device vanishes → falls back to default + notice
- [x] Screen lock 2 min → continues
- [x] Lid close / forced sleep → flush, resume on wake with gap marker
- [ ] Disk nearly full → warning at < 2 GB, graceful stop at < 300 MB
- [ ] Mic permission denied/revoked → guidance + button to System Settings (granted state + prompt verified 2026-10-15)
- [ ] Muted mic > 10 s → inline warning

## UI review (light + dark) — in the bundled app (WKWebView)
Cloud pre-check done with Playwright-WebKit on every state below (`docs/screenshots/`); the Mac pass confirms WKWebView specifics.
- [ ] Transcribe: empty · mic denied (guided fix) · recording (timer, level meter, banners, "Saved continuously") · paused · processing (stages, %, ETA, partial text, "Loading speech model…") · result (edit, Copy, Export ▾, Save audio, language badge → re-transcribe, mini player) · error (Show details/Copy details, Retry)
- [ ] Speak: compose · imported (source chip, warnings, large-document preview) · voice picker (previews play) · generating (player active on first segment) · result (Export ▾, Reveal) · "out of date" marker · error
- [ ] Settings (all six sections), recovery banner, drag-over overlay, toasts, confirm dialogs (Discard recording / New with edits / re-transcribe), quit dialog while recording
- [ ] Overlay title bar: traffic lights don't overlap the segmented control; the bar drags the window; buttons inside it still click
- [x] Window renders in WKWebView (dark, overlay title bar, traffic lights clear of the segmented control): `docs/screenshots-mac/transcribe-empty-dark.png`
- [ ] Contrast in both themes (`npm --prefix ui run check:contrast` passes; eyeball the text-3 placeholders)
- [ ] Reduced motion: no transitions with *System Settings → Accessibility → Reduce motion*
- [ ] Keyboard-only: ⌘1/⌘2, ⌘R, ⌘P, ⌘O, ⌘↩, ⇧⌘C, ⌘E, ⌘,, Esc, Space (play/pause outside text fields); Tab order is sensible; focus rings visible; menus/popovers trap and restore focus
- [ ] VoiceOver: segmented control announces "Transcribe, radio button, 1 of 2"; progress bars announce %; stage list is a live region; recorder timer not chatty; all icon buttons labelled
- [ ] Idle: no timers/animation while nothing happens (Activity Monitor → CPU 0.0 %, no wake-ups); status chip countdown only while a model is loaded

## Acceptance scenarios (BRIEF §14.2)
- [ ] 1–8 as listed

## Transcribe from URL (C-021)
- [ ] Empty screen: three cards (Record / file / link); Settings → Downloads → off hides the link card and ⌘V of a link does nothing
- [ ] Type a bad link (`youtube.com/x`, `ftp://…`): inline message, nothing starts
- [ ] ⌘V of a YouTube link on the empty screen starts a download; pasting while typing in the field does not
- [ ] Drag a link from a browser's address bar onto the window (see MAC_HANDOFF H11)
- [ ] Downloading stage shows the title, "Waiting N s", then percent · bytes · speed · ETA; Cancel works in both
- [ ] Cancel → empty screen, the link is still in the field, `downloads/` is empty
- [ ] Quit (⌘Q) during a download, relaunch: link refilled, nothing half-written on disk
- [ ] Transcript on screen → ⌘Q → relaunch: empty Transcribe screen, Record ready, "Last transcript" link restores it exactly; same via Dock → Quit (C-026)
- [ ] Transcript on screen → close the window (red X) → reopen from the Dock: the transcript is still on screen (no quit, nothing parked)
- [ ] Transcript on screen → `kill -9` → relaunch: the transcript is shown directly (a crash never parks it)
- [ ] Live stream, private/age-restricted (cookies hint + "Downloads settings"), Spotify (DRM), 404, Wi-Fi off, disk full: each shows its own message
- [ ] Result: host shown, player plays the kept audio, re-transcribe in another language doesn't download again, Save audio → "As downloaded" is byte-identical
- [ ] "New" deletes the downloaded audio (Settings → Storage → Downloaded audio goes to 0 B)
- [ ] Bundled → system yt-dlp fallback notice; wrong yt-dlp path override is flagged

