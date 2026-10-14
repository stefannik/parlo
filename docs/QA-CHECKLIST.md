# Parlo — Manual QA checklist

Run on the Mac with the bundled app. Tick with date + commit.

## Recording interruptions (BRIEF §8.2)
- [ ] Close window while recording → recording continues; reopen reattaches
- [ ] ⌘Q while recording → "Stop and save / Cancel" dialog
- [ ] `kill -9` while recording → relaunch shows recovery banner; file playable, ≤ 2 s lost
- [ ] AirPods connect/disconnect (System default follows) → same file, gap marker, notice
- [ ] Pinned device vanishes → falls back to default + notice
- [ ] Screen lock 2 min → continues
- [ ] Lid close / forced sleep → flush, resume on wake with gap marker
- [ ] Disk nearly full → warning at < 2 GB, graceful stop at < 300 MB
- [ ] Mic permission denied/revoked → guidance + button to System Settings
- [ ] Muted mic > 10 s → inline warning

## UI review (light + dark)
- [ ] Every state in BRIEF §11.2–11.4
- [ ] Keyboard-only use; focus rings visible
- [ ] VoiceOver labels on primary controls

## Acceptance scenarios (BRIEF §14.2)
- [ ] 1–8 as listed
