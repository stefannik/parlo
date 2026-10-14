//! Turn VAD speech spans into transcription chunks of ≤ `max_s` seconds (BRIEF §7.2 step 2).
//! Cuts happen in silences; an over-long continuous span is split at the lowest-energy point.

use super::vad::Span;

#[derive(Debug, Clone, Copy)]
pub struct ChunkParams {
    pub max_s: f64,
    /// padding added on both sides of a chunk (clamped to the file), so words at the edges aren't clipped
    pub pad_s: f64,
    /// never cut inside the final `min_tail_s` of a span when hard-splitting
    pub min_piece_s: f64,
}
impl Default for ChunkParams {
    fn default() -> Self {
        Self {
            max_s: 30.0,
            pad_s: 0.2,
            min_piece_s: 5.0,
        }
    }
}

/// `lowest_energy(a, b)` returns the time in `[a, b]` of the quietest 20 ms frame (used for hard splits).
pub fn build_chunks(
    spans: &[Span],
    total_s: f64,
    p: ChunkParams,
    lowest_energy: &mut dyn FnMut(f64, f64) -> f64,
) -> Vec<Span> {
    // 1) hard-split spans longer than max_s
    let mut pieces: Vec<Span> = Vec::new();
    for s in spans {
        let mut cur = *s;
        // leave room for the padding so padded chunks stay ≤ max_s
        let limit = p.max_s - 2.0 * p.pad_s;
        while cur.len() > limit {
            let lo = (cur.start_s + p.min_piece_s).max(cur.start_s + limit - 5.0);
            let hi = cur.start_s + limit;
            let cut = lowest_energy(lo.min(hi), hi).clamp(cur.start_s + 1.0, hi);
            pieces.push(Span {
                start_s: cur.start_s,
                end_s: cut,
            });
            cur.start_s = cut;
        }
        if !cur.is_empty() {
            pieces.push(cur);
        }
    }
    // 2) greedy merge: add the next piece while the (padded) chunk stays within max_s
    let mut chunks: Vec<Span> = Vec::new();
    for pc in pieces {
        match chunks.last_mut() {
            Some(c) if pc.end_s - c.start_s <= p.max_s - 2.0 * p.pad_s => c.end_s = pc.end_s,
            _ => chunks.push(pc),
        }
    }
    // 3) pad, but never into the neighbour's speech: split the gap at its midpoint if padding would collide
    let raw = chunks.clone();
    let n = raw.len();
    for (i, c) in chunks.iter_mut().enumerate() {
        let start = if i == 0 {
            (raw[i].start_s - p.pad_s).max(0.0)
        } else {
            (raw[i].start_s - p.pad_s).max((raw[i - 1].end_s + raw[i].start_s) / 2.0)
        };
        let end = if i + 1 == n {
            (raw[i].end_s + p.pad_s).min(total_s)
        } else {
            (raw[i].end_s + p.pad_s).min((raw[i].end_s + raw[i + 1].start_s) / 2.0)
        };
        *c = Span {
            start_s: start,
            end_s: end.max(raw[i].end_s),
        };
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sp(a: f64, b: f64) -> Span {
        Span {
            start_s: a,
            end_s: b,
        }
    }
    fn mid(a: f64, b: f64) -> f64 {
        (a + b) / 2.0
    }

    #[test]
    fn nearby_spans_merge_up_to_30_seconds_and_cut_in_the_gap() {
        let spans = [
            sp(0.0, 10.0),
            sp(11.0, 20.0),
            sp(21.0, 29.0),
            sp(31.0, 40.0),
        ];
        let c = build_chunks(&spans, 60.0, ChunkParams::default(), &mut mid);
        assert_eq!(c.len(), 2, "{c:?}");
        assert!(c.iter().all(|x| x.len() <= 30.0), "{c:?}");
        assert!(
            c[0].end_s <= 30.0 && c[1].start_s >= 29.0,
            "cut must be in the silence between 29 and 31: {c:?}"
        );
    }

    #[test]
    fn long_continuous_speech_is_hard_split_at_the_quietest_point() {
        let mut calls = vec![];
        let mut quiet = |a: f64, b: f64| {
            calls.push((a, b));
            b - 1.0
        };
        let c = build_chunks(&[sp(0.0, 100.0)], 100.0, ChunkParams::default(), &mut quiet);
        assert!(c.len() >= 4);
        assert!(c.iter().all(|x| x.len() <= 30.0 + 1e-9), "{c:?}");
        assert!(!calls.is_empty());
        // contiguous coverage of the whole span
        assert!(
            c.first().unwrap().start_s <= 0.0 + 1e-9 && c.last().unwrap().end_s >= 100.0 - 1e-9
        );
    }

    #[test]
    fn padding_never_runs_into_a_neighbour() {
        let c = build_chunks(
            &[sp(5.0, 10.0), sp(40.0, 45.0)],
            60.0,
            ChunkParams::default(),
            &mut mid,
        );
        assert_eq!(c.len(), 2);
        assert!(c[0].start_s < 5.0 && c[0].end_s > 10.0);
        let c = build_chunks(
            &[sp(5.0, 27.0), sp(27.2, 40.0)],
            60.0,
            ChunkParams::default(),
            &mut mid,
        );
        assert!(
            c.windows(2).all(|w| w[0].end_s <= w[1].start_s + 1e-9),
            "{c:?}"
        );
    }

    #[test]
    fn no_speech_means_no_chunks() {
        assert!(build_chunks(&[], 60.0, ChunkParams::default(), &mut mid).is_empty());
    }
}
