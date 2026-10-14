//! Text import snapshot tests (BRIEF §14.1): each format produces the expected text, not just "non-empty output".
//! Expected files live in fixtures/text/expected/; regenerate (and review the diff!) with UPDATE_SNAPSHOTS=1.
mod common;
use common::fixture;
use parlo_core::error::Error;
use parlo_core::platform::PdfTextExtractor;
use parlo_core::text::import::{finish, import_file};
use parlo_core::text::pdf::{clean_pdf_pages, NO_TEXT_LAYER};
use parlo_core::text::segment::{
    plan_chunks, MAX_CHUNK_CHARS, PAUSE_HEADING_MS, PAUSE_PARAGRAPH_MS,
};
use parlo_core::text::{blocks_to_text, Block};

fn snapshot(name: &str, actual: &str) {
    let path = fixture("text").join("expected").join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot {}; run with UPDATE_SNAPSHOTS=1 and review it",
            path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "snapshot {name} differs (UPDATE_SNAPSHOTS=1 to regenerate after review)"
    );
}

/// Page-text JSON fixtures stand in for PDFKit on Linux.
struct JsonPdf;
impl PdfTextExtractor for JsonPdf {
    fn page_texts(&self, pdf: &std::path::Path) -> parlo_core::Result<Vec<String>> {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(pdf).unwrap()).unwrap();
        Ok(v["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect())
    }
}

#[test]
fn every_txt_encoding_decodes_to_the_same_text() {
    let base = import_file(&fixture("text/plain_utf8.txt"), None).unwrap();
    assert!(base.warnings.is_empty());
    assert!(
        base.text().contains("café")
            && base.text().contains("€5")
            && base.text().contains("“like these”")
    );
    for f in [
        "plain_utf8_bom.txt",
        "plain_utf16le_bom.txt",
        "plain_utf16be_bom.txt",
        "plain_cp1252.txt",
    ] {
        let i = import_file(&fixture(&format!("text/{f}")), None).unwrap();
        assert_eq!(i.text(), base.text(), "{f}");
        if f.contains("cp1252") {
            assert!(
                i.warnings.iter().any(|w| w.starts_with("Read as")),
                "non-UTF-8 input is announced: {:?}",
                i.warnings
            );
        }
    }
    let l1 = import_file(&fixture("text/plain_latin1.txt"), None).unwrap();
    assert!(
        l1.text().contains("Café crème") && l1.text().contains("Zürich"),
        "{}",
        l1.text()
    );
    let cy = import_file(&fixture("text/plain_utf8_cyrillic.txt"), None).unwrap();
    let cp = import_file(&fixture("text/plain_cp1251.txt"), None).unwrap();
    assert_eq!(cp.text(), cy.text(), "Windows-1251 Cyrillic is detected");
    snapshot("plain_utf8", &base.text());
    snapshot("plain_latin1", &l1.text());
    snapshot("plain_cyrillic", &cp.text());
}

#[test]
fn markdown_rules() {
    let i = import_file(&fixture("text/sample.md"), None).unwrap();
    let t = i.text();
    assert!(t.contains("The Sample Document") && !t.contains('#'));
    assert!(
        t.contains("link to the docs") && !t.contains("https://example.com/docs"),
        "link text kept, URL dropped"
    );
    assert!(
        !t.contains("print(") && !t.contains("hello()"),
        "code blocks skipped"
    );
    assert!(
        t.contains("Two apples.") && t.contains("fresh bread."),
        "list items become sentences"
    );
    assert!(!t.contains("An image") && !t.contains("image.png"));
    assert!(i
        .blocks
        .iter()
        .any(|b| b.heading && b.text == "Shopping list."));
    assert!(i.warnings.iter().any(|w| w.contains("Code blocks")));
    snapshot("sample_md", &t);
}

#[test]
fn docx_reads_body_paragraphs_only() {
    let i = import_file(&fixture("text/sample.docx"), None).unwrap();
    let t = i.text();
    assert!(
        !t.contains("Header text") && !t.contains("Footer text"),
        "headers/footers are skipped"
    );
    assert!(t.contains("Sample Word Document") && t.contains("Closing paragraph after the table."));
    assert!(i.blocks.iter().any(|b| b.heading));
    snapshot("sample_docx", &t);
}

#[test]
fn epub_follows_spine_order_and_skips_navigation_and_scripts() {
    let i = import_file(&fixture("text/sample.epub"), None).unwrap();
    let t = i.text();
    let (a, b, c) = (
        t.find("Chapter One").unwrap(),
        t.find("Chapter Two").unwrap(),
        t.find("Chapter Three").unwrap(),
    );
    assert!(a < b && b < c, "spine order (the manifest lists ch3 first)");
    assert!(!t.contains("Contents"), "the nav document is skipped");
    assert!(!t.contains("skipped"), "script content is not text");
    assert!(
        t.contains("It began on a quiet Tuesday & nobody noticed."),
        "entities decoded: {t}"
    );
    assert_eq!(i.stats.chapters, Some(3));
    snapshot("sample_epub", &t);
}

#[test]
fn pdf_page_text_cleanup_removes_headers_footers_and_hyphenation() {
    let i = import_file(
        &fixture("text/pdf_pages_headers_hyphens.json"),
        Some(&JsonPdf),
    )
    .or_else(|_| {
        // `.json` isn't a PDF extension; go through the cleanup directly (what the Mac path does after PDFKit)
        let pages = JsonPdf.page_texts(&fixture("text/pdf_pages_headers_hyphens.json"))?;
        let c = clean_pdf_pages(&pages)?;
        finish(c.blocks, "pdf", Some(c.pages), None, c.warnings)
    });
    let i = i.unwrap();
    let t = i.text();
    assert!(
        !t.contains("ACME Corp"),
        "repeated running header removed: {t}"
    );
    assert!(
        !t.contains("Page 1 of 4") && !t.contains("Page 4 of 4"),
        "page-number footers removed"
    );
    assert!(
        t.contains("financial results")
            && t.contains("information services")
            && t.contains("Customer satisfaction"),
        "de-hyphenated: {t}"
    );
    assert!(!t.contains("fi-") && !t.contains("sat-"));
    assert_eq!(i.stats.pages, Some(4));
    snapshot("pdf_headers_hyphens", &t);
}

#[test]
fn scanned_pdf_is_diagnosed_clearly() {
    let pages = JsonPdf
        .page_texts(&fixture("text/pdf_pages_scanned.json"))
        .unwrap();
    match clean_pdf_pages(&pages) {
        Err(Error::Other(m)) => assert_eq!(m, NO_TEXT_LAYER),
        other => panic!("{:?}", other.map(|c| c.blocks.len())),
    }
}

#[test]
fn unsupported_extension_and_empty_text_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.rtf");
    std::fs::write(&p, "{\\rtf1 hi}").unwrap();
    assert!(matches!(import_file(&p, None), Err(Error::Other(m)) if m.contains("Supported")));
    let e = dir.path().join("empty.txt");
    std::fs::write(&e, "  \n\n ").unwrap();
    assert!(
        matches!(import_file(&e, None), Err(Error::Other(m)) if m.contains("no readable text"))
    );
}

