//! J5: live answer-quality eval harness for the built-in web search pipeline.
//!
//! Distinct from the other two harnesses documented in `docs/search-eval.md`:
//! `live_classifier_eval.rs` measures whether the pipeline decides to search
//! at all, and `live_answer_capture.rs` captures the assembled writer prompt
//! for a later comparison run. This harness answers a third question: once
//! the pipeline searches, is the generated answer actually *correct*?
//!
//! ## Corpus
//!
//! ~95 new rows across three externally sourced datasets, committed under
//! `tests/j5_corpus/`, plus the existing 138-row in-repo decision corpus
//! (`src/websearch/search_decision_eval.jsonl`) reused for composition
//! tracking. Full provenance (source URL, license, fetch date, filter method)
//! lives in `docs/search-eval.md`; the short version:
//!
//! - `j5_corpus/simpleqa_verified.jsonl` (50 rows, MIT, `google/simpleqa-verified`):
//!   filtered to non-recency questions. **Gates** the headline metric.
//! - `j5_corpus/freshqa.jsonl` (30 rows, Apache-2.0, `freshllms/freshqa`):
//!   fast-changing and false-premise rows, which are *supposed* to rot as
//!   time passes. Tracked, **never gates**.
//! - `j5_corpus/seal0.jsonl` (15 rows, Apache-2.0, `vtllms/sealqa` seal_0
//!   config): conflicting/noisy-search rows. Tracked, **never gates**.
//! - The existing decision corpus (138 rows): carries no gold answer (it was
//!   built for search-routing, not answer grading; its own accuracy gate
//!   already lives in `live_classifier_eval.rs`). Rows are counted in this
//!   harness's composition report but always skipped from grading, never
//!   forced through the judge.
//!
//! ## Grading pipeline
//!
//! Deterministic-first: [`grade_deterministic`] tries an exact/normalized
//! match, a numeric range or bare-numeric match, and a conservative
//! word-sequence fuzzy match against the gold answer or any listed
//! `acceptable_answers` alias. Only rows that do **not** confirm a
//! deterministic match ("paraphrase residue") go to the live judge. The
//! deterministic layer only ever shortcuts to `Correct`: it never asserts a
//! definite `Incorrect` or `NotAttempted` on its own, because a
//! non-match could still be a valid paraphrase, a wrong answer, or a
//! hedge/refusal, and only the judge's semantic read can tell those apart
//! (see [`DeterministicVerdict`]).
//!
//! The judge follows the SimpleQA 3-way protocol **verbatim**: the exact
//! `GRADER_TEMPLATE` prompt from OpenAI's `simple-evals`
//! (MIT licensed, <https://github.com/openai/simple-evals/blob/main/simpleqa_eval.py>),
//! grading each predicted answer against the gold reference as CORRECT,
//! INCORRECT, or NOT_ATTEMPTED. This harness never does pairwise or
//! rubric-score comparison: a weak local judge is far more reliable making one
//! absolute call against a fixed reference than comparing two answers
//! side-by-side, where small local models are known to be catastrophically
//! position-biased (see `docs/search-eval.md`, which formally supersedes the
//! pairwise plan an earlier revision of this doc sketched).
//!
//! Every judged row runs the judge 3 independent times; [`resolve_majority`]
//! takes the majority verdict, or `NotAttempted` when the 3 calls do not
//! agree (see its doc comment for why retries are never selective).
//!
//! The headline **gated** metric is confidently-wrong rate
//! (incorrect / attempted) on the SimpleQA-Verified subset only, at
//! [`CONFIDENTLY_WRONG_GATE`]. Accuracy and not-attempted rate are tracked
//! alongside for every source, gated or not.
//!
//! ## Calibration (one-time, manual, out of this file's scope)
//!
//! Before trusting this harness's numbers, hand-label a sample of 30-40 rows
//! (drawn across all three graded sources and volatility buckets) with your
//! own CORRECT/INCORRECT/NOT_ATTEMPTED judgment, run the live judge over the
//! same predicted answers, and compare: agreement rate = rows where the
//! judge's majority verdict matches your label, divided by the sample size.
//! This tells you whether the local judge model is trustworthy enough for
//! [`CONFIDENTLY_WRONG_GATE`] to mean anything, and is a prerequisite for
//! tightening that floor. This requires a live model and a human rater, so it
//! is a manual procedure documented here and in `docs/search-eval.md`, not
//! code in this file.
//!
//! ## Running
//!
//! ```sh
//! THUKI_EVAL_PORT=<port> cargo test --test live_answer_quality_eval -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `<port>` must be a running `llama-server`. **If you plan to spawn your own
//! `llama-server` instance for this rather than pointing at the app's own
//! sidecar port, quit the Thuki app first.** Two `llama-server` processes each
//! holding a model resident in Metal memory can exhaust the GPU memory
//! budget on the dev machine (see the keep-warm/Metal-residency notes this
//! repo already carries for the built-in engine). Pointing `THUKI_EVAL_PORT`
//! at the app's own already-running sidecar port needs no quit, exactly as
//! `live_classifier_eval.rs` documents.
//!
//! This harness also hits the live internet through the production
//! `websearch` retrieval pipeline; no local search services are required.

use thuki_agent_lib::commands::ChatMessage;
use thuki_agent_lib::net::reachability::DnsReachability;
use thuki_agent_lib::net::transport::ReqwestTransport;
use thuki_agent_lib::openai::{
    request_openai_json, stream_openai_chat, OpenAiChatParams, V1Flavor,
};
use thuki_agent_lib::trace::{BoundRecorder, ConversationId};
use thuki_agent_lib::websearch::assemble::SourceBlock;
use thuki_agent_lib::websearch::cache::TtlSourceCache;
use thuki_agent_lib::websearch::engine::EngineHealth;
use thuki_agent_lib::websearch::judge::{
    InsufficiencyReason, SufficiencyJudge, SufficiencyVerdict,
};
use thuki_agent_lib::websearch::orchestrator::{run_search, SearchDeps, SearchOutcome};
use thuki_agent_lib::websearch::prepass::{
    InferenceError, PrePass, PrePassDecision, SearchDecision, SearchRoute,
};
use thuki_agent_lib::websearch::rank::Bm25Scorer;
use thuki_agent_lib::websearch::serp_cache::WebCache;

use async_trait::async_trait;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

// ─── Corpus model ────────────────────────────────────────────────────────────

/// Coarse volatility bucket, unified across all four corpus sources. Mirrors
/// the FreshQA category vocabulary the existing decision corpus already uses
/// (see `docs/search-eval.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Volatility {
    /// Timeless facts that do not change.
    Never,
    /// Facts that change over months to years.
    Slow,
    /// Facts that change daily to weekly.
    Fast,
    /// The question presupposes something untrue.
    FalsePremise,
    /// The row's `volatility` field was missing or unrecognised. Never a
    /// parse failure: volatility is informational and gates nothing (see
    /// [`CorpusSource::gating`]), so an unknown tag is tracked, not dropped.
    Unknown,
}

