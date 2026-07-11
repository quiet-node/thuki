//! Post-generation citation-span verification.
//!
//! A purely mechanical, zero-LLM diagnostic that measures how often the
//! writer's bracket citations (`[1]`, `[2]`, ...) are actually supported by the
//! source text they point at. The writer is prompted to ground every claim in
//! the numbered sources, but small local models sometimes cite a source they
//! did not really read; this audit quantifies that failure class from streamed
//! output alone so a later UX or enforcement decision can be data-driven.
//!
//! The check is deliberately crude and total: for each citation marker it takes
//! the sentence containing the marker as the "claim", tokenizes the claim into
//! content tokens (longer words plus every number-like token), and scores the
//! claim by the fraction of those tokens that also appear in the cited source's
//! text. The score is bucketed into supported / weak / unsupported against the
//! thresholds in [`crate::config::defaults`]. Nothing here calls a model or the
//! network; every function is pure over its inputs and never panics on
//! malformed text.

use crate::config::defaults::{CITE_SUPPORTED_MIN, CITE_WEAK_MIN};
use crate::websearch::assemble::SourceBlock;

/// The outcome of auditing one answer's citations against its sources.
///
/// `cited` is the total number of citation references seen (a comma group such
/// as `[1, 2]` counts as two). `supported`, `weak`, and `unsupported` partition
/// those references by how well the citing sentence is backed by the cited
/// source. `unsupported_indices` lists the source numbers (as written in the
/// answer, 1-based) that were classified unsupported, including out-of-range
/// numbers that match no source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationAudit {
    /// Total citation references seen (comma groups counted per index).
    pub cited: usize,
    /// References whose sentence is well backed by the cited source.
    pub supported: usize,
    /// References with partial backing (score in the weak band).
    pub weak: usize,
    /// References with little or no backing, plus out-of-range numbers.
    pub unsupported: usize,
    /// The 1-based source numbers classified unsupported, in first-seen order.
    pub unsupported_indices: Vec<usize>,
}

/// One citation reference extracted from the answer: the 1-based source number
/// it names and the byte range of the whole marker span in the answer (so the
/// marker's own digits are never mistaken for the claim's content). A
/// comma-grouped marker yields one `CitationRef` per number, all sharing the
/// same span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CitationRef {
    /// The 1-based source number as written (`[3]` -> 3).
    index: usize,
    /// Byte offset of the marker's opening `[` in the answer.
    span_start: usize,
    /// Byte offset just past the marker's closing `]` in the answer.
    span_end: usize,
}

/// Audits every citation marker in `answer_text` against `sources`.
///
/// For each marker: the claim is the sentence containing the marker (with the
/// marker span itself removed so the citation's digits do not count as claim
/// content), the support score is the fraction of the claim's content tokens
/// found in the cited source's text (case-insensitive), and the score is
/// bucketed by the audit thresholds. An index that matches no source counts as
/// unsupported and is recorded in `unsupported_indices`. A claim with no content
/// tokens at all counts as supported (there is nothing to contradict). Pure and
/// total: malformed text degrades gracefully and never panics.
pub fn audit_citations(answer_text: &str, sources: &[SourceBlock]) -> CitationAudit {
    let refs = find_citation_refs(answer_text);
    let sentences = sentence_spans(answer_text);

    let mut audit = CitationAudit {
        cited: 0,
        supported: 0,
        weak: 0,
        unsupported: 0,
        unsupported_indices: Vec::new(),
    };

    for cref in refs {
        audit.cited += 1;
        let source = sources.iter().find(|s| s.index == cref.index);
        let class = match source {
            // Out-of-range citation: no source to back it, so unsupported.
            None => CiteClass::Unsupported,
            Some(source) => {
                let claim = claim_text(answer_text, &sentences, &cref);
                classify(support_score(&claim, &source.text))
            }
        };
        match class {
            CiteClass::Supported => audit.supported += 1,
            CiteClass::Weak => audit.weak += 1,
            CiteClass::Unsupported => {
                audit.unsupported += 1;
                audit.unsupported_indices.push(cref.index);
            }
        }
    }

    audit
}

/// The support buckets a citation's score falls into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CiteClass {
    /// Score at or above [`CITE_SUPPORTED_MIN`].
    Supported,
    /// Score at or above [`CITE_WEAK_MIN`] but below supported.
    Weak,
    /// Score below [`CITE_WEAK_MIN`], or an out-of-range citation.
    Unsupported,
}

