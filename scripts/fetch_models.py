#!/usr/bin/env python3
"""Fetch + verify models listed in models/manifest.toml (build/dev time only; never run by the shipped app).

Idempotent (skips verified files), resumable (curl -C -), verifies size + SHA-256.
Usage: fetch-models.sh [--only pockettts,whisper,vad,eval] [--match SUBSTR] [--verify-only] [--list]
"""
import argparse
import hashlib
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
HF = "https://huggingface.co"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for blk in iter(lambda: f.read(1 << 20), b""):
            h.update(blk)
    return h.hexdigest()


def entries(man, groups):
    """Yield (group, url, dest_path, size, sha256)."""
    for g in groups:
        sec = man[g]
        for f in sec.get("file", []):
            rev = f.get("revision", sec.get("revision"))
            url = f"{HF}/{sec['repo']}/resolve/{rev}/{f['path']}"
            if g == "pockettts":
                base = MODELS / sec["hf_cache_dir"] / "snapshots" / rev
                dest = base / f["path"]
            else:
                dest = MODELS / sec["dest_dir"] / Path(f["path"]).name
            yield g, url, dest, f["size"], f["sha256"]


def verified(dest: Path, size: int, sha: str) -> bool:
    return dest.exists() and dest.stat().st_size == size and sha256_file(dest) == sha


def fetch(url: str, dest: Path, size: int, sha: str) -> bool:
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    for attempt in range(1, 6):
        cmd = ["curl", "-fL", "--retry", "3", "--retry-delay", "2", "-C", "-", "-o", str(part), url]
        if sys.stderr.isatty():
            cmd.insert(1, "--progress-bar")
        else:
            cmd.insert(1, "-sS")
        rc = subprocess.run(cmd).returncode
        if part.exists() and part.stat().st_size > size:  # corrupted resume
            part.unlink()
            continue
        if rc == 0 and part.exists() and part.stat().st_size == size:
            break
        print(f"  attempt {attempt} failed (rc={rc})", file=sys.stderr)
    else:
        return False
    got = sha256_file(part)
    if got != sha:
        print(f"  SHA-256 MISMATCH for {dest.name}: got {got}, want {sha}", file=sys.stderr)
        part.unlink()
        return False
    part.replace(dest)
    return True


def write_refs(man):
    sec = man["pockettts"]
    refs = MODELS / sec["hf_cache_dir"] / "refs"
    refs.mkdir(parents=True, exist_ok=True)
    (refs / "main").write_text(sec["refs_main"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="pockettts,whisper,vad")
    ap.add_argument("--match", default="", help="only files whose name contains this substring")
    ap.add_argument("--verify-only", action="store_true")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()
    man = tomllib.loads((MODELS / "manifest.toml").read_text())
    groups = [g for g in a.only.split(",") if g]
    for g in groups:
        if g == "fixtures":
            print("note: 'fixtures' are produced by scripts/gen-fixtures.sh, skipping", file=sys.stderr)
    groups = [g for g in groups if g in man and "file" in man[g]]
    bad = 0
    for g, url, dest, size, sha in entries(man, groups):
        if a.match and a.match not in dest.name:
            continue
        rel = dest.relative_to(MODELS)
        if a.list:
            print(f"{g:10s} {size:>12d} {rel}")
            continue
        if verified(dest, size, sha):
            print(f"ok      {rel}")
            continue
        if a.verify_only:
            print(f"MISSING/BAD {rel}")
            bad += 1
            continue
        print(f"fetch   {rel} ({size/1e6:.1f} MB)", flush=True)
        if fetch(url, dest, size, sha):
            print(f"ok      {rel}")
        else:
            print(f"FAILED  {rel}")
            bad += 1
    if "pockettts" in groups and not a.list:
        write_refs(man)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
