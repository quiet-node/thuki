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
//!
//! A second, independent layer runs alongside the lexical score: a
//! numeric-consistency guard that extracts money figures, plain numbers,
//! percentages, and dates from the claim and checks each one against the
//! cited source's own numeric mentions, normalizing formatting differences
//! ("$615B" and "615 billion" and "615,000,000,000" all read as the same
//! value) so a real match is never missed on formatting alone. This exists
//! because token overlap alone cannot tell a swapped digit from a real
//! match: a sentence can be almost entirely correct wording with one
//! fabricated figure and still score high on lexical overlap. The guard
//! cannot raise a citation past what the lexical score already earned; it
//! can only cap a citation with a fabricated or absent figure down to
//! unsupported, or float an exact numeric match that lexical overlap missed
//! up to at least weak. See [`classify_with_numeric_guard`] for the exact
//! combination rule.
//!
//! A citation whose source text is too thin to check anything against (empty,
//! or below [`CITE_UNVERIFIABLE_MIN_SOURCE_BYTES`], the live-observed shape of
//! a JS-widget single-page-app result) is classified "unverifiable" instead of
//! "unsupported": there is no evidence the claim is wrong, only that it could
//! not be checked, so it is scored and treated separately (see
//! [`CiteClass::Unverifiable`]) and never counted toward the answer-facing
//! total-failure note built by [`honest_failure_note`].
//!
//! After the audit, Thuki may run a small number of writer repair rounds
//! (see [`crate::config::defaults::CITE_REPAIR_MAX_ATTEMPTS`]) and then
//! deterministically [`strip_unsupported_citations`]. Only a total
//! citation failure still surfaces an honest note to the user; partial
//! failures are cleaned without a guilt footer.

use crate::config::defaults::{
    CITE_MAGNITUDE_ABBREVIATIONS, CITE_MAGNITUDE_WORDS, CITE_MONTH_NAMES, CITE_SUPPORTED_MIN,
    CITE_UNVERIFIABLE_MIN_SOURCE_BYTES, CITE_WEAK_MIN,
};
use crate::websearch::assemble::SourceBlock;
use std::collections::{BTreeMap, HashSet};

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
    /// References whose cited source had too little text to check anything
    /// against (empty or below [`CITE_UNVERIFIABLE_MIN_SOURCE_BYTES`]).
    /// Never counted in `unsupported` and never named in
    /// `unsupported_indices`: there is no evidence of a wrong claim here,
    /// just an unverifiable one.
    pub unverifiable: usize,
    /// The 1-based source numbers classified unsupported, in first-seen order.
    pub unsupported_indices: Vec<usize>,
    /// Total claim numbers and dates checked by the numeric-consistency
    /// guard, summed across every citation that had both a resolvable
    /// source and at least one numeric or date mention in its claim.
    pub numeric_checked: usize,
    /// Of `numeric_checked`, how many were found in their cited source.
    pub numeric_matched: usize,
    /// Of `numeric_checked`, how many were absent from their cited source.
    /// Each one caps its citation's classification at unsupported; see
    /// [`classify_with_numeric_guard`].
    pub numeric_missing: usize,
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
/// tokens at all counts as supported (there is nothing to contradict). A
/// source whose text is empty or below [`CITE_UNVERIFIABLE_MIN_SOURCE_BYTES`]
/// is never scored at all: the citation is classified unverifiable instead,
/// since neither the lexical scorer nor the numeric guard has anything
/// substantive to check the claim against. Pure and total: malformed text
/// degrades gracefully and never panics.
pub fn audit_citations(answer_text: &str, sources: &[SourceBlock]) -> CitationAudit {
    let refs = find_citation_refs(answer_text);
    let sentences = sentence_spans(answer_text);

    let mut audit = CitationAudit {
        cited: 0,
        supported: 0,
        weak: 0,
        unsupported: 0,
        unverifiable: 0,
        unsupported_indices: Vec::new(),
        numeric_checked: 0,
        numeric_matched: 0,
        numeric_missing: 0,
    };

    for cref in refs {
        audit.cited += 1;
        let source = sources.iter().find(|s| s.index == cref.index);
        let class = match source {
            // Out-of-range citation: no source to back it, so unsupported.
            None => CiteClass::Unsupported,
            // Too little source text to check anything against: neither
            // score nor guard can say anything meaningful, so this is
            // unverifiable rather than unsupported.
            Some(source) if source.text.len() < CITE_UNVERIFIABLE_MIN_SOURCE_BYTES => {
                CiteClass::Unverifiable
            }
            Some(source) => {
                let claim = claim_text(answer_text, &sentences, &cref);
                let lexical_class = classify(support_score(&claim, &source.text));
                let claim_facts = extract_numeric_facts(&claim);
                // Only scan the (potentially large) source text when the
                // claim actually has a number or date to check against it.
                let (checked, missing) =
                    if claim_facts.numbers.is_empty() && claim_facts.dates.is_empty() {
                        (0, 0)
                    } else {
                        let source_facts = extract_numeric_facts(&source.text);
                        numeric_guard(&claim_facts, &source_facts)
                    };
                audit.numeric_checked += checked;
                audit.numeric_missing += missing;
                audit.numeric_matched += checked - missing;
                classify_with_numeric_guard(lexical_class, checked, missing)
            }
        };
        match class {
            CiteClass::Supported => audit.supported += 1,
            CiteClass::Weak => audit.weak += 1,
            CiteClass::Unsupported => {
                audit.unsupported += 1;
                audit.unsupported_indices.push(cref.index);
            }
            CiteClass::Unverifiable => audit.unverifiable += 1,
        }
    }

    audit
}

/// True when the answer cited at least one source and **every** citation was
/// unsupported (none supported or weak). That is the only case where we still
/// surface an honest failure note after repair attempts are exhausted; partial
/// failures are stripped silently instead of shaming the user with a footer.
pub fn is_total_citation_failure(audit: &CitationAudit) -> bool {
    audit.cited > 0
        && audit.supported == 0
        && audit.weak == 0
        && !audit.unsupported_indices.is_empty()
}

