#!/usr/bin/env python3
"""Write models/pockettts/english.local.yaml: the pocket-tts 2.1.0 `english` config with local weight/tokenizer paths."""
import re, sys
from pathlib import Path
root = Path(__file__).resolve().parent.parent
snap = root / "models/pockettts/hf/hub/models--kyutai--pocket-tts-without-voice-cloning/snapshots"
src = (root / "docs/reference/ctric/backend/pocket_tts_2.1.0_config_english.yaml").read_text()
m = snap / "d29db7978e464fb90cb3359ee0c69a273b9142cc/languages/english"
src = re.sub(r"^weights_path:.*$", f"weights_path: {m}/model.safetensors", src, flags=re.M)
src = re.sub(r"^weights_path_without_voice_cloning:.*$", f"weights_path_without_voice_cloning: {m}/model.safetensors", src, flags=re.M)
src = re.sub(r"tokenizer_path: hf://.*$", f"tokenizer_path: {m}/tokenizer.model", src, flags=re.M)
out = root / "models/pockettts/english.local.yaml"
out.write_text(src)
print(out)
