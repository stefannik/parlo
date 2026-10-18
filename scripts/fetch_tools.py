#!/usr/bin/env python3
"""Fetch + verify the pinned tools in tools/manifest.toml (build/dev time only; never run by the shipped app).

Idempotent (a stamp file records version/variant/sha), verifies SHA-256 before anything is unpacked or executed.
Usage: fetch-tools.sh [--variant onedir|onefile] [--platform macos|linux-x86_64|linux-aarch64] [--out DIR] [--verify-only]
"""
import argparse
import hashlib
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ROOT / "tools"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for blk in iter(lambda: f.read(1 << 20), b""):
            h.update(blk)
    return h.hexdigest()


def host_platform() -> str:
    m = platform.machine().lower()
    if sys.platform == "darwin":
        return "macos"
    return "linux-aarch64" if m in ("aarch64", "arm64") else "linux-x86_64"


def download(url: str, dest: Path) -> bool:
    for attempt in range(1, 4):
        rc = subprocess.run(["curl", "-fsSL", "--retry", "3", "--retry-delay", "2", "-o", str(dest), url]).returncode
        if rc == 0 and dest.exists():
            return True
        print(f"  attempt {attempt} failed (rc={rc})", file=sys.stderr)
    return False


def unpack_zip(zpath: Path, inner: str, out: Path) -> None:
    """Extract an onedir zip into `out`, renaming the executable `inner` to `yt-dlp` and restoring mode bits."""
    with zipfile.ZipFile(zpath) as z:
        for info in z.infolist():
            name = info.filename
            parts = Path(name).parts
            if name.startswith("/") or ".." in parts:
                raise SystemExit(f"refusing unsafe zip member {name!r}")
            target = out / ("yt-dlp" if name == inner else name)
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            if stat.S_ISLNK(info.external_attr >> 16):
                # zipfile would write the link as a plain file holding its target. The macOS build's
                # Python.framework (Versions/Current, Python → Versions/Current/Python) must keep its links,
                # or codesign rejects the framework as ambiguous.
                link = z.read(info).decode()
                resolved = (target.parent / link).resolve()
                if Path(link).is_absolute() or not resolved.is_relative_to(out.resolve()):
                    raise SystemExit(f"refusing zip symlink {name!r} -> {link!r}")
                target.symlink_to(link)
                continue
            with z.open(info) as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)
            mode = (info.external_attr >> 16) & 0o777
            target.chmod(mode or 0o644)


def relink_frameworks(root: Path) -> None:
    """The official macOS zip stores each *.framework with plain copies where a framework has symlinks
    (Versions/Current, and the top-level binary + Resources). codesign rejects that layout ("bundle format is
    ambiguous"), so rebuild the standard links when the copies are identical. Byte-for-byte the same code runs."""
    for fw in root.rglob("*.framework"):
        versions = fw / "Versions"
        current = versions / "Current"
        if not current.is_dir() or current.is_symlink():
            continue
        real = [v for v in versions.iterdir() if v.name != "Current" and v.is_dir() and not v.is_symlink()]
        if len(real) != 1:
            continue
        ver = real[0]

        def same(a: Path, b: Path) -> bool:
            if a.is_dir() and b.is_dir():
                fa = sorted(p.relative_to(a) for p in a.rglob("*") if p.is_file())
                fb = sorted(p.relative_to(b) for p in b.rglob("*") if p.is_file())
                return fa == fb and all(sha256_file(a / p) == sha256_file(b / p) for p in fa)
            return a.is_file() and b.is_file() and sha256_file(a) == sha256_file(b)

        if not same(current, ver):
            continue
        top = [p for p in fw.iterdir() if p.name != "Versions"]
        if not all(same(p, ver / p.name) for p in top):
            continue
        shutil.rmtree(current)
        current.symlink_to(ver.name)
        for p in top:
            shutil.rmtree(p) if p.is_dir() else p.unlink()
            p.symlink_to(f"Versions/Current/{p.name}")
        print(f"relink  {fw.relative_to(root)} (Versions/Current -> {ver.name})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--variant", default="onedir", choices=["onedir", "onefile"])
    ap.add_argument("--platform", default=host_platform())
    ap.add_argument("--out", default=None, help="destination directory (default: tools/yt-dlp)")
    ap.add_argument("--verify-only", action="store_true")
    a = ap.parse_args()
    man = tomllib.loads((TOOLS / "manifest.toml").read_text())["yt-dlp"]
    pick = [f for f in man["file"] if f["platform"] == a.platform and f["variant"] == a.variant]
    if not pick:
        raise SystemExit(f"no yt-dlp pin for platform={a.platform} variant={a.variant}")
    f = pick[0]
    out = Path(a.out) if a.out else TOOLS / man["dest_dir"]
    stamp = out / ".pinned"
    want = f"{man['version']} {a.variant} {f['sha256']}"
    exe = out / "yt-dlp"
    if exe.exists() and stamp.exists() and stamp.read_text().strip() == want:
        print(f"ok      {exe.relative_to(ROOT) if exe.is_relative_to(ROOT) else exe}  (yt-dlp {man['version']}, {a.variant})")
        return
    if a.verify_only:
        print(f"MISSING/STALE {out}")
        sys.exit(1)
    url = f"{man['base_url']}/{man['version']}/{f['asset']}"
    print(f"fetch   {f['asset']} ({f.get('size', 0)/1e6:.1f} MB) from {url}", flush=True)
    with tempfile.TemporaryDirectory(prefix="parlo-tools-") as td:
        dl = Path(td) / f["asset"]
        if not download(url, dl):
            print("FAILED  download", file=sys.stderr)
            sys.exit(1)
        if "size" in f and dl.stat().st_size != f["size"]:
            print(f"FAILED  size {dl.stat().st_size} != {f['size']}", file=sys.stderr)
            sys.exit(1)
        got = sha256_file(dl)
        if got != f["sha256"]:
            print(f"FAILED  SHA-256 mismatch for {f['asset']}: got {got}, want {f['sha256']}", file=sys.stderr)
            sys.exit(1)
        if out.exists():
            shutil.rmtree(out)
        out.mkdir(parents=True)
        if f["variant"] == "onedir":
            unpack_zip(dl, f["inner"], out)
            relink_frameworks(out)
        else:
            shutil.copyfile(dl, exe)
        exe.chmod(exe.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    stamp.write_text(want + "\n")
    print(f"ok      {exe}  (yt-dlp {man['version']}, {a.variant})")


if __name__ == "__main__":
    main()
