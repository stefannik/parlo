//! Text for TTS (BRIEF §9.2/9.3): import (txt/md/docx/pdf/epub), normalization, segmentation into job chunks.

pub mod docx;
pub mod encoding;
pub mod epub;
pub mod html;
pub mod import;
pub mod markdown;
pub mod normalize;
pub mod pdf;
pub mod segment;

pub use import::{import_file, ImportStats, Imported};

/// A structural unit of the source text. Headings get a longer pause after them (BRIEF §9.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub text: String,
    pub heading: bool,
}

impl Block {
    pub fn para(t: impl Into<String>) -> Self {
        Self {
            text: t.into(),
            heading: false,
        }
    }
    pub fn heading(t: impl Into<String>) -> Self {
        Self {
            text: t.into(),
            heading: true,
        }
    }
}

/// Join blocks into plain text (blank line between blocks).
pub fn blocks_to_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Split plain (pasted/typed) text into paragraph blocks on blank lines.
pub fn text_to_blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(Block::para(cur.join(" ")));
                cur.clear();
            }
        } else {
            cur.push(line.trim());
        }
    }
    if !cur.is_empty() {
        out.push(Block::para(cur.join(" ")));
    }
    out
}
