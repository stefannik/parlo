# Vendored `pocket-tts` 0.6.2 (babybirdprd/pocket-tts, MIT OR Apache-2.0) — Parlo patches

Upstream targets the older `b6369a24` model. Parlo uses the **pocket-tts 2.1.0 `english` weights** with the ten
precomputed voices from `kyutai/pocket-tts-without-voice-cloning`. Patches (all marked "Parlo patch" in the source):

1. **Offline only.** `weights.rs` rejects `hf://` paths; `hf-hub`, `intel-mkl-src` and `tokenizers/{http,progressbar}` are
   removed from `Cargo.toml` (no network-capable code in the dependency graph).
2. **2.1.0 weights.** `MimiConfig.inner_dim/outer_dim`; `ConvDownsample1d` takes an output dimension (32);
   `speaker_proj_weight` is `[1024, inner_dim]`.
3. **Voice states are KV caches.** New `voice_kv.rs`: loads `*.safetensors` voices (`<module>/cache` `[2,B,T,H,D]` +
   `offset`) into this crate's attention state.
4. **Python-faithful text handling.** `split_chunks_py` ports `split_into_best_sentences` (token-boundary chunking, max
   50 tokens), `prepare_text_prompt_py`, `max_gen_len = ceil((tokens/3 + 2) * 12.5)`, `frames_after_eos = guess + 2`, and
   the frame emitted at `eos + frames_after_eos` is not queued (as in Python). Verified identical chunking on the parity
   test set (incl. the 62-chunk long text).
5. **Pipelined decode.** `generate_latents_segment` + `MimiDecoder` split the flow-LM and Mimi halves so callers can
   overlap them on two threads (the Python reference does the same). RTF on the cloud VM: 1.5 → 1.0.
6. **Config from memory.** `TTSModel::load_from_yaml_str` / `load_from_config_file` take local-path configs.
7. `PROF_*` atomics: coarse profiling counters used by the spike binaries.
