//! Post-filters (BRIEF §7.2 step 5), adapted from ctric's conversation-mode heuristics
//! (`docs/reference/ctric/frontend/ChatConversationMode.vue`): energy gate before transcribing, hallucination filter
//! for near-silent chunks, no-speech/low-confidence segment drop, repeated n-gram loop collapse.

use super::engine::SegmentOut;
use regex::Regex;
use std::sync::OnceLock;

// ctric constants
pub const MIN_TRANSCRIBE_RMS: f32 = 0.004;
pub const MIN_TRANSCRIBE_PEAK: f32 = 0.02;
pub const MIN_TRANSCRIBE_VOICED_RATIO: f32 = 0.012;
pub const LOW_CONFIDENCE_RMS: f32 = 0.009;
pub const LOW_CONFIDENCE_VOICED_RATIO: f32 = 0.035;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    pub rms: f32,
    pub peak: f32,
    /// fraction of samples with |x| ≥ MIN_TRANSCRIBE_PEAK
    pub voiced_ratio: f32,
}

pub fn analyze(samples: &[f32]) -> Analysis {
    if samples.is_empty() {
        return Analysis {
            rms: 0.0,
            peak: 0.0,
            voiced_ratio: 0.0,
        };
    }
    let (mut sq, mut peak, mut voiced) = (0f64, 0f32, 0usize);
    for s in samples {
        let a = s.abs();
        sq += (*s as f64) * (*s as f64);
        peak = peak.max(a);
        if a >= MIN_TRANSCRIBE_PEAK {
            voiced += 1;
        }
    }
    Analysis {
        rms: (sq / samples.len() as f64).sqrt() as f32,
        peak,
        voiced_ratio: voiced as f32 / samples.len() as f32,
    }
}

/// Gate: chunks that are essentially silence are never sent to the model (prevents hallucinations, saves time).
pub fn should_transcribe(a: &Analysis) -> bool {
    a.peak >= MIN_TRANSCRIBE_PEAK
        && a.rms >= MIN_TRANSCRIBE_RMS
        && a.voiced_ratio >= MIN_TRANSCRIBE_VOICED_RATIO
}

fn hallucination_patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            r"^thank you[.!]?$",
            r"^thanks[.!]?$",
            r"^thanks for watching[.!]?$",
            r"^thank you for watching[.!]?$",
            r"^you[.!]?$",
            r"^bye[.!]?$",
            r"^(please )?(like and )?subscribe[^.]*[.!]?$",
            r"subtitles? (by|made by)\b",
            r"amara\.org",
            r"^untertitel(ung)? (der|von|im auftrag)\b",
            r"^sous-titres?\b",
            r"^subtítulos\b",
            r"^sottotitoli\b",
            r"^legendas?\b",
            r"^продолжение следует",
            r"^субтитры\b",
            r"^редактор субтитров",
            r"^благодаря(м)? за (просмотр|внимание)",
            r"^субтитри\b",
            r"^благодаря за (гледането|вниманието)",
            r"^(gracias|merci|danke|grazie|obrigad[oa]) (por|pour|für|per) (ver|regarder|zuschauen|guardare|assistir)",
        ]
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).expect("valid regex"))
        .collect()
    })
}

pub fn is_known_hallucination(text: &str) -> bool {
    let t = text.trim().trim_matches(|c: char| {
        c == '"' || c == '\'' || c == '♪' || c == '[' || c == ']' || c == '(' || c == ')'
    });
    let t = t.trim();
    hallucination_patterns().iter().any(|p| p.is_match(t))
}

/// Drop decision for a segment given the chunk's audio analysis (BRIEF §7.2 step 5).
pub fn should_drop_segment(seg: &SegmentOut, chunk: &Analysis) -> bool {
    let text = seg.text.trim();
    if text.is_empty() {
        return true;
    }
    // known silence-hallucination phrases are dropped when the chunk is low-energy (ctric rule)…
    let low_energy =
        chunk.rms < LOW_CONFIDENCE_RMS && chunk.voiced_ratio < LOW_CONFIDENCE_VOICED_RATIO;
    if low_energy && is_known_hallucination(text) {
        return true;
    }
    // …and also when the model itself says "probably no speech" with low confidence
    if seg.no_speech_prob > 0.6 && seg.avg_logprob < -1.0 {
        return true;
    }
    // pure music/noise annotations
    if text
        .chars()
        .all(|c| matches!(c, '♪' | '♫' | '-' | '.' | ' ' | '*'))
    {
        return true;
    }
    false
}

