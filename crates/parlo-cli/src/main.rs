//! parlo-cli: headless access to parlo-core (tests, benchmarks, verification).
use clap::{Parser, Subcommand};
use parlo_core::audio::{decode, prepare};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[derive(Parser)]
#[command(
    name = "parlo-cli",
    version,
    about = "Headless Parlo: decode, record, transcribe, speak"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Decode any supported audio/video file to 16 kHz mono PCM16 WAV (what the STT pipeline consumes)
    Decode { input: PathBuf, output: PathBuf },
    /// Record from the microphone (or a fake source for tests) into DIR; Ctrl-C / SIGTERM stops gracefully.
    Record {
        #[arg(long)]
        dir: PathBuf,
        /// mic | counter | sine | wav:<path>
        #[arg(long, default_value = "mic")]
        source: String,
        #[arg(long, default_value_t = 48_000)]
        rate: u32,
        /// accelerate fake sources (no wall-clock pacing)
        #[arg(long)]
        accelerated: bool,
        /// stop automatically after this many seconds of wall time (default: run until interrupted)
        #[arg(long)]
        seconds: Option<f64>,
        /// pin an input device id (default: follow the system default)
        #[arg(long)]
        device: Option<String>,
    },
    /// Transcribe an audio/video file (VAD → chunks → whisper → paragraphs), checkpointing every chunk
    Transcribe {
        input: PathBuf,
        /// whisper model file name under models/whisper/ (or a path)
        #[arg(long, default_value = "ggml-large-v3-turbo-q8_0.bin")]
        model: String,
        /// "auto" or an ISO-639-1 code
        #[arg(long, default_value = "auto")]
        language: String,
        #[arg(long, default_value_t = 4)]
        threads: usize,
        /// job directory (checkpoints); default: a temp dir under the cache
        #[arg(long)]
        job_dir: Option<PathBuf>,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// print the result as JSON
        #[arg(long)]
        json: bool,
        /// beam size (1 = greedy)
        #[arg(long, default_value_t = 1)]
        beam: i32,
        /// carry the previous chunk's tail as decoder context
        #[arg(long)]
        carry_prompt: bool,
    },
    /// Detect the spoken language of a file (VAD → first ≤ 3 chunks → vote), like the transcription pipeline does
    Lang {
        input: PathBuf,
        #[arg(long, default_value = "ggml-large-v3-turbo-q8_0.bin")]
        model: String,
        #[arg(long, default_value_t = 4)]
        threads: usize,
    },
    /// Speak text (or a document) with PocketTTS: long-text pipeline with segments, silences and cache
    Speak {
        /// text to read (omit with --file / --previews)
        text: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, default_value = "george")]
        voice: String,
        /// standard (lsd 4) | fast (lsd 1)
        #[arg(long, default_value = "standard")]
        quality: String,
        /// output file (.wav or .mp3); default: ./speak.wav
        #[arg(long)]
        out: Option<PathBuf>,
        /// generate the bundled ~3 s voice previews (resources/voice-previews/<voice>.mp3) instead
        #[arg(long)]
        previews: bool,
        #[arg(long)]
        job_dir: Option<PathBuf>,
        #[arg(long)]
        outputs_dir: Option<PathBuf>,
    },
    /// Benchmarks: `bench tts` (speed/TTFA/RSS), `bench tts --parity` (full reference comparison), `bench stt <file>`
    Bench {
        what: String,
        /// for `bench stt`: the audio file
        input: Option<PathBuf>,
        #[arg(long)]
        parity: bool,
        #[arg(long, default_value_t = 3)]
        repeat: usize,
        #[arg(long, default_value = "ggml-large-v3-turbo-q8_0.bin")]
        model: String,
        #[arg(long, default_value_t = 4)]
        threads: usize,
    },
    /// Repair interrupted recordings in DIR (header repair from file length) and list recoverable ones
    Recover { dir: PathBuf },
    /// List audio input devices
    Devices,
    /// Print stream info (codec, rate, channels, duration) without decoding
    Info { input: PathBuf },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> parlo_core::Result<()> {
    match cli.cmd {
        Cmd::Decode { input, output } => {
            let cancel = AtomicBool::new(false);
            let t = std::time::Instant::now();
            let p = prepare::prepare_audio(&input, &output, &cancel, |_| {})?;
            println!(
                "{} -> {} : {:.3} s, {} frames @ {} Hz ({:?}; {} Hz src, {} ch) in {:.2}s",
                input.display(),
                output.display(),
                p.duration_s,
                p.frames,
                prepare::STT_RATE,
                p.source.codec,
                p.source.sample_rate,
                p.source.channels,
                t.elapsed().as_secs_f64()
            );
        }
        Cmd::Transcribe {
            input,
            model,
            language,
            threads,
            job_dir,
            cache_dir,
            json,
            beam,
            carry_prompt,
        } => transcribe(
            input,
            model,
            language,
            threads,
            job_dir,
            cache_dir,
            json,
            beam,
            carry_prompt,
        )?,
        Cmd::Lang {
            input,
            model,
            threads,
        } => lang(input, model, threads)?,
        Cmd::Speak {
            text,
            file,
            voice,
            quality,
            out,
            previews,
            job_dir,
            outputs_dir,
        } => speak(
            text,
            file,
            voice,
            quality,
            out,
            previews,
            job_dir,
            outputs_dir,
        )?,
        Cmd::Bench {
            what,
            input,
            parity,
            repeat,
            model,
            threads,
        } => bench(what, input, parity, repeat, model, threads)?,
        Cmd::Devices => {
            let input = parlo_core::platform::cpal_input::CpalInput::new();
            for d in parlo_core::platform::AudioInput::devices(&input)? {
                println!(
                    "{}{}\t{}",
                    if d.is_default { "* " } else { "  " },
                    d.id,
                    d.name
                );
            }
        }
        Cmd::Recover { dir } => {
            let rec = parlo_core::recorder::recovery::recover_all(&dir)?;
            for r in &rec {
                println!(
                    "{}",
                    serde_json::json!({"id": r.id, "wav": r.wav, "frames": r.sidecar.frames_written, "duration_ms": r.sidecar.duration_ms, "sample_rate": r.sidecar.sample_rate})
                );
            }
            eprintln!("{} recoverable recording(s)", rec.len());
        }
        Cmd::Record {
            dir,
            source,
            rate,
            accelerated,
            seconds,
            device,
        } => record(dir, source, rate, accelerated, seconds, device)?,
        Cmd::Info { input } => {
            let i = decode::probe_info(&input)?;
            println!(
                "{}",
                serde_json::json!({"codec": i.codec, "sample_rate": i.sample_rate, "channels": i.channels, "duration_s": i.duration_s})
            );
        }
    }
    Ok(())
}

