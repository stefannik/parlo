//! Tolerant (X)HTML → blocks, used for EPUB chapters. Not a full parser: block-level tags break paragraphs,
//! `script/style/head/nav` content is skipped, entities are decoded. Robust against the non-well-formed XHTML that
//! real EPUBs contain (named entities like `&nbsp;` are invalid XML, so a strict XML parser would fail).

use super::Block;

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(end) = after.find(';').filter(|e| *e <= 10) {
            let ent = &after[..end];
            let rep: Option<String> = if let Some(num) = ent.strip_prefix('#') {
                let code = if let Some(h) = num.strip_prefix(['x', 'X']) {
                    u32::from_str_radix(h, 16).ok()
                } else {
                    num.parse::<u32>().ok()
                };
                code.and_then(char::from_u32).map(|c| c.to_string())
            } else {
                match ent {
                    "amp" => Some("&".into()),
                    "lt" => Some("<".into()),
                    "gt" => Some(">".into()),
                    "quot" => Some("\"".into()),
                    "apos" => Some("'".into()),
                    "nbsp" => Some(" ".into()),
                    "mdash" => Some("—".into()),
                    "ndash" => Some("–".into()),
                    "hellip" => Some("…".into()),
                    "lsquo" => Some("‘".into()),
                    "rsquo" => Some("’".into()),
                    "ldquo" => Some("“".into()),
                    "rdquo" => Some("”".into()),
                    "eacute" => Some("é".into()),
                    "egrave" => Some("è".into()),
                    "agrave" => Some("à".into()),
                    "uuml" => Some("ü".into()),
                    "ouml" => Some("ö".into()),
                    "auml" => Some("ä".into()),
                    "szlig" => Some("ß".into()),
                    _ => None,
                }
            };
            if let Some(r) = rep {
                out.push_str(&r);
                rest = &after[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "li",
    "blockquote",
    "tr",
    "section",
    "article",
    "pre",
    "figcaption",
    "dt",
    "dd",
    "caption",
];
const SKIP_TAGS: &[&str] = &["script", "style", "head", "nav", "svg", "math"];

/// Convert (X)HTML to blocks. Headings `h1`–`h6` become heading blocks.
pub fn html_to_blocks(html: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut cur = String::new();
    let mut cur_heading = false;
    let mut skip_depth: Vec<String> = Vec::new();
    let flush = |cur: &mut String, heading: &mut bool, blocks: &mut Vec<Block>| {
        let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            blocks.push(Block {
                text: t,
                heading: *heading,
            });
        }
        cur.clear();
        *heading = false;
    };
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            // comment / doctype / PI
            if html[i..].starts_with("<!--") {
                i = html[i..].find("-->").map(|e| i + e + 3).unwrap_or(b.len());
                continue;
            }
            let Some(end) = html[i..].find('>').map(|e| i + e) else {
                break;
            };
            let tag_src = &html[i + 1..end];
            i = end + 1;
            if tag_src.starts_with('!') || tag_src.starts_with('?') {
                continue;
            }
            let closing = tag_src.starts_with('/');
            let name: String = tag_src
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            let self_closing = tag_src.ends_with('/');
            if SKIP_TAGS.contains(&name.as_str()) {
                if closing {
                    if skip_depth.last().is_some_and(|d| *d == name) {
                        skip_depth.pop();
                    }
                } else if !self_closing {
                    skip_depth.push(name.clone());
                }
                continue;
            }
            // `<nav epub:type="toc">` is covered by SKIP_TAGS; also skip elements marked as TOC by role/type
            if !skip_depth.is_empty() {
                continue;
            }
            let is_heading =
                name.len() == 2 && name.starts_with('h') && name.as_bytes()[1].is_ascii_digit();
            if is_heading || BLOCK_TAGS.contains(&name.as_str()) || name == "br" || name == "hr" {
                flush(&mut cur, &mut cur_heading, &mut blocks);
                if is_heading && !closing {
                    cur_heading = true;
                }
            }
        } else {
            let end = html[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            if skip_depth.is_empty() {
                cur.push_str(&decode_entities(&html[i..end]));
            }
            i = end;
        }
    }
    flush(&mut cur, &mut cur_heading, &mut blocks);
    blocks
}
