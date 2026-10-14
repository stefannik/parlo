//! Parlo patch: load pocket-tts 2.1.0 precomputed voice states.
//!
//! The `*.safetensors` voice files shipped with `kyutai/pocket-tts-without-voice-cloning` are
//! serialized *model states* (not audio-prompt latents): per attention layer a KV `cache` of shape
//! `[2, B, T, H, D]` plus an `offset`, keyed `"<module>/cache"` and `"<module>/offset"`.
//! This converts them into this crate's attention state (`k_buf`/`v_buf` as `[B, H, T, D]` + cursor).

use crate::ModelState;
use crate::voice_state::{AttentionCursor, write_attention_cursor};
use anyhow::{Context, Result, bail};
use candle_core::{Device, Tensor};
use std::collections::HashMap;
use std::path::Path;

pub fn load_kv_voice_state<P: AsRef<Path>>(
    path: P,
    module_prefix: &str,
    device: &Device,
) -> Result<ModelState> {
    let tensors = candle_core::safetensors::load(path.as_ref(), device)
        .with_context(|| format!("reading voice file {}", path.as_ref().display()))?;
    let mut state: ModelState = HashMap::new();
    for (key, t) in &tensors {
        let Some(module) = key.strip_suffix("/cache") else {
            continue;
        };
        let (two, b, len, _h, _d) = t.dims5()?;
        if two != 2 || b != 1 {
            bail!("unexpected KV cache shape {:?} for {key}", t.dims());
        }
        let offset_key = format!("{module}/offset");
        let offset = tensors
            .get(&offset_key)
            .with_context(|| format!("missing {offset_key}"))?
            .flatten_all()?
            .to_vec1::<i64>()?[0] as usize;
        // [B, T, H, D] -> [B, H, T, D]
        let k = t.get(0)?.transpose(1, 2)?.contiguous()?;
        let v = t.get(1)?.transpose(1, 2)?.contiguous()?;
        // Only `offset` positions are valid (the rest may be NaN padding).
        let valid = offset.min(len);
        let k = k.narrow(2, 0, valid)?.contiguous()?;
        let v = v.narrow(2, 0, valid)?.contiguous()?;
        let mut m = HashMap::new();
        m.insert(crate::voice_state::ATTN_K_BUF_KEY.to_string(), k);
        m.insert(crate::voice_state::ATTN_V_BUF_KEY.to_string(), v);
        write_attention_cursor(
            &mut m,
            AttentionCursor {
                pos: offset,
                len: valid,
                head: 0,
            },
            device,
        )?;
        state.insert(format!("{module_prefix}{module}"), m);
    }
    if state.is_empty() {
        bail!("no KV-cache entries found in voice file");
    }
    Ok(state)
}
