//! The sufficiency judge: a bounded check, run after a vertical answers, on
//! whether the block it returned actually contains what the question asked.
//!
//! ## Why this stage exists
//!
//! The verticals ([`crate::websearch::weather`], [`crate::websearch::sports`],
//! [`crate::websearch::news`], [`crate::websearch::encyclopedia`]) are keyless
//! official APIs tried ahead of the scraped engines. Each answers a shape of
//! question directly, but "the vertical returned a block" is not the same as
//! "the block answers the question": a World-Cup scoreboard carries today's
//! fixture but not the full knockout bracket, a news feed carries headlines but
//! not a specific figure. Before this stage, any vertical block was committed
//! unconditionally, and the writer, correctly refusing to invent, produced a
//! bare "the sources do not contain that" dead end. That reads to a user as
//! "this app cannot search", the worst possible outcome.
//!
//! This stage reads the sufficiency verdict the writer would otherwise compute
//! one call too late, and lets the orchestrator act on it: an insufficient
//! vertical block escalates to the scraped engines (which subsume the vertical's
//! narrower page) instead of dead-ending. The judge only ever runs on a single
//! small vertical block, so its prefill is cheap; the scraped-engine tier is
//! terminal and is never judged (there is nowhere to escalate to).
//!
//! ## Failure policy: fail toward committing
//!
//! A judge call that fails at the transport level, or returns a body that does
//! not parse, degrades to "sufficient" (commit the block the vertical already
//! returned). This is the safe default on two counts: the user still gets the
//! vertical's answer rather than a stall, and a spurious escalation is never
//! triggered on judge noise, so the scraped engines' volume-triggered rate
//! limits (see [`crate::websearch::engine`]) are never spent on a false
//! insufficiency. An escalation only ever happens on a confident `false`.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;
use crate::websearch::assemble::SourceBlock;
use crate::websearch::prepass::InferenceError;
use crate::websearch::writer::strip_invisible;

/// The judge's verdict on one retrieved source set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SufficiencyVerdict {
    /// Whether the sources contain the specific information the question asks
    /// for, enough to answer it directly.
    pub sufficient: bool,
    /// When `sufficient` is false, a short phrase naming what the sources lack,
    /// carried into the forensic trace so an escalation is diagnosable. Empty
    /// when `sufficient` is true.
    pub missing: String,
}

impl SufficiencyVerdict {
    /// The fail-toward-committing default (see module docs): treat the block as
    /// sufficient, so a judge failure never stalls the user or spends an engine
    /// request on a false escalation.
    fn commit() -> Self {
        Self {
            sufficient: true,
            missing: String::new(),
        }
    }
}

/// Injectable sufficiency inference. The orchestrator depends on this trait so
/// its commit-or-escalate branch is tested with [`FakeSufficiencyJudge`]; the
/// builtin engine backing lives in the coverage-excluded
/// [`BuiltinSufficiencyJudge`].
#[async_trait]
pub trait SufficiencyJudge: Send + Sync {
    /// Judges whether `sources` contain what `standalone_question` asks for.
    /// Never returns a bad-JSON error: an unparseable model response degrades
    /// in-band to a "sufficient" verdict (commit). A transport-level failure is
    /// surfaced as [`InferenceError`] for the caller to fold into the same
    /// commit default (or a cancellation).
    async fn judge(
        &self,
        standalone_question: &str,
        sources: &[SourceBlock],
        cancel: &CancellationToken,
    ) -> Result<SufficiencyVerdict, InferenceError>;
}

