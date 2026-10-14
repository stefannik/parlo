//! File import for the Speak view (BRIEF §9.2): `.txt .md .docx .pdf .epub` → blocks + stats + warnings.

use super::{blocks_to_text, docx, encoding, epub, markdown, pdf, Block};
use crate::error::{Error, Result};
use crate::platform::PdfTextExtractor;
use std::path::Path;

/// Hard cap (BRIEF §9.2) and the point where the UI switches to a read-only preview.
pub const MAX_CHARS: usize = 5_000_000;
pub const LARGE_TEXT_CHARS: usize = 200_000;
/// Average narration speed used for the "~listening time" estimate at 1× (≈ 150 words per minute).
pub const WORDS_PER_MINUTE: f64 = 150.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ImportStats {
    pub chars: usize,
    pub words: usize,
    pub pages: Option<usize>,
    pub chapters: Option<usize>,
    pub est_minutes: f64,
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub blocks: Vec<Block>,
    pub stats: ImportStats,
    pub warnings: Vec<String>,
    pub kind: &'static str,
}

impl Imported {
    pub fn text(&self) -> String {
        blocks_to_text(&self.blocks)
    }
}

pub fn stats_for(blocks: &[Block], pages: Option<usize>, chapters: Option<usize>) -> ImportStats {
    let chars: usize = blocks.iter().map(|b| b.text.chars().count()).sum();
    let words: usize = blocks
        .iter()
        .map(|b| b.text.split_whitespace().count())
        .sum();
    ImportStats {
        chars,
        words,
        pages,
        chapters,
        est_minutes: words as f64 / WORDS_PER_MINUTE,
    }
}

pub fn import_file(path: &Path, pdf_extractor: Option<&dyn PdfTextExtractor>) -> Result<Imported> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut warnings = Vec::new();
    let (blocks, kind, pages, chapters): (Vec<Block>, &'static str, Option<usize>, Option<usize>) =
        match ext.as_str() {
            "txt" | "text" => {
                let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
                let d = encoding::decode_bytes(&bytes);
                if d.encoding != "UTF-8" {
                    warnings.push(format!("Read as {}.", d.encoding));
                }
                if d.lossy {
                    warnings.push("Some characters could not be read and were replaced.".into());
                }
                (super::text_to_blocks(&d.text), "txt", None, None)
            }
            "md" | "markdown" => {
                let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
                let d = encoding::decode_bytes(&bytes);
                let has_code = d.text.contains("```") || d.text.contains("~~~");
                if has_code {
                    warnings.push("Code blocks were skipped.".into());
                }
                (markdown::markdown_to_blocks(&d.text), "md", None, None)
            }
            "docx" => (docx::docx_to_blocks(path)?, "docx", None, None),
            "epub" => {
                let e = epub::epub_to_blocks(path)?;
                let ch = e.chapters;
                (e.blocks, "epub", None, Some(ch))
            }
            "pdf" => {
                let ex = pdf_extractor.ok_or_else(|| {
                    Error::Other("PDF import is only available in the Mac app.".into())
                })?;
                let pages_text = ex.page_texts(path)?;
                let c = pdf::clean_pdf_pages(&pages_text)?;
                warnings.extend(c.warnings);
                if c.removed_header_footer_lines > 0 {
                    warnings.push(format!(
                        "Removed {} repeated header/footer lines.",
                        c.removed_header_footer_lines
                    ));
                }
                (c.blocks, "pdf", Some(c.pages), None)
            }
            other => {
                return Err(Error::Other(format!(
                    "Parlo can't import .{other} files. Supported: .txt .md .docx .pdf .epub"
                )))
            }
        };
    finish(blocks, kind, pages, chapters, warnings)
}

/// Shared tail for every importer (also used by tests on page-text fixtures).
pub fn finish(
    blocks: Vec<Block>,
    kind: &'static str,
    pages: Option<usize>,
    chapters: Option<usize>,
    mut warnings: Vec<String>,
) -> Result<Imported> {
    let stats = stats_for(&blocks, pages, chapters);
    if stats.chars == 0 {
        return Err(Error::Other("This file contains no readable text.".into()));
    }
    if stats.chars > MAX_CHARS {
        return Err(Error::Other(format!("This document has {} characters; the limit is {} (about a thousand pages). Split it into parts.", stats.chars, MAX_CHARS)));
    }
    if stats.chars > LARGE_TEXT_CHARS {
        warnings.push("This is a long document; it will be shown as a read-only preview.".into());
    }
    Ok(Imported {
        blocks,
        stats,
        warnings,
        kind,
    })
}