/// Distinct unsupported source indices in first-seen order (deduped).
///
/// Pure helper shared by repair critique wording and failure notes.
pub fn distinct_unsupported_indices(audit: &CitationAudit) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut distinct = Vec::new();
    for &index in &audit.unsupported_indices {
        if seen.insert(index) {
            distinct.push(index);
        }
    }
    distinct
}

/// Formats `[n]` markers for a list of source indices, comma-separated.
fn format_index_markers(indices: &[usize]) -> String {
    indices
        .iter()
        .map(|i| format!("[{i}]"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// User-facing note used only after repair rounds are exhausted and **every**
/// citation still fails the audit. Speaks as Thuki owning the failure, not as
/// "the sources are unreliable," and optionally points at a stronger model.
/// Returns `None` when the audit is not a total failure.
pub fn honest_failure_note(audit: &CitationAudit) -> Option<String> {
    if !is_total_citation_failure(audit) {
        return None;
    }
    Some(
        "*Thuki could not confirm the cited facts against the pages it retrieved. \
         Try rephrasing, or switch to a larger model in Settings for a stronger answer.*"
            .to_string(),
    )
}

/// Builds the user-turn critique sent back to the writer on a repair round.
/// Names the failing `[n]` indices so the model knows which citations to drop
/// or re-ground. Pure: does not include source bodies (those stay in the
/// earlier writer messages).
pub fn repair_critique(audit: &CitationAudit) -> String {
    let markers = format_index_markers(&distinct_unsupported_indices(audit));
    format!(
        "Automatic citation check failed. These source numbers do not support the \
         claim(s) next to them: {markers}.\n\n\
         Rewrite the full answer from scratch using only the web sources already \
         provided. Rules:\n\
         - Place [n] only when that source's text actually contains the figure or fact.\n\
         - Do not invent numbers or dates.\n\
         - Prefer fewer accurate citations over many weak ones.\n\
         - Output only the rewritten answer, with no preamble or apology."
    )
}

/// Removes unsupported citation markers from `answer`, rewriting grouped
/// markers (`[1, 3]`) to keep only the still-supported indices. Spans with no
/// remaining indices are deleted; a single preceding space is trimmed when
/// present so "word [1]." becomes "word." Pure and total: never panics.
pub fn strip_unsupported_citations(answer: &str, unsupported_indices: &[usize]) -> String {
    if unsupported_indices.is_empty() || answer.is_empty() {
        return answer.to_string();
    }
    let drop: HashSet<usize> = unsupported_indices.iter().copied().collect();
    let refs = find_citation_refs(answer);
    // span_start -> (span_end, indices in that span, first-seen order)
    let mut spans: BTreeMap<usize, (usize, Vec<usize>)> = BTreeMap::new();
    for r in refs {
        let entry = spans
            .entry(r.span_start)
            .or_insert((r.span_end, Vec::new()));
        entry.0 = r.span_end;
        entry.1.push(r.index);
    }
    // Apply from the end so earlier offsets stay valid.
    let mut out = answer.to_string();
    for (&start, &(end, ref indices)) in spans.iter().rev() {
        let kept: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|i| !drop.contains(i))
            .collect();
        // Dedupe kept while preserving order inside the span.
        let mut seen = HashSet::new();
        let kept: Vec<usize> = kept.into_iter().filter(|i| seen.insert(*i)).collect();
        let replacement = if kept.is_empty() {
            String::new()
        } else {
            // Writer style: one index per bracket group.
            kept.iter().map(|i| format!("[{i}]")).collect::<String>()
        };
        let mut replace_start = start;
        // Drop one space immediately before a fully removed marker.
        if replacement.is_empty()
            && replace_start > 0
            && out.as_bytes().get(replace_start - 1) == Some(&b' ')
        {
            replace_start -= 1;
        }
        // Spans come from parse of `out`'s prior state; applying from the end
        // keeps offsets valid, so the range is always in bounds.
        out.replace_range(replace_start..end, &replacement);
    }
    out
}

/// Applies post-repair cleanup: strip remaining bad citations; append the
/// honest total-failure note only when nothing citable remains supported.
/// Pure over audit + answer text.
pub fn finalize_answer_after_audit(answer: &str, audit: &CitationAudit) -> String {
    if audit.unsupported_indices.is_empty() {
        return answer.to_string();
    }
    let stripped = strip_unsupported_citations(answer, &audit.unsupported_indices);
    match honest_failure_note(audit) {
        Some(note) => {
            if stripped.trim().is_empty() {
                note
            } else {
                format!("{stripped}\n\n{note}")
            }
        }
        None => stripped,
    }
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
    /// The cited source's text is empty or below
    /// [`CITE_UNVERIFIABLE_MIN_SOURCE_BYTES`]: nothing substantive to score
    /// the claim against, so the citation is neither trusted nor accused.
    Unverifiable,
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

/// The numeric and date mentions extracted from a span of text: money
/// figures, plain numbers, percentages, and magnitude-suffixed forms are all
/// normalized into `numbers`; calendar dates (ISO, `M/D/YYYY`, or an English
/// month name) are normalized into `dates`. Used by the numeric-consistency
/// guard in [`audit_citations`] to check a claim's figures against its cited
/// source without relying on lexical token overlap, which cannot tell a
/// swapped digit from a real match.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NumericFacts {
    /// Canonical value strings for every number, money, or percentage
    /// mention, in first-seen order. A percentage's canonical form carries a
    /// trailing `%` so it never collides with a plain number of the same
    /// magnitude.
    numbers: Vec<String>,
    /// Canonical `YYYY-MM-DD` strings for every date mention, in first-seen
    /// order.
    dates: Vec<String>,
}

/// Extracts every numeric and date mention from `text` for the citation
/// audit's numeric-consistency guard. Citation markers (`[6]`, `[1, 2]`) are
/// located first via [`find_citation_refs`] and excluded from the scan so a
/// bracket's own digits are never read as claim or source content; a date
/// pattern is then matched before a bare number at the same position so a
/// date's day and year are not also counted as separate plain numbers. Pure
/// and total: every scan only ever advances forward, so malformed or hostile
/// input still terminates.
fn extract_numeric_facts(text: &str) -> NumericFacts {
    let bytes = text.as_bytes();
    let marker_spans: Vec<(usize, usize)> = find_citation_refs(text)
        .into_iter()
        .map(|r| (r.span_start, r.span_end))
        .collect();
    let date_spans = find_date_spans(text, bytes);
    let mut exclude = marker_spans;
    exclude.extend(date_spans.iter().map(|&(s, e, _)| (s, e)));
    let numbers = find_number_spans(text, bytes, &exclude);
    let dates = date_spans.into_iter().map(|(_, _, canon)| canon).collect();
    NumericFacts { numbers, dates }
}

/// True if byte offset `i` falls inside any of `spans` (half-open ranges).
/// Used to keep the number scan from re-reading digits already claimed by a
/// citation marker or a date match.
fn in_excluded(i: usize, spans: &[(usize, usize)]) -> bool {
    spans.iter().any(|&(s, e)| i >= s && i < e)
}

/// Finds every calendar-date mention in `text`: an ISO `YYYY-MM-DD` date, a
/// `M/D/YYYY` or `MM/DD/YYYY` date, or an English month name followed by a
/// day and a 4-digit year (`July 9, 2026` or `July 9 2026`). Returns each
/// match's byte span and its canonical `YYYY-MM-DD` string. Only attempts a
/// match at the start of a digit or letter run (the byte before the start is
/// neither a digit nor a letter, as appropriate), so a date is never matched
/// starting mid-token.
fn find_date_spans(text: &str, bytes: &[u8]) -> Vec<(usize, usize, String)> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_digit()) {
            if let Some((end, canon)) = try_iso_date(bytes, i) {
                spans.push((i, end, canon));
                i = end;
                continue;
            }
            if let Some((end, canon)) = try_slash_date(bytes, i) {
                spans.push((i, end, canon));
                i = end;
                continue;
            }
        } else if bytes[i].is_ascii_alphabetic() && (i == 0 || !bytes[i - 1].is_ascii_alphabetic())
        {
            if let Some((end, canon)) = try_month_name_date(text, bytes, i) {
                spans.push((i, end, canon));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    spans
}

/// Parses a short run of already-validated ASCII digit bytes into a `u32`.
/// Callers only ever pass slices they have already confirmed are all
/// digits, so this never needs to handle a non-digit byte.
fn digits_to_u32(digits: &[u8]) -> u32 {
    digits
        .iter()
        .fold(0u32, |acc, &b| acc * 10 + (b - b'0') as u32)
}

/// Matches an ISO `YYYY-MM-DD` date starting at `start` (an ASCII digit).
/// Requires exactly 4 digits, `-`, 2 digits, `-`, 2 digits, with the month
/// and day in valid calendar ranges, and no digit immediately after the
/// match (so a longer digit run is never misread as a date's leading
/// fragment; the caller already guarantees no digit immediately precedes
/// it). Returns the byte offset just past the match and its canonical form
/// (the match itself, already `YYYY-MM-DD`).
fn try_iso_date(bytes: &[u8], start: usize) -> Option<(usize, String)> {
    let end = start + 10;
    if end > bytes.len() {
        return None;
    }
    let g = &bytes[start..end];
    let all_digits = |r: &[u8]| r.iter().all(u8::is_ascii_digit);
    if !all_digits(&g[0..4])
        || g[4] != b'-'
        || !all_digits(&g[5..7])
        || g[7] != b'-'
        || !all_digits(&g[8..10])
    {
        return None;
    }
    if end < bytes.len() && bytes[end].is_ascii_digit() {
        return None;
    }
    let month = digits_to_u32(&g[5..7]);
    let day = digits_to_u32(&g[8..10]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // `g` is entirely ASCII digits and `-`, so this is always valid UTF-8.
    Some((end, std::str::from_utf8(g).unwrap().to_string()))
}

/// Reads an ASCII digit run of `min..=max` digits starting at `start` and
/// returns its end offset and parsed value, or `None` if the run at `start`
/// is shorter than `min` digits or immediately followed by more than `max`
/// digits (so, for example, a 3-digit run is correctly rejected as a 1-2
/// digit month or day group, and a 5-digit run is rejected as a 4-digit
/// year).
fn digit_group(bytes: &[u8], start: usize, min: usize, max: usize) -> Option<(usize, u32)> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() && end - start < max {
        end += 1;
    }
    if end - start < min {
        return None;
    }
    if end < bytes.len() && bytes[end].is_ascii_digit() {
        return None;
    }
    Some((end, digits_to_u32(&bytes[start..end])))
}

/// Matches a `M/D/YYYY` or `MM/DD/YYYY` date starting at `start` (an ASCII
/// digit, guaranteed by the caller not to be preceded by another digit). The
/// month and day groups are 1 or 2 digits each; the year group must be
/// exactly 4 digits. Validates the month and day are in calendar range.
/// Returns the byte offset just past the match and its canonical
/// `YYYY-MM-DD` form.
fn try_slash_date(bytes: &[u8], start: usize) -> Option<(usize, String)> {
    let (month_end, month) = digit_group(bytes, start, 1, 2)?;
    if bytes.get(month_end) != Some(&b'/') {
        return None;
    }
    let (day_end, day) = digit_group(bytes, month_end + 1, 1, 2)?;
    if bytes.get(day_end) != Some(&b'/') {
        return None;
    }
    let (year_end, year) = digit_group(bytes, day_end + 1, 4, 4)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year_end, format!("{year:04}-{month:02}-{day:02}")))
}