fn record(
    dir: PathBuf,
    source: String,
    rate: u32,
    accelerated: bool,
    seconds: Option<f64>,
    device: Option<String>,
) -> parlo_core::Result<()> {
    use parlo_core::platform::file_source::{
        CounterSignal, FileSource, Mode, SineSignal, WavSignal,
    };
    use parlo_core::platform::*;
    use parlo_core::recorder::{start, RecorderConfig, RecorderDeps};
    use std::sync::Arc;
    let mode = if accelerated {
        Mode::Accelerated
    } else {
        Mode::Realtime
    };
    let input: Arc<dyn AudioInput> = match source.as_str() {
        "mic" => Arc::new(cpal_input::CpalInput::new()),
        "counter" => Arc::new(FileSource::new(Box::new(CounterSignal), rate, 1, mode)),
        "sine" => Arc::new(FileSource::new(
            Box::new(SineSignal {
                freq: 440.0,
                amp: 0.3,
                rate: rate as f32,
            }),
            rate,
            1,
            mode,
        )),
        s if s.starts_with("wav:") => Arc::new(FileSource::new(
            Box::new(WavSignal::open(std::path::Path::new(&s[4..]))?),
            rate,
            1,
            mode,
        )),
        other => {
            return Err(parlo_core::Error::Other(format!(
                "unknown source '{other}'"
            )))
        }
    };
    let mut cfg = RecorderConfig::new(&dir);
    if let Some(d) = device {
        cfg.device = DeviceSelector::Named(d);
    }
    let deps = RecorderDeps {
        input,
        power: Arc::new(NoopPowerGuard),
        sleep_wake: Arc::new(ManualSleepWake::new()),
        disk: Arc::new(SystemDisk),
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let h = start(cfg, deps, tx)?;
    println!("{}", serde_json::json!({"recording": h.id()}));
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s2 = stop.clone();
    ctrlc::set_handler(move || s2.store(true, std::sync::atomic::Ordering::SeqCst))
        .map_err(|e| parlo_core::Error::Other(e.to_string()))?;
    let t = std::time::Instant::now();
    while !stop.load(std::sync::atomic::Ordering::SeqCst)
        && seconds.is_none_or(|s| t.elapsed().as_secs_f64() < s)
    {
        std::thread::sleep(std::time::Duration::from_millis(100));
        for w in rx.try_iter() {
            eprintln!("warning: {w:?}");
        }
        let snap = h.snapshot();
        if snap.state == parlo_core::recorder::RecorderState::Stopped {
            break;
        }
    }
    let info = h.stop()?;
    println!("{}", serde_json::to_string(&info.sidecar).unwrap());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn transcribe(
    input: PathBuf,
    model: String,
    language: String,
    threads: usize,
    job_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    json: bool,
    beam: i32,
    carry_prompt: bool,
) -> parlo_core::Result<()> {
    use parlo_core::paths::{AppPaths, ModelPaths};
    use parlo_core::stt::pipeline::{run_transcription, LanguageSetting, SttEvent, SttJob};
    use parlo_core::stt::whisper::{SileroVad, WhisperConfig, WhisperEngine};
    use std::sync::Arc;
    let mp = ModelPaths::discover();
    let model_path = if std::path::Path::new(&model).exists() {
        PathBuf::from(&model)
    } else {
        mp.whisper(&model)
    };
    let app = AppPaths::discover();
    let mut job = SttJob::new(
        &input,
        job_dir.unwrap_or_else(|| app.jobs().join(format!("cli-{:x}", std::process::id()))),
        cache_dir.unwrap_or_else(|| app.decode_cache()),
    );
    job.threads = threads;
    job.carry_prompt = carry_prompt;
    job.language = if language == "auto" {
        LanguageSetting::Auto
    } else {
        LanguageSetting::Code(language)
    };
    let t0 = std::time::Instant::now();
    let mut engine = WhisperEngine::load(
        &model_path,
        WhisperConfig {
            beam_size: beam,
            ..Default::default()
        },
    )?;
    let load_s = t0.elapsed().as_secs_f64();
    let mut vad = SileroVad::load(&mp.vad(), 300)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    ctrlc::set_handler(move || c2.store(true, std::sync::atomic::Ordering::SeqCst))
        .map_err(|e| parlo_core::Error::Other(e.to_string()))?;
    let mut last = std::time::Instant::now();
    let t = run_transcription(
        &job,
        &mut engine,
        &mut vad,
        &mut |e: SttEvent| match e {
            SttEvent::Stage(s) => eprintln!("[stage] {s:?}"),
            SttEvent::Language(l) => eprintln!("[language] {} ({:.0}%)", l.code, l.prob * 100.0),
            SttEvent::Progress {
                done_s,
                total_s,
                eta_ms,
            } if last.elapsed().as_secs_f64() > 2.0 => {
                last = std::time::Instant::now();
                eprintln!(
                    "[progress] {:.0}/{:.0}s ETA {}",
                    done_s,
                    total_s,
                    eta_ms
                        .map(|m| format!("{}s", m / 1000))
                        .unwrap_or_else(|| "?".into())
                );
            }
            _ => {}
        },
        &cancel,
    )?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "text": t.text, "language": t.language.as_ref().map(|l| &l.code), "language_prob": t.language.as_ref().map(|l| l.prob),
                "duration_s": t.duration_s, "speech_s": t.speech_s, "words": t.word_count, "processing_s": t.processing_s, "model_load_s": load_s,
                "rtf": t.processing_s / t.duration_s.max(1e-9), "chunks": t.chunks_total, "paragraphs": t.paragraphs.len(),
                "segments": t.segments.iter().map(|s| serde_json::json!([s.start_s, s.end_s, s.text])).collect::<Vec<_>>(),
            })
        );
    } else {
        println!("{}", t.text);
        eprintln!("[done] {:.1}s audio, {} words, {} paragraphs, language {:?}, processed in {:.1}s (RTF {:.2}), model load {:.1}s", t.duration_s, t.word_count, t.paragraphs.len(), t.language.map(|l| l.code), t.processing_s, t.processing_s / t.duration_s.max(1e-9), load_s);
    }
    Ok(())
}

