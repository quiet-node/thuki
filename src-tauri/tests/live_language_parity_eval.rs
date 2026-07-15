//! Baseline-vs-branch classifier language-parity eval.
//!
//! Answers three questions the unit tests structurally cannot, against a REAL
//! llama-server, for the classifier prompt change that added the
//! language-preservation instruction and the `lang` field to `prepass.rs`:
//!
//! 1. Did the English-corpus decision accuracy (should-search vs
//!    should-not-search) move, comparing the BASELINE prompt (pre
//!    language-change, extracted verbatim from git history at the merge-base
//!    with `main`) against the BRANCH prompt (today's `CLASSIFIER_SYSTEM`, run
//!    through the real production [`BuiltinPrePass`])? Scoped to the ASCII
//!    rows of `search_decision_eval.jsonl`: every non-English row in that
//!    corpus carries a non-ASCII character, so `message.is_ascii()` is an
//!    exact, no-hand-tagging partition.
//! 2. For every non-English row, does the branch classifier's `lang` field
//!    match the row's true language (encoded as the category's language
//!    prefix, e.g. `vi_weather` -> `vi`)?
//! 3. For every non-English row, are `queries`/`standalone_question` written
//!    in the source language? Printed per-row for a human tally (an English
//!    companion query alongside the native one is EXPECTED and does not fail
//!    a row; see `prepass.rs` module docs).
//!
//! The BASELINE arm does not reuse [`BuiltinPrePass`] (which is hardwired to
//! today's `CLASSIFIER_SYSTEM`, the thing under test): it calls the real
//! production [`request_openai_json`] transport directly with the OLD prompt
//! and OLD schema, so the wire shape (temperature 0, the reasoning-suppression
//! `chat_template_kwargs`) is byte-identical to production and only the
//! prompt/schema differ.
//!
//! `#[ignore]`d exactly like its siblings: never touches CI, coverage, or a
//! network call by default. Run explicitly against one or two already-running
//! engines (`THUKI_EVAL_PORT_GPT_OSS` and, optionally, `THUKI_EVAL_PORT_GEMMA`
//! when a second model is loaded on its own port):
//!
//! ```sh
//! THUKI_EVAL_PORT_GPT_OSS=8812 THUKI_EVAL_PORT_GEMMA=8813 \
//!   cargo test --test live_language_parity_eval -- --ignored --nocapture --test-threads=1
//! ```

use thuki_agent_lib::commands::ChatMessage;
use thuki_agent_lib::config::defaults::{PREPASS_MAX_TOKENS, PREPASS_TIMEOUT_S};
use thuki_agent_lib::openai::{request_openai_json, V1Flavor};
use thuki_agent_lib::websearch::prefilter::{prefilter, PreFilterVerdict};
use thuki_agent_lib::websearch::prepass::{BuiltinPrePass, PrePass, SearchDecision};

use tokio_util::sync::CancellationToken;