/// Matches an English month name followed by a day and a 4-digit year
/// (`July 9, 2026` or `July 9 2026`, the comma is optional) starting at
/// `start`, which must index the first letter of the month word (the caller
/// already guarantees no letter immediately precedes it). Matching is
/// case-insensitive against [`CITE_MONTH_NAMES`]. Requires at least one
/// whitespace byte between each part. Returns the byte offset just past the
/// year and the canonical `YYYY-MM-DD` form.
fn try_month_name_date(text: &str, bytes: &[u8], start: usize) -> Option<(usize, String)> {
    let mut word_end = start;
    while word_end < bytes.len() && bytes[word_end].is_ascii_alphabetic() && word_end - start < 12 {
        word_end += 1;
    }
    if word_end < bytes.len() && bytes[word_end].is_ascii_alphabetic() {
        return None; // Longer than any month name: not a month word.
    }
    let word = text[start..word_end].to_ascii_lowercase();
    let month = CITE_MONTH_NAMES.iter().find(|entry| entry.0 == word)?.1;

    let mut i = word_end;
    let ws_start = i;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i == ws_start {
        return None;
    }
    let (day_end, day) = digit_group(bytes, i, 1, 2)?;
    if !(1..=31).contains(&day) {
        return None;
    }
    i = day_end;
    if bytes.get(i) == Some(&b',') {
        i += 1;
    }
    let ws2_start = i;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i == ws2_start {
        return None;
    }
    let (year_end, year) = digit_group(bytes, i, 4, 4)?;
    Some((year_end, format!("{year:04}-{month:02}-{day:02}")))
}

