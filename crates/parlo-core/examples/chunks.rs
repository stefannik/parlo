//! Print the internal TTS chunks for a text (parity check against pocket-tts' `split_into_best_sentences`).
//! Usage: cargo run --release -p parlo-core --example chunks -- <voice> <file>
use parlo_core::paths::ModelPaths;
use parlo_core::tts::pocket::PocketEngine;
fn main() {
    let mut a = std::env::args().skip(1);
    let (voice, file) = (a.next().unwrap(), a.next().unwrap());
    let text = std::fs::read_to_string(file).unwrap();
    let mut e = PocketEngine::load(&ModelPaths::discover()).unwrap();
    for c in e.debug_chunks(&voice, &text).unwrap() {
        println!("{}", serde_json::to_string(&c).unwrap());
    }
}
