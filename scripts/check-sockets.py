#!/usr/bin/env python3
"""Assert on a scripts/sample-sockets.py report (used by scripts/check-offline.sh).

  check-sockets.py report.json [--expect-downloader-sockets | --expect-no-child-sockets]

Always: the Parlo process itself opened no inet socket and the sampler really sampled (≥ 20 samples).
--expect-downloader-sockets: the downloader child (yt-dlp / curl) DID open sockets, all to loopback (the local test server)
                             — the positive control that proves the sampler can see sockets.
--expect-no-child-sockets:   nothing in the process tree opened an inet socket either (the classic flows).
"""
import json
import sys

rep = json.load(open(sys.argv[1]))
fail = []
if rep["samples"] < 20:
    fail.append(f"only {rep['samples']} samples — the sampler did not run properly")
if rep["own_inet"]:
    fail.append(f"the Parlo process itself opened inet sockets: {rep['own_inet']}")
downloader = [c for c in rep["child_inet"] if c.split()[0].lower().startswith(("yt-dlp", "yt_dlp", "curl", "python"))]
if "--expect-downloader-sockets" in sys.argv:
    if not downloader:
        fail.append(f"expected the downloader child to hold sockets (positive control); saw {rep['child_inet']}")
    elif not all("0100007F:" in c or "00000000000000000000000001000000:" in c for c in downloader):
        fail.append(f"downloader sockets must be loopback-only here: {downloader}")
if "--expect-no-child-sockets" in sys.argv and rep["child_inet"]:
    fail.append(f"descendant processes opened inet sockets: {rep['child_inet']}")
if fail:
    print("FAIL: " + "; ".join(fail), file=sys.stderr)
    print(json.dumps(rep, indent=2), file=sys.stderr)
    sys.exit(1)
print(f"  sockets: {rep['samples']} samples, Parlo PID: 0 inet sockets; descendants seen: {rep['descendants_seen']}; downloader sockets: {len(downloader)}")