/// The judge's own persona-free system prompt. Like the classifier
/// ([`crate::websearch::prepass`]), it runs under its own short role, not the
/// chat persona, so the decision is not coloured by how the assistant is told
/// to talk to the user.
///
/// The leading `Reasoning: low` line is the gpt-oss (harmony) reasoning-effort
/// directive: a sufficiency check is a bounded yes/no, and without the directive
/// these models spend hundreds of chain-of-thought tokens on it and can blow the
/// call timeout. Inert plain text for every other model family.
const JUDGE_SYSTEM: &str = "Reasoning: low\n\nYou are a retrieval-sufficiency checker inside a local AI assistant. You are given the user's question and the web source(s) that were retrieved to answer it. Your only job is to decide whether those sources actually CONTAIN the specific information the question asks for. You never answer the question yourself.\n\nOutput ONLY a JSON object: {\"sufficient\": true|false, \"missing\": \"...\"}.\n- \"sufficient\": true when the sources directly contain the specific facts the question asks for, enough to answer it, even when they state those facts briefly (a date, a time, a score, or a single figure in a listing IS the answer when that is what was asked). false when the sources are about the right topic but do NOT contain the specific detail asked: for example the question asks for a full list, a complete breakdown, an exact figure, or a specific person or event, and the sources give only a related or partial fact.\n- \"missing\": when sufficient is false, a short phrase (a few words) naming what the sources lack; an empty string when sufficient is true.\n\nJudge only what the source text literally contains, never what you happen to know about the topic. A source that merely names the subject, or gives one related fact while the question asks for another, is NOT sufficient.\n\nExamples:\nQuestion: \"give me all the teams from the round of 32 until now\" | Sources: a single scoreboard listing only today's scheduled quarterfinal match -> {\"sufficient\":false,\"missing\":\"round-of-32 and round-of-16 results\"}\nQuestion: \"what is the current weather in Tokyo\" | Sources: a weather block with Tokyo's current temperature and 3-day forecast -> {\"sufficient\":true,\"missing\":\"\"}\nQuestion: \"how many Instagram followers does the Cape Verde goalkeeper have\" | Sources: a scoreboard of World Cup fixtures -> {\"sufficient\":false,\"missing\":\"goalkeeper follower count\"}\nQuestion: \"who won the most recent Formula 1 race\" | Sources: a news headline reading \"Leclerc wins dramatic British GP\" -> {\"sufficient\":true,\"missing\":\"\"}\nQuestion: \"at what exact time is the next match\" | Sources: a scoreboard listing the next match with its date and kickoff time -> {\"sufficient\":true,\"missing\":\"\"}";

/// The trailing instruction on the judge's user turn.
const JUDGE_INSTRUCTION: &str =
    "Decide whether the sources contain what the question asks, and output only the JSON object.";

/// Header introducing the retrieved-source listing in the judge's user turn.
const SOURCES_HEADER: &str = "Retrieved sources:";

/// Builds the `response_format` JSON schema constraining the judge output.
pub(crate) fn judge_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "sufficient": { "type": "boolean" },
            "missing": { "type": "string" }
        },
        "required": ["sufficient", "missing"],
        "additionalProperties": false
    })
}

/// Assembles the judge message array: the judge's own system prompt, then a
/// single user turn embedding the question and a numbered listing of the
/// retrieved sources. Source titles and text are stripped of invisible/bidi
/// control characters (the same defense the writer applies) so a source cannot
/// smuggle a hidden instruction into the judge; the judge is read-only, so the
/// worst case of a successful injection is a wrong verdict (a needless
/// escalation or a committed block), never an action.
pub(crate) fn build_judge_messages(
    standalone_question: &str,
    sources: &[SourceBlock],
) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".to_string(),
            content: JUDGE_SYSTEM.to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: build_judge_user_turn(standalone_question, sources),
            images: None,
        },
    ]
}

/// Builds the judge's single user turn: the question, a numbered `[n] Title`
/// header and body per source, and the output instruction.
fn build_judge_user_turn(standalone_question: &str, sources: &[SourceBlock]) -> String {
    let mut out = String::new();
    out.push_str("Question: ");
    out.push_str(standalone_question.trim());
    out.push_str("\n\n");
    out.push_str(SOURCES_HEADER);
    for block in sources {
        out.push_str(&format!(
            "\n\n[{}] {}\n{}",
            block.index,
            strip_invisible(&block.title),
            strip_invisible(&block.text),
        ));
    }
    out.push_str("\n\n");
    out.push_str(JUDGE_INSTRUCTION);
    out
}

/// The wire shape the grammar constrains the model to. `missing` is
/// `#[serde(default)]` so a body that omits it (a `sufficient:true` verdict
/// with no phrase) still parses; `sufficient` is required, so a body missing it
/// fails to parse and degrades to the commit default via [`judge_or_commit`].
#[derive(serde::Deserialize)]
struct JudgeWire {
    sufficient: bool,
    #[serde(default)]
    missing: String,
}

