//! Paragraph assembly (BRIEF §7.2 step 6): a new paragraph starts when the gap between speech segments is ≥ 1.5 s.
//! Segment timestamps are kept (absolute seconds) for future SRT/VTT export.

use serde::{Deserialize, Serialize};

pub const PARAGRAPH_GAP_S: f64 = 1.5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seg {
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub index: usize,
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

/// Incremental assembler: `push` returns paragraphs that became complete; `finish` flushes the last one.
#[derive(Debug, Default)]
pub struct Assembler {
    gap_s: f64,
    cur: Option<Paragraph>,
    next_index: usize,
}

impl Assembler {
    pub fn new(gap_s: f64) -> Self {
        Self {
            gap_s,
            cur: None,
            next_index: 0,
        }
    }
    pub fn standard() -> Self {
        Self::new(PARAGRAPH_GAP_S)
    }

    pub fn push(&mut self, seg: &Seg) -> Option<Paragraph> {
        let text = seg.text.trim();
        if text.is_empty() {
            return None;
        }
        match self.cur.as_mut() {
            Some(p) if seg.start_s - p.end_s < self.gap_s => {
                p.text.push(' ');
                p.text.push_str(text);
                p.end_s = seg.end_s.max(p.end_s);
                None
            }
            _ => {
                let done = self.cur.take();
                self.cur = Some(Paragraph {
                    index: self.next_index,
                    start_s: seg.start_s,
                    end_s: seg.end_s,
                    text: text.to_string(),
                });
                self.next_index += 1;
                done
            }
        }
    }

    pub fn finish(&mut self) -> Option<Paragraph> {
        self.cur.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(a: f64, b: f64, t: &str) -> Seg {
        Seg {
            start_s: a,
            end_s: b,
            text: t.into(),
        }
    }

    #[test]
    fn paragraphs_split_on_gaps_of_at_least_1_5_seconds() {
        let mut a = Assembler::standard();
        assert!(a.push(&s(0.0, 2.0, "First sentence.")).is_none());
        assert!(a.push(&s(2.5, 4.0, "Same paragraph.")).is_none()); // 0.5 s gap
        let p0 = a
            .push(&s(5.5, 7.0, "New paragraph."))
            .expect("1.5 s gap starts a new paragraph");
        assert_eq!(p0.text, "First sentence. Same paragraph.");
        assert_eq!((p0.index, p0.start_s, p0.end_s), (0, 0.0, 4.0));
        let last = a.finish().unwrap();
        assert_eq!((last.index, last.text.as_str()), (1, "New paragraph."));
        assert!(a.finish().is_none());
    }

    #[test]
    fn empty_segments_are_ignored() {
        let mut a = Assembler::standard();
        assert!(a.push(&s(0.0, 1.0, "  ")).is_none());
        assert!(a.finish().is_none());
    }
}
