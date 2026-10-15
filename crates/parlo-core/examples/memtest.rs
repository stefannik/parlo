//! Memory-return investigation: load → synthesize → drop the TTS engine, printing RSS at each step.
use parlo_core::paths::ModelPaths;
use parlo_core::tts::pocket::PocketEngine;
use parlo_core::tts::{Quality, TtsEngine};
use std::sync::atomic::AtomicBool;

fn rss() -> u64 {
    let o = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        / 1024
}

fn main() {
    println!("start {} MB", rss());
    let mut e = PocketEngine::load(&ModelPaths::discover()).unwrap();
    println!("loaded {} MB", rss());
    let cancel = AtomicBool::new(false);
    e.synthesize(
        "Hello from the memory test. This is a short sentence.",
        "george",
        Quality::Standard,
        &mut |_s: &[f32]| Ok(()),
        &cancel,
    )
    .unwrap();
    println!("synthesized {} MB", rss());
    drop(e);
    println!("dropped {} MB", rss());
    std::thread::sleep(std::time::Duration::from_secs(1));
    println!("+2s {} MB", rss());
}