/// Finds every plain number, money figure, percentage, and
/// magnitude-suffixed number in `text`, skipping any byte offset inside
/// `exclude` (citation-marker spans and already-matched date spans, so a
/// date's day and year are never double-counted as separate bare numbers
/// and a marker's digits are never read as content). Returns each mention's
/// canonical value string, in first-seen order.
fn find_number_spans(text: &str, bytes: &[u8], exclude: &[(usize, usize)]) -> Vec<String> {
    let mut numbers = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if in_excluded(i, exclude) {
            i += 1;
            continue;
        }
        if bytes[i] == b'$'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && !in_excluded(i + 1, exclude)
        {
            let (end, canon) = parse_number_literal(text, bytes, i + 1);
            numbers.push(canon);
            i = end;
            continue;
        }
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_digit()) {
            let (end, canon) = parse_number_literal(text, bytes, i);
            numbers.push(canon);
            i = end;
            continue;
        }
        i += 1;
    }
    numbers
}

/// Reads a numeric literal's digits starting at `start`: a required leading
/// digit group, any number of comma-grouped digit groups immediately
/// following (a comma is only consumed when a digit follows it directly, so
/// a sentence comma like "3, 2, 1" is never pulled into the number), and an
/// optional decimal point plus digit group. Returns the byte offset just
/// past the literal, the digits with the commas and point removed, and how
/// many of those digits belong to the integer part (so the caller can
/// reinsert the point after a magnitude shift).
///
/// Precondition: `bytes[start]` is an ASCII digit. Both call sites (via
/// [`parse_number_literal`]) only ever invoke this after already checking
/// that byte, so the leading digit group can never come up empty; there is
/// no error case left to report, which is why this returns a plain tuple
/// rather than an `Option`.
fn parse_digits(bytes: &[u8], start: usize) -> (usize, String, usize) {
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    // The run is ASCII digits only, so this is always valid UTF-8.
    let mut digits = std::str::from_utf8(&bytes[start..i]).unwrap().to_string();
    while i < bytes.len()
        && bytes[i] == b','
        && i + 1 < bytes.len()
        && bytes[i + 1].is_ascii_digit()
    {
        let group_start = i + 1;
        let mut j = group_start;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        digits.push_str(std::str::from_utf8(&bytes[group_start..j]).unwrap());
        i = j;
    }
    let point_at = digits.len();
    if i < bytes.len() && bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        digits.push_str(std::str::from_utf8(&bytes[frac_start..j]).unwrap());
        i = j;
    }
    (i, digits, point_at)
}

/// Parses one numeric literal starting at `start` (an ASCII digit, the same
/// precondition as [`parse_digits`]) via `parse_digits`, then looks for, in
/// order, an attached letter suffix (`B`, `bn`, ...), a trailing `%`, or a
/// following word suffix (`billion`, ...), folding whichever is found into
/// the canonical value via [`shift_point`]. Returns the byte offset just
/// past the full match (literal plus any suffix) and the canonical value
/// string.
fn parse_number_literal(text: &str, bytes: &[u8], start: usize) -> (usize, String) {
    let (mut end, digits, point_at) = parse_digits(bytes, start);
    if let Some((sfx_end, exp)) = match_magnitude_abbrev(bytes, end) {
        return (sfx_end, shift_point(&digits, point_at, exp));
    }
    if bytes.get(end) == Some(&b'%') {
        end += 1;
        return (end, format!("{}%", shift_point(&digits, point_at, 0)));
    }
    if let Some((w_end, exp)) = match_word_magnitude(text, bytes, end) {
        return (w_end, shift_point(&digits, point_at, exp));
    }
    (end, shift_point(&digits, point_at, 0))
}

/// Matches an attached magnitude-abbreviation letter suffix (`B`, `bn`, `M`,
/// `mn`, `T`, `tn`, `K`) case-insensitively at `pos`, requiring the byte
/// right after the suffix to not be alphanumeric (so `615Bob` is never
/// misread as `615B` plus a stray word). Tries every entry in
/// [`CITE_MAGNITUDE_ABBREVIATIONS`]; entry order does not affect
/// correctness because a truncated match (matching `b` when the text is
/// actually `bn`) always fails its own boundary check and simply falls
/// through to try the next entry.
fn match_magnitude_abbrev(bytes: &[u8], pos: usize) -> Option<(usize, u32)> {
    for &(abbr, exp) in CITE_MAGNITUDE_ABBREVIATIONS.iter() {
        let len = abbr.len();
        if pos + len > bytes.len() {
            continue;
        }
        if bytes[pos..pos + len].eq_ignore_ascii_case(abbr.as_bytes()) {
            let after = pos + len;
            if after >= bytes.len() || !bytes[after].is_ascii_alphanumeric() {
                return Some((after, exp));
            }
        }
    }
    None
}