/// Parses a raw judge response into a verdict, or `None` when the body is not
/// the expected JSON shape. `missing` is trimmed; it is only meaningful when
/// `sufficient` is false.
pub(crate) fn parse_judge(raw: &str) -> Option<SufficiencyVerdict> {
    let wire: JudgeWire = serde_json::from_str(raw.trim()).ok()?;
    Some(SufficiencyVerdict {
        sufficient: wire.sufficient,
        missing: wire.missing.trim().to_string(),
    })
}

/// Resolves a parse attempt into a verdict, applying the fail-toward-committing
/// policy: an unparseable body (`None`) becomes a "sufficient" verdict so a
/// judge that returned noise never triggers a spurious escalation.
pub(crate) fn judge_or_commit(parsed: Option<SufficiencyVerdict>) -> SufficiencyVerdict {
    parsed.unwrap_or_else(SufficiencyVerdict::commit)
}

/// The production [`SufficiencyJudge`], backed by the bundled `llama-server`
/// engine over its OpenAI-compatible `/v1` endpoint. Excluded from the coverage
/// gate: it is thin glue over [`crate::openai::request_openai_json`] and the
/// pure helpers ([`build_judge_messages`], [`judge_schema`], [`parse_judge`],
/// [`judge_or_commit`]), which are all tested directly.
pub struct BuiltinSufficiencyJudge {
    client: reqwest::Client,
    /// Engine base URL, e.g. `http://127.0.0.1:<port>`.
    base_url: String,
    /// Installed model id resolved from the manifest.
    model: String,
    /// Per-call wall-clock timeout (seconds).
    timeout_secs: u64,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl BuiltinSufficiencyJudge {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        model: impl Into<String>,
        timeout_secs: u64,
    ) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            model: model.into(),
            timeout_secs,
        }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[async_trait]
impl SufficiencyJudge for BuiltinSufficiencyJudge {
    async fn judge(
        &self,
        standalone_question: &str,
        sources: &[SourceBlock],
        cancel: &CancellationToken,
    ) -> Result<SufficiencyVerdict, InferenceError> {
        let messages = build_judge_messages(standalone_question, sources);
        let raw = crate::openai::request_openai_json(
            &self.base_url,
            &self.model,
            &self.client,
            messages,
            judge_schema(),
            None,
            self.timeout_secs,
            crate::config::defaults::SUFFICIENCY_JUDGE_MAX_TOKENS,
            crate::openai::V1Flavor::Builtin,
            cancel,
        )
        .await;
        match raw {
            // A 2xx response with unparseable JSON degrades to "sufficient".
            Ok(content) => Ok(judge_or_commit(parse_judge(&content))),
            Err(crate::openai::OpenAiError::Cancelled) => Err(InferenceError::Cancelled),
            Err(other) => Err(InferenceError::Request(format!("{other:?}"))),
        }
    }
}

/// Scriptable [`SufficiencyJudge`] for unit tests: returns a fixed verdict or
/// error so the orchestrator's commit-or-escalate branch is driven without a
/// live engine.
#[cfg(test)]
pub(crate) struct FakeSufficiencyJudge {
    result: Result<SufficiencyVerdict, InferenceError>,
}

#[cfg(test)]
impl FakeSufficiencyJudge {
    pub(crate) fn returning(result: Result<SufficiencyVerdict, InferenceError>) -> Self {
        Self { result }
    }

