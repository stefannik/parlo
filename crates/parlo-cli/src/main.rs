//! parlo-cli: headless access to parlo-core (tests, benchmarks, verification).
use clap::{Parser, Subcommand};
use parlo_core::audio::{decode, prepare};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[derive(Parser)]
#[command(name = "parlo-cli", version, about = "Headless Parlo: decode, record, transcribe, speak")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Decode any supported audio/video file to 16 kHz mono PCM16 WAV (what the STT pipeline consumes)
    Decode { input: PathBuf, output: PathBuf },
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
                input.display(), output.display(), p.duration_s, p.frames, prepare::STT_RATE, p.source.codec, p.source.sample_rate, p.source.channels, t.elapsed().as_secs_f64()
            );
        }
        Cmd::Info { input } => {
            let i = decode::probe_info(&input)?;
            println!("{}", serde_json::json!({"codec": i.codec, "sample_rate": i.sample_rate, "channels": i.channels, "duration_s": i.duration_s}));
        }
    }
    Ok(())
}
