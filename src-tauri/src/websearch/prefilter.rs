//! The deterministic search pre-filter: stage one of the two-stage decision.
//!
//! Before any model call, this pure function inspects the user's latest message
//! and resolves the obvious cases without spending a decode slot:
//!
//! - [`PreFilterVerdict::ForceWeb`] when the message carries an unambiguous
//!   freshness or temporal signal ("latest", "weather", "who won", a current or
//!   future year, an explicit "search"/URL). These are the questions small local
//!   models most reliably get wrong by answering from stale parametric memory,
//!   so the decision is taken away from the model here.
//! - [`PreFilterVerdict::ForceNo`] when the message is a self-contained turn that
//!   provably needs no web: a greeting or acknowledgement, a pure arithmetic
//!   expression, or a creative/transform request over text the user supplied.
//! - [`PreFilterVerdict::Ambiguous`] for everything else, which proceeds to the
//!   persona-free classifier (stage two).
//!
//! ## Design stance
//!
//! The skip rules are deliberately high-precision: when a rule is not certain a
//! turn is trivial, the verdict falls through to `Ambiguous` and the classifier
//! (biased to search on uncertainty) decides. In particular, context-dependent
//! follow-ups ("what about there?") are never force-skipped here: they carry no
//! standalone signal, so resolving them needs the conversation history the
//! classifier holds, not a keyword match. Force-search signals take precedence
//! over skip signals, so "summarise the latest news" searches rather than being
//! caught by the "summarise" transform rule.
//!
//! The scan is bounded ([`PREFILTER_MAX_SCAN_CHARS`]) and tokenised in a single
//! linear pass with no backtracking, so a pathologically large pasted message
//! cannot turn the per-turn decision into a CPU denial-of-service.

use crate::config::defaults::PREFILTER_MAX_SCAN_CHARS;

/// The stage-one verdict for a single user turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreFilterVerdict {
    /// The turn provably needs no web search: answer directly, no model
    /// decision call.
    ForceNo,
    /// The turn carries an unambiguous freshness signal: search, without asking
    /// the model whether to.
    ForceWeb,
    /// Undecided: hand to the persona-free classifier.
    Ambiguous,
}

/// Single-token freshness and temporal signals. Any one present forces a search.
/// Deliberately excludes broad words that only sometimes imply freshness (e.g.
/// bare "version", "cost", "now"): those are left to the classifier so the
/// forced-search set stays high-precision and bounds third-party query volume.
const FORCE_WEB_WORDS: &[&str] = &[
    // Temporal adverbs.
    "latest",
    "current",
    "currently",
    "today",
    "tonight",
    "yesterday",
    "tomorrow",
    "recent",
    "recently",
    "nowadays",
    "upcoming",
    "ongoing",
    // Freshness nouns.
    "weather",
    "forecast",
    "temperature",
    "price",
    "prices",
    "stock",
    "stocks",
    "news",
    "headline",
    "headlines",
    "score",
    "scores",
    "standings",
    "election",
    "elections",
    // Explicit retrieval intent.
    "google",
];

/// Multi-word freshness phrases. Matched against the space-padded normalised
/// message so each is a whole-phrase hit, never a substring of a longer word.
const FORCE_WEB_PHRASES: &[&str] = &[
    "who won",
    "who is winning",
    "who's winning",
    "right now",
    "most recent",
    "as of",
    "this year",
    "this week",
    "this month",
    "this morning",
    "look up",
    "search for",
    "search the web",
    "how much is",
    "how much does",
    "up to date",
    "these days",
    "at the moment",
    "release date",
    "stock price",
    "exchange rate",
    "box office",
];

/// Greeting, acknowledgement, and filler tokens. A message whose every token is
/// in this set (and is short) is a social turn that needs no retrieval.
const GREETING_ACK_WORDS: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "yo",
    "hiya",
    "howdy",
    "sup",
    "greetings",
    "good",
    "morning",
    "afternoon",
    "evening",
    "night",
    "thanks",
    "thank",
    "thankyou",
    "ty",
    "thx",
    "cheers",
    "appreciated",
    "appreciate",
    "ok",
    "okay",
    "k",
    "kk",
    "cool",
    "nice",
    "great",
    "awesome",
    "perfect",
    "gotcha",
    "understood",
    "yes",
    "yeah",
    "yep",
    "yup",
    "no",
    "nope",
    "bye",
    "goodbye",
    "cya",
    "later",
    "please",
    "you",
    "u",
    "so",
    "much",
    "very",
    "there",
    "again",
    "mate",
    "friend",
    "lol",
    "haha",
];