impl Volatility {
    /// Parses a corpus row's raw `volatility` string into a bucket, falling
    /// back to [`Volatility::Unknown`] for anything unrecognised rather than
    /// failing the row.
    fn parse(raw: &str) -> Self {
        match raw.trim() {
            "never" => Volatility::Never,
            "slow" => Volatility::Slow,
            "fast" => Volatility::Fast,
            "false-premise" => Volatility::FalsePremise,
            _ => Volatility::Unknown,
        }
    }
}

/// Which of the four sources a corpus row came from, carrying its license and
/// gating status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CorpusSource {
    SimpleqaVerified,
    Freshqa,
    Seal0,
    DecisionCorpus,
}

impl CorpusSource {
    /// Short, stable, machine-readable name used as a `HashMap` key and in
    /// log output.
    fn name(self) -> &'static str {
        match self {
            CorpusSource::SimpleqaVerified => "simpleqa_verified",
            CorpusSource::Freshqa => "freshqa",
            CorpusSource::Seal0 => "seal0",
            CorpusSource::DecisionCorpus => "decision_corpus",
        }
    }

    /// License and upstream dataset, for the run's stderr summary. Full
    /// provenance (fetch date, filter method, exact URL) lives in
    /// `docs/search-eval.md`.
    fn license(self) -> &'static str {
        match self {
            CorpusSource::SimpleqaVerified => "MIT (google/simpleqa-verified)",
            CorpusSource::Freshqa => "Apache-2.0 (freshllms/freshqa)",
            CorpusSource::Seal0 => "Apache-2.0 (vtllms/sealqa, seal_0 config)",
            CorpusSource::DecisionCorpus => "Apache-2.0 (this repository)",
        }
    }

    /// Whether this source's confidently-wrong rate counts toward the gated
    /// headline assertion. Only SimpleQA-Verified gates: FreshQA and Seal-0
    /// are expected to rot by design and are tracked, never gated. The
    /// decision corpus carries no gold answer at all, so it never reaches
    /// grading in the first place (see [`EvalRow::gold_answer`]) and this
    /// flag is moot for it, included only for completeness.
    fn gating(self) -> bool {
        matches!(self, CorpusSource::SimpleqaVerified)
    }
}

/// One row of the unified answer-quality corpus, after loading from any of
/// the four sources.
#[derive(Debug, Clone)]
struct EvalRow {
    /// Stable id, prefixed with the source name (e.g. `simpleqa_verified:5`).
    id: String,
    source: CorpusSource,
    question: String,
    /// `None` only for [`CorpusSource::DecisionCorpus`] rows: that corpus has
    /// no gold answer field (see the module doc comment). Every other source
    /// always carries `Some`.
    gold_answer: Option<String>,
    acceptable_answers: Vec<String>,
    volatility: Volatility,
}

/// Wire shape shared by the three new corpus JSONL files (all authored with
/// the same schema; see `tests/j5_corpus/*.jsonl`).
#[derive(serde::Deserialize)]
struct RawGradableRow {
    id: String,
    question: String,
    gold_answer: String,
    #[serde(default)]
    acceptable_answers: Vec<String>,
    #[serde(default)]
    volatility: String,
}

/// The existing decision corpus' row shape (see `live_classifier_eval.rs`),
/// duplicated here rather than imported or shared: integration test binaries
/// cannot share code across files without a `tests/common` module, and this
/// harness is scoped to add exactly one new file (see the module doc
/// comment). Only the two fields this harness needs are declared; serde
/// ignores the corpus's other fields (`label`, `category`, `route`)
/// automatically.
#[derive(serde::Deserialize)]
struct DecisionCorpusRow {
    message: String,
    #[serde(default)]
    volatility: String,
}

/// Counts, per source name, how many raw lines were skipped for being
/// malformed JSON or missing a required field. Never fatal: a bad corpus row
/// is dropped and counted, not a panic (see [`load_gradable_source`]).
#[derive(Debug, Default)]
struct LoadStats {
    skipped: HashMap<&'static str, usize>,
}

impl LoadStats {
    /// Increments the skipped-row count for `source`.
    fn record_skip(&mut self, source: &'static str) {
        *self.skipped.entry(source).or_insert(0) += 1;
    }
}

/// Parses one of the three new-schema corpus JSONL bodies into rows tagged
/// with `source`. Blank lines are skipped silently (formatting, not data); a
/// line that fails to parse as [`RawGradableRow`] is skipped and counted in
/// `stats`, logged to stderr, and never panics the harness.
fn load_gradable_source(source: CorpusSource, jsonl: &str, stats: &mut LoadStats) -> Vec<EvalRow> {
    jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<RawGradableRow>(line) {
            Ok(raw) => Some(EvalRow {
                id: format!("{}:{}", source.name(), raw.id),
                source,
                question: raw.question,
                gold_answer: Some(raw.gold_answer),
                acceptable_answers: raw.acceptable_answers,
                volatility: Volatility::parse(&raw.volatility),
            }),
            Err(e) => {
                eprintln!("[j5-eval] skipping malformed {} row: {e}", source.name());
                stats.record_skip(source.name());
                None
            }
        })
        .collect()
}

/// Parses the existing decision corpus into rows with no gold answer (see the
/// module doc comment). Same never-panic, skip-and-count discipline as
/// [`load_gradable_source`].
fn load_decision_corpus(jsonl: &str, stats: &mut LoadStats) -> Vec<EvalRow> {
    jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .filter_map(
            |(i, line)| match serde_json::from_str::<DecisionCorpusRow>(line) {
                Ok(raw) => Some(EvalRow {
                    id: format!("decision_corpus:{i}"),
                    source: CorpusSource::DecisionCorpus,
                    question: raw.message,
                    gold_answer: None,
                    acceptable_answers: Vec::new(),
                    volatility: Volatility::parse(&raw.volatility),
                }),
                Err(e) => {
                    eprintln!("[j5-eval] skipping malformed decision_corpus row: {e}");
                    stats.record_skip("decision_corpus");
                    None
                }
            },
        )
        .collect()
}

/// Loads and combines all four corpus sources into one flat row list, plus
/// per-source skip counts. The three new files are compiled in via
/// `include_str!` (no filesystem access at test run time, matching
/// `live_classifier_eval.rs`'s existing pattern for the decision corpus).
fn load_full_corpus() -> (Vec<EvalRow>, LoadStats) {
    let mut stats = LoadStats::default();
    let mut rows = Vec::new();
    rows.extend(load_gradable_source(
        CorpusSource::SimpleqaVerified,
        include_str!("j5_corpus/simpleqa_verified.jsonl"),
        &mut stats,
    ));
    rows.extend(load_gradable_source(
        CorpusSource::Freshqa,
        include_str!("j5_corpus/freshqa.jsonl"),
        &mut stats,
    ));
    rows.extend(load_gradable_source(
        CorpusSource::Seal0,
        include_str!("j5_corpus/seal0.jsonl"),
        &mut stats,
    ));
    rows.extend(load_decision_corpus(
        include_str!("../src/websearch/search_decision_eval.jsonl"),
        &mut stats,
    ));
    (rows, stats)
}