/// The classifier system prompt as it existed BEFORE the language-preservation
/// change: extracted verbatim (byte-for-byte, by script, never hand-retyped)
/// from `git show 2a4d749:src-tauri/src/websearch/prepass.rs`, where `2a4d749`
/// is the merge-base this branch diverged from `main` at. This is the CONTROL
/// arm of measurement 1 and must never be edited to chase a result.
const BASELINE_CLASSIFIER_SYSTEM: &str = "Reasoning: low\n\nYou are a retrieval-routing classifier inside a local AI assistant. Your only job is to decide whether answering the user's latest message needs a fresh web search, to pick which source best answers it, and if so to rewrite it into a standalone search query. You never answer the message itself.\n\nOutput ONLY a JSON object: {\"search\": \"no\"|\"cached\"|\"web\", \"route\": \"weather\"|\"news\"|\"wiki\"|\"sports\"|\"web\", \"standalone_question\": \"...\", \"queries\": [\"...\"], \"explicit_search\": true|false}.\n\nChoose \"search\":\n- \"web\" when a good answer needs any of: (a) recent events, news, or announcements; (b) current prices, rates, scores, weather, or statistics; (c) a fact about a specific person, organization, or product that can change after your training cutoff, such as an age, title, role, employer, team, marital status, ownership, net worth, or current status; (d) an explicit request to search or verify; or any release, version, schedule, or other live fact. A present-tense attribute of a person or entity (\"how old is X now\", \"is Z still married\") is a \"web\" turn even with no freshness word: your training is frozen and the date you are given does not refresh what you remember.\n- \"cached\" ONLY when this message repeats or rephrases a question the assistant already searched and answered earlier in this same conversation, and those exact sources still answer it. A follow-up that drills into a NEW detail of the same topic (an exact time, an exact figure, a breakdown) is NOT cached: choose \"web\" with a refined standalone question, because the earlier sources did not carry that detail.\n- \"no\" only for a stable answer you can give confidently: an established or historical fact, math, a science or coding fundamental, a creative or text-transform task, analysis of text already provided, or a greeting or conversational turn.\nWhen you are unsure whether your knowledge is up to date, choose \"web\": a needless search is far cheaper than a confidently wrong answer.\n\nChoose \"route\" (which source best answers it):\n- \"weather\" for current weather or forecast for a place.\n- \"news\" for current events, elections, and anything asking the latest, current, or recent state of an evolving topic (a conflict, a company, a policy) that is not a live score, fixture, or standings.\n- \"wiki\" for stable definitional or historical facts that do not change from month to month.\n- \"sports\" for live scores, fixtures, or standings for a named competition or team, or the status of an ongoing match or tournament.\n- \"web\" for everything else (software versions, prices, product specs, niche live facts).\nWhen a question is about the present state of an ongoing event, route \"news\" (or \"sports\" for a score/fixture/standings question), never \"wiki\", even if it is phrased like \"what is ...\". Always set a route, even when search is \"no\".\n\n\"standalone_question\": the latest message rewritten as one self-contained question, resolving pronouns and references from the conversation, including entities named in the assistant's previous answers, not only in the user's questions. When the follow-up is an ellipsis like \"how about X?\" or \"what about X?\", keep the SAME question the conversation was already asking and swap in only the new subject X; do not invent a different kind of question.\n\"queries\": 1 to 3 short keyword search queries, not full sentences.\n\"explicit_search\": true ONLY when the user explicitly asks you to look it up, search, verify, double-check, or confirm (\"can you look it up\", \"search for it\", \"double-check that\"); otherwise false. When true, also set \"search\":\"web\" and put the FULL topic being looked up into the standalone_question, resolved from the conversation, never the literal words \"look it up\".\n\nExamples (message -> JSON):\n\"who is the CEO of OpenAI right now\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current CEO of OpenAI\",\"queries\":[\"openai ceo\"]}\n\"what is the boiling point of water\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what is the boiling point of water\",\"queries\":[\"boiling point of water\"]}\n\"what is photosynthesis\" -> {\"search\":\"web\",\"route\":\"wiki\",\"standalone_question\":\"what is photosynthesis\",\"queries\":[\"photosynthesis\"]}\n\"weather in Paris\" -> {\"search\":\"web\",\"route\":\"weather\",\"standalone_question\":\"what is the current weather in Paris\",\"queries\":[\"paris weather\"]}\n\"what's the latest status of the World Cup 2026\" -> {\"search\":\"web\",\"route\":\"news\",\"standalone_question\":\"what is the current status of the 2026 World Cup\",\"queries\":[\"world cup 2026 status\"]}\n\"who won the most recent F1 race\" -> {\"search\":\"web\",\"route\":\"news\",\"standalone_question\":\"who won the most recent Formula 1 race\",\"queries\":[\"latest f1 race winner\"]}\n\"what's the score of the Lakers game\" -> {\"search\":\"web\",\"route\":\"sports\",\"standalone_question\":\"what is the current score of the Los Angeles Lakers game\",\"queries\":[\"lakers score\"]}\n(you already searched and answered \"what's the latest stable Rust version\" with web sources earlier in this conversation) \"what's the latest stable Rust version\" -> {\"search\":\"cached\",\"route\":\"web\",\"standalone_question\":\"what is the latest stable Rust version\",\"queries\":[\"rust latest stable version\"]}\n\"write a short poem about autumn\" -> {\"search\":\"no\",\"route\":\"web\",\"standalone_question\":\"write a short poem about autumn\",\"queries\":[\"autumn poem\"]}\n(after discussing France) \"and its population?\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what is the population of France\",\"queries\":[\"france population\"]}\n(after discussing the US president) \"what about Argentina?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current president of Argentina\",\"queries\":[\"argentina president\"]}\n(you just told the user Elon Musk's net worth is about $240 billion) \"How about Donald Trump?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"what is Donald Trump's net worth\",\"queries\":[\"donald trump net worth\"]}\n(your previous answer said Jensen Huang is the CEO of Nvidia) \"how much is he worth?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"what is Jensen Huang's net worth\",\"queries\":[\"jensen huang net worth\"]}\n(you just told the user Elon Musk's net worth is about $240 billion) \"and how old is he now?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"how old is Elon Musk\",\"queries\":[\"elon musk age\"],\"explicit_search\":false}\n\"how old is the Pope\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"how old is the current Pope\",\"queries\":[\"pope age\"],\"explicit_search\":false}\n(you told the user the Belgium vs Spain 2026 World Cup match is today but the scoreboard carried no kickoff time) \"can you look it up please?\" -> {\"search\":\"web\",\"route\":\"sports\",\"standalone_question\":\"what time is the Belgium vs Spain 2026 World Cup match today\",\"queries\":[\"belgium spain world cup kickoff time\"],\"explicit_search\":true}\n(you just gave the World Cup match's final score) \"and at what exact time did it kick off?\" -> {\"search\":\"web\",\"route\":\"sports\",\"standalone_question\":\"what time did the Belgium vs Spain World Cup match kick off\",\"queries\":[\"belgium spain world cup kickoff time\"],\"explicit_search\":false}";