#[test]
fn chunks_follow_paragraphs_with_headings_pausing_longer() {
    let blocks = vec![
        Block::heading("Chapter One"),
        Block::para("First paragraph. It has two sentences."),
        Block::para("Second."),
    ];
    let c = plan_chunks(&blocks, MAX_CHUNK_CHARS);
    assert_eq!(c.len(), 3);
    assert_eq!(c[0].pause_after_ms, PAUSE_HEADING_MS);
    assert_eq!(c[1].pause_after_ms, PAUSE_PARAGRAPH_MS);
    assert!(c[0].heading && !c[1].heading);
}

#[test]
fn a_long_paragraph_is_split_at_sentence_boundaries_below_the_cap() {
    let sentence =
        "This is one fairly ordinary sentence of moderate length, used to build a long paragraph. ";
    let long = sentence.repeat(60); // ≈ 5 000 chars, no paragraph breaks
    let c = plan_chunks(&[Block::para(long.trim())], MAX_CHUNK_CHARS);
    assert!(c.len() >= 5, "{}", c.len());
    for ch in &c {
        assert!(ch.text.chars().count() <= MAX_CHUNK_CHARS);
        assert!(
            ch.text.ends_with('.'),
            "chunk should end on a sentence boundary: {:?}",
            &ch.text[ch.text.len().saturating_sub(30)..]
        );
    }
    assert_eq!(
        c.iter()
            .filter(|x| x.pause_after_ms == PAUSE_PARAGRAPH_MS)
            .count(),
        1,
        "only the last piece gets the paragraph pause"
    );
    let rejoined = c
        .iter()
        .map(|x| x.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        rejoined.split_whitespace().count(),
        long.split_whitespace().count(),
        "no words lost or duplicated"
    );
}

#[test]
fn abbreviations_are_not_sentence_breaks() {
    let t = "Dr. Smith met Mr. Jones at 5 p.m. on Main St. yesterday. ".repeat(40);
    let c = plan_chunks(&[Block::para(t.trim())], 300);
    for ch in &c {
        assert!(
            !ch.text.ends_with("Dr.") && !ch.text.ends_with("Mr.") && !ch.text.ends_with("St."),
            "{:?}",
            ch.text
        );
    }
}

#[test]
fn text_blocks_roundtrip() {
    let b = parlo_core::text::text_to_blocks("one\ntwo\n\nthree\n\n\nfour");
    assert_eq!(blocks_to_text(&b), "one two\n\nthree\n\nfour");
}