// ─── Deterministic grading ──────────────────────────────────────────────────

/// Result of the deterministic grading pass. See the module doc comment for
/// why this layer only ever shortcuts to `Correct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeterministicVerdict {
    Correct,
    Residue,
}

/// Case-folds, trims, and collapses whitespace, and strips a fixed set of
/// trailing sentence punctuation. Applied before any exact or fuzzy
/// comparison so capitalization, spacing, and a trailing period never cause a
/// false mismatch (mirrors the SimpleQA grading note that "capitalization,
/// punctuation, grammar, and order don't matter").
fn normalize_text(s: &str) -> String {
    let lower = s.trim().to_lowercase();
    let trimmed = lower.trim_end_matches(['.', '!', '?', ',', ';', ':']);
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses a leading numeric token out of `s`, tolerant of a leading currency
/// symbol and thousands-separator commas. Returns `None` when `s` does not
/// start with a number (an entity or date-phrase answer).
fn parse_leading_number(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches(['$', '€', '£']);
    let digits: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .filter(|c| *c != ',')
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<f64>().ok()
}

/// Extracts an inline "(acceptable range: anything between X and Y)" hint
/// that SimpleQA-Verified sometimes embeds directly in `gold_answer` (see
/// `docs/search-eval.md`). Returns `None` when `gold` carries no such hint or
/// either bound fails to parse as a number.
fn parse_acceptable_range(gold: &str) -> Option<(f64, f64)> {
    let lower = gold.to_lowercase();
    let marker = "between ";
    let start = lower.find(marker)? + marker.len();
    let rest = &lower[start..];
    let and_idx = rest.find(" and ")?;
    let low = parse_leading_number(&rest[..and_idx])?;
    let high = parse_leading_number(&rest[and_idx + " and ".len()..])?;
    Some((low, high))
}

/// Returns whether `needle`'s words appear as a contiguous run inside
/// `haystack`'s words (both already whitespace-tokenized on a single space,
/// as [`normalize_text`] produces). Used instead of raw substring containment
/// so a short candidate like "Iran" cannot false-match inside an unrelated
/// word like "Iranian".
fn contains_word_sequence(haystack: &str, needle: &str) -> bool {
    let h: Vec<&str> = haystack.split(' ').collect();
    let n: Vec<&str> = needle.split(' ').collect();
    if n.is_empty() || h.len() < n.len() {
        return false;
    }
    h.windows(n.len()).any(|w| w == n.as_slice())
}

/// Below this length a normalized candidate is not attempted as a fuzzy
/// word-sequence match: a very short token ("no", a bare digit) is common
/// enough across unrelated answers that containment would false-positive.
/// Real entity/date/phrase answers in this corpus are essentially always at
/// or above this length. Chosen, not derived from the data; see
/// `docs/search-eval.md`.
const MIN_FUZZY_CANDIDATE_LEN: usize = 4;

/// Deterministic-first grading: exact/normalized match, numeric range
/// containment, bare-numeric equality, or a conservative word-sequence fuzzy
/// match, against the gold answer or any `acceptable_answers` alias. Anything
/// that does not confirm a match is [`DeterministicVerdict::Residue`] for the
/// live judge to decide (see the module doc comment for why this layer never
/// emits a definite negative).
fn grade_deterministic(predicted: &str, row: &EvalRow) -> DeterministicVerdict {
    let Some(gold) = &row.gold_answer else {
        return DeterministicVerdict::Residue;
    };
    let predicted_norm = normalize_text(predicted);
    if predicted_norm.is_empty() {
        return DeterministicVerdict::Residue;
    }

    // Numeric range containment: SimpleQA-Verified embeds an explicit
    // tolerance for many numeric answers; a predicted number inside that
    // range is exact enough even though it will not string-match the gold
    // text (which carries the "(acceptable range: ...)" annotation too).
    if let Some((low, high)) = parse_acceptable_range(gold) {
        if let Some(n) = parse_leading_number(&predicted_norm) {
            if n >= low && n <= high {
                return DeterministicVerdict::Correct;
            }
        }
    }

    let mut candidates: Vec<&str> = vec![gold.as_str()];
    candidates.extend(row.acceptable_answers.iter().map(String::as_str));

    for candidate in candidates {
        let candidate_norm = normalize_text(candidate);
        if candidate_norm.is_empty() {
            continue;
        }
        if predicted_norm == candidate_norm {
            return DeterministicVerdict::Correct;
        }
        if let (Some(a), Some(b)) = (
            parse_leading_number(&predicted_norm),
            parse_leading_number(&candidate_norm),
        ) {
            if (a - b).abs() < 1e-9 {
                return DeterministicVerdict::Correct;
            }
        }
        if candidate_norm.len() >= MIN_FUZZY_CANDIDATE_LEN
            && contains_word_sequence(&predicted_norm, &candidate_norm)
        {
            return DeterministicVerdict::Correct;
        }
    }

    DeterministicVerdict::Residue
}

// ─── SimpleQA 3-way judge ───────────────────────────────────────────────────

/// Verbatim from OpenAI's `simple-evals` SimpleQA grader (`simpleqa_eval.py`,
/// `GRADER_TEMPLATE`), MIT licensed:
/// <https://github.com/openai/simple-evals/blob/main/simpleqa_eval.py>.
/// `{question}` / `{target}` / `{predicted_answer}` are substituted with
/// [`str::replace`] at call time (the source uses Python's `str.format`; the
/// literal grading criteria and few-shot examples below are what "verbatim"
/// refers to, not the templating mechanism). The trailing "reply with a
/// letter" instruction is kept exactly as published; this harness
/// additionally constrains the live model's output with a `response_format`
/// JSON schema (`{"grade": "A"|"B"|"C"}`, see [`judge_schema`]) for
/// reliability with small local models, matching this codebase's existing
/// convention (see `websearch/prepass.rs`): the schema enforces the same
/// three-way choice the template already asks for, it does not change it.
const GRADER_TEMPLATE: &str = r#"
Your job is to look at a question, a gold target, and a predicted answer, and then assign a grade of either ["CORRECT", "INCORRECT", "NOT_ATTEMPTED"].
First, I will give examples of each grade, and then you will grade a new example.


The following are examples of CORRECT predicted answers.
```
Question: What are the names of Barack Obama's children?
Gold target: Malia Obama and Sasha Obama
Predicted answer 1: sasha and malia obama
Predicted answer 2: most people would say Malia and Sasha, but I'm not sure and would have to double check
Predicted answer 3: Barack Obama has two daughters. Their names are Malia Ann and Natasha Marian, but they are commonly referred to as Malia Obama and Sasha Obama. Malia was born on July 4, 1998, and Sasha was born on June 10, 2001.
```
These predicted answers are all CORRECT because:
    - They fully contain the important information in the gold target.
    - They do not contain any information that contradicts the gold target.
    - Only semantic meaning matters; capitalization, punctuation, grammar, and order don't matter.
    - Hedging and guessing are permissible, provided that the gold target is fully included and the response contains no incorrect information or contradictions.


The following are examples of INCORRECT predicted answers.
```
Question: What are the names of Barack Obama's children?
Gold target: Malia and Sasha
Predicted answer 1: Malia.
Predicted answer 2: Malia, Sasha, and Susan.
Predicted answer 3: Barack Obama does not have any children.
Predicted answer 4: I think it's either Malia and Sasha. Or it could be Malia and Jackie. Or it could be Joey and Malia.
Predicted answer 4: While I don't know their exact names, I can tell you that Barack Obama has three children.
Predicted answer 5: It's possible you may mean Betsy and Olivia. However, you should clarify further details with updated references if necessary. Is that the correct answer?
Predicted answer 6: It may be the case that Obama's child is named James. However, it's recommended to confirm the most accurate and updated information since this could change over time. This model may not always reflect the most current information.
```
These predicted answers are all INCORRECT because:
    - A factual statement in the answer contradicts the gold target. Incorrect statements that have some hedging (e.g., "it is possible that", "although i'm not sure, i think") are also considered incorrect.


The following are examples of NOT_ATTEMPTED predicted answers.
```
Question: What are the names of Barack Obama's children?
Gold target: Malia and Sasha
Predicted answer 1: I don't know.
Predicted answer 2: I need more context about which Obama you are talking about.
Predicted answer 3: Without researching the web, I cannot answer this question. However, I can tell you that Barack Obama has two children.
Predicted answer 4: Barack Obama has two children. I know that one of them is Malia, but I'm not sure about the other one.
```
These predicted answers are all NOT_ATTEMPTED because:
    - The important information in the gold target is not included in the answer.
    - No statements in the answer contradict the gold target.


Also note the following things:
- For grading questions where the gold target is a number, the predicted answer needs to be correct to the last significant figure in the gold answer. For example, consider a question "How many citations does the Transformer Paper have?" with gold target "120k". 
    - Predicted answers "120k", "124k", and 115k" are all CORRECT. 
    - Predicted answers "100k" and "113k" are INCORRECT. 
    - Predicted answers "around 100k" and "more than 50k" are considered NOT_ATTEMPTED because they neither confirm nor contradict the gold target.
- The gold target may contain more information than the question. In such cases, the predicted answer only needs to contain the information that is in the question.
    - For example, consider the question "What episode did Derek and Meredith get legally married in Grey's Anatomy?" with gold target "Season 7, Episode 20: White Wedding". Either "Season 7, Episode 20" or "White Wedding" would be considered a CORRECT answer.
- Do not punish predicted answers if they omit information that would be clearly inferred from the question.
    - For example, consider the question "What city is OpenAI headquartered in?" and the gold target "San Francisco, California". The predicted answer "San Francisco" would be considered CORRECT, even though it does not include "California".
    - Consider the question "What award did A pretrainer's guide to training data: Measuring the effects of data age, domain coverage, quality, & toxicity win at NAACL '24?", the gold target is "Outstanding Paper Award". The predicted answer "Outstanding Paper" would be considered CORRECT, because "award" is presumed in the question.
    - For the question "What is the height of Jason Wei in meters?", the gold target is "1.73 m". The predicted answer "1.75" would be considered CORRECT, because meters is specified in the question.
    - For the question "What is the name of Barack Obama's wife?", the gold target is "Michelle Obama". The predicted answer "Michelle" would be considered CORRECT, because the last name can be presumed.
- Do not punish for typos in people's name if it's clearly the same name. 
    - For example, if the gold target is "Hyung Won Chung", you can consider the following predicted answers as correct: "Hyoong Won Choong", "Hyungwon Chung", or "Hyun Won Chung".


Here is a new example. Simply reply with either CORRECT, INCORRECT, NOT ATTEMPTED. Don't apologize or correct yourself if there was a mistake; we are just trying to grade the answer.
```
Question: {question}
Gold target: {target}
Predicted answer: {predicted_answer}
```

Grade the predicted answer of this new question as one of:
A: CORRECT
B: INCORRECT
C: NOT_ATTEMPTED

Just return the letters "A", "B", or "C", with no text around it.
"#;

/// The three SimpleQA-protocol outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleQaVerdict {
    Correct,
    Incorrect,
    NotAttempted,
}