fn lang(input: PathBuf, model: String, threads: usize) -> parlo_core::Result<()> {
    use parlo_core::audio::wav::PcmFile;
    use parlo_core::paths::{AppPaths, ModelPaths};
    use parlo_core::stt::chunker::build_chunks;
    use parlo_core::stt::engine::SttEngine;
    use parlo_core::stt::filters::{analyze, should_transcribe};
    use parlo_core::stt::vad::detect_file;
    use parlo_core::stt::whisper::{SileroVad, WhisperConfig, WhisperEngine};
    let mp = ModelPaths::discover();
    let model_path = if std::path::Path::new(&model).exists() {
        PathBuf::from(&model)
    } else {
        mp.whisper(&model)
    };
    let app = AppPaths::discover();
    let cancel = AtomicBool::new(false);
    let pcm_path = app
        .decode_cache()
        .join(format!("lang-{:x}.wav", std::process::id()));
    std::fs::create_dir_all(app.decode_cache())?;
    prepare::prepare_audio(&input, &pcm_path, &cancel, |_| {})?;
    let pcm = PcmFile::open(&pcm_path)?;
    let total = pcm.frames as f64 / 16000.0;
    let mut vad = SileroVad::load(&mp.vad(), 300)?;
    let spans = detect_file(&pcm, &mut vad, &cancel, 0.3, |_| {})?;
    let chunks = build_chunks(&spans, total, Default::default(), &mut |a, b| (a + b) / 2.0);
    let mut engine = WhisperEngine::load(&model_path, WhisperConfig::default())?;
    let mut buf = Vec::new();
    let mut guesses = Vec::new();
    for c in chunks.iter().take(3) {
        pcm.read_f32(
            (c.start_s * 16000.0) as u64,
            (c.len().min(30.0) * 16000.0) as usize,
            &mut buf,
        )?;
        if !should_transcribe(&analyze(&buf)) {
            continue;
        }
        let g = engine.detect_language(&buf, threads)?;
        guesses.push(serde_json::json!([g.code, g.prob]));
    }
    let _ = std::fs::remove_file(&pcm_path);
    println!("{}", serde_json::json!({"guesses": guesses}));
    Ok(())
}

