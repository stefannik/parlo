#!/usr/bin/env python3
"""Extract a small, deterministic FLEURS subset (CC-BY-4.0) from the pinned HF parquet files using HTTP range reads
(only the needed row groups are downloaded). Dev/test time only.

Output (fixtures/generated/fleurs/):  <lang>/<id>.wav (16 kHz mono PCM16) and refs.jsonl
  {"id","lang","iso","text"(raw_transcription),"norm"(FLEURS normalized transcription),"duration_s"}
Selection per language: the first N utterances (file order) with 5 s <= duration <= 20 s.
Run via:  uv run --no-project --with pyarrow --with huggingface_hub --with soundfile --with numpy scripts/fleurs_subset.py
"""
import argparse, io, json, sys, tomllib
from pathlib import Path
import numpy as np
import pyarrow.parquet as pq
import soundfile as sf
from huggingface_hub import HfFileSystem

ROOT = Path(__file__).resolve().parent.parent
ISO = {"en_us": "en", "de_de": "de", "fr_fr": "fr", "es_419": "es", "it_it": "it", "bg_bg": "bg", "ru_ru": "ru", "pt_br": "pt"}

def resample16k(x: np.ndarray, sr: int) -> np.ndarray:
    if sr == 16000:
        return x
    import math
    from scipy.signal import resample_poly
    g = math.gcd(16000, sr)
    return resample_poly(x, 16000 // g, sr // g).astype(np.float32)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per-language", type=int)
    ap.add_argument("--out", default=str(ROOT / "fixtures/generated/fleurs"))
    a = ap.parse_args()
    man = tomllib.loads((ROOT / "models/manifest.toml").read_text())["fleurs"]
    n = a.per_language or man["per_language"]
    fs = HfFileSystem()
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    refs_path = out / "refs.jsonl"
    done = {json.loads(l)["id"] for l in refs_path.read_text().splitlines()} if refs_path.exists() else set()
    refs = refs_path.open("a")
    for lang in man["languages"]:
        have = sum(1 for d in done if d.startswith(lang + "/"))
        if have >= n:
            print(f"{lang}: have {have}"); continue
        path = f"datasets/{man['repo']}@{man['revision']}/parquet-data/{lang}/test-00000-of-00001.parquet"
        with fs.open(path, "rb", block_size=8 << 20) as f:
            pf = pq.ParquetFile(f)
            print(f"{lang}: {pf.metadata.num_rows} rows, {pf.num_row_groups} row groups", flush=True)
            got = have
            for rg in range(pf.num_row_groups):
                tbl = pf.read_row_group(rg, columns=["id", "num_samples", "audio", "transcription", "raw_transcription"])
                for r in tbl.to_pylist():
                    dur = r["num_samples"] / 16000
                    if not (5 <= dur <= 20):
                        continue
                    uid = f"{lang}/{r['id']}"
                    if uid in done:
                        continue
                    x, sr = sf.read(io.BytesIO(r["audio"]["bytes"]), dtype="float32")
                    if x.ndim > 1:
                        x = x.mean(axis=1)
                    x = resample16k(x, sr)
                    (out / lang).mkdir(exist_ok=True)
                    sf.write(out / lang / f"{r['id']}.wav", x, 16000, subtype="PCM_16")
                    refs.write(json.dumps(dict(id=uid, lang=lang, iso=ISO[lang], text=r["raw_transcription"], norm=r["transcription"], duration_s=round(len(x) / 16000, 2)), ensure_ascii=False) + "\n"); refs.flush()
                    got += 1
                    if got >= n:
                        break
                if got >= n:
                    break
        print(f"{lang}: wrote {got}", flush=True)

if __name__ == "__main__":
    main()
