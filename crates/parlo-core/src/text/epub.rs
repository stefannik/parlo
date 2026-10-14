//! `.epub` → blocks: container.xml → OPF → spine order → XHTML text; skips the navigation document and non-linear items.

use super::html::html_to_blocks;
use super::Block;
use crate::error::{Error, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == key)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

fn resolve(base_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href);
    let mut parts: Vec<&str> = if base_dir.is_empty() {
        vec![]
    } else {
        base_dir.split('/').collect()
    };
    for seg in href.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

pub struct EpubText {
    pub blocks: Vec<Block>,
    pub chapters: usize,
}

pub fn epub_to_blocks(path: &Path) -> Result<EpubText> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut zip = zip::ZipArchive::new(f)
        .map_err(|e| Error::Other(format!("This .epub file is damaged: {e}")))?;
    let read = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Result<String> {
        let mut s = String::new();
        let mut entry = zip
            .by_name(name)
            .map_err(|_| Error::Other(format!("The .epub is missing {name}.")))?;
        let mut raw = Vec::new();
        entry.read_to_end(&mut raw).map_err(Error::IoBare)?;
        s.push_str(&String::from_utf8_lossy(&raw));
        Ok(s)
    };
    let container = read(&mut zip, "META-INF/container.xml")?;
    let mut opf_path = None;
    let mut r = Reader::from_str(&container);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == "rootfile" => {
                opf_path = attr(&e, "full-path");
                break;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    let opf_path =
        opf_path.ok_or_else(|| Error::Other("The .epub has no package document.".into()))?;
    let opf = read(&mut zip, &opf_path)?;
    let base = opf_path
        .rsplit_once('/')
        .map(|x| x.0)
        .unwrap_or("")
        .to_string();

    // manifest: id -> (href, properties); spine: (idref, linear)
    let mut manifest: HashMap<String, (String, String)> = HashMap::new();
    let mut spine: Vec<(String, bool)> = Vec::new();
    let mut r = Reader::from_str(&opf);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "item" => {
                    if let (Some(id), Some(href)) = (attr(&e, "id"), attr(&e, "href")) {
                        manifest.insert(id, (href, attr(&e, "properties").unwrap_or_default()));
                    }
                }
                "itemref" => {
                    if let Some(idref) = attr(&e, "idref") {
                        spine.push((idref, attr(&e, "linear").as_deref() != Some("no")));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(Error::Other(format!(
                    "The .epub package document is damaged: {e}"
                )))
            }
            _ => {}
        }
    }
    let mut blocks = Vec::new();
    let mut chapters = 0;
    for (idref, linear) in spine {
        let Some((href, props)) = manifest.get(&idref) else {
            continue;
        };
        if !linear || props.split_whitespace().any(|p| p == "nav") {
            continue;
        }
        let name = resolve(&base, href);
        let Ok(html) = read(&mut zip, &name) else {
            continue;
        };
        let b = html_to_blocks(&html);
        if !b.is_empty() {
            chapters += 1;
            blocks.extend(b);
        }
    }
    Ok(EpubText { blocks, chapters })
}