fn peak_rss_mb() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as f64;
    // Linux reports KB, macOS bytes
    if cfg!(target_os = "macos") {
        v / 1_048_576.0
    } else {
        v / 1024.0
    }
}

#[allow(clippy::too_many_arguments)]
fn speak(
    text: Option<String>,
    file: Option<PathBuf>,
    voice: String,
    quality: String,
    out: Option<PathBuf>,
    previews: bool,
    job_dir: Option<PathBuf>,
    outputs_dir: Option<PathBuf>,
) -> parlo_core::Result<()> {
    use parlo_core::paths::{AppPaths, ModelPaths};
    use parlo_core::tts::pocket::PocketEngine;
    use parlo_core::tts::{run_tts, Quality, TtsEngine, TtsEvent, TtsJob};
    let mp = ModelPaths::discover();
    let app = AppPaths::discover();
    let t_load = std::time::Instant::now();
    let mut engine = PocketEngine::load(&mp)?;
    eprintln!(
        "[load] speech model ready in {:.2}s",
        t_load.elapsed().as_secs_f64()
    );
    let q = Quality::parse(&quality);
    let cancel = AtomicBool::new(false);
    if previews {
        let dir = mp.previews_dir();
        std::fs::create_dir_all(&dir)?;
        for v in engine.voices() {
            let sentence = if v.language == "es" {
                format!(
                    "Hola, soy {}. Así sueno cuando leo tu texto en voz alta.",
                    v.label
                )
            } else {
                format!(
                    "Hello, I'm {}. This is how I sound when I read your text aloud.",
                    v.label
                )
            };
            let job = TtsJob {
                blocks: parlo_core::text::text_to_blocks(&sentence),
                voice: v.id.clone(),
                quality: Quality::Standard,
                outputs_dir: app.cache.join("preview-outputs"),
                job_dir: app.cache.join(format!("preview-job-{}", v.id)),
            };
            let o = run_tts(&job, &mut engine, &mut |_: TtsEvent| {}, &cancel)?;
            let dest = dir.join(format!("{}.mp3", v.id));
            parlo_core::audio::encode::export_mp3(&o.segments, &dest, 64)?;
            eprintln!(
                "[preview] {} -> {} ({:.1}s)",
                v.id,
                dest.display(),
                o.meta.duration_ms as f64 / 1000.0
            );
        }
        return Ok(());
    }
    let blocks = if let Some(f) = file {
        parlo_core::text::import_file(&f, None)?.blocks
    } else {
        parlo_core::text::text_to_blocks(
            &text.ok_or_else(|| parlo_core::Error::Other("give some text, or --file".into()))?,
        )
    };
    let out = out.unwrap_or_else(|| PathBuf::from("speak.wav"));
    let job = TtsJob {
        blocks,
        voice,
        quality: q,
        outputs_dir: outputs_dir.unwrap_or_else(|| app.tts_outputs()),
        job_dir: job_dir
            .unwrap_or_else(|| app.jobs().join(format!("cli-tts-{:x}", std::process::id()))),
    };
    let t = std::time::Instant::now();
    let mut ttfa: Option<f64> = None;
    let o = run_tts(
        &job,
        &mut engine,
        &mut |e: TtsEvent| match e {
            TtsEvent::SegmentReady {
                index, duration_ms, ..
            } => {
                ttfa.get_or_insert(t.elapsed().as_secs_f64());
                eprintln!(
                    "[segment {index}] ready ({:.1}s of audio)",
                    duration_ms as f64 / 1000.0
                );
            }
            TtsEvent::Progress {
                done_chars,
                total_chars,
                eta_ms,
            } => eprintln!(
                "[progress] {done_chars}/{total_chars} chars, ETA {}",
                eta_ms
                    .map(|m| format!("{}s", m / 1000))
                    .unwrap_or_else(|| "?".into())
            ),
            TtsEvent::Done { .. } => {}
        },
        &cancel,
    )?;
    match out.extension().and_then(|e| e.to_str()) {
        Some("mp3") => parlo_core::audio::encode::export_mp3(&o.segments, &out, 96)?,
        _ => {
            parlo_core::audio::encode::export_wav(&o.segments, &out)?;
        }
    }
    let wall = t.elapsed().as_secs_f64();
    eprintln!("[done] {:.1}s of audio in {:.1}s (RTF {:.2}), first segment after {:.1}s, cache_hit={}, peak RSS {:.0} MB -> {}", o.meta.duration_ms as f64 / 1000.0, wall, wall / (o.meta.duration_ms as f64 / 1000.0).max(1e-9), ttfa.unwrap_or(0.0), o.cache_hit, peak_rss_mb(), out.display());
    Ok(())
}

