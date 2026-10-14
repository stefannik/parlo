//! PDF text via PDFKit (`PDFDocument` → `PDFPage.string` per page). DRAFTED, UNCHECKED on a Mac.
//! The portable cleanup (de-hyphenation, header/footer removal, scanned-PDF diagnosis) is `parlo_core::text::pdf`.

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_foundation::{NSString, NSURL};
use objc2_pdf_kit::PDFDocument;
use parlo_core::error::{Error, Result};
use parlo_core::platform::PdfTextExtractor;
use std::path::Path;

pub struct PdfKitExtractor;

impl PdfTextExtractor for PdfKitExtractor {
    fn page_texts(&self, pdf: &Path) -> Result<Vec<String>> {
        let path = pdf
            .to_str()
            .ok_or_else(|| Error::Other("PDF path is not UTF-8".into()))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let doc: Retained<PDFDocument> =
            unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }.ok_or_else(|| {
                Error::Other(
                    "This PDF could not be opened (it may be damaged or password-protected)."
                        .into(),
                )
            })?;
        let n = unsafe { doc.pageCount() };
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let text = unsafe { doc.pageAtIndex(i) }
                .and_then(|p| unsafe { p.string() })
                .map(|s| s.to_string())
                .unwrap_or_default();
            out.push(text);
        }
        Ok(out)
    }
}