/// Buckets a support score into a [`CiteClass`] using the baked-in thresholds.
fn classify(score: f64) -> CiteClass {
    if score >= CITE_SUPPORTED_MIN {
        CiteClass::Supported
    } else if score >= CITE_WEAK_MIN {
        CiteClass::Weak
    } else {
        CiteClass::Unsupported
    }
}

/// Extracts every citation reference from `text`, mirroring the frontend's
/// lenient marker regex (`[N]`, `[1, 2]`, `[1,2]`, `[1 , 2]`). Scans for `[`,
/// then reads a comma-separated list of runs of ASCII digits separated only by
/// whitespace and commas, terminated by `]`. Anything that does not match this
/// exact shape (letters inside the brackets, an empty group, a missing `]`) is
/// skipped, leaving the scanner positioned to find later markers. A comma group
/// yields one reference per number, all carrying the same marker span so the
/// digits are excluded from the claim text.
fn find_citation_refs(text: &str) -> Vec<CitationRef> {
    let bytes = text.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        match parse_marker(bytes, i) {
            Some((indices, end)) => {
                for index in indices {
                    refs.push(CitationRef {
                        index,
                        span_start: i,
                        span_end: end,
                    });
                }
                i = end;
            }
            None => i += 1,
        }
    }
    refs
}

/// Parses a single citation marker beginning at `start` (which must index a
/// `[`). Returns the list of 1-based indices it names and the byte offset just
/// past the closing `]`, or `None` if the bytes from `start` are not a
/// well-formed comma-grouped numeric marker. Number-run overflow (an
/// absurdly long digit run) saturates rather than panicking; such a marker is
/// still parsed so the scanner advances past it.
fn parse_marker(bytes: &[u8], start: usize) -> Option<(Vec<usize>, usize)> {
    let mut i = start + 1; // past '['
    let mut indices = Vec::new();
    loop {
        // Skip whitespace before a number.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Require at least one digit.
        let digit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == digit_start {
            return None; // no digit where one was required
        }
        let mut value: usize = 0;
        for &b in &bytes[digit_start..i] {
            value = value.saturating_mul(10).saturating_add((b - b'0') as usize);
        }
        indices.push(value);
        // Skip whitespace after a number.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            Some(b']') => return Some((indices, i + 1)),
            Some(b',') => i += 1, // another index follows
            _ => return None,     // unexpected byte or end of input
        }
    }
}

/// Byte ranges of the claim-scope spans in `text`.
///
/// A newline is always a hard span boundary, split first: each line (a
/// markdown bullet item, a plain prose line, ...) becomes its own claim
/// scope before any punctuation-based splitting runs. This matters because
/// the model's usual multi-source answer format is a period-free bullet
/// list (`* $64,123 [3]`); without a newline boundary, a whole list with no
/// terminal punctuation collapses into one sentence span, and every
/// citation in it gets scored against every other bullet's text as noise. A
/// CRLF line ending has its trailing `\r` trimmed from the emitted span so
/// it never reads as line content; a lone `\r` with no following `\n` is
/// left as ordinary text. Within each line, the existing rule still
/// applies: `.`, `!`, or `?` followed by ASCII whitespace still closes a
/// sentence, so a multi-sentence prose line still yields multiple spans. A
/// blank line yields one trivial empty span. Empty input yields a single
/// empty span so lookups always resolve. Kept deliberately simple: this is
/// a claim-scoping heuristic, not a linguistic sentence splitter or a
/// markdown parser.
fn sentence_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut line_start = 0;
    loop {
        let newline_at = bytes[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|rel| line_start + rel);
        let raw_line_end = newline_at.unwrap_or(bytes.len());
        // Trim a trailing '\r' so a CRLF ending is never read as line
        // content. Only trim when an actual '\n' was found: a lone '\r' at
        // the very end of the text (no following '\n') is ordinary text.
        let line_end = if newline_at.is_some()
            && raw_line_end > line_start
            && bytes[raw_line_end - 1] == b'\r'
        {
            raw_line_end - 1
        } else {
            raw_line_end
        };
        split_line_into_sentences(text, line_start, line_end, &mut spans);
        match newline_at {
            // Resume just past the '\n'; the next line's own scan starts fresh.
            Some(n) => line_start = n + 1,
            None => break,
        }
    }
    spans
}