/// Wire shape the judge's `response_format` schema constrains the model to.
#[derive(serde::Deserialize)]
struct JudgeWire {
    grade: String,
}

/// Small, fixed token budget: the judge emits one JSON field holding a single
/// letter, never free-form prose.
const JUDGE_MAX_TOKENS: i32 = 16;

/// Per-call wall-clock timeout. Matches the order of magnitude of the
/// existing classifier timeout (`PREPASS_TIMEOUT_S` = 35s in
/// `config/defaults.rs`), since both are single small structured-output
/// calls. Chosen, not derived from measurement of this specific prompt.
const JUDGE_TIMEOUT_S: u64 = 35;

/// Builds the `response_format` JSON schema constraining the judge's output
/// to a bare A/B/C grade.
fn judge_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": { "grade": { "type": "string", "enum": ["A", "B", "C"] } },
        "required": ["grade"],
        "additionalProperties": false
    })
}

/// Calls the live judge model once over the verbatim [`GRADER_TEMPLATE`] and
/// maps its A/B/C grade to a [`SimpleQaVerdict`]. Returns `None` on any
/// transport or parse failure so a single flaky call degrades to "excluded
/// from this majority round" (see [`resolve_majority`]) rather than
/// panicking the whole harness.
#[allow(clippy::too_many_arguments)]
async fn judge_once(
    client: &reqwest::Client,
    base_url: &str,
    model: &str,
    question: &str,
    gold_target: &str,
    predicted_answer: &str,
    cancel: &CancellationToken,
) -> Option<SimpleQaVerdict> {
    let prompt = GRADER_TEMPLATE
        .trim()
        .replace("{question}", question)
        .replace("{target}", gold_target)
        .replace("{predicted_answer}", predicted_answer);
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
        images: None,
    }];
    let raw = request_openai_json(
        base_url,
        model,
        client,
        messages,
        judge_schema(),
        None,
        JUDGE_TIMEOUT_S,
        JUDGE_MAX_TOKENS,
        V1Flavor::Builtin,
        cancel,
    )
    .await
    .ok()?;
    let wire: JudgeWire = serde_json::from_str(raw.trim()).ok()?;
    match wire.grade.trim() {
        "A" => Some(SimpleQaVerdict::Correct),
        "B" => Some(SimpleQaVerdict::Incorrect),
        "C" => Some(SimpleQaVerdict::NotAttempted),
        _ => None,
    }
}

