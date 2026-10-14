//! TTS text normalization (BRIEF §9.2): minimal and testable. NFC, smart quotes/dashes, whitespace collapse, emoji and
//! control characters removed, bare URLs replaced with "link". Numbers are not rewritten (parity tests decide).

use regex::Regex;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s<>"]+"#).unwrap())
}
fn num_range_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(\d)\s*[–—]\s*(\d)").unwrap())
}
fn dash_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s*[–—―]\s*").unwrap())
}

fn is_removed(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x00AD | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0xFEFF | 0xFE00..=0xFE0F
        | 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2300..=0x23FF | 0xE0000..=0xE007F)
        || (c.is_control() && !c.is_whitespace())
}

pub fn normalize_block(s: &str) -> String {
    let s: String = s.nfc().collect();
    // URLs first (trailing punctuation stays)
    let s = url_re().replace_all(&s, |caps: &regex::Captures| {
        let m = &caps[0];
        let trimmed = m.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '”', '"', '\'']);
        format!("link{}", &m[trimmed.len()..])
    });
    let s = num_range_re().replace_all(&s, "$1 to $2");
    let s = dash_re().replace_all(&s, ", ");
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_removed(c) {
            continue;
        }
        match c {
            '“' | '”' | '„' | '‟' | '«' | '»' => out.push('"'),
            '‘' | '’' | '‚' | '‛' | '´' | '`' => out.push('\''),
            '…' => out.push_str("..."),
            '•' | '·' | '◦' | '▪' => out.push(' '),
            '\u{00A0}' | '\u{2007}' | '\u{202F}' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_dashes_ellipsis_and_whitespace() {
        assert_eq!(
            normalize_block("“Hello,” she said — “it’s here…”"),
            "\"Hello,\" she said, \"it's here...\""
        );
        assert_eq!(
            normalize_block("pages 5–10 and 3 — 4"),
            "pages 5 to 10 and 3 to 4"
        );
        assert_eq!(normalize_block("a  b\t\u{00A0}c\n d"), "a b c d");
        assert_eq!(
            normalize_block("co\u{00AD}operate"),
            "cooperate",
            "soft hyphens vanish"
        );
    }

    #[test]
    fn nfc_emoji_and_control_characters() {
        assert_eq!(normalize_block("cafe\u{0301}"), "café", "NFC composes");
        assert_eq!(normalize_block("great 👍🏽 job 🎉!"), "great job !");
        assert_eq!(normalize_block("a\u{0007}b\u{200B}c"), "abc");
    }

    #[test]
    fn urls_become_link_and_keep_trailing_punctuation() {
        assert_eq!(
            normalize_block("See https://example.com/a?b=1&c=2."),
            "See link."
        );
        assert_eq!(
            normalize_block("(www.example.org/x), then http://a.b/c!"),
            "(link), then link!"
        );
        assert_eq!(
            normalize_block("no url here: example"),
            "no url here: example"
        );
    }

    #[test]
    fn numbers_and_dates_are_left_alone() {
        assert_eq!(
            normalize_block("On March 3rd, 2024, $1,250.75 for 3 tickets at 5:30 p.m."),
            "On March 3rd, 2024, $1,250.75 for 3 tickets at 5:30 p.m."
        );
    }
}
