//! PocketTTS through the patched Candle port (`crates/pocket-tts`): Kyutai's 2026-09 English model (`english_2026-09`)
//! and the Spanish model, temperature 0.3, sampler steps 1 (Standard) / 4 (High), 50 tokens per internal chunk, 24 kHz mono.
//! Offline by construction: weights/tokenizers/voices are explicit local paths under `models/pockettts/<language>/`
//! (laid out by `scripts/fetch-models.sh`; the vendored crate has no network code).
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

/// Kyutai `pocket-tts-without-voice-cloning` @575d1437: English = `english_2026-09` (not `english`/`english_2026-09_2`,
/// which cut off digit-heavy text — DECISIONS C-020), Spanish = `spanish`.
pub const MODEL_REV: &str = "575d1437";

/// (id, label, language code). English voices come with the English model, Spanish with the Spanish model.
pub const VOICES: [(&str, &str, &str); 22] = [
    ("alba", "Alba", "en"),
    ("anna", "Anna", "en"),
    ("azelma", "Azelma", "en"),
    ("bill_boerst", "Bill Boerst", "en"),
    ("caro_davy", "Caro Davy", "en"),
    ("charles", "Charles", "en"),
    ("cosette", "Cosette", "en"),
    ("eponine", "Eponine", "en"),
    ("eve", "Eve", "en"),
    ("fantine", "Fantine", "en"),
    ("george", "George", "en"),
    ("jane", "Jane", "en"),
    ("javert", "Javert", "en"),
    ("jean", "Jean", "en"),
    ("marius", "Marius", "en"),
    ("mary", "Mary", "en"),
    ("michael", "Michael", "en"),
    ("paul", "Paul", "en"),
    ("peter_yearsley", "Peter Yearsley", "en"),
    ("stuart_bell", "Stuart Bell", "en"),
    ("vera", "Vera", "en"),
    ("lola", "Lola", "es"),
];
/// Kyutai's default for every shipped model ("0.3 beats 0.7 on WER and UTMOS", human evals agreed).
const TEMPERATURE: f32 = 0.3;
const EOS_THRESHOLD: f32 = -4.0;
const MAX_TOKENS_PER_CHUNK: usize = 50;

fn lang_dir(code: &str) -> &'static str {
    if code == "es" {
        "spanish"
    } else {
        "english"
    }
}
fn template(code: &str) -> &'static str {
    if code == "es" {
        include_str!("spanish.yaml.tmpl")
    } else {
        include_str!("english.yaml.tmpl")
    }
}
fn language_of<'a>(available: &'a [VoiceInfo], voice: &str) -> Option<&'a str> {
    available
        .iter()
        .find(|v| v.id == voice)
        .map(|v| v.language.as_str())
}

fn yaml_quote(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\'', "''")
}

fn voices_on_disk(root: &std::path::Path) -> Vec<VoiceInfo> {
    VOICES
        .iter()
        .filter(|(id, _, lang)| {
            root.join(lang_dir(lang))
                .join("embeddings")
                .join(format!("{id}.safetensors"))
                .exists()
        })
        .map(|(id, label, lang)| VoiceInfo {
            id: (*id).into(),
            label: (*label).into(),
            language: (*lang).into(),
        })
        .collect()
}

/// Voices present on disk, without loading the model (the UI lists them at startup; previews play without the engine).
pub fn discover_voices(models: &ModelPaths) -> Vec<VoiceInfo> {
    voices_on_disk(&models.pockettts())
}

pub struct PocketEngine {
    root: PathBuf,
    /// one model per language, loaded on first use (English at start)
    models: HashMap<&'static str, TTSModel>,
    cache: HashMap<String, ModelState>,
    available: Vec<VoiceInfo>,
    revision: String,
}

fn cerr(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("speech engine: {e}"))
}