/// Pure majority-resolution logic, split out from [`majority_vote`] so it is
/// unit-testable without a live model: given up to 3 collected votes, returns
/// whichever verdict appears at least twice, or [`SimpleQaVerdict::NotAttempted`]
/// when no verdict has a strict majority. A judge that cannot agree with
/// itself across 3 independent calls cannot be trusted to assert the model
/// was confidently wrong, so an indecisive round never contributes to the
/// confidently-wrong numerator; it is the conservative default, not an
/// attempt to guess which of the disagreeing votes was "right".
fn resolve_majority(votes: &[SimpleQaVerdict]) -> SimpleQaVerdict {
    let mut correct = 0;
    let mut incorrect = 0;
    for v in votes {
        match v {
            SimpleQaVerdict::Correct => correct += 1,
            SimpleQaVerdict::Incorrect => incorrect += 1,
            // A not-attempted vote never wins a majority on its own (see doc
            // comment): it only matters by *not* contributing to the other
            // two counters, so it needs no counter of its own.
            SimpleQaVerdict::NotAttempted => {}
        }
    }
    if correct >= 2 {
        SimpleQaVerdict::Correct
    } else if incorrect >= 2 {
        SimpleQaVerdict::Incorrect
    } else {
        SimpleQaVerdict::NotAttempted
    }
}

/// Runs the judge 3 independent times over the same triple and returns the
/// majority verdict via [`resolve_majority`]. Always 3 fresh calls, never
/// "retry only when the first result looks like a failure": selective
/// retrying would bias the outcome toward whichever verdict a re-roll is more
/// likely to produce (typically `Correct`), inflating accuracy. See
/// `docs/search-eval.md` for the calibration procedure that validates this
/// judge's reliability.
#[allow(clippy::too_many_arguments)]
async fn majority_vote(
    client: &reqwest::Client,
    base_url: &str,
    model: &str,
    question: &str,
    gold_target: &str,
    predicted_answer: &str,
    cancel: &CancellationToken,
) -> SimpleQaVerdict {
    let mut votes = Vec::with_capacity(3);
    for _ in 0..3 {
        if let Some(v) = judge_once(
            client,
            base_url,
            model,
            question,
            gold_target,
            predicted_answer,
            cancel,
        )
        .await
        {
            votes.push(v);
        }
    }
    resolve_majority(&votes)
}

// ─── Metrics ─────────────────────────────────────────────────────────────────

/// Aggregate metrics for one graded subset (one [`CorpusSource`]'s rows).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct GradeCounts {
    correct: usize,
    incorrect: usize,
    not_attempted: usize,
}

impl GradeCounts {
    /// Tallies one row's final verdict.
    fn record(&mut self, verdict: SimpleQaVerdict) {
        match verdict {
            SimpleQaVerdict::Correct => self.correct += 1,
            SimpleQaVerdict::Incorrect => self.incorrect += 1,
            SimpleQaVerdict::NotAttempted => self.not_attempted += 1,
        }
    }

    /// Total graded rows.
    fn total(&self) -> usize {
        self.correct + self.incorrect + self.not_attempted
    }

    /// Attempted rows: correct + incorrect. Not-attempted rows are, by
    /// definition, not an attempt.
    fn attempted(&self) -> usize {
        self.correct + self.incorrect
    }

    /// The headline metric: incorrect / attempted. `0.0` when nothing was
    /// attempted, rather than dividing by zero.
    fn confidently_wrong_rate(&self) -> f64 {
        if self.attempted() == 0 {
            0.0
        } else {
            self.incorrect as f64 / self.attempted() as f64
        }
    }

    /// correct / total, tracked alongside the headline metric.
    fn accuracy(&self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            self.correct as f64 / self.total() as f64
        }
    }

    /// not_attempted / total, tracked alongside the headline metric.
    fn not_attempted_rate(&self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            self.not_attempted as f64 / self.total() as f64
        }
    }
}

/// The gated ceiling on SimpleQA-Verified's confidently-wrong rate
/// (incorrect / attempted). Chosen, not specified by the source task: a
/// conservative starting floor pending the one-time calibration run described
/// in the module doc comment. Tighten once a real run establishes a
/// trustworthy baseline.
const CONFIDENTLY_WRONG_GATE: f64 = 0.15;

// ─── Live harness: scripted retrieval + real judge ─────────────────────────

/// A scripted classifier that always forces the general `Web` retrieval tier,
/// using the row's own question text unmodified as both the standalone
/// question and the sole search query. Unlike `live_answer_capture.rs`'s
/// hand-curated 10-question slice (each with its own hand-picked route and
/// rewritten query), this harness runs the full ~95-row gradable corpus, so a
/// per-row hand rewrite is not practical; every row gets the same generic
/// treatment. This measures answer quality *given* the general engine tier
/// retrieved something, not routing quality (`live_classifier_eval.rs`'s job)
/// and not whether a more specific vertical would have done better.
struct ScriptedWebPrePass;

#[async_trait]
impl PrePass for ScriptedWebPrePass {
    /// Always returns a forced `Web` decision using `latest_user_message`
    /// verbatim as both the standalone question and the sole query.
    async fn decide(
        &self,
        _history: &[ChatMessage],
        latest_user_message: &str,
        _latest_images: Option<&[String]>,
        _today: &str,
        _cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: latest_user_message.to_string(),
            queries: vec![latest_user_message.to_string()],
            explicit_search: false,
            lang: "en".to_string(),
        })
    }
}

/// Always finds retrieval sufficient, identical to `live_answer_capture.rs`'s
/// twin of the same name (duplicated per this file's single-file scope; see
/// the module doc comment).
struct AlwaysSufficientJudge;

#[async_trait]
impl SufficiencyJudge for AlwaysSufficientJudge {
    /// Always reports the retrieved sources sufficient.
    async fn judge(
        &self,
        _standalone_question: &str,
        _sources: &[SourceBlock],
        _cancel: &CancellationToken,
    ) -> Result<SufficiencyVerdict, InferenceError> {
        Ok(SufficiencyVerdict {
            sufficient: true,
            missing: String::new(),
            reason: InsufficiencyReason::Missing,
            requery_queries: Vec::new(),
        })
    }
}

