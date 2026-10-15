//! Print the English model's token ids for the given strings (parity check against pocket-tts' tokenizer).
use parlo_core::paths::ModelPaths;
use parlo_core::tts::pocket::PocketEngine;
fn main() {
    let mut e = PocketEngine::load(&ModelPaths::discover()).unwrap();
    for s in std::env::args().skip(1) {
        println!("{:?}", e.debug_tokens(&s).unwrap());
    }
}
