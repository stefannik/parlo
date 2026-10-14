//! PDF page-text cleanup (BRIEF §9.2). Extraction itself is platform code (PDFKit on macOS, via `PdfTextExtractor`);
//! this is the portable part, tested on page-text fixtures: de-hyphenation, repeated header/footer and page-number
//! removal, line joining into paragraphs, headings, and the "scanned PDF" diagnosis.

use super::Block;
use crate::error::{Error, Result};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

fn page_number_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*[-–—]?\s*(page\s+)?(\d{1,5}|[ivxlc]{1,7})(\s*(of|/)\s*\d{1,5})?\s*[-–—]?\s*$",
        )
        .unwrap()
    })
}

fn shape(line: &str) -> String {
    let mut s = String::with_capacity(line.len());
    for c in line.trim().chars() {
        if c.is_ascii_digit() {
            if !s.ends_with('#') {
                s.push('#');
            }
        } else {
            s.extend(c.to_lowercase());
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ends_sentence(s: &str) -> bool {
    s.trim_end()
        .ends_with(['.', '!', '?', ':', '"', '”', '’', ')'])
}

pub const NO_TEXT_LAYER: &str =
    "This PDF has no selectable text (it looks scanned). OCR isn't supported yet.";

pub struct CleanedPdf {
    pub blocks: Vec<Block>,
    pub pages: usize,
    pub removed_header_footer_lines: usize,
    pub warnings: Vec<String>,
}

pub fn clean_pdf_pages(pages: &[String]) -> Result<CleanedPdf> {
    let n_pages = pages.len();
    let with_text = pages
        .iter()
        .filter(|p| p.trim().chars().any(|c| c.is_alphanumeric()))
        .count();
    if with_text == 0 {
        return Err(Error::Other(NO_TEXT_LAYER.into()));
    }
    let mut warnings = Vec::new();
    if with_text * 2 < n_pages {
        warnings.push(format!(
            "{} of {} pages have no selectable text and were skipped.",
            n_pages - with_text,
            n_pages
        ));
    }
    // 1) header/footer candidates: first/last 2 non-empty lines of each page
    let page_lines: Vec<Vec<&str>> = pages
        .iter()
        .map(|p| p.lines().map(|l| l.trim()).collect())
        .collect();
    let mut freq: HashMap<String, usize> = HashMap::new();
    let cands = |lines: &Vec<&str>| -> Vec<usize> {
        let idx: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.is_empty())
            .map(|(i, _)| i)
            .collect();
        let mut v: Vec<usize> = idx.iter().take(2).copied().collect();
        v.extend(idx.iter().rev().take(2).copied());
        v.sort_unstable();
        v.dedup();
        v
    };
    for lines in &page_lines {
        let mut seen = std::collections::HashSet::new();
        for i in cands(lines) {
            let sh = shape(lines[i]);
            if !sh.is_empty() && seen.insert(sh.clone()) {
                *freq.entry(sh).or_default() += 1;
            }
        }
    }
    let threshold = if n_pages >= 3 {
        (n_pages as f64 * 0.5).ceil() as usize
    } else {
        usize::MAX
    };
    let mut removed = 0usize;
    // 2) rebuild each page without header/footer lines and bare page numbers
    let mut cleaned_pages: Vec<Vec<String>> = Vec::new();
    for lines in &page_lines {
        let c: std::collections::HashSet<usize> = cands(lines).into_iter().collect();
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if c.contains(&i) && !l.is_empty() {
                let sh = shape(l);
                if freq.get(&sh).copied().unwrap_or(0) >= threshold || page_number_re().is_match(l)
                {
                    removed += 1;
                    continue;
                }
            }
            out.push((*l).to_string());
        }
        cleaned_pages.push(out);
    }
    // 3) typical full-line width, for paragraph-break inference
    let mut widths: Vec<usize> = cleaned_pages
        .iter()
        .flatten()
        .map(|l| l.chars().count())
        .filter(|w| *w > 30)
        .collect();
    widths.sort_unstable();
    let typical = widths.get(widths.len() / 2).copied().unwrap_or(60) as f64;

    // 4) lines -> blocks (with de-hyphenation across lines and pages)
    let mut blocks: Vec<Block> = Vec::new();
    let mut cur = String::new();
    let mut cur_cont_candidate = false;
    let flush = |cur: &mut String, blocks: &mut Vec<Block>, heading: bool| {
        let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            blocks.push(Block { text: t, heading });
        }
        cur.clear();
    };
    for page in &cleaned_pages {
        let mut first_line_of_page = true;
        let mut i = 0;
        while i < page.len() {
            let line = page[i].as_str();
            i += 1;
            if line.is_empty() {
                flush(&mut cur, &mut blocks, false);
                first_line_of_page = false;
                continue;
            }
            let len = line.chars().count() as f64;
            let looks_heading = len < 60.0
                && !ends_sentence(line)
                && !line.ends_with([',', ';', '-'])
                && line
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
                && line.split_whitespace().count() <= 8;
            if looks_heading
                && (cur.is_empty() || ends_sentence(&cur))
                && page.get(i).is_none_or(|n| {
                    n.is_empty() || n.chars().next().is_some_and(|c| c.is_uppercase())
                })
            {
                flush(&mut cur, &mut blocks, false);
                blocks.push(Block::heading(line.to_string()));
                first_line_of_page = false;
                cur_cont_candidate = false;
                continue;
            }
            // continuation of a paragraph interrupted by a page break?
            if first_line_of_page
                && cur_cont_candidate
                && !cur.is_empty()
                && !ends_sentence(&cur)
                && line.chars().next().is_some_and(|c| c.is_lowercase())
            {
                // keep appending to `cur`
            } else if first_line_of_page {
                flush(&mut cur, &mut blocks, false);
            }
            // de-hyphenate: "informa-" + "tion"
            if cur.ends_with('-')
                && cur.chars().rev().nth(1).is_some_and(|c| c.is_alphabetic())
                && line.chars().next().is_some_and(|c| c.is_lowercase())
            {
                cur.pop();
                cur.push_str(line);
            } else {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(line);
            }
            // a short line that ends a sentence ends the paragraph
            if ends_sentence(line) && len < typical * 0.7 {
                flush(&mut cur, &mut blocks, false);
            }
            first_line_of_page = false;
        }
        cur_cont_candidate = !cur.is_empty();
    }
    flush(&mut cur, &mut blocks, false);
    Ok(CleanedPdf {
        blocks,
        pages: n_pages,
        removed_header_footer_lines: removed,
        warnings,
    })
}