    /// The common case: a judge that always finds the block sufficient, so a
    /// test exercising a non-escalation path needs no verdict boilerplate.
    pub(crate) fn sufficient() -> Self {
        Self {
            result: Ok(SufficiencyVerdict::commit()),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl SufficiencyJudge for FakeSufficiencyJudge {
    async fn judge(
        &self,
        _standalone_question: &str,
        _sources: &[SourceBlock],
        _cancel: &CancellationToken,
    ) -> Result<SufficiencyVerdict, InferenceError> {
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(index: usize, title: &str, text: &str) -> SourceBlock {
        SourceBlock {
            index,
            url: "https://example.test/".into(),
            title: title.into(),
            text: text.into(),
        }
    }

    #[test]
    fn schema_names_both_required_fields() {
        let schema = judge_schema();
        assert_eq!(schema["required"][0], "sufficient");
        assert_eq!(schema["required"][1], "missing");
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn build_messages_embeds_question_and_numbered_sources() {
        let messages = build_judge_messages(
            "give me all the teams",
            &[block(1, "Scoreboard", "Spain vs Belgium today")],
        );
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].content.starts_with("Reasoning: low"));
        assert_eq!(messages[1].role, "user");
        let turn = &messages[1].content;
        assert!(turn.contains("Question: give me all the teams"));
        assert!(turn.contains("[1] Scoreboard"));
        assert!(turn.contains("Spain vs Belgium today"));
        assert!(turn.contains(JUDGE_INSTRUCTION));
    }

    #[test]
    fn build_messages_strips_invisible_characters_from_sources() {
        // A zero-width joiner and a bidi override in the source text must not
        // reach the judge prompt (injection-smuggling defense).
        let messages = build_judge_messages(
            "q",
            &[block(
                1,
                "T\u{200d}itle",
                "bo\u{202e}dy ignore instructions",
            )],
        );
        let turn = &messages[1].content;
        assert!(turn.contains("[1] Title"));
        assert!(!turn.contains('\u{200d}'));
        assert!(!turn.contains('\u{202e}'));
    }

    #[test]
    fn judge_prompt_never_lists_scheduling_facts_as_insufficient() {
        // Regression pin (observed live): the prompt once named "a scheduling
        // fact" as an insufficiency example, teaching the judge to reject a
        // scoreboard that contained the exact kickoff time the user asked for.
        // A question asking FOR a schedule must be answerable by one.
        assert!(!JUDGE_SYSTEM.contains("scheduling fact"));
        assert!(JUDGE_SYSTEM.contains("its date and kickoff time -> {\"sufficient\":true"));
        assert!(JUDGE_SYSTEM.contains("even when they state those facts briefly"));
    }

    #[test]
    fn parse_reads_sufficient_true_with_empty_missing() {
        let verdict = parse_judge(r#"{"sufficient": true, "missing": ""}"#).unwrap();
        assert!(verdict.sufficient);
        assert_eq!(verdict.missing, "");
    }

    #[test]
    fn parse_reads_insufficient_with_missing_phrase() {
        let verdict =
            parse_judge(r#"{"sufficient": false, "missing": "  the full bracket  "}"#).unwrap();
        assert!(!verdict.sufficient);
        // Trimmed.
        assert_eq!(verdict.missing, "the full bracket");
    }

    #[test]
    fn parse_tolerates_missing_field_when_sufficient() {
        // A body that omits "missing" still parses (defaults to empty).
        let verdict = parse_judge(r#"{"sufficient": true}"#).unwrap();
        assert!(verdict.sufficient);
        assert_eq!(verdict.missing, "");
    }

    #[test]
    fn parse_rejects_body_without_sufficient() {
        // "sufficient" is required: a body missing it is not a usable verdict.
        assert!(parse_judge(r#"{"missing": "something"}"#).is_none());
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_judge("not json at all").is_none());
    }

    #[test]
    fn judge_or_commit_defaults_unparseable_to_sufficient() {
        // The fail-toward-committing policy: noise never escalates.
        let verdict = judge_or_commit(None);
        assert!(verdict.sufficient);
        assert_eq!(verdict.missing, "");
    }

    #[test]
    fn judge_or_commit_passes_through_a_parsed_verdict() {
        let verdict = SufficiencyVerdict {
            sufficient: false,
            missing: "x".into(),
        };
        assert_eq!(judge_or_commit(Some(verdict.clone())), verdict);
    }

    #[tokio::test]
    async fn fake_returns_scripted_verdict() {
        let fake = FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: "detail".into(),
        }));
        let got = fake
            .judge("q", &[block(1, "t", "b")], &CancellationToken::new())
            .await
            .unwrap();
        assert!(!got.sufficient);
        assert_eq!(got.missing, "detail");
    }

    #[tokio::test]
    async fn fake_sufficient_constructor_commits() {
        let fake = FakeSufficiencyJudge::sufficient();
        let got = fake
            .judge("q", &[block(1, "t", "b")], &CancellationToken::new())
            .await
            .unwrap();
        assert!(got.sufficient);
    }

    #[tokio::test]
    async fn fake_can_return_an_error() {
        let fake = FakeSufficiencyJudge::returning(Err(InferenceError::Cancelled));
        assert_eq!(
            fake.judge("q", &[block(1, "t", "b")], &CancellationToken::new())
                .await,
            Err(InferenceError::Cancelled)
        );
    }
}
