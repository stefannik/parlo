//! PocketTTS through the patched Candle port (`crates/pocket-tts`), BRIEF §3.2 parameters:
//! English, temperature 0.7, `lsd_decode_steps` 4 (Standard) / 1 (Fast), 50 tokens per internal chunk, 24 kHz mono.
//! Offline by construction: weights/tokenizer/voices are explicit local paths from the HF-cache layout that
//! `scripts/fetch-models.sh` creates (the vendored crate has no network code).
//!
//! Flow-LM and the Mimi decoder run on two threads (as the Python reference does) so decode overlaps generation.

use super::engine::{Quality, TtsEngine, VoiceInfo};
use crate::error::{Error, Result};
use crate::paths::ModelPaths;
use candle_core::{Device, Tensor};
use pocket_tts::voice_kv::load_kv_voice_state;
use pocket_tts::{ModelState, TTSModel};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;

pub const WEIGHTS_REV: &str = "d29db7978e464fb90cb3359ee0c69a273b9142cc";
pub const VOICES_REV: &str = "e041936c75475d350b405bc870bcf7c22da4e9e6";
pub const VOICE_IDS: [&str; 10] = [
    "alba", "anna", "charles", "cosette", "george", "jane", "mary", "michael", "paul", "vera",
];
const TEMPERATURE: f32 = 0.7;
const EOS_THRESHOLD: f32 = -4.0;
const MAX_TOKENS_PER_CHUNK: usize = 50;
const TEMPLATE: &str = include_str!("english.yaml.tmpl");

fn yaml_quote(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\'', "''")
}

/// Voices present on disk, without loading the model (the UI lists them at startup; previews play without the engine).
pub fn discover_voices(models: &ModelPaths) -> Vec<VoiceInfo> {
    let dir = models
        .pockettts()
        .join("hf/hub/models--kyutai--pocket-tts-without-voice-cloning/snapshots")
        .join(VOICES_REV)
        .join("languages/english/embeddings");
    VOICE_IDS
        .iter()
        .filter(|v| dir.join(format!("{v}.safetensors")).exists())
        .map(|v| VoiceInfo {
            id: (*v).into(),
            label: v[..1].to_uppercase() + &v[1..],
        })
        .collect()
}

pub struct PocketEngine {
    model: TTSModel,
    voices_dir: PathBuf,
    cache: HashMap<String, ModelState>,
    available: Vec<VoiceInfo>,
    revision: String,
}

fn cerr(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("speech engine: {e}"))
}

impl PocketEngine {
    pub fn load(models: &ModelPaths) -> Result<Self> {
        let snap = models
            .pockettts()
            .join("hf/hub/models--kyutai--pocket-tts-without-voice-cloning/snapshots");
        let w = snap.join(WEIGHTS_REV).join("languages/english");
        let weights = w.join("model.safetensors");
        let tokenizer = w.join("tokenizer.model");
        let voices_dir = snap.join(VOICES_REV).join("languages/english/embeddings");
        for f in [&weights, &tokenizer] {
            if !f.exists() {
                return Err(Error::Other(format!("Speech model file not found: {}. Run scripts/fetch-models.sh --only pockettts.", f.display())));
            }
        }
        let available: Vec<VoiceInfo> = VOICE_IDS
            .iter()
            .filter(|v| voices_dir.join(format!("{v}.safetensors")).exists())
            .map(|v| VoiceInfo {
                id: (*v).into(),
                label: v[..1].to_uppercase() + &v[1..],
            })
            .collect();
        if available.len() < VOICE_IDS.len() {
            tracing::warn!(
                "{} of {} voices are missing on disk and are hidden",
                VOICE_IDS.len() - available.len(),
                VOICE_IDS.len()
            );
        }
        if available.is_empty() {
            return Err(Error::Other(
                "No voices found. Run scripts/fetch-models.sh --only pockettts.".into(),
            ));
        }
        let yaml = TEMPLATE
            .replace("@@MODEL@@", &yaml_quote(&weights))
            .replace("@@TOKENIZER@@", &yaml_quote(&tokenizer));
        let device = Self::device();
        let model = TTSModel::load_from_yaml_str(
            &yaml,
            TEMPERATURE,
            Quality::Standard.lsd_decode_steps(),
            EOS_THRESHOLD,
            None,
            &device,
        )
        .map_err(cerr)?;
        Ok(Self {
            model,
            voices_dir,
            cache: HashMap::new(),
            available,
            revision: format!("{}+{}", &WEIGHTS_REV[..8], &VOICES_REV[..8]),
        })
    }

    #[cfg(feature = "tts-metal")]
    fn device() -> Device {
        Device::new_metal(0).unwrap_or(Device::Cpu)
    }
    #[cfg(not(feature = "tts-metal"))]
    fn device() -> Device {
        Device::Cpu
    }

    fn voice_state(&mut self, voice: &str) -> Result<ModelState> {
        if !self.cache.contains_key(voice) {
            if !self.available.iter().any(|v| v.id == voice) {
                return Err(Error::Other(format!(
                    "The voice '{voice}' is not available."
                )));
            }
            let st = load_kv_voice_state(
                self.voices_dir.join(format!("{voice}.safetensors")),
                "flow_lm.",
                &self.model.device,
            )
            .map_err(cerr)?;
            self.cache.insert(voice.to_string(), st);
        }
        Ok(self.cache[voice].clone())
    }
}

impl TtsEngine for PocketEngine {
    fn id(&self) -> &str {
        "pockettts-candle"
    }
    fn model_revision(&self) -> &str {
        &self.revision
    }
    fn sample_rate(&self) -> u32 {
        self.model.sample_rate as u32
    }
    fn voices(&self) -> Vec<VoiceInfo> {
        self.available.clone()
    }

    fn synthesize(
        &mut self,
        text: &str,
        voice: &str,
        quality: Quality,
        sink: &mut dyn FnMut(&[f32]) -> Result<()>,
        abort: &AtomicBool,
    ) -> Result<()> {
        let state = self.voice_state(voice)?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" "); // ctric: whitespace-normalized
        let mut model = self.model.clone(); // cheap (Arc-backed tensors)
        model.lsd_decode_steps = quality.lsd_decode_steps();
        let chunks = model
            .split_chunks_py(&text, MAX_TOKENS_PER_CHUNK)
            .map_err(cerr)?;
        for chunk in chunks {
            if abort.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let (tx, rx) = sync_channel::<std::result::Result<Tensor, String>>(16);
            let result: Result<()> = std::thread::scope(|s| {
                let m = &model;
                let st = &state;
                s.spawn(move || {
                    for q in m.generate_latents_segment(chunk, st) {
                        if abort.load(Ordering::Relaxed)
                            || tx.send(q.map_err(|e| e.to_string())).is_err()
                        {
                            return;
                        }
                    }
                });
                let mut dec = model.new_mimi_decoder();
                let mut buf: Vec<f32> = Vec::new();
                for q in rx {
                    if abort.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    let q = q.map_err(cerr)?;
                    let audio = dec.decode(&q).map_err(cerr)?;
                    buf.clear();
                    buf.extend(
                        audio
                            .flatten_all()
                            .and_then(|t| t.to_vec1::<f32>())
                            .map_err(cerr)?,
                    );
                    sink(&buf)?;
                }
                if abort.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                Ok(())
            });
            result?;
        }
        Ok(())
    }
}