impl PocketEngine {
    pub fn load(models: &ModelPaths) -> Result<Self> {
        let root = models.pockettts();
        let available = voices_on_disk(&root);
        if !available.iter().any(|v| v.language == "en") {
            return Err(Error::Other(
                "No English voices found. Run scripts/fetch-models.sh --only pockettts.".into(),
            ));
        }
        if available.len() < VOICES.len() {
            tracing::warn!(
                "{} of {} voices are missing on disk and are hidden",
                VOICES.len() - available.len(),
                VOICES.len()
            );
        }
        let mut e = Self {
            root,
            models: HashMap::new(),
            cache: HashMap::new(),
            available,
            revision: format!("{MODEL_REV}+english_2026-09+spanish"),
        };
        e.model_for("en")?; // English now, so a broken install fails at load, not mid-job
        Ok(e)
    }

    /// The model for a language code, loaded on first use. Only one language's model is kept in memory (loading takes
    /// ~60 ms; two resident models would push peak RSS from ~650 MB to ~1.1 GB, over the 800 MB budget).
    fn model_for(&mut self, code: &str) -> Result<&mut TTSModel> {
        let key: &'static str = if code == "es" { "es" } else { "en" };
        if !self.models.contains_key(key) {
            self.models.clear();
            let dir = self.root.join(lang_dir(key));
            let weights = dir.join("model.safetensors");
            let tokenizer = dir.join("tokenizer.model");
            for f in [&weights, &tokenizer] {
                if !f.exists() {
                    return Err(Error::Other(format!("Speech model file not found: {}. Run scripts/fetch-models.sh --only pockettts.", f.display())));
                }
            }
            let yaml = template(key)
                .replace("@@MODEL@@", &yaml_quote(&weights))
                .replace("@@TOKENIZER@@", &yaml_quote(&tokenizer));
            let model = TTSModel::load_from_yaml_str(
                &yaml,
                TEMPERATURE,
                Quality::Standard.lsd_decode_steps(),
                EOS_THRESHOLD,
                None,
                &Self::device(),
            )
            .map_err(cerr)?;
            self.models.insert(key, model);
        }
        Ok(self.models.get_mut(key).expect("just inserted"))
    }

    #[cfg(feature = "tts-metal")]
    fn device() -> Device {
        Device::new_metal(0).unwrap_or(Device::Cpu)
    }
    #[cfg(not(feature = "tts-metal"))]
    fn device() -> Device {
        Device::Cpu
    }

    /// The internal text chunks the engine would synthesize for `text` with `voice` (diagnostics / parity checks).
    pub fn debug_chunks(&mut self, voice: &str, text: &str) -> Result<Vec<String>> {
        let (_, model) = self.voice_state(voice)?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        model
            .split_chunks_py(&text, MAX_TOKENS_PER_CHUNK)
            .map_err(cerr)
    }

    /// Token ids of `text` for the English model (diagnostics / parity checks).
    pub fn debug_tokens(&mut self, text: &str) -> Result<Vec<u32>> {
        let m = self.model_for("en")?;
        m.conditioner.encode_ids(text).map_err(cerr)
    }

    fn voice_state(&mut self, voice: &str) -> Result<(ModelState, TTSModel)> {
        let Some(lang) = language_of(&self.available, voice).map(str::to_string) else {
            return Err(Error::Other(format!(
                "The voice '{voice}' is not available."
            )));
        };
        let path = self
            .root
            .join(lang_dir(&lang))
            .join("embeddings")
            .join(format!("{voice}.safetensors"));
        let model = self.model_for(&lang)?.clone(); // cheap (Arc-backed tensors)
        if !self.cache.contains_key(voice) {
            let st = load_kv_voice_state(path, "flow_lm.", &model.device).map_err(cerr)?;
            self.cache.insert(voice.to_string(), st);
        }
        Ok((self.cache[voice].clone(), model))
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
        24_000
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
        let (state, mut model) = self.voice_state(voice)?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" "); // ctric: whitespace-normalized
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