/// The `response_format` schema paired with [`BASELINE_CLASSIFIER_SYSTEM`]: no
/// `lang` field, `lang` not required. Matches the schema shape at `2a4d749`.
fn baseline_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "search": { "type": "string", "enum": ["no", "cached", "web"] },
            "route": { "type": "string", "enum": ["weather", "news", "wiki", "sports", "web"] },
            "standalone_question": { "type": "string" },
            "queries": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": 3
            },
            "explicit_search": { "type": "boolean" }
        },
        "required": ["search", "route", "standalone_question", "queries", "explicit_search"],
        "additionalProperties": false
    })
}

/// Mirrors `prepass::build_classifier_user_turn`'s output for the no-history,
/// no-images shape every corpus row uses. That helper is `pub(crate)` and
/// unreachable from an integration test; this text-assembly shape is
/// unchanged by the language-preservation prompt edit, so duplicating only
/// this (not `CLASSIFIER_SYSTEM`) does not touch the code under measurement.
fn user_turn(message: &str, today: &str) -> String {
    format!(
        "Latest message: {}\n\nToday's date is {}.\nDecide for the latest message and output only the JSON object.",
        message.trim(),
        today
    )
}

/// Calls the engine at `base_url` with the BASELINE prompt/schema via the same
/// production [`request_openai_json`] transport [`BuiltinPrePass`] uses (so
/// `temperature: 0` and the reasoning-suppression `chat_template_kwargs` are
/// byte-identical to production; only the prompt/schema differ). A transport
/// failure or unparseable body degrades to "no search", mirroring production's
/// failure policy.
async fn baseline_is_search(base_url: &str, message: &str, today: &str) -> bool {
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: BASELINE_CLASSIFIER_SYSTEM.to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_turn(message, today),
            images: None,
        },
    ];
    let raw = request_openai_json(
        base_url,
        "eval",
        &reqwest::Client::new(),
        messages,
        baseline_schema(),
        None,
        PREPASS_TIMEOUT_S,
        PREPASS_MAX_TOKENS,
        V1Flavor::Builtin,
        &CancellationToken::new(),
    )
    .await;
    let content = match raw {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[parity] baseline transport error on {message:?}: {e:?}");
            return false;
        }
    };
    #[derive(serde::Deserialize)]
    struct Wire {
        search: String,
    }
    match serde_json::from_str::<Wire>(content.trim()) {
        Ok(w) => matches!(
            w.search.trim().to_ascii_lowercase().as_str(),
            "cached" | "web"
        ),
        Err(e) => {
            eprintln!("[parity] baseline unparseable body on {message:?}: {e} :: {content}");
            false
        }
    }
}

