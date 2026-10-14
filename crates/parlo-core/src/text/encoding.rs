//! Text decoding for `.txt` (BRIEF §9.2): UTF-8/16 BOM first, else strict UTF-8, else `chardetng` + `encoding_rs`.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub text: String,
    pub encoding: &'static str,
    /// true if some bytes could not be represented and were replaced
    pub lossy: bool,
}

pub fn decode_bytes(b: &[u8]) -> Decoded {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let (t, lossy) = match std::str::from_utf8(rest) {
            Ok(s) => (s.to_string(), false),
            Err(_) => (String::from_utf8_lossy(rest).into_owned(), true),
        };
        return Decoded {
            text: t,
            encoding: "UTF-8",
            lossy,
        };
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        let (t, _, bad) = encoding_rs::UTF_16LE.decode(rest);
        return Decoded {
            text: t.into_owned(),
            encoding: "UTF-16LE",
            lossy: bad,
        };
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let (t, _, bad) = encoding_rs::UTF_16BE.decode(rest);
        return Decoded {
            text: t.into_owned(),
            encoding: "UTF-16BE",
            lossy: bad,
        };
    }
    if let Ok(s) = std::str::from_utf8(b) {
        return Decoded {
            text: s.to_string(),
            encoding: "UTF-8",
            lossy: false,
        };
    }
    let mut det = EncodingDetector::new(Iso2022JpDetection::Deny);
    det.feed(b, true);
    let enc = det.guess(None, Utf8Detection::Allow);
    let (t, _, bad) = enc.decode(b);
    Decoded {
        text: t.into_owned(),
        encoding: enc.name(),
        lossy: bad,
    }
}
