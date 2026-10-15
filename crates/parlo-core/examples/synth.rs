//! Engine-only synthesis (no pipeline): prints the audio length. Usage: synth <voice> <text...>
use parlo_core::paths::ModelPaths;
use parlo_core::tts::pocket::PocketEngine;
use parlo_core::tts::{Quality, TtsEngine};
use std::sync::atomic::AtomicBool;
fn main() {
    let mut a = std::env::args().skip(1);
    let voice = a.next().unwrap();
    let text = a.collect::<Vec<_>>().join(" ");
    let mut e = PocketEngine::load(&ModelPaths::discover()).unwrap();
    let mut n = 0usize;
    e.synthesize(
        &text,
        &voice,
        Quality::Standard,
        &mut |s: &[f32]| {
            n += s.len();
            Ok(())
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    println!("{:.2}s", n as f64 / 24000.0);
}