/// Collapse ≥ 3× consecutive repeats of an n-gram (n = 2..=8) and ≥ 4× repeats of a single word to one occurrence
/// (decoder loops: "the the the the…"). Genuine short repeats ("no no no") are left alone.
pub fn collapse_repeats(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let norm = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let n_words = words.len();
    let mut out: Vec<&str> = Vec::with_capacity(n_words);
    let mut i = 0;
    'outer: while i < n_words {
        for n in 1..=8usize {
            let min_rep = if n == 1 { 4 } else { 3 };
            if i + n * min_rep > n_words {
                continue;
            }
            let gram: Vec<String> = words[i..i + n].iter().map(|w| norm(w)).collect();
            if gram.iter().all(|g| g.is_empty()) {
                continue;
            }
            let mut reps = 1;
            while i + n * (reps + 1) <= n_words
                && words[i + n * reps..i + n * (reps + 1)]
                    .iter()
                    .map(|w| norm(w))
                    .eq(gram.iter().cloned())
            {
                reps += 1;
            }
            if reps >= min_rep {
                out.extend_from_slice(&words[i..i + n]);
                i += n * reps;
                continue 'outer;
            }
        }
        out.push(words[i]);
        i += 1;
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, nsp: f32, lp: f32) -> SegmentOut {
        SegmentOut {
            start_s: 0.0,
            end_s: 1.0,
            text: text.into(),
            no_speech_prob: nsp,
            avg_logprob: lp,
        }
    }
    const QUIET: Analysis = Analysis {
        rms: 0.002,
        peak: 0.01,
        voiced_ratio: 0.0,
    };
    const LOUD: Analysis = Analysis {
        rms: 0.1,
        peak: 0.6,
        voiced_ratio: 0.5,
    };

    #[test]
    fn energy_gate_rejects_silence_and_accepts_speech_levels() {
        assert!(!should_transcribe(&analyze(&vec![0.0; 16_000])));
        assert!(!should_transcribe(&analyze(&vec![0.001; 16_000])));
        let speechy: Vec<f32> = (0..16_000)
            .map(|i| ((i as f32) * 0.05).sin() * 0.2)
            .collect();
        assert!(should_transcribe(&analyze(&speechy)));
    }

    #[test]
    fn hallucination_phrases_are_dropped_only_for_quiet_chunks() {
        for t in [
            "Thank you.",
            "Thanks for watching!",
            "you",
            "Subtitles by the Amara.org community",
            "Untertitel der Amara.org-Community",
            "Продолжение следует...",
            "Sous-titres réalisés par la communauté d'Amara.org",
        ] {
            assert!(should_drop_segment(&seg(t, 0.0, -0.1), &QUIET), "{t}");
            assert!(
                !should_drop_segment(&seg("Thank you.", 0.0, -0.1), &LOUD),
                "a real 'Thank you.' in loud audio must survive"
            );
        }
    }

    #[test]
    fn model_says_no_speech_drops_low_confidence_segments() {
        assert!(should_drop_segment(&seg("blah blah", 0.9, -1.5), &LOUD));
        assert!(
            !should_drop_segment(&seg("blah blah", 0.9, -0.3), &LOUD),
            "confident text is kept even if no_speech is high"
        );
        assert!(should_drop_segment(&seg("   ", 0.0, 0.0), &LOUD));
        assert!(should_drop_segment(&seg("♪ ♪", 0.0, 0.0), &LOUD));
    }

    #[test]
    fn repeated_ngram_loops_collapse() {
        assert_eq!(
            collapse_repeats("and then and then and then and then we left"),
            "and then we left"
        );
        assert_eq!(collapse_repeats("the the the the the end"), "the end");
        assert_eq!(
            collapse_repeats("I said no no no and left"),
            "I said no no no and left",
            "short genuine repeats stay"
        );
        assert_eq!(collapse_repeats("ha ha ha ha ha ha"), "ha");
        assert_eq!(
            collapse_repeats("normal sentence without loops."),
            "normal sentence without loops."
        );
        assert_eq!(collapse_repeats("a b c a b c a b c d"), "a b c d");
    }
}
