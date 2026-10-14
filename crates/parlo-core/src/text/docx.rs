//! `.docx` → blocks: `word/document.xml` paragraphs only (headers, footers, comments, footnotes live in other parts).

use super::Block;
use crate::error::{Error, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Read;
use std::path::Path;

pub fn docx_to_blocks(path: &Path) -> Result<Vec<Block>> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut zip = zip::ZipArchive::new(f)
        .map_err(|e| Error::Other(format!("This .docx file is damaged: {e}")))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| {
            Error::Other("This doesn't look like a Word document (no word/document.xml).".into())
        })?
        .read_to_string(&mut xml)
        .map_err(Error::IoBare)?;
    parse_document_xml(&xml)
}

pub fn parse_document_xml(xml: &str) -> Result<Vec<Block>> {
    let mut r = Reader::from_str(xml);
    let mut blocks = Vec::new();
    let mut cur = String::new();
    let mut heading = false;
    let mut in_t = false;
    let mut in_del = 0usize; // tracked deletions are not text
    let mut tbl = 0usize; // table nesting: a row is read as one sentence ("Name, Qty.")
    let mut cell: Vec<String> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                "p" => {
                    cur.clear();
                    heading = false;
                }
                "t" => in_t = true,
                "del" => in_del += 1,
                "tbl" => tbl += 1,
                "pStyle" => {}
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "tab" => cur.push(' '),
                "br" | "cr" => cur.push(' '),
                "pStyle" => {
                    for a in e.attributes().flatten() {
                        if a.key.local_name().as_ref() == "val" {
                            let v = a
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map(|c| c.to_ascii_lowercase())
                                .unwrap_or_default();
                            if v.starts_with("heading") || v == "title" || v == "subtitle" {
                                heading = true;
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "t" => in_t = false,
                "del" => in_del = in_del.saturating_sub(1),
                "tbl" => tbl = tbl.saturating_sub(1),
                "tc" if tbl > 0 => {
                    let t = cell.join(" ");
                    if !t.is_empty() {
                        row.push(t);
                    }
                    cell.clear();
                }
                "tr" if tbl > 0 => {
                    if !row.is_empty() {
                        blocks.push(Block::para(format!("{}.", row.join(", "))));
                    }
                    row.clear();
                }
                "p" => {
                    let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !t.is_empty() {
                        if tbl > 0 {
                            cell.push(t);
                        } else {
                            blocks.push(Block { text: t, heading });
                        }
                    }
                    cur.clear();
                    heading = false;
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_t && in_del == 0 => {
                cur.push_str(&t.xml10_content());
            }
            Ok(Event::GeneralRef(g)) if in_t && in_del == 0 => {
                // quick-xml 0.4x reports entity references (&amp; …) separately
                let name = g.xml10_content();
                match name.as_ref() {
                    "amp" => cur.push('&'),
                    "lt" => cur.push('<'),
                    "gt" => cur.push('>'),
                    "quot" => cur.push('"'),
                    "apos" => cur.push('\''),
                    n => {
                        if let Some(num) = n.strip_prefix('#') {
                            let code = if let Some(h) = num.strip_prefix('x') {
                                u32::from_str_radix(h, 16).ok()
                            } else {
                                num.parse().ok()
                            };
                            if let Some(c) = code.and_then(char::from_u32) {
                                cur.push(c);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Other(format!("This .docx file is damaged: {e}"))),
            _ => {}
        }
    }
    Ok(blocks)
}