/// One labelled corpus row. `category` doubles as the non-English language tag
/// for rows outside the original English set: a `<lang>_` prefix (e.g.
/// `vi_weather`, `ja_wiki`) rather than a new JSON field, so the row schema
/// stays exactly what `search_decision_eval.jsonl` already declares.
#[derive(serde::Deserialize, Clone)]
struct EvalRow {
    message: String,
    label: String,
    category: String,
}

fn corpus() -> Vec<EvalRow> {
    include_str!("../src/websearch/search_decision_eval.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid corpus row"))
        .collect()
}

/// The true language for a non-English row, read off its `<lang>_` category
/// prefix, or `None` for an English (ASCII) row.
fn true_lang(row: &EvalRow) -> Option<&str> {
    const KNOWN_PREFIXES: &[&str] = &["vi", "ja", "zh", "ar", "th", "bn"];
    let prefix = row.category.split('_').next()?;
    KNOWN_PREFIXES.contains(&prefix).then_some(prefix)
}

/// The production two-stage decision collapsed to "would this turn search?",
/// against the BRANCH classifier (today's real [`BuiltinPrePass`]).
async fn branch_is_search(prepass: &BuiltinPrePass, message: &str, today: &str) -> bool {
    match prefilter(message, today) {
        PreFilterVerdict::ForceNo => false,
        PreFilterVerdict::ForceWeb => true,
        PreFilterVerdict::Ambiguous => match prepass
            .decide(&[], message, None, today, &CancellationToken::new())
            .await
        {
            Ok(decision) => !matches!(decision.decision, SearchDecision::No),
            Err(e) => {
                eprintln!("[parity] branch classifier error on {message:?}: {e}");
                false
            }
        },
    }
}

/// Against BASELINE.
async fn baseline_would_search(base_url: &str, message: &str, today: &str) -> bool {
    match prefilter(message, today) {
        PreFilterVerdict::ForceNo => false,
        PreFilterVerdict::ForceWeb => true,
        PreFilterVerdict::Ambiguous => baseline_is_search(base_url, message, today).await,
    }
}

/// Runs Measurement 1 (English-only decision accuracy, baseline vs branch)
/// against one engine, plus a direct probe of the literal handoff phrase
/// "what is photosynthesis" (not itself a corpus row; the corpus carries
/// "explain how photosynthesis works" instead) for the drift adjudication.
async fn run_measurement_1(base_url: &str, model_label: &str) {
    let today = "2026-07-08";
    let rows: Vec<EvalRow> = corpus().into_iter().filter(|r| r.message.is_ascii()).collect();

    let mut baseline_correct = 0usize;
    let mut branch_correct = 0usize;
    let mut baseline_misses = Vec::new();
    let mut branch_misses = Vec::new();
    let prepass = BuiltinPrePass::new(
        reqwest::Client::new(),
        base_url.to_string(),
        "eval".to_string(),
        PREPASS_TIMEOUT_S,
    );

    for row in &rows {
        let want_search = row.label == "search";

        let got_baseline = baseline_would_search(base_url, &row.message, today).await;
        if got_baseline == want_search {
            baseline_correct += 1;
        } else {
            baseline_misses.push(format!(
                "  BASELINE MISS [{}] ({}): want {} got {} :: {}",
                row.label,
                row.category,
                want_search,
                got_baseline,
                row.message
            ));
        }

        let got_branch = branch_is_search(&prepass, &row.message, today).await;
        if got_branch == want_search {
            branch_correct += 1;
        } else {
            branch_misses.push(format!(
                "  BRANCH MISS [{}] ({}): want {} got {} :: {}",
                row.label,
                row.category,
                want_search,
                got_branch,
                row.message
            ));
        }
    }

    let total = rows.len();
    eprintln!(
        "\n[parity][{model_label}] MEASUREMENT 1 (English corpus, n={total}): baseline_correct={baseline_correct} ({:.3}) branch_correct={branch_correct} ({:.3})",
        baseline_correct as f64 / total as f64,
        branch_correct as f64 / total as f64,
    );
    for m in &baseline_misses {
        eprintln!("{m}");
    }
    for m in &branch_misses {
        eprintln!("{m}");
    }

    // The handoff's exact phrase, not itself a corpus row: the prompt's own
    // few-shot (present in BOTH baseline and branch, unchanged by this diff)
    // says "what is photosynthesis" -> "search":"web". Report what each side
    // actually decides for the literal string.
    let photosynthesis = "what is photosynthesis";
    let baseline_photo = baseline_would_search(base_url, photosynthesis, today).await;
    let branch_photo = branch_is_search(&prepass, photosynthesis, today).await;
    eprintln!(
        "[parity][{model_label}] photosynthesis literal probe {photosynthesis:?}: baseline_search={baseline_photo} branch_search={branch_photo} (prompt's own few-shot says web for both)"
    );
}