/// Matches a following word-form magnitude suffix (`thousand`, `million`,
/// `billion`, `trillion`) at `pos`: at least one whitespace byte, then a
/// case-insensitive whole-word match against [`CITE_MAGNITUDE_WORDS`].
/// Returns the byte offset just past the matched word and its exponent, or
/// `None` if there is no leading whitespace or the following word does not
/// match, in which case the caller's scan position is left untouched so no
/// text is wrongly consumed on a failed attempt.
fn match_word_magnitude(text: &str, bytes: &[u8], pos: usize) -> Option<(usize, u32)> {
    let mut i = pos;
    let ws_start = i;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i == ws_start {
        return None;
    }
    let word_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == word_start {
        return None;
    }
    let word = text[word_start..i].to_ascii_lowercase();
    CITE_MAGNITUDE_WORDS
        .iter()
        .find(|entry| entry.0 == word)
        .map(|entry| (i, entry.1))
}

/// Moves the decimal point in `digits` (a string of decimal digits with no
/// separators) right by `exp` places: the way this module folds a magnitude
/// suffix ("615" plus billion's 9 zeros) or a plain decimal ("917.3" with a
/// 0 shift) into one directly comparable value string. `point_at` is how
/// many of `digits`' characters belong to the integer part before the
/// shift. Implemented as pure string manipulation, with no integer or float
/// arithmetic, specifically so an arbitrarily long digit run can never
/// overflow or lose precision: it can only ever grow the string. Leading
/// zeros are trimmed from the integer part (at least one digit is kept) and
/// trailing zeros are trimmed from any remaining fractional part, so "3.40"
/// and "3.4" and "3.400" all normalize to the same "3.4".
///
/// A claim's "$917.3 billion" and a source's "917 billion" are deliberately
/// kept distinct here (they canonicalize to different strings): this only
/// normalizes representation, not precision, so a rounded figure never
/// silently passes as an exact match. Adding tolerance for that kind of
/// rounding is a possible future refinement, not something this guard
/// attempts.
fn shift_point(digits: &str, point_at: usize, exp: u32) -> String {
    let new_point = point_at + exp as usize;
    let mut d = digits.to_string();
    if new_point > d.len() {
        d.extend(std::iter::repeat_n('0', new_point - d.len()));
    }
    let split = new_point.min(d.len());
    let (int_part, frac_part) = d.split_at(split);
    let int_trimmed = int_part.trim_start_matches('0');
    let int_final = if int_trimmed.is_empty() {
        "0"
    } else {
        int_trimmed
    };
    let frac_trimmed = frac_part.trim_end_matches('0');
    if frac_trimmed.is_empty() {
        int_final.to_string()
    } else {
        format!("{int_final}.{frac_trimmed}")
    }
}

/// Checks a claim's numeric and date mentions against its cited source's,
/// folding each source date's year into the source's number set first (so a
/// claim's bare year, "in 2026", counts as present when the source only
/// carries a full date containing that year, like "July 9, 2026", rather
/// than the year as its own standalone token; keeping this fold one-directional,
/// source dates into the number set rather than trying to match a claim's
/// bare year against a source date's substring directly, keeps the check a
/// simple set-membership test). Returns how many claim mentions were checked
/// in total and how many of those were absent from the source. A claim with
/// no numeric or date content is reported as nothing to check.
fn numeric_guard(claim: &NumericFacts, source: &NumericFacts) -> (usize, usize) {
    if claim.numbers.is_empty() && claim.dates.is_empty() {
        return (0, 0);
    }
    let mut source_numbers: HashSet<&str> = source.numbers.iter().map(String::as_str).collect();
    let source_dates: HashSet<&str> = source.dates.iter().map(String::as_str).collect();
    let source_years: Vec<String> = source.dates.iter().map(|d| d[..4].to_string()).collect();
    for y in &source_years {
        source_numbers.insert(y.as_str());
    }
    let missing_numbers = claim
        .numbers
        .iter()
        .filter(|n| !source_numbers.contains(n.as_str()))
        .count();
    let missing_dates = claim
        .dates
        .iter()
        .filter(|d| !source_dates.contains(d.as_str()))
        .count();
    let checked = claim.numbers.len() + claim.dates.len();
    (checked, missing_numbers + missing_dates)
}