fn bench(
    what: String,
    input: Option<PathBuf>,
    parity: bool,
    repeat: usize,
    model: String,
    threads: usize,
) -> parlo_core::Result<()> {
    use parlo_core::paths::ModelPaths;
    match (what.as_str(), parity) {
        ("tts", true) => {
            // The full parity suite (Python reference vs the Rust engine, WER, speaker similarity, artifacts, RTF, RSS) is
            // orchestrated by scripts/tts-parity.sh (uv venv + the Rust runners); this just launches it.
            let root = ModelPaths::discover().root.join("..");
            let status = std::process::Command::new("bash")
                .arg(root.join("scripts/tts-parity.sh"))
                .status()
                .map_err(parlo_core::Error::IoBare)?;
            if !status.success() {
                return Err(parlo_core::Error::Other(
                    "parity suite failed (see spike/tts-parity/out/report.md)".into(),
                ));
            }
            Ok(())
        }
        ("tts", false) => {
            use parlo_core::tts::pocket::PocketEngine;
            use parlo_core::tts::{Quality, TtsEngine};
            let t = std::time::Instant::now();
            let mut e = PocketEngine::load(&ModelPaths::discover())?;
            let load_s = t.elapsed().as_secs_f64();
            let text = "The old lighthouse stood at the edge of the cliff, its white paint peeling after decades of salt and wind. Every evening, the keeper climbed the spiral staircase, lit the great lamp, and watched the ships glide past in the dark.";
            let cancel = AtomicBool::new(false);
            for i in 0..repeat {
                let t = std::time::Instant::now();
                let mut first: Option<f64> = None;
                let mut n = 0usize;
                e.synthesize(
                    text,
                    "george",
                    Quality::Standard,
                    &mut |s: &[f32]| {
                        first.get_or_insert(t.elapsed().as_secs_f64());
                        n += s.len();
                        Ok(())
                    },
                    &cancel,
                )?;
                let wall = t.elapsed().as_secs_f64();
                let audio = n as f64 / e.sample_rate() as f64;
                println!(
                    "{}",
                    serde_json::json!({"run": i, "audio_s": audio, "gen_s": wall, "rtf": wall / audio, "ttfa_s": first, "model_load_s": load_s, "peak_rss_mb": peak_rss_mb()})
                );
            }
            Ok(())
        }
        ("stt", _) => {
            let input =
                input.ok_or_else(|| parlo_core::Error::Other("bench stt <audio file>".into()))?;
            transcribe(
                input,
                model,
                "auto".into(),
                threads,
                None,
                None,
                true,
                1,
                false,
            )
        }
        (other, _) => Err(parlo_core::Error::Other(format!(
            "unknown benchmark '{other}' (tts | stt)"
        ))),
    }
}
