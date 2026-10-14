# Parlo — Mac handoff checklist

Executed top to bottom by the local Mac session (`docs/MAC_FINISH_PROMPT.md`). Status values: *not started* / *drafted, unchecked* / *type-checked* / *verified on Mac*.

| # | Item | Where | Status | How to verify | Brief § |
|---|---|---|---|---|---|
| 1 | Metal confirmation for TTS runtime (RTF ≤ reference, TTFA ≤ 500 ms, load ≤ 3 s) | TBD after §6.1 | not started | `parlo-cli bench tts --parity` | 6.1, 9.5 |
| 2 | Metal confirmation for STT (≥ 15× RT, RAM ≤ 1.5 GB, load ≤ 2 s) | TBD after §6.2 | not started | `parlo-cli bench stt` | 6.2, 7.4 |
| 3 | Shell idle baseline | `scripts/measure-idle-memory.sh` | not started | run on bundled empty shell | 6.3, 12 |
| 4 | Real recorder via cpal/CoreAudio, TCC, AirPods, sleep/wake | `parlo-core::platform` / `audio::capture` | not started | manual + QA checklist | 6.4, 8 |

_(Filled in as phases progress.)_