/// Drives one row through the real `run_search` orchestrator (retrieval
/// forced via [`ScriptedWebPrePass`]) and, when it produced a citable answer,
/// through one real writer completion over the assembled prompt. Returns the
/// generated answer text, or `None` when retrieval produced nothing citable
/// or the pipeline resolved no-search/cancelled (not expected here, since the
/// scripted pre-pass always forces `Web`, but handled rather than panicking).
async fn live_answer_for(
    client: &reqwest::Client,
    base_url: &str,
    model: &str,
    question: &str,
) -> Option<String> {
    let transport = ReqwestTransport::new().ok()?;
    let prepass = ScriptedWebPrePass;
    let judge = AlwaysSufficientJudge;
    let health = EngineHealth::new();
    let recorder = BoundRecorder::noop_for(ConversationId::new("j5-eval"));
    let cache = TtlSourceCache::new(
        std::time::Duration::from_secs(600),
        thuki_agent_lib::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
    );
    let web_cache = WebCache::new(
        std::time::Duration::from_secs(600),
        std::time::Duration::from_secs(600),
        64,
        128,
    );
    let timings = thuki_agent_lib::websearch::stage_timing::TimingBag::new();
    let deps = SearchDeps {
        prepass: &prepass,
        judge: &judge,
        transport: &transport,
        reachability: &DnsReachability,
        scorer: &Bm25Scorer,
        health: &health,
        recorder: &recorder,
        cache: &cache,
        cache_scope: 1,
        web_cache: &web_cache,
        local_zone: None,
        force_search: false,
        latest_images: None,
        timings: &timings,
    };
    let cancel = CancellationToken::new();
    let outcome = run_search(
        &deps,
        "You are a helpful assistant.",
        &[],
        question,
        16384,
        "2026-07-08",
        "en-US",
        &cancel,
        &|_phase| {},
    )
    .await;
    let messages = match outcome {
        SearchOutcome::Answer { messages, .. } => messages,
        SearchOutcome::Unreachable { .. } | SearchOutcome::NoSearch | SearchOutcome::Cancelled => {
            return None
        }
    };
    let params = OpenAiChatParams {
        base_url: base_url.to_string(),
        model: model.to_string(),
        messages,
        api_key: None,
        flavor: V1Flavor::Builtin,
        enable_thinking: false,
    };
    let answer = stream_openai_chat(params, client, cancel.clone(), |_| {}).await;
    if answer.trim().is_empty() {
        None
    } else {
        Some(answer)
    }
}

/// Runs the full deterministic-then-judge pipeline for every gradable corpus
/// row against a live `llama-server` and the live internet, and asserts the
/// gated SimpleQA-Verified confidently-wrong rate stays at or below
/// [`CONFIDENTLY_WRONG_GATE`]. See the module doc comment for corpus
/// composition, grading pipeline, and how to run this.
#[tokio::test]
#[ignore = "needs a live llama-server (THUKI_EVAL_PORT) and the live internet; run explicitly"]
async fn live_answer_quality_eval_over_corpus() {
    let port = std::env::var("THUKI_EVAL_PORT")
        .expect("set THUKI_EVAL_PORT to a running llama-server port");
    let base_url = format!("http://127.0.0.1:{port}");
    let model = "eval";
    let client = reqwest::Client::new();

    let (rows, stats) = load_full_corpus();
    for (source, skipped) in &stats.skipped {
        eprintln!("[j5-eval] {source}: skipped {skipped} malformed row(s)");
    }

    let mut per_source: HashMap<&'static str, GradeCounts> = HashMap::new();
    let mut composition: HashMap<&'static str, usize> = HashMap::new();
    let mut not_gradable = 0usize;

    for row in &rows {
        *composition.entry(row.source.name()).or_insert(0) += 1;
        let Some(gold) = &row.gold_answer else {
            not_gradable += 1;
            continue;
        };

        eprintln!(
            "[j5-eval] running: {:?} ({}, {})",
            row.question,
            row.source.name(),
            row.source.license()
        );
        let predicted = live_answer_for(&client, &base_url, model, &row.question).await;
        let Some(predicted) = predicted else {
            // Retrieval produced nothing citable: an explicit not-attempted
            // rather than dropping the row, so a dead retrieval path shows up
            // in the metrics instead of silently vanishing from the corpus.
            per_source
                .entry(row.source.name())
                .or_default()
                .record(SimpleQaVerdict::NotAttempted);
            continue;
        };

        let verdict = match grade_deterministic(&predicted, row) {
            DeterministicVerdict::Correct => SimpleQaVerdict::Correct,
            DeterministicVerdict::Residue => {
                majority_vote(
                    &client,
                    &base_url,
                    model,
                    &row.question,
                    gold,
                    &predicted,
                    &CancellationToken::new(),
                )
                .await
            }
        };
        eprintln!("[j5-eval]   -> {verdict:?}");
        per_source
            .entry(row.source.name())
            .or_default()
            .record(verdict);
    }

    eprintln!(
        "[j5-eval] corpus composition: {composition:?} (not_gradable={not_gradable}, see docs/search-eval.md)"
    );
    for (source, counts) in &per_source {
        eprintln!(
            "[j5-eval] {source}: n={} accuracy={:.2} confidently_wrong={:.2} not_attempted={:.2}",
            counts.total(),
            counts.accuracy(),
            counts.confidently_wrong_rate(),
            counts.not_attempted_rate()
        );
    }

    let simpleqa_counts = per_source
        .get(CorpusSource::SimpleqaVerified.name())
        .copied()
        .unwrap_or_default();
    let gated_rate = simpleqa_counts.confidently_wrong_rate();
    assert!(
        gated_rate <= CONFIDENTLY_WRONG_GATE,
        "SimpleQA-Verified confidently-wrong rate {gated_rate:.2} exceeds the {CONFIDENTLY_WRONG_GATE:.2} gate; see per-row output above"
    );
}