/// Combines the lexical support score's bucket with the numeric-consistency
/// guard's verdict. A claim with no numeric content (`checked == 0`) is
/// unaffected: the lexical bucket stands as-is. Otherwise: any claim number
/// or date absent from the source caps the result at unsupported, since a
/// fabricated figure must never pass on prose overlap alone; when every
/// claim number and date is present, the result is floored at weak, so an
/// exact numeric match can no longer be buried by token-formatting noise,
/// but it still only reaches supported if the lexical score clears
/// [`CITE_SUPPORTED_MIN`] on its own. The guard only ever adds a floor and a
/// cap; it never manufactures a "supported" verdict by itself, which is the
/// simpler of the two combination rules the design considered (the other
/// being: also promote straight to supported when every number matches and
/// the claim is majority-numeric).
fn classify_with_numeric_guard(
    lexical_class: CiteClass,
    checked: usize,
    missing: usize,
) -> CiteClass {
    if checked == 0 {
        return lexical_class;
    }
    if missing > 0 {
        return CiteClass::Unsupported;
    }
    match lexical_class {
        CiteClass::Unsupported => CiteClass::Weak,
        other => other,
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
                unverifiable: 0,
                unsupported_indices: vec![],
                numeric_checked: 0,
                numeric_matched: 0,
                numeric_missing: 0,
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

    // ── numeric-consistency guard ───────────────────────────────────────────

    fn numbers_in(s: &str) -> Vec<String> {
        extract_numeric_facts(s).numbers
    }

    fn dates_in(s: &str) -> Vec<String> {
        extract_numeric_facts(s).dates
    }

    #[test]
    fn numeric_normalization_matrix_matches_across_formats() {
        // "$615B" == "615 billion" == "615,000,000,000".
        assert_eq!(numbers_in("$615B"), vec!["615000000000"]);
        assert_eq!(numbers_in("615 billion"), vec!["615000000000"]);
        assert_eq!(numbers_in("615,000,000,000"), vec!["615000000000"]);
        // "3.4T".
        assert_eq!(numbers_in("3.4T"), vec!["3400000000000"]);
        // "12%".
        assert_eq!(numbers_in("12%"), vec!["12%"]);
        // "$1,053,000,000,000" == "1.053 trillion".
        assert_eq!(numbers_in("$1,053,000,000,000"), vec!["1053000000000"]);
        assert_eq!(numbers_in("1.053 trillion"), vec!["1053000000000"]);
    }

    #[test]
    fn date_normalization_matches_across_formats() {
        assert_eq!(dates_in("2026-07-09"), vec!["2026-07-09"]);
        assert_eq!(dates_in("7/9/2026"), vec!["2026-07-09"]);
        assert_eq!(dates_in("07/09/2026"), vec!["2026-07-09"]);
        assert_eq!(dates_in("July 9, 2026"), vec!["2026-07-09"]);
        assert_eq!(dates_in("July 9 2026"), vec!["2026-07-09"]);
    }

    #[test]
    fn citation_marker_digits_are_never_read_as_numbers() {
        let facts = extract_numeric_facts("Revenue was 50 [12] this year.");
        assert_eq!(facts.numbers, vec!["50"]);
    }

    #[test]
    fn magnitude_abbreviation_requires_word_boundary() {
        // "615Bob" is a stray word, not "615" plus a billion suffix.
        assert_eq!(numbers_in("615Bob"), vec!["615"]);
    }

    #[test]
    fn plain_number_without_magnitude_word_stays_plain() {
        assert_eq!(numbers_in("500 apples"), vec!["500"]);
    }

    #[test]
    fn shift_point_trims_leading_int_zeros_and_trailing_frac_zeros() {
        assert_eq!(shift_point("00615", 5, 0), "615");
        assert_eq!(shift_point("340", 1, 0), "3.4");
        assert_eq!(shift_point("000", 3, 0), "0");
    }

    #[test]
    fn shift_point_keeps_fractional_remainder_when_exponent_is_small() {
        // "3.4567" shifted by 2 (hundred) is 345.67.
        assert_eq!(shift_point("34567", 1, 2), "345.67");
    }

    #[test]
    fn iso_date_rejects_when_a_digit_immediately_follows() {
        // "2026-07-091" has an 11th digit right after an otherwise valid
        // ISO date, so it is not a clean 10-character match.
        let text = "2026-07-091";
        assert_eq!(try_iso_date(text.as_bytes(), 0), None);
    }

    #[test]
    fn slash_date_rejects_out_of_range_month_with_valid_group_shapes() {
        // Every group parses as a well-formed 2/2/4-digit date, but month
        // 13 is not a real calendar month.
        let text = "13/09/2026";
        assert_eq!(try_slash_date(text.as_bytes(), 0), None);
    }

    #[test]
    fn month_name_date_rejects_a_word_longer_than_any_month_name() {
        let text = "Extraordinarily 9, 2026";
        assert_eq!(try_month_name_date(text, text.as_bytes(), 0), None);
    }

    #[test]
    fn month_name_date_rejects_missing_whitespace_after_month() {
        let text = "July9, 2026";
        assert_eq!(try_month_name_date(text, text.as_bytes(), 0), None);
    }

    #[test]
    fn month_name_date_rejects_out_of_range_day() {
        let text = "July 45, 2026";
        assert_eq!(try_month_name_date(text, text.as_bytes(), 0), None);
    }

    #[test]
    fn month_name_date_rejects_missing_whitespace_before_year() {
        let text = "July 9,2026";
        assert_eq!(try_month_name_date(text, text.as_bytes(), 0), None);
    }

    #[test]
    fn numeric_guard_folds_source_date_year_for_bare_year_claims() {
        // A bare year in the claim counts as present when the source only
        // carries a full date containing that year.
        let claim = NumericFacts {
            numbers: vec!["2026".to_string()],
            dates: vec![],
        };
        let source = NumericFacts {
            numbers: vec![],
            dates: vec!["2026-07-09".to_string()],
        };
        assert_eq!(numeric_guard(&claim, &source), (1, 0));
    }

    #[test]
    fn numeric_guard_flags_bare_year_absent_from_source() {
        let claim = NumericFacts {
            numbers: vec!["2030".to_string()],
            dates: vec![],
        };
        let source = NumericFacts {
            numbers: vec![],
            dates: vec!["2026-07-09".to_string()],
        };
        assert_eq!(numeric_guard(&claim, &source), (1, 1));
    }

    #[test]
    fn numeric_guard_no_claim_numbers_is_nothing_to_check() {
        let claim = NumericFacts {
            numbers: vec![],
            dates: vec![],
        };
        let source = NumericFacts {
            numbers: vec!["1".to_string()],
            dates: vec![],
        };
        assert_eq!(numeric_guard(&claim, &source), (0, 0));
    }

    #[test]
    fn classify_with_numeric_guard_covers_all_branches() {
        // No numeric content: lexical bucket passes through unchanged.
        assert_eq!(
            classify_with_numeric_guard(CiteClass::Supported, 0, 0),
            CiteClass::Supported
        );
        // Any missing number caps at unsupported, even from supported.
        assert_eq!(
            classify_with_numeric_guard(CiteClass::Supported, 2, 1),
            CiteClass::Unsupported
        );
        // All present floors an unsupported lexical score up to weak.
        assert_eq!(
            classify_with_numeric_guard(CiteClass::Unsupported, 1, 0),
            CiteClass::Weak
        );
        // All present never downgrades an already-weak or supported score.
        assert_eq!(
            classify_with_numeric_guard(CiteClass::Weak, 1, 0),
            CiteClass::Weak
        );
        assert_eq!(
            classify_with_numeric_guard(CiteClass::Supported, 1, 0),
            CiteClass::Supported
        );
    }

    #[test]
    fn numeric_guard_counts_aggregate_across_citations() {
        let s1 = source(1, "Revenue reached $64,123 in the period.");
        let s2 = source(2, "Nothing numeric here at all.");
        let answer = "* $64,123 [1]\n* mostly words here [2]";
        let audit = audit_citations(answer, &[s1, s2]);
        // Only citation 1's claim has a number to check.
        assert_eq!(audit.numeric_checked, 1);
        assert_eq!(audit.numeric_matched, 1);
        assert_eq!(audit.numeric_missing, 0);
    }

    #[test]
    fn numeric_extraction_hostile_input_never_panics() {
        let text = "\u{1F600} $ % / - [1,2] 99999999999999999999999999999999 \
                     [ [999] 3/45/6 2026-13-99 [broken \u{4e2d}\u{6587}";
        let facts = extract_numeric_facts(text);
        let _ = facts.numbers.len();
        let _ = facts.dates.len();
        let src = source(1, text);
        let _ = audit_citations(&format!("claim {text} [1]"), &[src]);
    }

    // ── live-smoke regressions (2026-07-11) ─────────────────────────────────
    //
    // The four failure cases from the live-smoke forensics: token overlap
    // alone flagged an exact numeric match as unsupported (formatting noise
    // hid the match) and passed two fabricated figures and a wrong date
    // (a swapped digit hid inside an otherwise-matching sentence). The
    // numeric-consistency guard fixes all four without touching the lexical
    // scorer.

    #[test]
    fn live_smoke_exact_numeric_match_rescued_to_weak() {
        // Claim and source both name $615 billion, but the claim spells it
        // out while the source abbreviates it ("$615B"), so the tokens never
        // literally match and the lexical score alone comes back 0 (fully
        // unsupported). The numeric guard recognizes the two forms as the
        // same value and floors the citation at weak.
        let src = source(1, "# 1 Elon Musk / $615B / net worth as of 7/9/2026");
        let answer = "It totals $615 billion at last check [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unsupported, 0);
        assert_eq!(audit.weak, 1);
        assert_eq!(audit.numeric_checked, 1);
        assert_eq!(audit.numeric_matched, 1);
        assert_eq!(audit.numeric_missing, 0);
    }

    #[test]
    fn live_smoke_fabricated_money_figure_capped_unsupported() {
        // The sentence is otherwise a near-identical paraphrase of the
        // source (high lexical overlap), but the claim's figure ($951.9B)
        // does not match the source's ($917.3B): a swapped-digit fabrication
        // the lexical scorer alone would have passed as supported.
        let src = source(1, "Net worth is $917.3 billion currently, per filings.");
        let answer = "Net worth is $951.9 billion currently [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unsupported, 1);
        assert_eq!(audit.numeric_checked, 1);
        assert_eq!(audit.numeric_matched, 0);
        assert_eq!(audit.numeric_missing, 1);
    }

    #[test]
    fn live_smoke_absent_bloomberg_figure_capped_unsupported() {
        // Mirrors the live-smoke case where a "$957 billion" figure
        // attributed to a named source appeared nowhere in the cited page
        // (the page had a different figure entirely). High word overlap
        // around the figure would have passed this as supported without the
        // numeric guard.
        let src = source(1, "The figure reached $945 billion, per Bloomberg.");
        let answer = "The figure reached $957 billion [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unsupported, 1);
        assert_eq!(audit.numeric_checked, 1);
        assert_eq!(audit.numeric_matched, 0);
        assert_eq!(audit.numeric_missing, 1);
    }

    #[test]
    fn live_smoke_date_only_mismatch_detected() {
        // Same wording, one digit off in the date (July 9 claimed, source
        // says July 10): a date-only fabrication the lexical scorer alone
        // would have passed as supported.
        let src = source(1, "It was published on July 10, 2026, per staff.");
        let answer = "It was published on July 9, 2026 [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unsupported, 1);
        assert_eq!(audit.numeric_checked, 1);
        assert_eq!(audit.numeric_matched, 0);
        assert_eq!(audit.numeric_missing, 1);
    }

    // ── unverifiable outcome (thin/empty source text) ───────────────────────

    #[test]
    fn empty_source_text_is_unverifiable_not_unsupported() {
        let src = source(1, "");
        let answer = "The company earned $500 million last year [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unverifiable, 1);
        assert_eq!(audit.unsupported, 0);
        assert!(audit.unsupported_indices.is_empty());
        // Nothing to check the claim's figure against, so the numeric guard
        // is never even attempted.
        assert_eq!(audit.numeric_checked, 0);
    }

    #[test]
    fn source_text_below_byte_threshold_is_unverifiable() {
        // One byte short of the threshold: still unverifiable regardless of
        // any accidental lexical overlap with the claim.
        let thin_text = "x".repeat(CITE_UNVERIFIABLE_MIN_SOURCE_BYTES - 1);
        assert_eq!(thin_text.len(), CITE_UNVERIFIABLE_MIN_SOURCE_BYTES - 1);
        let src = source(1, &thin_text);
        let answer = "Some claim about this page [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unverifiable, 1);
        assert_eq!(audit.unsupported, 0);
    }

    #[test]
    fn source_text_at_exact_byte_threshold_is_scored_normally() {
        // Exactly at the threshold: no longer "below" it, so the citation
        // falls through to ordinary lexical/numeric scoring instead of being
        // classified unverifiable, regardless of what that scoring decides.
        let boundary_text = "x".repeat(CITE_UNVERIFIABLE_MIN_SOURCE_BYTES);
        assert_eq!(boundary_text.len(), CITE_UNVERIFIABLE_MIN_SOURCE_BYTES);
        let src = source(1, &boundary_text);
        let answer = "Some claim about this page [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(audit.cited, 1);
        assert_eq!(audit.unverifiable, 0);
    }

    #[test]
    fn unverifiable_citation_never_drives_the_failure_note() {
        // The end-to-end path: a source too thin to verify must not surface
        // as an answer-facing failure note, only as its own separate outcome.
        let src = source(1, "");
        let answer = "The company earned $500 million last year [1].";
        let audit = audit_citations(answer, &[src]);
        assert_eq!(honest_failure_note(&audit), None);
    }

    // ── post-audit cleanup / repair helpers ──────────────────────────────────

    /// Builds a minimal audit carrying the given unsupported indices.
    /// `supported`/`weak` default to 0 (total failure shape) unless overridden.
    fn audit_with_unsupported_indices(indices: Vec<usize>) -> CitationAudit {
        CitationAudit {
            cited: indices.len().max(1),
            supported: 0,
            weak: 0,
            unsupported: indices.len(),
            unverifiable: 0,
            unsupported_indices: indices,
            numeric_checked: 0,
            numeric_matched: 0,
            numeric_missing: 0,
        }
    }

    #[test]
    fn honest_failure_note_none_when_nothing_unsupported() {
        assert_eq!(
            honest_failure_note(&audit_with_unsupported_indices(vec![])),
            None
        );
    }

    #[test]
    fn honest_failure_note_none_on_partial_failure() {
        // Some citations still supported: strip only, no user-facing note.
        let audit = CitationAudit {
            cited: 3,
            supported: 1,
            weak: 0,
            unsupported: 2,
            unverifiable: 0,
            unsupported_indices: vec![2, 5],
            numeric_checked: 0,
            numeric_matched: 0,
            numeric_missing: 0,
        };
        assert!(!is_total_citation_failure(&audit));
        assert_eq!(honest_failure_note(&audit), None);
    }

    #[test]
    fn honest_failure_note_fires_on_total_failure() {
        let note = honest_failure_note(&audit_with_unsupported_indices(vec![2, 5]))
            .expect("total failure yields a note");
        assert!(note.contains("Thuki could not confirm"));
        assert!(note.contains("larger model"));
    }

    #[test]
    fn repair_critique_names_failing_markers() {
        let critique = repair_critique(&audit_with_unsupported_indices(vec![5, 2, 5]));
        assert!(critique.contains("[5], [2]"));
        assert!(critique.contains("Rewrite the full answer"));
    }

    #[test]
    fn strip_unsupported_citations_removes_bad_markers_and_keeps_good_ones() {
        let answer = "Elon is 55 [1][2][3]. Born in 1971 [2].";
        let stripped = strip_unsupported_citations(answer, &[1, 3]);
        assert_eq!(stripped, "Elon is 55[2]. Born in 1971 [2].");
    }

    #[test]
    fn strip_unsupported_citations_noop_when_empty_input_or_indices() {
        assert_eq!(strip_unsupported_citations("keep [1]", &[]), "keep [1]");
        assert_eq!(strip_unsupported_citations("", &[1]), "");
    }

    #[test]
    fn finalize_answer_after_audit_noop_when_nothing_unsupported() {
        let audit = CitationAudit {
            cited: 1,
            supported: 1,
            weak: 0,
            unsupported: 0,
            unverifiable: 0,
            unsupported_indices: vec![],
            numeric_checked: 0,
            numeric_matched: 0,
            numeric_missing: 0,
        };
        assert_eq!(finalize_answer_after_audit("ok [1]", &audit), "ok [1]");
    }

    #[test]
    fn finalize_answer_after_audit_note_only_when_strip_empties_answer() {
        // Answer is only a bad citation marker: strip leaves whitespace, note alone.
        let answer = " [1] ";
        let audit = audit_with_unsupported_indices(vec![1]);
        let out = finalize_answer_after_audit(answer, &audit);
        assert_eq!(
            out,
            "*Thuki could not confirm the cited facts against the pages it retrieved. \
             Try rephrasing, or switch to a larger model in Settings for a stronger answer.*"
        );
    }

    #[test]
    fn strip_unsupported_citations_rewrites_grouped_markers() {
        let answer = "Revenue hit 100 [1, 3].";
        let stripped = strip_unsupported_citations(answer, &[1]);
        // Space before the marker is kept when the span is rewritten, not removed.
        assert_eq!(stripped, "Revenue hit 100 [3].");
    }

    #[test]
    fn strip_unsupported_citations_drops_space_before_fully_removed_marker() {
        let answer = "He is tall [9].";
        let stripped = strip_unsupported_citations(answer, &[9]);
        assert_eq!(stripped, "He is tall.");
    }

    #[test]
    fn finalize_answer_after_audit_strips_partial_without_note() {
        let answer = "A is true [1]. B is false [2].";
        let audit = CitationAudit {
            cited: 2,
            supported: 1,
            weak: 0,
            unsupported: 1,
            unverifiable: 0,
            unsupported_indices: vec![2],
            numeric_checked: 0,
            numeric_matched: 0,
            numeric_missing: 0,
        };
        let out = finalize_answer_after_audit(answer, &audit);
        assert_eq!(out, "A is true [1]. B is false.");
        assert!(!out.contains("Thuki could not confirm"));
    }

    #[test]
    fn finalize_answer_after_audit_appends_honest_note_on_total_failure() {
        let answer = "Fake number 999 [1].";
        let audit = audit_with_unsupported_indices(vec![1]);
        let out = finalize_answer_after_audit(answer, &audit);
        assert!(out.contains("Fake number 999."));
        assert!(out.contains("Thuki could not confirm"));
    }
}