/// Leading verbs that mark a creative or text-transform request over content the
/// user has supplied, which needs no web. Only matched at the start of the
/// message and only after the force-search check, so "summarise the latest news"
/// still searches.
const TRANSFORM_LEAD_VERBS: &[&str] = &[
    "write",
    "compose",
    "draft",
    "rephrase",
    "reword",
    "rewrite",
    "paraphrase",
    "proofread",
    "translate",
    "summarize",
    "summarise",
    "refactor",
    "reformat",
];

/// Words treated as connective filler when deciding whether a message is a pure
/// arithmetic expression. Stripping them leaves only the numbers and operators.
const MATH_FILLER_WORDS: &[&str] = &[
    "what",
    "whats",
    "is",
    "the",
    "a",
    "calculate",
    "compute",
    "evaluate",
    "solve",
    "plus",
    "minus",
    "times",
    "multiplied",
    "divided",
    "by",
    "of",
    "percent",
    "mod",
    "power",
    "squared",
    "cubed",
    "sqrt",
    "square",
    "root",
    "sum",
    "product",
    "equals",
    "equal",
    "to",
    "and",
];

/// Maximum token count for a message to still qualify as a pure greeting or
/// acknowledgement. Longer all-filler messages are rare and safer left to the
/// classifier.
const MAX_GREETING_TOKENS: usize = 6;

/// Resolves the deterministic verdict for `message`. `today` is the `YYYY-MM-DD`
/// date string used to recognise current-or-future year tokens as a freshness
/// signal. Pure and total: any input yields a verdict.
pub fn prefilter(message: &str, today: &str) -> PreFilterVerdict {
    // Bound the scan so tokenisation cost is a small constant regardless of a
    // hostile or accidentally huge pasted message.
    let bounded: String = message
        .chars()
        .take(PREFILTER_MAX_SCAN_CHARS)
        .flat_map(char::to_lowercase)
        .collect();

    // Normalise punctuation to spaces for whole-word/phrase matching, keeping a
    // separate operator-bearing copy for arithmetic detection.
    let normalised = to_normalised(&bounded);
    let tokens: Vec<&str> = normalised.split_whitespace().collect();

    if tokens.is_empty() {
        return PreFilterVerdict::ForceNo;
    }

    // Force-search signals win over every skip rule.
    if has_force_web_signal(&bounded, &normalised, &tokens, today) {
        return PreFilterVerdict::ForceWeb;
    }

    if is_greeting_or_ack(&tokens) || is_pure_math(&bounded, &tokens) || has_transform_lead(&tokens)
    {
        return PreFilterVerdict::ForceNo;
    }

    PreFilterVerdict::Ambiguous
}

/// Lowercases-and-normalises `bounded` (already lowercased and length-capped) by
/// replacing every non-alphanumeric character with a space, so tokenisation and
/// phrase matching see clean word boundaries.
fn to_normalised(bounded: &str) -> String {
    bounded
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect()
}

/// Whether the message carries any freshness or explicit-retrieval signal: a URL,
/// a current-or-future year, a single-token signal, or a multi-word phrase.
/// `bounded` is the raw lowercased prefix (punctuation intact, for URL matching);
/// `normalised`/`tokens` are its punctuation-stripped forms.
fn has_force_web_signal(bounded: &str, normalised: &str, tokens: &[&str], today: &str) -> bool {
    if contains_url(bounded) {
        return true;
    }
    let current_year = parse_year(today);
    if tokens
        .iter()
        .any(|t| FORCE_WEB_WORDS.contains(t) || is_current_or_future_year(t, current_year))
    {
        return true;
    }
    // Pad so a phrase matches only on whole-word boundaries.
    let padded = format!(" {normalised} ");
    FORCE_WEB_PHRASES
        .iter()
        .any(|phrase| padded.contains(&format!(" {phrase} ")))
}

/// Whether the raw message contains a web URL. Matched on the scheme/host
/// punctuation ("http://", "https://", "www.") so the bare acronym "HTTP" or a
/// stray "www" word is not mistaken for a link.
fn contains_url(bounded: &str) -> bool {
    bounded.contains("http://") || bounded.contains("https://") || bounded.contains("www.")
}

