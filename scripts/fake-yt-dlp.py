#!/usr/bin/env python3
"""A fake `yt-dlp` for the "Transcribe from URL" tests (no network, no yt-dlp). It speaks yt-dlp's real output formats
(`-J` JSON, `--progress-template` lines, `[download] Sleeping …`, `ERROR: …` on stderr) and writes a fixture audio file.

Copy this script next to a few flag files (all optional) and call it like yt-dlp:
  <dir>/yt-dlp               the script
  <dir>/audio                path of the audio file to "download"   (default: fixtures/generated/url/episode.m4a)
  <dir>/version              version string for --version          (default 2026.08.19)
  <dir>/no_impersonate       --list-impersonate-targets shows Chrome as (unavailable)
  <dir>/always_extractor_fail  every URL fails with an extractor error (to drive the system-yt-dlp fallback)
  <dir>/argv.log             every invocation's argv, one JSON list per line (written by the script)

The *mode* is the first path segment of the URL: https://fake.example/<mode>/…
  success | progress (same) | sleep (rate-limit sleep line first) | slow (progress takes ~1.5 s) | live | private | age |
  drm | notfound | network | noaudio | playlist | extractor | diskfull | hang (download never ends; leaves a grandchild)
"""
import json
import os
import re
import shutil
import subprocess
import sys
import time
from urllib.parse import urlparse

HERE = os.path.dirname(os.path.abspath(__file__))


def flag(name):
    return os.path.exists(os.path.join(HERE, name))


def read(name, default):
    p = os.path.join(HERE, name)
    return open(p).read().strip() if os.path.exists(p) else default


def err(msg, code=1):
    sys.stderr.write(msg + "\n")
    sys.stderr.flush()
    sys.exit(code)


with open(os.path.join(HERE, "argv.log"), "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")

argv = sys.argv[1:]
if "--version" in argv:
    print(read("version", "2026.08.19"))
    sys.exit(0)
if "--list-impersonate-targets" in argv:
    print("[info] Available impersonate targets")
    print("Client          OS           Source")
    print("--------------------------------------")
    if flag("no_impersonate"):
        print("Chrome          -            curl_cffi (unavailable)")
        print("Safari          -            curl_cffi (unavailable)")
    else:
        print("Chrome-133      Macos-15     curl_cffi")
        print("Safari-18.0     Ios-18.0     curl_cffi")
    sys.exit(0)

url = argv[argv.index("--") + 1] if "--" in argv else argv[-1]
path = urlparse(url).path.strip("/").split("/")
mode = path[0] if path and path[0] else "success"
vid = "fakevid"


def arg(name):
    return argv[argv.index(name) + 1] if name in argv else None


if flag("always_extractor_fail"):
    err(f"ERROR: [youtube] {vid}: Unable to extract player response; please report this issue on  https://github.com/yt-dlp/yt-dlp/issues")

GOOD = {
    "id": vid,
    "title": "Fake Episode: Local first",
    "duration": 22.0,
    "uploader": "Fake Show",
    "is_live": False,
    "live_status": "not_live",
    "formats": [
        {"format_id": "140", "ext": "m4a", "acodec": "mp4a.40.2", "vcodec": "none", "filesize_approx": 174309},
        {"format_id": "18", "ext": "mp4", "acodec": "mp4a.40.2", "vcodec": "avc1"},
    ],
}

if "-J" in argv:
    if mode == "live":
        print(json.dumps({**GOOD, "is_live": True, "live_status": "is_live"}))
    elif mode == "private":
        err(f"ERROR: [youtube] {vid}: Private video. Sign in if you've been granted access to this video")
    elif mode == "age":
        err(f"ERROR: [youtube] {vid}: Sign in to confirm your age. This video may be inappropriate for some users.")
    elif mode == "drm":
        err("ERROR: [DRM] The requested site is known to use DRM protection. It will NOT be supported.")
    elif mode == "notfound":
        err(f"ERROR: [generic] {vid}: Unable to download webpage: HTTP Error 404: Not Found (caused by <HTTPError 404: Not Found>)")
    elif mode == "network":
        err(f"ERROR: [generic] {vid}: Unable to download webpage: Failed to establish a new connection: [Errno 111] Connection refused")
    elif mode == "noaudio":
        print(json.dumps({**GOOD, "formats": [{"format_id": "18", "ext": "mp4", "acodec": "none", "vcodec": "avc1"}]}))
    elif mode == "playlist":
        print(json.dumps({"_type": "playlist", "title": "A playlist", "entries": [GOOD, {**GOOD, "title": "Second"}]}))
    elif mode == "extractor":
        err(f"ERROR: [youtube] {vid}: Unable to extract player response; please report this issue")
    else:
        print(json.dumps(GOOD))
    sys.exit(0)

# ------------------------------------------------------------------ download
if mode == "diskfull":
    err("ERROR: unable to write data: [Errno 28] No space left on device")
if mode == "extractor":
    err(f"ERROR: [youtube] {vid}: Unable to extract player response; please report this issue")
out_tpl = arg("-o") or "media.%(ext)s"
stage = os.path.dirname(out_tpl) or "."
final = out_tpl.replace("%(ext)s", "m4a")
audio = read("audio", os.path.join(HERE, "..", "..", "..", "fixtures", "generated", "url", "episode.m4a"))
size = os.path.getsize(audio)
tpl = (arg("--progress-template") or "download:%(progress.downloaded_bytes)s").split(":", 1)[1]

print(f"[generic] Extracting URL: {url}", flush=True)
print(f"[info] {vid}: Downloading 1 format(s): 140", flush=True)
if arg("--sleep-interval") is not None:
    print("[download] Sleeping 7.00 seconds ..." if mode == "sleep" else "[download] Sleeping 0.30 seconds ...", flush=True)
    time.sleep(0.3)
print(f"[download] Destination: {final}", flush=True)


def line(status, got, total, speed, eta):
    vals = {"status": status, "downloaded_bytes": got, "total_bytes": total, "total_bytes_estimate": "NA", "speed": speed, "eta": eta}
    return re.sub(r"%\(progress\.(\w+)\)s", lambda m: str(vals[m.group(1)]), tpl)


if mode == "hang":
    child = subprocess.Popen(["sleep", "300"])
    open(os.path.join(HERE, "child.pid"), "w").write(str(child.pid))
    print(line("downloading", 1024, size, 5000.0, 10), flush=True)
    time.sleep(300)
    sys.exit(0)

steps = 12 if mode == "slow" else 4
part = final + ".part"
with open(part, "wb") as fh:
    for i in range(1, steps + 1):
        got = size * i // steps
        fh.write(b"\0" * (got - fh.tell()))
        fh.flush()
        print(line("downloading", got, size, 60000.0, steps - i), flush=True)
        time.sleep(0.13 if mode == "slow" else 0.12)
shutil.copyfile(audio, part)
os.rename(part, final)  # yt-dlp renames .part → final when complete
print(line("finished", size, size, 60000.0, "NA"), flush=True)
sys.exit(0)