/// Splits one newline-delimited line, `text[start..end]`, into sentence spans
/// on `.`, `!`, or `?` followed by ASCII whitespace, appending them to `spans`.
/// A blank line (`start == end`) still contributes its own empty span so
/// every line maps to at least one span. The whitespace-skip bound
/// (`i + 1 < end`) keeps a terminator at the very end of a line from being
/// treated as "followed by whitespace": that whitespace, if any, belongs to
/// the next line's boundary handling, not this one's.
fn split_line_into_sentences(
    text: &str,
    start: usize,
    end: usize,
    spans: &mut Vec<(usize, usize)>,
) {
    let bytes = text.as_bytes();
    let mut seg_start = start;
    let mut i = start;
    while i < end {
        let b = bytes[i];
        let is_end = b == b'.' || b == b'!' || b == b'?';
        let next_is_ws = i + 1 < end && bytes[i + 1].is_ascii_whitespace();
        if is_end && next_is_ws {
            spans.push((seg_start, i + 1));
            // Advance past the terminator and the following whitespace run.
            i += 1;
            while i < end && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            seg_start = i;
        } else {
            i += 1;
        }
    }
    if seg_start < end {
        // Leftover content after the last split (or the whole line, if no
        // split happened at all).
        spans.push((seg_start, end));
    } else if start == end {
        // A genuinely blank line: still its own trivial empty span.
        spans.push((start, end));
    }
    // Otherwise the line's trailing terminator + whitespace already closed
    // the last span exactly at `end`; no empty tail to add (mirrors the
    // pre-existing rule that a terminator right before trailing whitespace
    // does not spawn an extra empty sentence).
}

/// Returns the claim text for a citation: the sentence containing the marker,
/// with the marker span itself excised so the citation's own digits never count
/// as claim content. The containing sentence is the first span whose range
/// covers the marker's start; if none do (a marker past the last split point),
/// the final sentence is used. String slicing stays on the char boundaries the
/// span offsets already fall on (sentence terminators and `[` are ASCII).
fn claim_text(text: &str, sentences: &[(usize, usize)], cref: &CitationRef) -> String {
    let (s, e) = sentences
        .iter()
        .copied()
        .find(|&(s, e)| cref.span_start >= s && cref.span_start < e)
        .unwrap_or_else(|| *sentences.last().expect("sentence_spans is never empty"));
    let sentence = &text[s..e];
    // Excise the marker span (offsets are absolute; rebase into the sentence
    // and clamp BOTH ends into it: a comma group's span can run past a
    // sentence split point, and on the fallback path the marker sits entirely
    // outside the chosen sentence, where an unclamped rebase would underflow
    // or slice out of bounds. Clamped, the excision degenerates to a no-op
    // and the whole sentence is the claim, which is the right reading for a
    // marker that belongs to no sentence).
    let rel_start = cref.span_start.saturating_sub(s).min(sentence.len());
    let rel_end = cref
        .span_end
        .saturating_sub(s)
        .min(sentence.len())
        .max(rel_start);
    let mut claim = String::with_capacity(sentence.len());
    claim.push_str(&sentence[..rel_start]);
    claim.push_str(&sentence[rel_end..]);
    claim
}

/// The fraction of `claim`'s content tokens that also appear in `source`,
/// case-insensitive. Content tokens are ASCII-lowercased alphanumeric runs that
/// are either longer than three characters or number-like (contain at least one
/// digit: scores, prices, ages, and dates are the load-bearing facts, so they
/// count regardless of length). A claim with no content tokens scores 1.0
/// (nothing to contradict). The source's own content tokens form the lookup
/// set, so matching is whole-token, not substring.
fn support_score(claim: &str, source: &str) -> f64 {
    let claim_tokens = content_tokens(claim);
    if claim_tokens.is_empty() {
        return 1.0;
    }
    let source_tokens: std::collections::HashSet<String> =
        content_tokens(source).into_iter().collect();
    let found = claim_tokens
        .iter()
        .filter(|t| source_tokens.contains(*t))
        .count();
    found as f64 / claim_tokens.len() as f64
}

/// Splits `text` into lowercased content tokens: maximal runs of ASCII
/// alphanumerics, kept only when longer than three characters or number-like
/// (containing a digit). Non-alphanumeric bytes are separators. Non-ASCII bytes
/// are treated as token content so accented words are not shredded; a run
/// counts as number-like only via ASCII digits, which is all the facts we care
/// about (`3.5`, `2026`, `$42`) use.
fn content_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || !ch.is_ascii() {
            current.extend(ch.to_lowercase());
        } else {
            push_token(&mut tokens, &mut current);
        }
    }
    push_token(&mut tokens, &mut current);
    tokens
}

