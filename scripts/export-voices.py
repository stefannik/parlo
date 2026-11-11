#!/usr/bin/env python3
"""Make Parlo's own PocketTTS voice states from pinned, permissively licensed recordings (build/dev time only).

Kyutai ships one Spanish voice (Lola). Extra voices are derived here: the pinned source recording is downloaded and
verified, then encoded with Kyutai's *voice-cloning* PocketTTS weights (gated on Hugging Face: accept the terms at
https://huggingface.co/kyutai/pocket-tts and log in with `hf auth login`), and the resulting voice state is written next
to Kyutai's own (`models/pockettts/<lang>/embeddings/<id>.safetensors`). The shipped app never clones: it only reads
the finished state, exactly like the stock voices. See models/manifest.toml [pockettts_voices] and DECISIONS C-028.

Usage (needs the pinned pocket-tts package; uv fetches it into a throwaway environment):
  uv run --python 3.12 --with pocket-tts==3.3.0 scripts/export-voices.py [--only enrique] [--force] [--pin]
--pin writes the size + SHA-256 of the exported files back into the manifest (do it once, then commit).
"""
import argparse
import hashlib
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
MANIFEST = MODELS / "manifest.toml"
POCKET_TTS_VERSION = "3.3.0"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for blk in iter(lambda: f.read(1 << 20), b""):
            h.update(blk)
    return h.hexdigest()


def fetch_source(sec: dict, v: dict) -> Path:
    dest = MODELS / sec["dest_dir"] / "voice-sources" / f"{v['id']}{Path(v['source']).suffix}"
    if dest.exists() and sha256_file(dest) == v["source_sha256"]:
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    url = f"https://huggingface.co/{sec['source_repo']}/resolve/{v['source_revision']}/{v['source']}"
    subprocess.run(["curl", "-fsSL", "--retry", "3", "-o", str(dest), url], check=True)
    got = sha256_file(dest)
    if got != v["source_sha256"]:
        dest.unlink()
        sys.exit(f"SHA-256 mismatch for the {v['id']} recording: got {got}, want {v['source_sha256']}")
    return dest


def pin(entries: dict[str, tuple[int, str]]) -> None:
    """Rewrite `size`/`sha256` of the given voice ids in the manifest, keeping its layout and comments."""
    text = MANIFEST.read_text()
    for vid, (size, sha) in entries.items():
        pat = re.compile(r'(\[\[pockettts_voices\.voice\]\]\nid = "%s"\n(?:(?!\[\[).*\n)*?)size = \d+\nsha256 = "[0-9a-f]*"' % re.escape(vid))
        text, n = pat.subn(lambda m: f'{m.group(1)}size = {size}\nsha256 = "{sha}"', text)
        if n != 1:
            sys.exit(f"could not find the manifest entry for {vid}")
    MANIFEST.write_text(text)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="", help="comma-separated voice ids (default: all)")
    ap.add_argument("--force", action="store_true", help="re-export even if the file matches the manifest")
    ap.add_argument("--pin", action="store_true", help="write size + SHA-256 of the results into the manifest")
    a = ap.parse_args()

    from importlib.metadata import version

    if version("pocket-tts") != POCKET_TTS_VERSION:  # its config pins the gated weights' revision
        sys.exit(f"pocket-tts {POCKET_TTS_VERSION} required, found {version('pocket-tts')}")
    from pocket_tts import TTSModel  # imported late so --help works without the package
    from pocket_tts.models.model_state import export_model_state

    man = tomllib.loads(MANIFEST.read_text())
    sec = man["pockettts_voices"]
    only = {x for x in a.only.split(",") if x}
    voices = [v for v in sec["voice"] if not only or v["id"] in only]
    models: dict[str, TTSModel] = {}
    pinned: dict[str, tuple[int, str]] = {}
    for v in voices:
        dest = MODELS / sec["dest_dir"] / v["dest"]
        if not a.force and v.get("sha256") and dest.exists() and sha256_file(dest) == v["sha256"]:
            print(f"ok      {dest.relative_to(MODELS)}")
            continue
        src = fetch_source(sec, v)
        lang = v["language"]
        if lang not in models:
            models[lang] = TTSModel.load_model(language=lang)
            if not models[lang].has_voice_cloning:
                sys.exit(
                    "The voice-cloning weights are not accessible. Accept the terms at "
                    "https://huggingface.co/kyutai/pocket-tts and run `hf auth login` (a read token is enough)."
                )
        state = models[lang].get_state_for_audio_prompt(audio_conditioning=src, truncate=True)
        dest.parent.mkdir(parents=True, exist_ok=True)
        export_model_state(state, dest)
        size, sha = dest.stat().st_size, sha256_file(dest)
        pinned[v["id"]] = (size, sha)
        same = "matches the manifest" if sha == v.get("sha256") else "NEW (differs from the manifest)"
        print(f"export  {dest.relative_to(MODELS)}  {size} B  sha256 {sha}  {same}")
    if a.pin and pinned:
        pin(pinned)
        print(f"pinned {', '.join(pinned)} in {MANIFEST.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