/// Parses the leading `YYYY` of a `YYYY-MM-DD` date string, or `None` if it is
/// not a 4-digit year. Used only to recognise year tokens as a freshness signal.
fn parse_year(today: &str) -> Option<u32> {
    let year = today.get(0..4)?;
    year.parse::<u32>().ok()
}

/// Whether `token` is a 4-digit year at or after `current_year` (and within a
/// decade of it, so an unrelated large number is not mistaken for a year). A
/// future or current year signals a question about the present, not history.
fn is_current_or_future_year(token: &str, current_year: Option<u32>) -> bool {
    let Some(current) = current_year else {
        return false;
    };
    if token.len() != 4 {
        return false;
    }
    match token.parse::<u32>() {
        Ok(year) => year >= current && year <= current + 10,
        Err(_) => false,
    }
}

/// Whether every token is a greeting/acknowledgement/filler word and the message
/// is short enough to be a social turn rather than an incidental all-common-words
/// question.
fn is_greeting_or_ack(tokens: &[&str]) -> bool {
    tokens.len() <= MAX_GREETING_TOKENS && tokens.iter().all(|t| GREETING_ACK_WORDS.contains(t))
}

/// Whether the message is a pure arithmetic expression: after removing filler
/// words, only digits and operator characters remain, and at least one digit is
/// present. `bounded` is the lowercased, length-capped raw message (operators
/// intact); `tokens` is its normalised word list used to strip filler words.
fn is_pure_math(bounded: &str, tokens: &[&str]) -> bool {
    // Every token must be a filler word or a bare number; any real word means
    // this is prose, not arithmetic.
    let all_numeric_or_filler = tokens
        .iter()
        .all(|t| MATH_FILLER_WORDS.contains(t) || t.bytes().all(|b| b.is_ascii_digit()));
    if !all_numeric_or_filler {
        return false;
    }
    let mut has_digit = false;
    let mut has_content = false;
    for c in bounded.chars() {
        if c.is_whitespace() {
            continue;
        }
        if c.is_ascii_digit() {
            has_digit = true;
            has_content = true;
            continue;
        }
        if is_math_operator(c) {
            has_content = true;
            continue;
        }
        // A letter here belongs to a filler word (already vetted above); skip it.
        if c.is_alphabetic() {
            continue;
        }
        // Any other symbol disqualifies the expression.
        return false;
    }
    has_digit && has_content
}

/// Whether `c` is an arithmetic operator or grouping character permitted in a
/// pure-math expression.
fn is_math_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')' | '.' | ',' | '='
    )
}