/// Moves `current` into `tokens` when it qualifies as a content token (longer
/// than three chars or containing an ASCII digit), then clears it. A no-op for
/// an empty or too-short non-numeric run.
fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        let is_number_like = current.chars().any(|c| c.is_ascii_digit());
        if current.chars().count() > 3 || is_number_like {
            tokens.push(std::mem::take(current));
        } else {
            current.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(index: usize, text: &str) -> SourceBlock {
        SourceBlock {
            index,
            url: format!("https://example.test/{index}"),
            title: format!("Source {index}"),
            text: text.into(),
        }
    }

    // ── content_tokens / push_token ───────────────────────────────────────────

    #[test]
    fn content_tokens_keeps_long_words_and_all_numbers() {
        let toks = content_tokens("The cat ate 3 big fish at 4pm.");
        // "the" (3 chars) dropped, "cat"/"ate"/"big" dropped, "3"/"4pm" kept as
        // number-like, "fish" kept as long, lowercased throughout.
        assert_eq!(toks, vec!["3", "fish", "4pm"]);
    }

    #[test]
    fn content_tokens_empty_input_is_empty() {
        assert!(content_tokens("   .,;  ").is_empty());
    }

    // ── support_score ─────────────────────────────────────────────────────────

    #[test]
    fn support_score_full_overlap_is_one() {
        let claim = "Photosynthesis converts sunlight into chemical energy";
        let src = "Photosynthesis converts sunlight into chemical energy in plants.";
        assert_eq!(support_score(claim, src), 1.0);
    }

    #[test]
    fn support_score_no_content_tokens_is_one() {
        // Only short non-numeric words: no content tokens, nothing to contradict.
        assert_eq!(support_score("it is on to", "unrelated source text"), 1.0);
    }

    #[test]
    fn support_score_partial_overlap_is_fraction() {
        // Two content tokens in the claim, one present in the source.
        let score = support_score("alpha bravo", "the alpha appears here");
        assert!((score - 0.5).abs() < f64::EPSILON, "score was {score}");
    }

    // ── find_citation_refs / parse_marker ─────────────────────────────────────

    #[test]
    fn finds_single_and_comma_group_markers() {
        let refs = find_citation_refs("A [1] and B [2, 3] end.");
        let idx: Vec<usize> = refs.iter().map(|r| r.index).collect();
        assert_eq!(idx, vec![1, 2, 3]);
        // The comma group's two refs share one span.
        assert_eq!(refs[1].span_start, refs[2].span_start);
        assert_eq!(refs[1].span_end, refs[2].span_end);
    }

    #[test]
    fn skips_malformed_brackets() {
        // Letters, empty brackets, and an unterminated marker all skipped.
        let refs = find_citation_refs("[x] [] [12 no bracket and [7]");
        let idx: Vec<usize> = refs.iter().map(|r| r.index).collect();
        assert_eq!(idx, vec![7]);
    }

    #[test]
    fn parse_marker_saturates_on_absurd_run() {
        // A digit run far past usize range parses (saturating) instead of
        // panicking, so the scanner still advances.
        let long = "9".repeat(40);
        let text = format!("[{long}]");
        let refs = find_citation_refs(&text);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].index, usize::MAX);
    }

    // ── sentence_spans / claim_text ───────────────────────────────────────────

    #[test]
    fn sentence_spans_split_on_terminator_then_space() {
        let text = "One fact. Two fact! Three?";
        let spans = sentence_spans(text);
        let sents: Vec<&str> = spans.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(sents, vec!["One fact.", "Two fact!", "Three?"]);
    }

    #[test]
    fn sentence_spans_empty_input_has_one_empty_span() {
        assert_eq!(sentence_spans(""), vec![(0, 0)]);
    }

    #[test]
    fn bullet_list_without_periods_splits_one_span_per_line() {
        // The regression itself: a period-free markdown list must not
        // collapse into a single sentence span.
        let text = "* first item\n* second item\n* third item";
        let spans = sentence_spans(text);
        let lines: Vec<&str> = spans.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(lines, vec!["* first item", "* second item", "* third item"]);
    }

    #[test]
    fn blank_lines_get_their_own_trivial_span() {
        let text = "first\n\nthird";
        let spans = sentence_spans(text);
        let lines: Vec<&str> = spans.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(lines, vec!["first", "", "third"]);
    }

    #[test]
    fn crlf_line_endings_split_and_trim_trailing_cr() {
        let text = "* first [1]\r\n* second [2]\r\n";
        let spans = sentence_spans(text);
        let lines: Vec<&str> = spans.iter().map(|&(s, e)| &text[s..e]).collect();
        // Trailing '\r' is trimmed from each line; the final '\r\n' also
        // opens a trailing blank line, same as a bare trailing '\n' would.
        assert_eq!(lines, vec!["* first [1]", "* second [2]", ""]);
    }

    #[test]
    fn mixed_prose_and_list_keeps_period_splitting_within_a_line() {
        let text =
            "Prices vary widely across regions. Here is the breakdown:\n* $10 [1]\n* $20 [2]";
        let spans = sentence_spans(text);
        let chunks: Vec<&str> = spans.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(
            chunks,
            vec![
                "Prices vary widely across regions.",
                "Here is the breakdown:",
                "* $10 [1]",
                "* $20 [2]",
            ]
        );
    }

    #[test]
    fn utf8_heavy_and_hostile_input_never_panics() {
        let text =
            "\u{1F600}\u{1F525} caf\u{e9}. na\u{ef}ve\r\n\n[1] \u{4e2d}\u{6587}\u{884c} [2]\r broken [ [999999999999999999999999] \n\n\n";
        let spans = sentence_spans(text);
        // Every span must be a valid, in-bounds, char-boundary-safe slice.
        for &(s, e) in &spans {
            assert!(s <= e && e <= text.len());
            let _ = &text[s..e];
        }
        let _ = audit_citations(text, &[]);
    }

    #[test]
    fn claim_text_excises_marker_and_scopes_to_sentence() {
        let text = "Alpha beta [1] gamma. Delta epsilon.";
        let refs = find_citation_refs(text);
        let spans = sentence_spans(text);
        let claim = claim_text(text, &spans, &refs[0]);
        // Marker digits removed, second sentence excluded.
        assert_eq!(claim, "Alpha beta  gamma.");
    }

    #[test]
    fn claim_text_marker_past_last_split_uses_final_sentence() {
        // No terminator: one sentence covers the whole answer, marker included.
        let text = "Only sentence with cite [2]";
        let refs = find_citation_refs(text);
        let spans = sentence_spans(text);
        let claim = claim_text(text, &spans, &refs[0]);
        assert_eq!(claim, "Only sentence with cite ");
    }

    #[test]
    fn claim_text_out_of_span_marker_falls_back_to_final_sentence() {
        // Defensive fallback contract, driven directly: `audit_citations`
        // cannot produce a marker whose start sits outside every sentence span
        // (a marker starts with '[', never whitespace, and the trailing
        // segment is always spanned), but `claim_text` is pure over its
        // inputs, so the fallback to the final sentence is pinned here with a
        // hand-built reference rather than left as untested dead weight.
        let text = "First fact. Second fact. ";
        let spans = sentence_spans(text);
        // The text ends in a terminator plus whitespace, so no trailing span
        // is emitted and a start offset at text end is outside every span.
        let cref = CitationRef {
            index: 1,
            span_start: text.len(),
            span_end: text.len(),
        };
        let claim = claim_text(text, &spans, &cref);
        // Falls back to the last sentence; the marker excision degenerates to
        // appending nothing (relative offsets clamp to the sentence end).
        assert_eq!(claim, "Second fact.");
    }

    // ── audit_citations (end to end) ──────────────────────────────────────────

    #[test]
    fn no_markers_is_all_zero_audit() {
        let audit = audit_citations("A plain answer with no citations.", &[source(1, "x")]);
        assert_eq!(
            audit,
            CitationAudit {
                cited: 0,
                supported: 0,
                weak: 0,
                unsupported: 0,
                unsupported_indices: vec![],
            }
        );
    }

    #[test]
    fn extractive_claim_is_supported() {
        let src = source(
            1,
            "The Eiffel Tower stands 330 metres tall in Paris, completed in 1889.",
        );
        let answer = "The Eiffel Tower stands 330 metres tall in Paris [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.supported, 1);
        assert_eq!(audit.weak, 0);
        assert_eq!(audit.unsupported, 0);
    }

    #[test]
    fn fabricated_numbers_are_unsupported() {
        // The source has no figures; the claim invents a price and a year. The
        // number-like tokens "499" and "2027" are absent from the source, so the
        // digit facts drive the score below the weak band.
        let src = source(
            1,
            "The company announced a new phone at its event with several colours.",
        );
        let answer = "The phone costs 499 dollars launching in 2027 quarter [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unsupported, 1);
        assert_eq!(audit.supported, 0);
        assert_eq!(audit.unsupported_indices, vec![1]);
    }

    #[test]
    fn partial_overlap_is_weak() {
        // Exactly half the claim's six content tokens appear in the source, so
        // the score (0.5) lands in the weak band [0.3, 0.6).
        let src = source(1, "Mercury orbits closest around.");
        let answer = "Mercury orbits closest through frozen distant [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.weak, 1);
        assert_eq!(audit.supported, 0);
        assert_eq!(audit.unsupported, 0);
    }

    #[test]
    fn out_of_range_index_is_unsupported_and_recorded() {
        let src = source(1, "Only one source exists here.");
        let answer = "A claim citing a missing source [9].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unsupported, 1);
        assert_eq!(audit.unsupported_indices, vec![9]);
    }

    #[test]
    fn comma_group_audits_each_index() {
        let s1 = source(1, "Alpha beta gamma delta epsilon zeta eta theta.");
        let s2 = source(2, "Completely different unrelated wording nothing shared.");
        // One sentence, one marker, two indices: index 1 well backed, index 2 not.
        let answer = "Alpha beta gamma delta epsilon zeta [1, 2].";
        let audit = audit_citations(answer, &[s1, s2]);
        assert_eq!(audit.cited, 2);
        assert_eq!(audit.supported, 1);
        assert_eq!(audit.unsupported, 1);
        assert_eq!(audit.unsupported_indices, vec![2]);
    }

    #[test]
    fn multi_sentence_attributes_each_marker_to_its_own_sentence() {
        let s1 = source(1, "Jupiter is the largest planet in the solar system.");
        let s2 = source(2, "Saturn is famous for its prominent ring system.");
        let answer = "Jupiter is the largest planet [1]. Saturn is famous for its rings [2].";
        let audit = audit_citations(answer, &[s1, s2]);
        assert_eq!(audit.cited, 2);
        // Each claim is scored against its own source's text, both supported.
        assert_eq!(audit.supported, 2);
        assert_eq!(audit.unsupported, 0);
    }

    #[test]
    fn claim_with_no_content_tokens_counts_supported() {
        // The sentence around the marker has only short non-numeric words.
        let src = source(1, "totally unrelated source body text here now");
        let answer = "It is on to it [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.supported, 1);
    }

    #[test]
    fn newline_scopes_each_citation_to_its_own_line() {
        // Regression guard: if newline splitting regressed, citation 2 would
        // be scored against the merged blob (line 1's 8 tokens plus its own
        // 2), diluting its score from weak (0.5, correctly scoped) down to
        // unsupported (~0.09, merged) even though nothing about its own
        // line's overlap changed.
        let s1 = source(1, "alpha bravo charlie delta echo foxtrot golf hotel");
        let s2 = source(2, "juliet only, nothing else here");
        let answer = "alpha bravo charlie delta echo foxtrot golf hotel [1]\nindia juliet [2]";
        let audit = audit_citations(answer, &[s1, s2]);
        assert_eq!(audit.cited, 2);
        assert_eq!(audit.supported, 1);
        assert_eq!(audit.weak, 1);
        assert_eq!(audit.unsupported, 0);
    }

    #[test]
    fn bullet_list_regression_from_live_smoke_all_supported() {
        // The exact failure class from the 2026-07-11 smoke: a bullet list
        // of per-source figures (`* $64,123 [3]`-style lines), each line's
        // own number present verbatim in its own cited source. Before the
        // newline fix, all three lines merged into one sentence span, so
        // every citation was scored against the other two lines' numbers as
        // noise and came back diluted below the support thresholds. Scoped
        // per line, all three are cleanly supported.
        let s1 = source(1, "Total revenue reached $64,123 in the reported period.");
        let s2 = source(2, "The measured metric was $58,000 across all regions.");
        let s3 = source(3, "Final tally came to $71,500 for the fiscal year.");
        let answer = "* $64,123 [1]\n* $58,000 [2]\n* $71,500 [3]";
        let audit = audit_citations(answer, &[s1, s2, s3]);
        assert_eq!(audit.cited, 3);
        assert_eq!(audit.supported, 3);
        assert_eq!(audit.weak, 0);
        assert_eq!(audit.unsupported, 0);
    }
}