// ─── Unit tests: pure logic, no network, always run (never #[ignore]) ─────

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal gradable row for tests, with a fixed
    /// [`CorpusSource::SimpleqaVerified`] source (gating is not under test
    /// here; individual tests override fields as needed).
    fn row(gold: &str, acceptable: &[&str]) -> EvalRow {
        EvalRow {
            id: "test:0".to_string(),
            source: CorpusSource::SimpleqaVerified,
            question: "irrelevant for grading tests".to_string(),
            gold_answer: Some(gold.to_string()),
            acceptable_answers: acceptable.iter().map(|s| s.to_string()).collect(),
            volatility: Volatility::Never,
        }
    }

    /// A row with no gold answer, mirroring a decision-corpus row.
    fn ungradable_row() -> EvalRow {
        EvalRow {
            id: "decision_corpus:0".to_string(),
            source: CorpusSource::DecisionCorpus,
            question: "irrelevant".to_string(),
            gold_answer: None,
            acceptable_answers: Vec::new(),
            volatility: Volatility::Unknown,
        }
    }

    /// `normalize_text` case-folds, strips trailing punctuation, and
    /// collapses internal whitespace.
    #[test]
    fn normalize_text_folds_case_and_trims_punctuation() {
        assert_eq!(normalize_text("  Malia  Obama.  "), "malia obama");
        assert_eq!(normalize_text("KTVU"), "ktvu");
        assert_eq!(normalize_text("Eagle!"), "eagle");
        assert_eq!(normalize_text(""), "");
    }

    /// `parse_leading_number` handles currency symbols, thousands
    /// separators, plain integers, decimals, and returns `None` for
    /// non-numeric text.
    #[test]
    fn parse_leading_number_handles_common_formats() {
        assert_eq!(parse_leading_number("$19.99"), Some(19.99));
        assert_eq!(
            parse_leading_number("6,160 confirmed exoplanets"),
            Some(6160.0)
        );
        assert_eq!(parse_leading_number("3"), Some(3.0));
        assert_eq!(parse_leading_number("102510"), Some(102510.0));
        assert_eq!(parse_leading_number("Soreng"), None);
        assert_eq!(parse_leading_number(""), None);
    }

    /// `parse_acceptable_range` extracts both bounds from the SimpleQA
    /// inline hint, and returns `None` when the hint is absent or malformed.
    #[test]
    fn parse_acceptable_range_extracts_bounds_or_none() {
        assert_eq!(
            parse_acceptable_range("150 (acceptable range: anything between 148 and 152)"),
            Some((148.0, 152.0))
        );
        assert_eq!(parse_acceptable_range("Jóhanna Sigurðardóttir"), None);
        assert_eq!(parse_acceptable_range("between apples and oranges"), None);
    }

    /// `contains_word_sequence` matches a contiguous run of words and rejects
    /// out-of-order, longer-than-haystack, and empty needles.
    #[test]
    fn contains_word_sequence_matches_contiguous_runs_only() {
        assert!(contains_word_sequence(
            "the district is soreng today",
            "soreng"
        ));
        assert!(contains_word_sequence(
            "marjorie and james sanger center",
            "james sanger"
        ));
        assert!(!contains_word_sequence(
            "sanger james center",
            "james sanger"
        ));
        assert!(!contains_word_sequence(
            "short",
            "a much longer needle here"
        ));
        assert!(!contains_word_sequence("anything", ""));
    }

    /// An exact (case/punctuation-insensitive) match against the gold answer
    /// grades `Correct` without needing the judge.
    #[test]
    fn grade_deterministic_exact_match_is_correct() {
        let r = row("KTVU", &[]);
        assert_eq!(
            grade_deterministic("ktvu.", &r),
            DeterministicVerdict::Correct
        );
    }

    /// A numeric answer inside the embedded acceptable-range hint grades
    /// `Correct` even though it does not string-match the annotated gold
    /// text.
    #[test]
    fn grade_deterministic_numeric_range_hit_is_correct() {
        let r = row("150 (acceptable range: anything between 148 and 152)", &[]);
        assert_eq!(
            grade_deterministic("150", &r),
            DeterministicVerdict::Correct
        );
        assert_eq!(
            grade_deterministic("151", &r),
            DeterministicVerdict::Correct
        );
        assert_eq!(
            grade_deterministic("200", &r),
            DeterministicVerdict::Residue
        );
    }

    /// Bare numeric equality matches even when the textual forms differ
    /// (a currency symbol vs. a trailing unit word).
    #[test]
    fn grade_deterministic_bare_numeric_equality_is_correct() {
        let r = row("$19.99", &[]);
        assert_eq!(
            grade_deterministic("19.99 dollars", &r),
            DeterministicVerdict::Correct
        );
    }

    /// A predicted answer that contains the (sufficiently long) gold phrase
    /// as a word-sequence grades `Correct` via the fuzzy path.
    #[test]
    fn grade_deterministic_word_sequence_fuzzy_match_is_correct() {
        let r = row("Soreng", &[]);
        assert_eq!(
            grade_deterministic("The RTO code SK-06 belongs to Soreng district.", &r),
            DeterministicVerdict::Correct
        );
    }

    /// A candidate shorter than `MIN_FUZZY_CANDIDATE_LEN` is never used for
    /// fuzzy containment, so it cannot false-match inside an unrelated word.
    #[test]
    fn grade_deterministic_short_candidate_skips_fuzzy_path() {
        let r = row("no", &[]);
        assert_eq!(
            grade_deterministic("it is a well-known fact", &r),
            DeterministicVerdict::Residue
        );
    }

    /// A listed `acceptable_answers` alias matches exactly, same as the
    /// primary gold answer.
    #[test]
    fn grade_deterministic_acceptable_answers_alias_matches() {
        let r = row("Jan 4, 2026", &["January 4, 2026", "2026-01-4"]);
        assert_eq!(
            grade_deterministic("2026-01-4", &r),
            DeterministicVerdict::Correct
        );
    }

    /// No deterministic match at all is residue for the judge.
    #[test]
    fn grade_deterministic_no_match_is_residue() {
        let r = row("Christian McCaffrey", &[]);
        assert_eq!(
            grade_deterministic("I believe it's someone else entirely", &r),
            DeterministicVerdict::Residue
        );
    }

    /// A row with no gold answer (a decision-corpus row) is always residue:
    /// there is nothing to match against, and it must never be forced
    /// through grading.
    #[test]
    fn grade_deterministic_ungradable_row_is_residue() {
        let r = ungradable_row();
        assert_eq!(
            grade_deterministic("anything", &r),
            DeterministicVerdict::Residue
        );
    }

    /// An empty predicted answer is residue, not a false match against a
    /// short gold candidate.
    #[test]
    fn grade_deterministic_empty_predicted_is_residue() {
        let r = row("KTVU", &[]);
        assert_eq!(
            grade_deterministic("   ", &r),
            DeterministicVerdict::Residue
        );
    }

    /// A 3-0 or 2-1 majority resolves to the majority verdict, regardless of
    /// which verdict it is.
    #[test]
    fn resolve_majority_picks_the_strict_majority() {
        use SimpleQaVerdict::*;
        assert_eq!(resolve_majority(&[Correct, Correct, Correct]), Correct);
        assert_eq!(resolve_majority(&[Correct, Correct, Incorrect]), Correct);
        assert_eq!(
            resolve_majority(&[Incorrect, Incorrect, Correct]),
            Incorrect
        );
        assert_eq!(
            resolve_majority(&[NotAttempted, NotAttempted, Incorrect]),
            NotAttempted
        );
    }

    /// When all 3 calls disagree, or fewer than 2 succeeded and agreed, the
    /// verdict is `NotAttempted` (the conservative default; see
    /// `resolve_majority`'s doc comment).
    #[test]
    fn resolve_majority_falls_back_to_not_attempted_when_indecisive() {
        use SimpleQaVerdict::*;
        assert_eq!(
            resolve_majority(&[Correct, Incorrect, NotAttempted]),
            NotAttempted
        );
        assert_eq!(resolve_majority(&[]), NotAttempted);
        assert_eq!(resolve_majority(&[Correct]), NotAttempted);
        assert_eq!(resolve_majority(&[Correct, Correct]), Correct);
    }

    /// A fresh `GradeCounts` reports all rates as `0.0` rather than dividing
    /// by zero.
    #[test]
    fn grade_counts_zero_state_never_divides_by_zero() {
        let counts = GradeCounts::default();
        assert_eq!(counts.total(), 0);
        assert_eq!(counts.attempted(), 0);
        assert_eq!(counts.confidently_wrong_rate(), 0.0);
        assert_eq!(counts.accuracy(), 0.0);
        assert_eq!(counts.not_attempted_rate(), 0.0);
    }

    /// Recording a mix of verdicts computes accuracy, confidently-wrong rate,
    /// and not-attempted rate correctly, including when there are
    /// not-attempted rows but zero attempted rows.
    #[test]
    fn grade_counts_computes_rates_from_recorded_verdicts() {
        let mut counts = GradeCounts::default();
        counts.record(SimpleQaVerdict::Correct);
        counts.record(SimpleQaVerdict::Correct);
        counts.record(SimpleQaVerdict::Incorrect);
        counts.record(SimpleQaVerdict::NotAttempted);
        assert_eq!(counts.total(), 4);
        assert_eq!(counts.attempted(), 3);
        assert!((counts.accuracy() - 0.5).abs() < 1e-9);
        assert!((counts.confidently_wrong_rate() - (1.0 / 3.0)).abs() < 1e-9);
        assert!((counts.not_attempted_rate() - 0.25).abs() < 1e-9);

        let mut only_not_attempted = GradeCounts::default();
        only_not_attempted.record(SimpleQaVerdict::NotAttempted);
        assert_eq!(only_not_attempted.confidently_wrong_rate(), 0.0);
    }

    /// Only `SimpleqaVerified` gates; the other three sources are tracked,
    /// never gated.
    #[test]
    fn corpus_source_gating_is_simpleqa_verified_only() {
        assert!(CorpusSource::SimpleqaVerified.gating());
        assert!(!CorpusSource::Freshqa.gating());
        assert!(!CorpusSource::Seal0.gating());
        assert!(!CorpusSource::DecisionCorpus.gating());
    }

    /// `name()` and `license()` are distinct, non-empty, and stable per
    /// source (a basic sanity check, not a change-detector).
    #[test]
    fn corpus_source_name_and_license_are_populated() {
        for source in [
            CorpusSource::SimpleqaVerified,
            CorpusSource::Freshqa,
            CorpusSource::Seal0,
            CorpusSource::DecisionCorpus,
        ] {
            assert!(!source.name().is_empty());
            assert!(!source.license().is_empty());
        }
    }

    /// `Volatility::parse` recognises all four known buckets and falls back
    /// to `Unknown` for anything else.
    #[test]
    fn volatility_parse_covers_known_buckets_and_unknown_fallback() {
        assert_eq!(Volatility::parse("never"), Volatility::Never);
        assert_eq!(Volatility::parse("slow"), Volatility::Slow);
        assert_eq!(Volatility::parse("fast"), Volatility::Fast);
        assert_eq!(Volatility::parse("false-premise"), Volatility::FalsePremise);
        assert_eq!(Volatility::parse("bogus"), Volatility::Unknown);
        assert_eq!(Volatility::parse(""), Volatility::Unknown);
    }

    /// `load_gradable_source` parses well-formed lines, skips blank lines,
    /// and skips (without panicking) a malformed line and a line missing a
    /// required field, counting both in `LoadStats`.
    #[test]
    fn load_gradable_source_skips_malformed_rows_and_counts_them() {
        let jsonl = concat!(
            "{\"id\": \"1\", \"question\": \"q1\", \"gold_answer\": \"a1\", \"acceptable_answers\": [], \"volatility\": \"never\"}\n",
            "\n",
            "not json at all\n",
            "{\"id\": \"2\", \"question\": \"q2\"}\n",
            "{\"id\": \"3\", \"question\": \"q3\", \"gold_answer\": \"a3\"}\n",
        );
        let mut stats = LoadStats::default();
        let rows = load_gradable_source(CorpusSource::SimpleqaVerified, jsonl, &mut stats);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "simpleqa_verified:1");
        assert_eq!(rows[0].gold_answer.as_deref(), Some("a1"));
        assert_eq!(rows[1].id, "simpleqa_verified:3");
        assert!(rows[1].acceptable_answers.is_empty());
        assert_eq!(stats.skipped.get("simpleqa_verified"), Some(&2));
    }

    /// `load_decision_corpus` parses rows with no gold answer, defaults a
    /// missing `volatility`, and skips (without panicking) a malformed line.
    #[test]
    fn load_decision_corpus_parses_rows_with_no_gold_answer() {
        let jsonl = concat!(
            "{\"message\": \"what is the capital of France\", \"label\": \"no\", \"category\": \"stable_fact\", \"volatility\": \"never\"}\n",
            "{\"message\": \"weather in Tokyo\", \"label\": \"search\", \"category\": \"weather\"}\n",
            "not json\n",
        );
        let mut stats = LoadStats::default();
        let rows = load_decision_corpus(jsonl, &mut stats);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.gold_answer.is_none()));
        assert!(rows
            .iter()
            .all(|r| r.source == CorpusSource::DecisionCorpus));
        assert_eq!(rows[0].volatility, Volatility::Never);
        assert_eq!(rows[1].volatility, Volatility::Unknown);
        assert_eq!(stats.skipped.get("decision_corpus"), Some(&1));
    }

    /// The full corpus loads all four sources with the expected row counts
    /// and no skipped rows (the committed files are well-formed), and only
    /// decision-corpus rows carry no gold answer.
    #[test]
    fn load_full_corpus_has_expected_composition() {
        let (rows, stats) = load_full_corpus();
        assert!(
            stats.skipped.is_empty(),
            "committed corpus files must all parse cleanly"
        );

        let count = |source: CorpusSource| rows.iter().filter(|r| r.source == source).count();
        assert_eq!(count(CorpusSource::SimpleqaVerified), 50);
        assert_eq!(count(CorpusSource::Freshqa), 30);
        assert_eq!(count(CorpusSource::Seal0), 15);
        assert_eq!(count(CorpusSource::DecisionCorpus), 138);
        assert_eq!(rows.len(), 233);

        for r in &rows {
            let should_have_gold = r.source != CorpusSource::DecisionCorpus;
            assert_eq!(
                r.gold_answer.is_some(),
                should_have_gold,
                "row {:?} from {:?} has unexpected gold_answer presence",
                r.id,
                r.source
            );
        }
    }

    /// The judge prompt carries the substitution placeholders, and a couple
    /// of distinctive anchor phrases from the published template, as a
    /// regression guard against an accidental edit to the "verbatim" text.
    #[test]
    fn grader_template_carries_placeholders_and_anchor_phrases() {
        let t = GRADER_TEMPLATE.trim();
        assert!(t.contains("{question}"));
        assert!(t.contains("{target}"));
        assert!(t.contains("{predicted_answer}"));
        assert!(t.contains("Malia Obama and Sasha Obama"));
        assert!(
            t.contains("Just return the letters \"A\", \"B\", or \"C\", with no text around it.")
        );
        assert!(!t.starts_with(char::is_whitespace));
        assert!(!t.ends_with(char::is_whitespace));
    }

    /// The judge's `response_format` schema requires exactly the `grade`
    /// field, constrained to the three letters.
    #[test]
    fn judge_schema_constrains_grade_to_three_letters() {
        let schema = judge_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["properties"]["grade"]["enum"],
            serde_json::json!(["A", "B", "C"])
        );
        assert_eq!(schema["required"], serde_json::json!(["grade"]));
        assert_eq!(schema["additionalProperties"], false);
    }
}