/// Runs Measurements 2 and 3 (branch-only: the `lang` field and source-language
/// queries) against one engine, over every non-English corpus row.
async fn run_measurements_2_and_3(base_url: &str, model_label: &str) {
    let today = "2026-07-08";
    let prepass = BuiltinPrePass::new(
        reqwest::Client::new(),
        base_url.to_string(),
        "eval".to_string(),
        PREPASS_TIMEOUT_S,
    );
    let rows: Vec<EvalRow> = corpus()
        .into_iter()
        .filter(|r| !r.message.is_ascii())
        .collect();

    let mut lang_correct = 0usize;
    let mut lang_total = 0usize;
    eprintln!("\n[parity][{model_label}] MEASUREMENT 2 & 3 (non-English rows, n={})", rows.len());
    for row in &rows {
        let Some(want_lang) = true_lang(&row) else {
            eprintln!("  SKIP (no known lang prefix): {} :: {}", row.category, row.message);
            continue;
        };
        match prepass
            .decide(&[], &row.message, None, today, &CancellationToken::new())
            .await
        {
            Ok(decision) => {
                lang_total += 1;
                let lang_hit = decision.lang == want_lang;
                if lang_hit {
                    lang_correct += 1;
                }
                eprintln!(
                    "  [{}] want_lang={want_lang} got_lang={:?} lang_ok={lang_hit} standalone={:?} queries={:?} :: {}",
                    row.category, decision.lang, decision.standalone_question, decision.queries, row.message
                );
            }
            Err(e) => {
                eprintln!(
                    "  CLASSIFIER ERROR ({}): {e} :: {}",
                    row.category, row.message
                );
            }
        }
    }
    eprintln!(
        "[parity][{model_label}] lang field accuracy: {lang_correct}/{lang_total} ({:.3})",
        if lang_total > 0 {
            lang_correct as f64 / lang_total as f64
        } else {
            0.0
        }
    );
}

/// Runs the full parity eval against one engine `base_url`, labelled
/// `model_label` in the output (e.g. "gpt-oss", "gemma").
async fn run_parity_eval(base_url: &str, model_label: &str) {
    run_measurement_1(base_url, model_label).await;
    run_measurements_2_and_3(base_url, model_label).await;
}

#[tokio::test]
#[ignore = "needs a live llama-server; set THUKI_EVAL_PORT_GPT_OSS / THUKI_EVAL_PORT_GEMMA"]
async fn live_language_parity_eval() {
    let mut ran_any = false;

    if let Ok(port) = std::env::var("THUKI_EVAL_PORT_GPT_OSS") {
        run_parity_eval(&format!("http://127.0.0.1:{port}"), "gpt-oss").await;
        ran_any = true;
    } else {
        eprintln!("[parity] THUKI_EVAL_PORT_GPT_OSS not set; skipping gpt-oss");
    }

    if let Ok(port) = std::env::var("THUKI_EVAL_PORT_GEMMA") {
        run_parity_eval(&format!("http://127.0.0.1:{port}"), "gemma").await;
        ran_any = true;
    } else {
        eprintln!(
            "[parity] THUKI_EVAL_PORT_GEMMA not set; gemma UNMEASURED. Not simulated, not extrapolated."
        );
    }

    assert!(
        ran_any,
        "set at least THUKI_EVAL_PORT_GPT_OSS to run this eval"
    );
}