/// Whether the first token is a creative or text-transform lead verb, marking a
/// request over user-supplied content that needs no web.
fn has_transform_lead(tokens: &[&str]) -> bool {
    tokens
        .first()
        .is_some_and(|first| TRANSFORM_LEAD_VERBS.contains(first))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "2026-07-07";

    fn verdict(message: &str) -> PreFilterVerdict {
        prefilter(message, TODAY)
    }

    // ── the three live-smoke failures, pinned deterministically ───────────────
    //
    // These exact turns were answered from stale memory by the old trigger. They
    // must now resolve to ForceWeb without any model call.

    #[test]
    fn smoke_failure_rust_version_forces_web() {
        assert_eq!(
            verdict("what's the latest stable version of Rust?"),
            PreFilterVerdict::ForceWeb
        );
    }

    #[test]
    fn smoke_failure_tokyo_weather_forces_web() {
        assert_eq!(verdict("weather in Tokyo"), PreFilterVerdict::ForceWeb);
    }

    #[test]
    fn smoke_failure_recent_f1_race_forces_web() {
        assert_eq!(
            verdict("who won the most recent F1 race"),
            PreFilterVerdict::ForceWeb
        );
    }

    // ── force-web signals ─────────────────────────────────────────────────────

    #[test]
    fn temporal_adverbs_force_web() {
        for m in [
            "current bitcoin price",
            "what happened today",
            "recent developments in fusion",
            "upcoming apple event",
        ] {
            assert_eq!(verdict(m), PreFilterVerdict::ForceWeb, "{m}");
        }
    }

    #[test]
    fn freshness_nouns_force_web() {
        for m in ["tesla stock", "latest news on the strike", "nba standings"] {
            assert_eq!(verdict(m), PreFilterVerdict::ForceWeb, "{m}");
        }
    }

    #[test]
    fn multiword_phrases_force_web() {
        for m in [
            "who won the election",
            "how much is a gallon of milk",
            "what is trending right now",
        ] {
            assert_eq!(verdict(m), PreFilterVerdict::ForceWeb, "{m}");
        }
    }

    #[test]
    fn current_or_future_year_forces_web() {
        assert_eq!(verdict("best laptops of 2026"), PreFilterVerdict::ForceWeb);
        assert_eq!(
            verdict("what will change in 2030"),
            PreFilterVerdict::ForceWeb
        );
    }

    #[test]
    fn past_year_does_not_force_web() {
        // 1919 is history, not a freshness signal; falls to the classifier.
        assert_eq!(
            verdict("when was the treaty of versailles signed in 1919"),
            PreFilterVerdict::Ambiguous
        );
    }

    #[test]
    fn url_forces_web() {
        assert_eq!(
            verdict("summarize https://example.com/article"),
            PreFilterVerdict::ForceWeb
        );
        assert_eq!(
            verdict("what does www.rust-lang.org say"),
            PreFilterVerdict::ForceWeb
        );
    }

    #[test]
    fn force_web_beats_transform_lead() {
        // "summarise" is a transform verb, but the freshness signal wins.
        assert_eq!(
            verdict("summarize the latest news on the merger"),
            PreFilterVerdict::ForceWeb
        );
    }

    #[test]
    fn year_signal_uses_today_not_a_hardcoded_year() {
        // With a 2020 "today", 2026 is future -> force; 2019 is past -> not.
        assert_eq!(
            prefilter("outlook for 2026", "2020-01-01"),
            PreFilterVerdict::ForceWeb
        );
        assert_eq!(
            prefilter("what happened in 2019", "2020-01-01"),
            PreFilterVerdict::Ambiguous
        );
    }

    #[test]
    fn malformed_today_disables_year_signal_only() {
        // A non-date `today` cannot yield a year, so the year rule is inert, but
        // other signals still fire.
        assert_eq!(
            prefilter("outlook for 2027", "not-a-date"),
            PreFilterVerdict::Ambiguous
        );
        assert_eq!(
            prefilter("tokyo weather", "not-a-date"),
            PreFilterVerdict::ForceWeb
        );
    }

    // ── force-no signals ──────────────────────────────────────────────────────

    #[test]
    fn greetings_and_acks_force_no() {
        for m in [
            "hi",
            "hello there",
            "thanks so much",
            "thank you",
            "ok cool",
            "good morning",
            "great, thanks!",
        ] {
            assert_eq!(verdict(m), PreFilterVerdict::ForceNo, "{m}");
        }
    }

    #[test]
    fn pure_math_forces_no() {
        for m in ["2 + 2", "what is 15% of 240", "(12 * 4) - 3", "compute 7^3"] {
            assert_eq!(verdict(m), PreFilterVerdict::ForceNo, "{m}");
        }
    }

    #[test]
    fn transform_requests_force_no() {
        for m in [
            "write a haiku about the sea",
            "rephrase this sentence for me",
            "translate good morning into french",
        ] {
            assert_eq!(verdict(m), PreFilterVerdict::ForceNo, "{m}");
        }
    }

    #[test]
    fn empty_or_whitespace_forces_no() {
        assert_eq!(verdict(""), PreFilterVerdict::ForceNo);
        assert_eq!(verdict("   \n\t "), PreFilterVerdict::ForceNo);
    }

    // ── ambiguous middle ──────────────────────────────────────────────────────

    #[test]
    fn stable_knowledge_and_followups_are_ambiguous() {
        for m in [
            "what is the capital of France",
            "explain how photosynthesis works",
            "what about there?",
            "and its population?",
            "how do I reverse a linked list in rust",
        ] {
            assert_eq!(verdict(m), PreFilterVerdict::Ambiguous, "{m}");
        }
    }

    #[test]
    fn long_all_greeting_message_is_not_forced_no() {
        // Over the token cap, even if every word is filler, it is not treated as
        // a bare greeting.
        assert_eq!(
            verdict("thanks so much you are very very great indeed"),
            PreFilterVerdict::Ambiguous
        );
    }

    #[test]
    fn number_heavy_but_wordy_question_is_not_math() {
        // Real words beyond the filler list -> not pure arithmetic.
        assert_eq!(
            verdict("what is the population of Tokyo"),
            PreFilterVerdict::Ambiguous
        );
    }

    #[test]
    fn math_with_stray_symbol_is_not_forced_no() {
        // A '@' is neither digit, operator, nor letter -> not a clean expression.
        assert_eq!(verdict("2 + 2 @"), PreFilterVerdict::Ambiguous);
    }

    #[test]
    fn numeric_token_outside_filler_still_reads_as_math() {
        // "12345" is not a filler word but is purely numeric, so the expression
        // stays math-eligible.
        assert_eq!(verdict("12345 / 5"), PreFilterVerdict::ForceNo);
    }

    // ── bounded scan / DoS ────────────────────────────────────────────────────

    #[test]
    fn scan_is_bounded_signal_past_the_cap_is_ignored() {
        // A freshness word only past the scan cap is not matched; the visible
        // prefix is plain filler text -> falls through to the classifier.
        let mut msg = "a".repeat(PREFILTER_MAX_SCAN_CHARS);
        msg.push_str(" weather");
        assert_eq!(prefilter(&msg, TODAY), PreFilterVerdict::Ambiguous);
    }

    #[test]
    fn huge_input_is_handled_in_bounded_time() {
        // Sanity: a multi-megabyte message returns without scanning all of it.
        let msg = "latest ".to_string() + &"x".repeat(4_000_000);
        assert_eq!(prefilter(&msg, TODAY), PreFilterVerdict::ForceWeb);
    }

    // ── curated eval corpus (the measurement instrument) ──────────────────────
    //
    // The committed `search_decision_eval.jsonl` is the labelled should-search /
    // should-not-search set. This test certifies the DETERMINISTIC pre-filter
    // against it: it may never hard-skip a should-search turn (`ForceNo`) nor
    // hard-force a should-not-search turn (`ForceWeb`). Ambiguous is always
    // allowed: those turns are the classifier's job, whose live decision quality
    // is validated by the real-model smoke test, not here.

    #[derive(serde::Deserialize)]
    struct EvalRow {
        message: String,
        label: String,
    }

    fn eval_rows() -> Vec<EvalRow> {
        include_str!("search_decision_eval.jsonl")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("eval row is valid JSON"))
            .collect()
    }

    #[test]
    fn prefilter_never_contradicts_a_labelled_row() {
        for row in eval_rows() {
            let v = prefilter(&row.message, TODAY);
            // A should-search row may never be force-skipped; a should-not-search
            // row may never be force-searched. Label validity itself is checked in
            // `corpus_is_a_meaningful_size_and_balance`.
            let forbidden = if row.label == "search" {
                PreFilterVerdict::ForceNo
            } else {
                PreFilterVerdict::ForceWeb
            };
            let msg = &row.message;
            assert_ne!(v, forbidden, "row violated its label: {msg}");
        }
    }

    #[test]
    fn corpus_is_a_meaningful_size_and_balance() {
        let rows = eval_rows();
        let total = rows.len();
        assert!(total >= 45, "eval corpus too small: {total}");
        let search = rows.iter().filter(|r| r.label == "search").count();
        let no = rows.iter().filter(|r| r.label == "no").count();
        // Every row carries a known label, and both directions are represented.
        assert_eq!(search + no, total, "corpus has an unknown label");
        assert!(search >= 15, "too few should-search rows: {search}");
        assert!(no >= 15, "too few should-not-search rows: {no}");
    }

    #[test]
    fn prefilter_deterministically_catches_most_should_search_turns() {
        // Quantifies the pre-filter's in-gate recall: a clear majority of the
        // should-search corpus is resolved to ForceWeb without any model call.
        let rows = eval_rows();
        let search: Vec<_> = rows.iter().filter(|r| r.label == "search").collect();
        let total = search.len();
        let forced = search
            .iter()
            .filter(|r| prefilter(&r.message, TODAY) == PreFilterVerdict::ForceWeb)
            .count();
        assert!(
            forced * 10 >= total * 6,
            "only {forced}/{total} should-search turns caught deterministically"
        );
    }

    #[test]
    fn corpus_pins_the_three_live_smoke_failures() {
        let messages: Vec<String> = eval_rows().into_iter().map(|r| r.message).collect();
        for needle in [
            "what's the latest stable version of Rust?",
            "weather in Tokyo",
            "who won the most recent F1 race",
        ] {
            assert!(
                messages.iter().any(|m| m == needle),
                "smoke failure missing from corpus: {needle}"
            );
        }
    }
}
