//! Long-text planning (BRIEF §9.3): blocks → job chunks of whole paragraphs (≤ ~1000 chars; long paragraphs are split at
//! sentence boundaries). Pauses are *generated silence* (PocketTTS can't produce pauses from text):
//! ~400 ms after a paragraph, ~800 ms after a heading, a short breath between pieces of one paragraph.

use super::normalize::normalize_block;
use super::Block;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_CHUNK_CHARS: usize = 1000;
pub const PAUSE_PARAGRAPH_MS: u32 = 400;
pub const PAUSE_HEADING_MS: u32 = 800;
pub const PAUSE_PIECE_MS: u32 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub heading: bool,
    pub pause_after_ms: u32,
}

const ABBREV: &[&str] = &[
    "mr.", "mrs.", "ms.", "dr.", "prof.", "st.", "sr.", "jr.", "vs.", "etc.", "e.g.", "i.e.",
    "no.", "fig.", "approx.", "inc.", "ltd.", "u.s.", "a.m.", "p.m.",
];

fn split_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in text.unicode_sentences() {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        let prev_is_abbrev = out.last().is_some_and(|p: &String| {
            let last = p.split_whitespace().last().unwrap_or("").to_lowercase();
            ABBREV.contains(&last.as_str())
                && s.chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
        });
        if prev_is_abbrev {
            let last = out.last_mut().unwrap();
            last.push(' ');
            last.push_str(s);
        } else {
            out.push(s.to_string());
        }
    }
    out
}

/// Split one over-long sentence at commas/semicolons, then at spaces, into pieces ≤ `max`.
fn split_long_sentence(s: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        let would = cur.chars().count() + 1 + word.chars().count();
        if !cur.is_empty() && would > max {
            pieces.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        if cur.chars().count() >= max / 2
            && (cur.ends_with(',') || cur.ends_with(';') || cur.ends_with(':'))
        {
            pieces.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
}

pub fn plan_chunks(blocks: &[Block], max_chars: usize) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    for b in blocks {
        let text = normalize_block(&b.text);
        if text.is_empty() || !text.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        let end_pause = if b.heading {
            PAUSE_HEADING_MS
        } else {
            PAUSE_PARAGRAPH_MS
        };
        if text.chars().count() <= max_chars {
            chunks.push(Chunk {
                text,
                heading: b.heading,
                pause_after_ms: end_pause,
            });
            continue;
        }
        // split at sentence boundaries, grouping up to max_chars
        let mut pieces: Vec<String> = Vec::new();
        let mut cur = String::new();
        for s in split_sentences(&text) {
            let parts = if s.chars().count() > max_chars {
                split_long_sentence(&s, max_chars)
            } else {
                vec![s]
            };
            for p in parts {
                if !cur.is_empty() && cur.chars().count() + 1 + p.chars().count() > max_chars {
                    pieces.push(std::mem::take(&mut cur));
                }
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(&p);
            }
        }
        if !cur.is_empty() {
            pieces.push(cur);
        }
        let n = pieces.len();
        for (i, p) in pieces.into_iter().enumerate() {
            chunks.push(Chunk {
                text: p,
                heading: b.heading && n == 1,
                pause_after_ms: if i + 1 == n {
                    end_pause
                } else {
                    PAUSE_PIECE_MS
                },
            });
        }
    }
    chunks
}
